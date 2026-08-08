# Filesystem Read Tools + Path Allowlist (Agent Step 3) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give the Mimir agent bounded, allowlisted read access to registered project folders (`read_file`, `list_project_files`), managed from a new Settings page, with every file access — including `scan_project` — flowing through one `resolve_safe_path` security gate.

**Architecture:** A global `project_roots` registry (migration 054) + a `project_roots.rs` module whose `resolve_safe_path` canonicalizes a requested path (resolving symlinks), rejects secret-adjacent paths, and requires it to sit under a registered root. Two new agent tools are gated into `tool_schemas` only when a root is registered; the agent's `scan_project` is retro-gated; a Settings page does root CRUD.

**Tech Stack:** Rust (Tauri 2, sqlx), React 19 + TypeScript + Tailwind, Postgres.

## Global Constraints

- Rust: `$N` placeholders; non-macro `sqlx::query()`; commands return `Result<T,String>` with `database: State<'_, Database>` last; structs `#[serde(rename_all="camelCase")]`.
- **Security (non-negotiable):** the allowlist check uses `std::path::Path::starts_with` (component-wise), NEVER a string prefix. Canonicalize BOTH requested path and roots before comparing. On Windows `canonicalize` yields a `\\?\C:\…` prefix on both sides — consistent, so comparison holds; the stored root path is the canonical one.
- Truncate file content with `crate::text_util::truncate_chars` (never byte-slice).
- Migrations append-only; highest is 053 → this adds **054**.
- No new crates or npm deps. (Tests use `std::env::temp_dir()` + `uuid` — already a dep — not `tempfile`.)
- Work stays on branch `fix-compilation-errors`; commit locally, do NOT push.
- **Build/test in the isolated target dir.** Before any cargo command:
  `export CARGO_TARGET_DIR="C:/Users/dhruv/AppData/Local/Temp/claude/C--Users-dhruv-projects-Yggdrasil/497ed12f-520a-4774-9c54-620ab075f017/scratchpad/scan-target"`
- Frontend has no unit-test harness; the Settings page is verified in the runtime checklist (Task 4).

## File Structure

| File | Change | Task |
|---|---|---|
| `src-tauri/migrations/054_project_roots.sql` | Create the registry table | 1 |
| `src-tauri/src/project_roots.rs` | Create — `resolve_safe_path`, `is_denied`, CRUD, 3 commands | 1 |
| `src-tauri/src/main.rs` | `mod project_roots;` + register 3 commands | 1 |
| `src-tauri/src/mimir_agent.rs` | `tool_schemas` arity; `ToolCtx.project_roots`; `read_file`/`list_project_files` arms; `scan_project` retro-gate; tests | 2 |
| `src-tauri/src/mimir_retrieval.rs` | Load roots → `ToolCtx.project_roots` + system-prompt line | 2 |
| `src/pages/SettingsPage.tsx` | Create — Project Folders CRUD | 3 |
| `src/App.tsx` | Route `/settings` | 3 |
| `src/layouts/MainLayout.tsx` | Nav link | 3 |

---

### Task 1: Registry + the security gate (migration + module + commands)

**Files:**
- Create: `src-tauri/migrations/054_project_roots.sql`, `src-tauri/src/project_roots.rs`
- Modify: `src-tauri/src/main.rs` (`mod project_roots;` near :37; add 3 commands to the `invoke_handler` macro)

**Interfaces:**
- Produces: `ProjectRoot { id, label, path }` (camelCase); `resolve_safe_path(requested, &[ProjectRoot]) -> Result<PathBuf,String>`; `is_denied(&Path) -> bool`; `get_project_roots(pool) -> Vec<ProjectRoot>`; `add_project_root(pool, label, path) -> Result<ProjectRoot,String>`; `remove_project_root(pool, id) -> Result<(),String>`; commands `get_project_roots_cmd`/`add_project_root_cmd`/`remove_project_root_cmd`.

- [ ] **Step 1: Migration 054**

`src-tauri/migrations/054_project_roots.sql`:
```sql
CREATE TABLE IF NOT EXISTS project_roots (
    id         TEXT PRIMARY KEY,
    label      TEXT NOT NULL,
    path       TEXT NOT NULL UNIQUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

- [ ] **Step 2: Write the failing tests for `resolve_safe_path`**

Create `src-tauri/src/project_roots.rs` with the struct, empty stubs (so tests compile), and tests:
```rust
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

pub(crate) fn is_denied(_canon: &Path) -> bool { false } // stub — Step 4

