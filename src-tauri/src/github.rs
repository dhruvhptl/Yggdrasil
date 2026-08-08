// src-tauri/src/github.rs

use base64::{Engine as _, engine::general_purpose};

pub(crate) fn parse_github_url(url: &str) -> Option<(String, String)> {
    let url = url.trim().trim_end_matches('/');

    let path = url
        .strip_prefix("https://github.com/")
        .or_else(|| url.strip_prefix("http://github.com/"))
        .or_else(|| url.strip_prefix("github.com/"))?;

    let parts: Vec<&str> = path.splitn(3, '/').collect();
    if parts.len() < 2 {
        return None;
    }

    let owner = parts[0].to_string();
    let repo = parts[1].trim_end_matches(".git").to_string();

    if owner.is_empty() || repo.is_empty() {
        return None;
    }

    Some((owner, repo))
}

// ─── GitHub file helper ──────────────────────────────────────────────────────

pub(crate) async fn fetch_github_file(
    gh: &reqwest::Client,
    url: &str,
    max_chars: usize,
) -> Option<String> {
    let r = gh.get(url).send().await.ok()?;
    if !r.status().is_success() {
        return None;
    }
    let data: serde_json::Value = r.json().await.ok()?;
    let encoded = data["content"].as_str()?.replace('\n', "");
    if encoded.is_empty() {
        return None;
    }
    let decoded = general_purpose::STANDARD.decode(&encoded).ok()?;
    let content = String::from_utf8_lossy(&decoded).to_string();
    if content.is_empty() {
        return None;
    }
    Some(if content.len() > max_chars {
        let mut boundary = max_chars;
        while !content.is_char_boundary(boundary) {
            boundary -= 1;
        }
        format!("{}…", &content[..boundary])
    } else {
        content
    })
}

// ─── Tiered repo analysis helpers ─────────────────────────────────────────────

/// Fetch the full file tree from a GitHub repo using the Trees API (single API call).
/// Filters out noise directories, lock files, and files over 100KB.
pub(crate) async fn fetch_repo_tree(gh: &reqwest::Client, base: &str) -> Result<Vec<String>, String> {
    let url = format!("{}/git/trees/HEAD?recursive=1", base);
    let resp = gh
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("GitHub Trees API error: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("GitHub Trees API returned {}", resp.status()));
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse tree response: {}", e))?;

    let skip_prefixes = ["node_modules/", "target/", ".git/", "dist/", "build/", ".sqlx/", "__pycache__/", ".venv/", "sidecar/"];

    let mut paths: Vec<String> = body["tree"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|entry| {
                    if entry["type"].as_str()? != "blob" {
                        return None;
                    }
                    let path = entry["path"].as_str()?;
                    // Skip noise directories
                    if skip_prefixes.iter().any(|p| path.starts_with(p) || path.contains(&format!("/{}", p))) {
                        return None;
                    }
                    // Skip lock files
                    if path.ends_with(".lock") || path.ends_with("-lock.json") || path.ends_with("-lock.yaml") {
                        return None;
                    }
                    // Skip files over 100KB
                    if let Some(size) = entry["size"].as_u64() {
                        if size > 100_000 {
                            return None;
                        }
                    }
                    Some(path.to_string())
                })
                .collect()
        })
        .unwrap_or_default();

    paths.sort();
    Ok(paths)
}

/// Fetch a list of files from GitHub, respecting per-file and total char budgets.
pub(crate) async fn fetch_relevant_files(
    gh: &reqwest::Client,
    base: &str,
    file_paths: &[String],
    max_per_file: usize,
    total_budget: usize,
) -> Vec<(String, String)> {
    let mut results = Vec::new();
    let mut total_chars = 0usize;

    for path in file_paths {
        if total_chars >= total_budget {
            break;
        }
        let url = format!("{}/contents/{}", base, path);
        if let Some(content) = fetch_github_file(gh, &url, max_per_file).await {
            total_chars += content.len();
            results.push((path.clone(), content));
            println!("  📄 Fetched: {} ({} chars)", path, results.last().unwrap().1.len());
        }
    }
    results
}

/// Fallback file selection when Phase 1 doesn't return relevant_files.
/// Balanced across project layers: backend, sidecar, components, pages.
pub(crate) fn select_fallback_files(all_paths: &[String], max: usize) -> Vec<String> {
    // Layers with their path prefixes and allowed extensions
    let layers: &[(&str, &[&str])] = &[
        ("src-tauri/src/", &[".rs"]),
        ("src/components/", &[".tsx", ".ts"]),
        ("src/pages/", &[".tsx", ".ts"]),
    ];

    let per_layer = 2usize;
    let mut selected: Vec<String> = Vec::new();
    let mut layer_counts: Vec<(&str, usize)> = Vec::new();

    for (prefix, exts) in layers {
        if selected.len() >= max {
            break;
        }
        // Collect matching files, skip test/spec
        let mut candidates: Vec<&String> = all_paths
            .iter()
            .filter(|p| {
                let lower = p.to_lowercase();
                lower.starts_with(prefix)
                    && exts.iter().any(|ext| lower.ends_with(ext))
                    && !lower.contains("test")
                    && !lower.contains("spec")
                    && !selected.contains(p)
            })
            .collect();
        // Sort by path length descending (longer paths = deeper/more specific files)
        candidates.sort_by(|a, b| b.len().cmp(&a.len()));
        let take = per_layer.min(max - selected.len());
        let picked: Vec<String> = candidates.into_iter().take(take).cloned().collect();
        let count = picked.len();
        selected.extend(picked);
        layer_counts.push((prefix, count));
    }

    // Fill remaining budget with any source files not yet included
    if selected.len() < max {
        let general_exts = [".rs", ".ts", ".tsx", ".py", ".go", ".java", ".js", ".jsx"];
        for path in all_paths {
            if selected.len() >= max {
                break;
            }
            let lower = path.to_lowercase();
            if general_exts.iter().any(|ext| lower.ends_with(ext))
                && !lower.starts_with("sidecar/")
                && !lower.contains("test")
                && !lower.contains("spec")
                && !selected.contains(path)
            {
                selected.push(path.clone());
            }
        }
    }

    // Log layer coverage
    let coverage: Vec<String> = layer_counts
        .iter()
        .map(|(prefix, count)| {
            let label = prefix.split('/').next().unwrap_or(prefix);
            format!("{}={}", label, count)
        })
        .collect();
    println!("  Fallback coverage: {}", coverage.join(", "));

    selected
}
