use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProjectRoot {
    pub id: String,
    pub label: String,
    pub path: String,
}

const DENY_COMPONENTS: &[&str] = &[
    ".git", ".ssh", ".aws", ".gnupg", ".docker", ".kube", ".azure", ".gcloud",
];

/// True if the canonical path is secret-adjacent (a blocked dir component or a
/// secret-looking file name). Case-insensitive.
pub(crate) fn is_denied(canon: &Path) -> bool {
    for comp in canon.components() {
        if let std::path::Component::Normal(os) = comp {
            let s = os.to_string_lossy().to_lowercase();
            if DENY_COMPONENTS.contains(&s.as_str()) {
                return true;
            }
        }
    }
    if let Some(name) = canon.file_name().map(|n| n.to_string_lossy().to_lowercase()) {
        if name == ".env" || name.starts_with(".env.") { return true; }
        if name.ends_with(".pem") || name.ends_with(".key") { return true; }
        if name.ends_with(".p12") || name.ends_with(".pfx") || name.ends_with(".jks") || name.ends_with(".keystore") { return true; }
        if name.starts_with("id_") { return true; }
        if name.starts_with("credentials") { return true; }
        if name == ".netrc" || name == ".pgpass" { return true; }
        if name == ".git-credentials" || name == ".npmrc" || name == ".dockercfg" || name == ".pypirc" || name == ".htpasswd" { return true; }
    }
    false
}

/// Canonicalize (resolving symlinks) → denylist → allowlist. Any failure → Err;
/// never returns a path outside a registered root.
pub(crate) fn resolve_safe_path(requested: &str, roots: &[ProjectRoot]) -> Result<PathBuf, String> {
    let canon = std::fs::canonicalize(requested)
        .map_err(|_| format!("path not found or inaccessible: {}", requested))?;
    if is_denied(&canon) {
        return Err("that path is blocked (secret-adjacent)".to_string());
    }
    for root in roots {
        if let Ok(root_canon) = std::fs::canonicalize(&root.path) {
            if canon.starts_with(&root_canon) {
                return Ok(canon);
            }
        }
    }
    Err("path is outside your registered project folders".to_string())
}

/// Strip Windows' verbatim `\\?\` prefix for display/storage. `\\?\C:\x` → `C:\x`,
/// `\\?\UNC\server\share` → `\\server\share`. Non-verbatim paths pass through.
/// std::fs::canonicalize emits verbatim paths on Windows; they're valid for FS
/// ops but ugly to show the user, so we normalize before storing/displaying.
pub(crate) fn strip_verbatim(p: &str) -> String {
    if let Some(rest) = p.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{}", rest)
    } else if let Some(rest) = p.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        p.to_string()
    }
}

pub(crate) async fn get_project_roots(pool: &PgPool) -> Vec<ProjectRoot> {
    let rows = sqlx::query("SELECT id, label, path FROM project_roots ORDER BY created_at ASC")
        .fetch_all(pool)
        .await
        .unwrap_or_default();
    rows.iter()
        .map(|r| ProjectRoot {
            id: r.try_get("id").unwrap_or_default(),
            label: r.try_get("label").unwrap_or_default(),
            // Normalize away any legacy \\?\ prefix stored before this cleanup.
            path: strip_verbatim(&r.try_get::<String, _>("path").unwrap_or_default()),
        })
        .collect()
}

pub(crate) async fn add_project_root(pool: &PgPool, label: &str, path: &str) -> Result<ProjectRoot, String> {
    let canon = std::fs::canonicalize(path).map_err(|_| format!("Folder not found: {}", path))?;
    if !canon.is_dir() {
        return Err("That path is not a folder.".to_string());
    }
    if is_denied(&canon) {
        return Err("That folder is blocked (secret-adjacent).".to_string());
    }
    // Store the human-readable path (no \\?\ prefix); canonicalize still gates
    // access at resolve time, so this is purely for clean storage/display.
    let canon_str = strip_verbatim(&canon.to_string_lossy());
    let label = if label.trim().is_empty() {
        canon.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "Project".to_string())
    } else {
        label.trim().to_string()
    };
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO project_roots (id, label, path) VALUES ($1, $2, $3) ON CONFLICT (path) DO NOTHING")
        .bind(&id).bind(&label).bind(&canon_str)
        .execute(pool).await.map_err(|e| e.to_string())?;
    let row = sqlx::query("SELECT id, label, path FROM project_roots WHERE path = $1")
        .bind(&canon_str)
        .fetch_one(pool).await.map_err(|e| e.to_string())?;
    Ok(ProjectRoot {
        id: row.try_get("id").unwrap_or_default(),
        label: row.try_get("label").unwrap_or_default(),
        path: row.try_get("path").unwrap_or_default(),
    })
}