pub(crate) fn resolve_safe_path(_requested: &str, _roots: &[ProjectRoot]) -> Result<PathBuf, String> {
    Err("unimplemented".to_string()) // stub — Step 4
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
```
Add `mod project_roots;` to `main.rs` (near the other `mod` lines, ~:37) so the tests compile into the binary.

- [ ] **Step 3: Run the tests — verify they FAIL**

Run: `cargo test --manifest-path src-tauri/Cargo.toml project_roots 2>&1 | tail -20`
Expected: `allows_file_inside_registered_root` FAILS (stub returns Err); the reject-tests may pass against the stub (Err) — that's fine, the allow-test failing proves the stub is wrong. (You'll re-run after Step 4 and all must pass, including the allow case.)

- [ ] **Step 4: Implement `is_denied` + `resolve_safe_path`**

Replace the two stubs:
```rust
const DENY_COMPONENTS: &[&str] = &[".git", ".ssh", ".aws", ".gnupg"];

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
        if name.starts_with("id_") { return true; }
        if name.starts_with("credentials") { return true; }
        if name == ".git-credentials" || name == ".npmrc" { return true; }
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
```

- [ ] **Step 5: Run the tests — verify they PASS**

Run: `cargo test --manifest-path src-tauri/Cargo.toml project_roots 2>&1 | tail -20`
Expected: all 6 tests pass.

- [ ] **Step 6: Add CRUD + Tauri commands**

Append to `project_roots.rs`:
```rust
pub(crate) async fn get_project_roots(pool: &PgPool) -> Vec<ProjectRoot> {
    let rows = sqlx::query("SELECT id, label, path FROM project_roots ORDER BY created_at ASC")
        .fetch_all(pool)
        .await
        .unwrap_or_default();
    rows.iter()
        .map(|r| ProjectRoot {
            id: r.try_get("id").unwrap_or_default(),
            label: r.try_get("label").unwrap_or_default(),
            path: r.try_get("path").unwrap_or_default(),
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
    let canon_str = canon.to_string_lossy().to_string();
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
```

- [ ] **Step 7: Register the commands in `main.rs`**

Add the three commands to the `tauri::generate_handler![ … ]` list in `main.rs` (alongside the other commands):
```rust
            project_roots::get_project_roots_cmd,
            project_roots::add_project_root_cmd,
            project_roots::remove_project_root_cmd,
```

- [ ] **Step 8: Build + test**

Run: `cargo test --manifest-path src-tauri/Cargo.toml 2>&1 | tail -20`
Expected: build clean; all tests pass (the 6 new + existing).

- [ ] **Step 9: Commit**

```bash
git add src-tauri/migrations/054_project_roots.sql src-tauri/src/project_roots.rs src-tauri/src/main.rs
git commit -m "feat(fs): project_roots registry + resolve_safe_path allowlist gate"
```

---

### Task 2: Agent integration (tools + gating + retro-gate)

**Files:**
- Modify: `src-tauri/src/mimir_agent.rs` (`tool_schemas` :38 + :143 + call site :721 + tests :837-849; `ToolCtx` :325-338; `execute_tool` arms — add `read_file`/`list_project_files`, gate `scan_project`)
- Modify: `src-tauri/src/mimir_retrieval.rs` (load roots ~:1152; system-prompt line after :1162; `ToolCtx` build :1182-1195)

**Interfaces:**
- Consumes: `crate::project_roots::{ProjectRoot, resolve_safe_path, get_project_roots}` (Task 1).
- Produces: `ToolCtx.project_roots: Vec<crate::project_roots::ProjectRoot>`; `tool_schemas(hound_available: bool, fs_tools_available: bool)`.

- [ ] **Step 1: `tool_schemas` gains `fs_tools_available`**

Change the signature (mimir_agent.rs:38) to `pub(crate) fn tool_schemas(hound_available: bool, fs_tools_available: bool) -> Vec<serde_json::Value>`. Before the final `tools` return, add:
```rust
    if fs_tools_available {
        tools.push(json!({
            "type": "function",
            "function": {
                "name": "read_file",
                "description": "Read a text file from one of the user's registered project folders. Use an absolute path under a registered folder. Returns file content (capped).",
                "parameters": { "type": "object", "properties": {
                    "path": { "type": "string", "description": "Absolute path to the file, under a registered project folder." }
                }, "required": ["path"] }
            }
        }));
        tools.push(json!({
            "type": "function",
            "function": {
                "name": "list_project_files",
                "description": "List the entries (files + subfolders) directly inside a folder within a registered project folder. Use to explore a project's structure.",
                "parameters": { "type": "object", "properties": {
                    "path": { "type": "string", "description": "Absolute path to the folder, under a registered project folder." }
                }, "required": ["path"] }
            }
        }));
    }
```
Update the call site (:721): `let tools = tool_schemas(ctx.hound_base_url.is_some(), !ctx.project_roots.is_empty());`

- [ ] **Step 2: Update the `tool_schemas` unit test**

The existing test (mimir_agent.rs:837-849) calls `tool_schemas(false)` / `tool_schemas(true)`. Change both to the 2-arg form and add FS-tool assertions:
```rust
    #[test]
    fn tool_schemas_registers_web_and_fs_tools_only_when_available() {
        let base: Vec<String> = tool_schemas(false, false)
            .iter().map(|t| t["function"]["name"].as_str().unwrap().to_string()).collect();
        assert!(!base.contains(&"smart_search".to_string()));
        assert!(!base.contains(&"read_file".to_string()));

        let with_web: Vec<String> = tool_schemas(true, false)
            .iter().map(|t| t["function"]["name"].as_str().unwrap().to_string()).collect();
        assert!(with_web.contains(&"smart_search".to_string()));
        assert!(!with_web.contains(&"read_file".to_string()));

        let with_fs: Vec<String> = tool_schemas(false, true)
            .iter().map(|t| t["function"]["name"].as_str().unwrap().to_string()).collect();
        assert!(with_fs.contains(&"read_file".to_string()));
        assert!(with_fs.contains(&"list_project_files".to_string()));
        assert!(!with_fs.contains(&"smart_search".to_string()));
    }
```
(Replace the old `tool_schemas_registers_web_tools_only_when_available` test; keep any index-based assertions only if still valid — prefer the `contains` form above.)

- [ ] **Step 3: `ToolCtx` gains `project_roots`**

Add to `ToolCtx` (after `queue`):
```rust
    pub project_roots: Vec<crate::project_roots::ProjectRoot>,
```

- [ ] **Step 4: `read_file` + `list_project_files` execute arms**

In `execute_tool`, add two arms (before the `other =>` catch-all):
```rust
        "read_file" => {
            let path = args["path"].as_str().ok_or("read_file requires a 'path'")?;
            let safe = crate::project_roots::resolve_safe_path(path, &ctx.project_roots)?;
            let content = std::fs::read_to_string(&safe)
                .map_err(|e| format!("could not read file: {}", e))?;
            let capped = crate::text_util::truncate_chars(&content, 100_000);
            Ok(format!("{}\n\n{}", safe.display(), capped))
        }
        "list_project_files" => {
            let path = args["path"].as_str().ok_or("list_project_files requires a 'path'")?;
            let safe = crate::project_roots::resolve_safe_path(path, &ctx.project_roots)?;
            let skip = ["node_modules", "target", ".venv", ".git", "dist", "build"];
            let mut out = String::new();
            let mut count = 0u32;
            match std::fs::read_dir(&safe) {
                Ok(entries) => {
                    for e in entries.flatten() {
                        if count >= 200 { out.push_str("… (more entries omitted)\n"); break; }
                        let name = e.file_name().to_string_lossy().to_string();
                        if skip.contains(&name.as_str()) { continue; }
                        let meta = e.metadata().ok();
                        if meta.as_ref().map(|m| m.is_dir()).unwrap_or(false) {
                            out.push_str(&format!("{}/\n", name));
                        } else {
                            let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
                            out.push_str(&format!("{} ({} bytes)\n", name, size));
                        }
                        count += 1;
                    }
                }
                Err(e) => return Ok(format!("could not list directory: {}", e)),
            }
            if out.is_empty() { out.push_str("(empty)"); }
            Ok(format!("{}\n{}", safe.display(), out))
        }
```

- [ ] **Step 5: Retro-gate `scan_project`**

Replace the `"scan_project"` arm body with a version that resolves the path first (keep the no-tree / no-queue guards):
```rust
        "scan_project" => {
            let path = args["path"].as_str().ok_or("scan_project requires a 'path' argument")?;
            let Some(tree_id) = ctx.tree_id.as_deref() else {
                return Ok("No active tree to scan into — open a tree first.".to_string());
            };
            let Some(queue) = ctx.queue else {
                return Ok("Background scanning is unavailable right now.".to_string());
            };
            let safe = match crate::project_roots::resolve_safe_path(path, &ctx.project_roots) {
                Ok(p) => p.to_string_lossy().to_string(),
                Err(e) => return Ok(format!(
                    "Can't scan that path: {}. Add the folder in Settings → Project Folders first.", e
                )),
            };
            queue.send(crate::orchestrator::OrchestratorJob::ScanProject {
                path: safe, tree_id: tree_id.to_string(),
            }).await?;
            Ok(format!("Scan of '{}' started in the background — I'll surface the results when it finishes.", path))
        }
```

- [ ] **Step 6: Load roots + system-prompt line + `ToolCtx.project_roots` (mimir_retrieval.rs)**

After the existing tool-guidance `push_str` (ends ~:1162), and before `// 6b`, add:
```rust
    let project_roots = crate::project_roots::get_project_roots(&database.pool).await;
    if !project_roots.is_empty() {
        let list = project_roots.iter()
            .map(|r| format!("{} → {}", r.label, r.path))
            .collect::<Vec<_>>()
            .join("; ");
        agent_system_prompt.push_str(&format!(
            "\n\nYou may read files ONLY in these registered project folders (use absolute paths under them): {}. \
             read_file reads a file; list_project_files lists a folder's entries. \
             Refuse to read anything outside these folders.",
            list
        ));
    }
```
Then in the `ToolCtx { … }` build (~:1194, after `queue: Some(&queue),`), add:
```rust
                    project_roots: project_roots.clone(),
```
(Use `.clone()` — `project_roots` was borrowed by the system-prompt block above and may be used again if the agent path isn't taken.)

- [ ] **Step 7: Build + test**

Run: `cargo test --manifest-path src-tauri/Cargo.toml 2>&1 | tail -20`
Expected: build clean; all tests pass (the updated `tool_schemas` test + existing).

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/mimir_agent.rs src-tauri/src/mimir_retrieval.rs
git commit -m "feat(fs): read_file/list_project_files agent tools + scan_project retro-gate"
```

---

### Task 3: Settings page

**Files:**
- Create: `src/pages/SettingsPage.tsx`
- Modify: `src/App.tsx` (import + `<Route path="/settings" element={<SettingsPage/>}/>`), `src/layouts/MainLayout.tsx` (import `Settings` from lucide-react; add a `SidebarLink`)

**Interfaces:**
- Consumes: `get_project_roots_cmd` → `ProjectRoot[]` (`{ id, label, path }`); `add_project_root_cmd({ label, path })`; `remove_project_root_cmd({ id })`.

- [ ] **Step 1: Create `SettingsPage.tsx`**

```tsx
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { FolderPlus, Trash2 } from "lucide-react";

interface ProjectRoot { id: string; label: string; path: string; }

export default function SettingsPage() {
  const [roots, setRoots] = useState<ProjectRoot[]>([]);
  const [label, setLabel] = useState("");
  const [path, setPath] = useState("");
  const [error, setError] = useState<string | null>(null);

  const load = () => invoke<ProjectRoot[]>("get_project_roots_cmd").then(setRoots).catch(() => {});
  useEffect(() => { load(); }, []);

  const add = async () => {
    setError(null);
    try {
      await invoke<ProjectRoot>("add_project_root_cmd", { label, path });
      setLabel(""); setPath(""); load();
    } catch (e) { setError(String(e)); }
  };
  const remove = async (id: string) => { await invoke("remove_project_root_cmd", { id }); load(); };

  return (
    <div className="p-8 max-w-2xl">
      <h1 className="text-2xl font-semibold mb-6">Settings</h1>
      <section>
        <h2 className="text-lg font-medium mb-1">Project Folders</h2>
        <p className="text-sm opacity-60 mb-4">
          Folders the Mimir agent may read from (read-only). Only these are accessible; secret files (.env, keys, .ssh, .git) are always blocked.
        </p>
        <div className="space-y-2 mb-4">
          {roots.length === 0 && <p className="text-sm opacity-50">No folders registered yet.</p>}
          {roots.map((r) => (
            <div key={r.id} className="flex items-center gap-3 rounded-md border border-white/10 bg-white/5 px-3 py-2">
              <div className="min-w-0 flex-1">
                <div className="text-sm font-medium">{r.label}</div>
                <div className="text-xs opacity-60 truncate">{r.path}</div>
              </div>
              <button onClick={() => remove(r.id)} className="opacity-60 hover:opacity-100" title="Remove">
                <Trash2 className="w-4 h-4" />
              </button>
            </div>
          ))}
        </div>
        <div className="flex gap-2 items-start">
          <input value={label} onChange={(e) => setLabel(e.target.value)} placeholder="Label (optional)"
            className="rounded-md bg-white/5 border border-white/10 px-3 py-2 text-sm w-40" />
          <input value={path} onChange={(e) => setPath(e.target.value)} placeholder="C:\Users\you\projects\repo"
            className="rounded-md bg-white/5 border border-white/10 px-3 py-2 text-sm flex-1" />
          <button onClick={add} disabled={!path.trim()}
            className="rounded-md bg-white/10 hover:bg-white/20 px-3 py-2 text-sm flex items-center gap-1 disabled:opacity-40">
            <FolderPlus className="w-4 h-4" /> Add
          </button>
        </div>
        {error && <p className="mt-2 text-sm text-red-400">{error}</p>}
      </section>
    </div>
  );
}
```

- [ ] **Step 2: Route + nav**

In `src/App.tsx`: add `import SettingsPage from "./pages/SettingsPage";` (with the other page imports) and `<Route path="/settings" element={<SettingsPage />} />` (with the other routes).
In `src/layouts/MainLayout.tsx`: add `Settings` to the lucide-react import, and a nav link after "Skills":
```tsx
          <SidebarLink to="/settings" icon={<Settings className="w-4 h-4" />} label="Settings" />
```

- [ ] **Step 3: Verify the frontend compiles**

Run: `npx tsc --noEmit` (repo root). Expected: exit 0.

- [ ] **Step 4: Commit**

```bash
git add src/pages/SettingsPage.tsx src/App.tsx src/layouts/MainLayout.tsx
git commit -m "feat(fs): Settings page for managing project folders"
```

---

### Task 4: Live verification (runtime checklist)

**Files:** none.

- [ ] **Step 1:** `bash dev.sh` (applies migration 054). Open Settings → Project Folders; add `C:\Users\dhruv\projects\Yggdrasil` (label "Yggdrasil"). Confirm it lists (path shown may carry a `\\?\` prefix — acceptable).
- [ ] **Step 2:** In a tree's Mimir chat: "list the files in the Yggdrasil src-tauri/src folder" → `list_project_files` returns entries. "read src-tauri/src/main.rs" → `read_file` returns content.
- [ ] **Step 3:** "read my .env file" (or any path outside the root) → the agent refuses (denylist / outside-roots), no crash.
- [ ] **Step 4:** "scan my project at C:\Users\dhruv\projects\spacetime" (unregistered) → refused with "Add the folder in Settings first". Then add a valid folder and scan it → runs (Step 2's background job).
- [ ] **Step 5:** Remove the folder in Settings → ask the agent to read a file → it no longer offers/does FS reads (tools gated off when no roots).

---

## Self-Review

**Spec coverage:**
- Migration 054 → Task 1 Step 1. ✓
- `project_roots.rs` `resolve_safe_path` (canonicalize→denylist→allowlist, `Path::starts_with`) + CRUD + commands → Task 1 Steps 2-7. ✓
- `tool_schemas` `fs_tools_available` + `read_file`/`list_project_files` → Task 2 Steps 1,4. ✓
- `ToolCtx.project_roots` + load + system-prompt line → Task 2 Steps 3,6. ✓
- `scan_project` retro-gate → Task 2 Step 5. ✓
- Settings page + route + nav → Task 3. ✓
- Behavior change (agent limited to registered roots) → Task 2 Step 5 + Task 4 Step 4. ✓

**Placeholder scan:** No TBD/TODO; real code throughout. Task 1 Step 7 says "add to the `generate_handler!` list" without reproducing the whole list — that's a locate-and-insert, not a placeholder.

**Type consistency:** `ProjectRoot { id, label, path }` (camelCase) identical in Rust (Task 1) and the TS interface (Task 3). `resolve_safe_path(&str, &[ProjectRoot]) -> Result<PathBuf,String>` used identically in read_file/list/scan arms. `tool_schemas(bool, bool)` — call site + test + definition all 2-arg. Command names `get/add/remove_project_root_cmd` match between Rust registration and the frontend `invoke` calls.

**Security self-check:** allowlist uses `Path::starts_with` (component-wise) per Global Constraints; canonicalize precedes the allowlist check (symlink defense); denylist runs on the canonical path; `add_project_root` also denylists + stores canonical. TDD covers allow/outside/.env/.ssh/nonexistent/empty-roots.

**Note on TDD:** the security core (`resolve_safe_path`/`is_denied`) is TDD'd (Task 1 Steps 2-5). DB CRUD, agent arms, and the frontend are verified by build/test + the Task 4 runtime checklist (no DB/Tauri runtime here; no frontend harness).

## Explicitly out of scope
Recursive/glob listing; a `grep` tool; binary-file handling beyond lossy read; gating the extension's scan path; per-tree scoping; writing/editing files.