pub(crate) async fn remove_project_root(pool: &PgPool, id: &str) -> Result<(), String> {
    sqlx::query("DELETE FROM project_roots WHERE id = $1")
        .bind(id)
        .execute(pool).await.map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn get_project_roots_cmd(database: tauri::State<'_, crate::database::Database>) -> Result<Vec<ProjectRoot>, String> {
    Ok(get_project_roots(&database.pool).await)
}

#[tauri::command]
pub async fn add_project_root_cmd(label: String, path: String, database: tauri::State<'_, crate::database::Database>) -> Result<ProjectRoot, String> {
    add_project_root(&database.pool, &label, &path).await
}

#[tauri::command]
pub async fn remove_project_root_cmd(id: String, database: tauri::State<'_, crate::database::Database>) -> Result<(), String> {
    remove_project_root(&database.pool, &id).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp_root() -> PathBuf {
        let d = std::env::temp_dir().join(format!("ygg_roots_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&d).unwrap();
        fs::canonicalize(&d).unwrap()
    }
    fn root_of(p: &Path) -> ProjectRoot {
        ProjectRoot { id: "1".into(), label: "t".into(), path: p.to_string_lossy().into() }
    }

    #[test]
    fn strip_verbatim_normalizes_windows_prefixes() {
        assert_eq!(strip_verbatim(r"\\?\C:\Users\dhruv\projects\kelvin"), r"C:\Users\dhruv\projects\kelvin");
        assert_eq!(strip_verbatim(r"\\?\UNC\server\share\proj"), r"\\server\share\proj");
        // Non-verbatim paths pass through unchanged.
        assert_eq!(strip_verbatim(r"C:\Users\dhruv\projects\kelvin"), r"C:\Users\dhruv\projects\kelvin");
        assert_eq!(strip_verbatim("/home/user/proj"), "/home/user/proj");
    }

    #[test]
    fn allows_file_inside_registered_root() {
        let root = tmp_root();
        let f = root.join("main.rs");
        fs::write(&f, "fn main(){}").unwrap();
        let got = resolve_safe_path(f.to_str().unwrap(), &[root_of(&root)]).unwrap();
        assert_eq!(got, fs::canonicalize(&f).unwrap());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn rejects_path_outside_all_roots() {
        let root = tmp_root();
        let other = tmp_root();
        let f = other.join("x.txt");
        fs::write(&f, "x").unwrap();
        assert!(resolve_safe_path(f.to_str().unwrap(), &[root_of(&root)]).is_err());
        fs::remove_dir_all(&root).ok();
        fs::remove_dir_all(&other).ok();
    }

    #[test]
    fn rejects_env_file_inside_root() {
        let root = tmp_root();
        let f = root.join(".env");
        fs::write(&f, "SECRET=1").unwrap();
        assert!(resolve_safe_path(f.to_str().unwrap(), &[root_of(&root)]).is_err());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn rejects_ssh_component_and_key_inside_root() {
        let root = tmp_root();
        let ssh = root.join(".ssh");
        fs::create_dir_all(&ssh).unwrap();
        let f = ssh.join("id_rsa");
        fs::write(&f, "KEY").unwrap();
        assert!(resolve_safe_path(f.to_str().unwrap(), &[root_of(&root)]).is_err());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn rejects_expanded_secret_files_and_dirs() {
        let root = tmp_root();
        let roots = [root_of(&root)];
        for name in [".netrc", ".pgpass", "keystore.p12", "cert.pfx", "app.jks", ".dockercfg", ".pypirc", ".htpasswd"] {
            let f = root.join(name);
            fs::write(&f, "x").unwrap();
            assert!(resolve_safe_path(f.to_str().unwrap(), &roots).is_err(), "{} should be denied", name);
        }
        for dir in [".docker", ".kube", ".azure", ".gcloud"] {
            let d = root.join(dir);
            fs::create_dir_all(&d).unwrap();
            let f = d.join("config");
            fs::write(&f, "x").unwrap();
            assert!(resolve_safe_path(f.to_str().unwrap(), &roots).is_err(), "{}/config should be denied", dir);
        }
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn rejects_nonexistent_path() {
        let root = tmp_root();
        let missing = root.join("nope.txt");
        assert!(resolve_safe_path(missing.to_str().unwrap(), &[root_of(&root)]).is_err());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn empty_roots_rejects_everything() {
        let root = tmp_root();
        let f = root.join("a.txt");
        fs::write(&f, "x").unwrap();
        assert!(resolve_safe_path(f.to_str().unwrap(), &[]).is_err());
        fs::remove_dir_all(&root).ok();
    }
}
