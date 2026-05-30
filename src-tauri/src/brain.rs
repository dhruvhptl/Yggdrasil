// src-tauri/src/brain.rs
// Thin shell — logic lives in llm_client, github, prompt_builders, tree_persistence.

use tauri::State;
use crate::database::Database;
use crate::github::{parse_github_url, fetch_repo_tree, fetch_relevant_files, select_fallback_files, fetch_github_file};
use crate::prompt_builders::{
    extract_concept_graph, build_graph_context, build_repo_profile, build_prd_profile,
    build_outline_system_prompt, build_repo_outline_system_prompt,
    generate_tree_two_stage, fetch_paper_text,
};
use crate::tree_persistence::{save_tree_to_database, auto_match_tree_nodes, diff_and_carry_state, get_mastered_concepts};

// Re-export log_prompt_call so existing call sites (mimir.rs, skill_commands.rs, etc.)
// continue to work as `crate::brain::log_prompt_call`.
pub(crate) use crate::llm_client::log_prompt_call;

// ─── Commands ────────────────────────────────────────────────────────────────

async fn generate_skill_tree_inner(
    project_id: String,
    prd_text: String,
    mastered_concepts: &[String],
    app: &tauri::AppHandle,
    client: &reqwest::Client,
    database: &Database,
) -> Result<String, String> {
    generate_skill_tree_inner_with_context(
        project_id, prd_text, None, None, mastered_concepts, app, client, database,
    ).await
}

async fn generate_skill_tree_inner_with_context(
    project_id: String,
    prd_text: String,
    skill_context: Option<String>,
    skill_id: Option<String>,
    mastered_concepts: &[String],
    app: &tauri::AppHandle,
    client: &reqwest::Client,
    database: &Database,
) -> Result<String, String> {
    // If a skill-graph context block was provided, prepend it to the PRD so
    // every downstream stage (concept graph extraction, profile, outline,
    // expansion) sees the skill knowledge graph as primary context.
    let prd_text = match skill_context.as_deref() {
        Some(ctx) if !ctx.trim().is_empty() => {
            if prd_text.trim().is_empty() {
                format!("## Knowledge Graph Context\n\n{}\n", ctx.trim())
            } else {
                format!("## Knowledge Graph Context\n\n{}\n\n---\n\n{}", ctx.trim(), prd_text)
            }
        }
        _ => prd_text,
    };

    println!("\n=== Generate Skill Tree (PRD) ===");
    println!("Project ID: {}", project_id);
    println!("PRD length: {} chars (skill_context: {})\n",
        prd_text.len(),
        if skill_context.is_some() { "yes" } else { "no" });

    // Phase 1: Extract concept dependency graph
    // Keep sorted concepts alive for PRD profile + concept_slug population.
    let (graph_context, prd_profile_section, prd_sorted_concepts) = match extract_concept_graph(client, &prd_text, None, Some(&database.pool)).await {
        Ok(graph) => {
            println!(
                "🧠 Concept graph extracted: {} concepts",
                graph.concepts.len()
            );
            let sorted = crate::prompt_builders::topological_sort(graph.concepts);
            for (i, c) in sorted.iter().enumerate() {
                println!("  {}. {} — {}", i + 1, c.name, c.description);
            }
            let gc = build_graph_context(&sorted);

            // Fix 2: PRD profile — grounding step identical to repo path's build_repo_profile
            let profile = match build_prd_profile(client, &prd_text, &sorted, Some(&database.pool)).await {
                Ok(p) => {
                    let section = format!(
                        "## PROJECT PROFILE (authoritative — use this to anchor checkpoints)\n\n{}\n",
                        p
                    );
                    section
                }
                Err(e) => {
                    println!("⚠️  PRD profile failed (non-fatal): {}", e);
                    String::new()
                }
            };

            (Some(gc), profile, Some(sorted))
        }
        Err(e) => {
            println!("⚠️  Concept graph extraction failed: {} — falling back to single-phase generation", e);
            (None, String::new(), None)
        }
    };

    // Fix 3: Cap PRD text at 6000 chars for the outline input
    let prd_excerpt = if prd_text.len() > 6000 { &prd_text[..6000] } else { &prd_text };

    // Phase 2: Two-stage tree generation
    let outline_user_prompt = if let Some(ref gc) = graph_context {
        format!(
            "Project ID: {}\n\n{}\n\nPRD:\n{}\n\nGenerate the skeleton outline JSON:",
            project_id, gc, prd_excerpt
        )
    } else {
        format!(
            "Project ID: {}\n\nPRD:\n{}\n\nGenerate the skeleton outline JSON:",
            project_id, prd_excerpt
        )
    };

    // Fix 1 + Fix 2: Expansion context includes concept graph order AND profile.
    // This mirrors what the repo path passes into expansion (profile_section + repo info).
    let expansion_context = if !prd_profile_section.is_empty() {
        if let Some(ref gc) = graph_context {
            format!(
                "Project ID: {}\n\n{}\n\n{}\nPRD excerpt:\n{}",
                project_id,
                gc,
                prd_profile_section,
                if prd_text.len() > 1500 { &prd_text[..1500] } else { &prd_text }
            )
        } else {
            format!(
                "Project ID: {}\n\n{}\nPRD excerpt:\n{}",
                project_id,
                prd_profile_section,
                if prd_text.len() > 2000 { &prd_text[..2000] } else { &prd_text }
            )
        }
    } else {
        // Fallback: graph context + raw PRD if profile failed
        if let Some(ref gc) = graph_context {
            format!(
                "Project ID: {}\n\n{}\n\nPRD:\n{}",
                project_id,
                gc,
                if prd_text.len() > 2000 { &prd_text[..2000] } else { &prd_text }
            )
        } else {
            format!(
                "Project ID: {}\n\nProject description:\n{}",
                project_id,
                if prd_text.len() > 3000 { &prd_text[..3000] } else { &prd_text }
            )
        }
    };

    let tree_model = std::env::var("TREE_GEN_MODEL")
        .unwrap_or_else(|_| "google/gemini-2.5-flash".to_string());
    let tree_api_key = std::env::var("TREE_GEN_API_KEY")
        .unwrap_or_else(|_| std::env::var("OPENROUTER_API_KEY").unwrap_or_default());
    let tree_base_url = std::env::var("TREE_GEN_BASE_URL")
        .unwrap_or_else(|_| "https://openrouter.ai/api/v1/chat/completions".to_string());
    // Kimi K2 does not support response_format: json_object via OpenRouter
    let json_mode = !tree_model.contains("kimi");

    println!("🌲 Tree gen (PRD two-stage): model={} json_mode={}", tree_model, json_mode);

    let mut skill_tree = generate_tree_two_stage(
        client,
        &tree_model,
        &tree_base_url,
        &tree_api_key,
        &build_outline_system_prompt(mastered_concepts),
        &outline_user_prompt,
        &expansion_context,
        &project_id,
        json_mode,
        Some(&database.pool),
        mastered_concepts,
    ).await?;
    skill_tree.project_id = project_id;

    println!(
        "✅ Tree generated: {} phases, {} total skills",
        skill_tree.phases.len(),
        skill_tree.phases.iter().map(|p| p.skills.len()).sum::<usize>()
    );

    let (tree_id, leaf_node_ids) = save_tree_to_database(&skill_tree, database, prd_sorted_concepts.as_deref()).await?;
    println!("💾 Saved to database with tree_id: {}", tree_id);

    // Anchor skill-seeded trees in skill_trees (migration 043).
    if let Some(ref sid) = skill_id {
        if let Err(e) = sqlx::query(
            "INSERT INTO skill_trees (skill_id, tree_id) VALUES ($1, $2) ON CONFLICT DO NOTHING"
        )
        .bind(sid)
        .bind(&tree_id)
        .execute(&database.pool)
        .await {
            println!("⚠️  skill_trees insert failed (non-fatal): {}", e);
        } else {
            println!("🔗 Linked skill {} → tree {}", sid, tree_id);
        }
    }

    auto_match_tree_nodes(&database.pool, client, &leaf_node_ids).await;
    crate::orchestrator::on_tree_generated(&database.pool, app, &tree_id, &skill_tree.project_id).await;

    let mut response_json = serde_json::json!(skill_tree);
    response_json["tree_id"] = serde_json::json!(tree_id);

    Ok(serde_json::to_string_pretty(&response_json).unwrap())
}

#[tauri::command]
pub async fn generate_skill_tree(
    project_id: String,
    prd_text: String,
    skill_context: Option<String>,
    skill_id: Option<String>,
    app: tauri::AppHandle,
    client: State<'_, reqwest::Client>,
    database: State<'_, Database>,
) -> Result<String, String> {
    generate_skill_tree_inner_with_context(
        project_id, prd_text, skill_context, skill_id, &[], &app, &*client, &*database,
    ).await
}

async fn analyze_repo_inner(
    project_id: String,
    github_url: String,
    paper_url: Option<String>,
    paper_pdf: Option<String>,
    mastered_concepts: &[String],
    app: &tauri::AppHandle,
    client: &reqwest::Client,
    database: &Database,
) -> Result<String, String> {
    let (owner, repo) = parse_github_url(&github_url).ok_or_else(|| {
        "Invalid GitHub URL. Expected: https://github.com/owner/repo".to_string()
    })?;

    println!("\n=== Analyze Repo: {}/{} ===", owner, repo);

    // Optional reference paper — non-fatal if it fails
    let paper_text: Option<String> = match fetch_paper_text(
        client,
        paper_url.as_deref(),
        paper_pdf.as_deref(),
    ).await {
        Ok(t) => t,
        Err(e) => {
            println!("⚠️  Paper fetch failed (non-fatal): {}", e);
            None
        }
    };

    // Build GitHub HTTP client
    let mut header_map = reqwest::header::HeaderMap::new();
    header_map.insert(reqwest::header::USER_AGENT, "Yggdrasil".parse().unwrap());
    header_map.insert(
        reqwest::header::ACCEPT,
        "application/vnd.github.v3+json".parse().unwrap(),
    );
    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        if let Ok(val) = format!("Bearer {}", token).parse() {
            header_map.insert(reqwest::header::AUTHORIZATION, val);
        }
    }

    let gh = reqwest::Client::builder()
        .default_headers(header_map)
        .build()
        .map_err(|e| e.to_string())?;

    let base = format!("https://api.github.com/repos/{}/{}", owner, repo);

    // 1. Repo metadata
    let repo_info: serde_json::Value = gh
        .get(&base)
        .send()
        .await
        .map_err(|e| format!("GitHub API error: {}", e))?
        .json()
        .await
        .map_err(|e| format!("Failed to parse repo info: {}", e))?;

    if repo_info.get("message").and_then(|m| m.as_str()) == Some("Not Found") {
        return Err(
            "Repository not found or is private. Add GITHUB_TOKEN to .env for private repos."
                .to_string(),
        );
    }

    let repo_description = repo_info["description"].as_str().unwrap_or("").to_string();
    let primary_language = repo_info["language"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();
    let topics: Vec<String> = repo_info["topics"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|t| t.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    println!("  Language: {}, Topics: {:?}", primary_language, topics);

    // 2. Fetch full file tree (single API call)
    let all_paths = fetch_repo_tree(&gh, &base).await.unwrap_or_default();
    println!("  File tree: {} files", all_paths.len());

    // Root-level file names (for README + dep file detection)
    let root_files: Vec<String> = all_paths
        .iter()
        .filter(|p| !p.contains('/'))
        .cloned()
        .collect();
    println!("  Root files: {:?}", root_files);

    // 3. README
    let readme_text = {
        let readme_name = all_paths.iter().find(|f| {
            let lower = f.to_lowercase();
            lower == "readme.md"
                || lower == "readme.rst"
                || lower == "readme.txt"
                || lower == "readme"
        });
        if let Some(name) = readme_name {
            let url = format!("{}/contents/{}", base, name);
            fetch_github_file(&gh, &url, 8000).await.unwrap_or_default()
        } else {
            String::new()
        }
    };
    println!("  README: {} chars", readme_text.len());

    // 4. Dependency files (lightweight for Phase 1: 500 chars each)
    let dep_candidates = [
        "package.json",
        "Cargo.toml",
        "requirements.txt",
        "pyproject.toml",
        "go.mod",
        "pom.xml",
        "Gemfile",
        "composer.json",
    ];
    let mut dep_contents: Vec<(String, String)> = Vec::new();
    for dep_file in &dep_candidates {
        // Search all paths for any file ending with this name (not just root)
        let matches: Vec<&String> = all_paths
            .iter()
            .filter(|f| {
                f.as_str() == *dep_file
                    || f.ends_with(&format!("/{}", dep_file))
            })
            .collect();
        if matches.is_empty() {
            println!("  [dep] {} — not found", dep_file);
        } else {
            println!("  [dep] {} — {} match(es): {:?}", dep_file, matches.len(), matches);
        }
        for matched_path in matches {
            let url = format!("{}/contents/{}", base, matched_path);
            if let Some(content) = fetch_github_file(&gh, &url, 1500).await {
                dep_contents.push((matched_path.clone(), content));
                println!("  Fetched: {}", matched_path);
            }
        }
    }

    // 5. Build Phase 1 context (lightweight: README + deps + file path list)
    let mut phase1_context = format!("# Repository: {}/{}\n\n", owner, repo);
    if !repo_description.is_empty() {
        phase1_context.push_str(&format!("**Description:** {}\n", repo_description));
    }
    phase1_context.push_str(&format!("**Primary Language:** {}\n\n", primary_language));

    if !readme_text.is_empty() {
        phase1_context.push_str(&format!("## README\n\n{}\n\n", readme_text));
    }
    for (filename, content) in &dep_contents {
        phase1_context.push_str(&format!("## {}\n\n```\n{}\n```\n\n", filename, content));
    }
    // Include full file path list so LLM can pick relevant files
    phase1_context.push_str("## FILE PATHS\n\n");
    for path in &all_paths {
        phase1_context.push_str(&format!("- {}\n", path));
    }
    println!("  Phase 1 context: {} chars", phase1_context.len());

    // 6. Recent commit history
    let recent_commits: Vec<String> = match gh
        .get(format!("{}/commits?per_page=15", base))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => {
            let commits: Vec<serde_json::Value> = r.json().await.unwrap_or_default();
            commits
                .iter()
                .filter_map(|c| {
                    let sha = c["sha"].as_str()?.chars().take(7).collect::<String>();
                    let message = c["commit"]["message"].as_str().unwrap_or("");
                    let first_line = message.lines().next().unwrap_or("");
                    let msg = if first_line.len() > 80 {
                        let mut b = 80;
                        while !first_line.is_char_boundary(b) { b -= 1; }
                        &first_line[..b]
                    } else {
                        first_line
                    };
                    let author = c["commit"]["author"]["name"].as_str().unwrap_or("unknown");
                    let date = c["commit"]["author"]["date"].as_str().unwrap_or("");
                    let date_short = &date[..date.len().min(10)];
                    Some(format!("[{}] {} — {} ({})", sha, msg, author, date_short))
                })
                .collect()
        }
        _ => vec![],
    };
    println!("  Commits: {}", recent_commits.len());

    // 7. Open issues
    let open_issues: Vec<String> = match gh
        .get(format!(
            "{}/issues?state=open&per_page=10&sort=created&direction=desc",
            base
        ))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => {
            let issues: Vec<serde_json::Value> = r.json().await.unwrap_or_default();
            issues
                .iter()
                .filter_map(|i| {
                    if !i["pull_request"].is_null() {
                        return None;
                    }
                    let number = i["number"].as_i64()?;
                    let title = i["title"].as_str().unwrap_or("");
                    let labels: Vec<&str> = i["labels"]
                        .as_array()
                        .map(|l| l.iter().filter_map(|lb| lb["name"].as_str()).collect())
                        .unwrap_or_default();
                    if labels.is_empty() {
                        Some(format!("#{}: {}", number, title))
                    } else {
                        Some(format!("#{}: {} [{}]", number, title, labels.join(", ")))
                    }
                })
                .collect()
        }
        _ => vec![],
    };
    println!("  Open issues: {}", open_issues.len());

    // 8. Recently merged PRs
    let merged_prs: Vec<String> = match gh
        .get(format!(
            "{}/pulls?state=closed&per_page=5&sort=updated&direction=desc",
            base
        ))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => {
            let prs: Vec<serde_json::Value> = r.json().await.unwrap_or_default();
            prs.iter()
                .filter_map(|pr| {
                    if pr["merged_at"].is_null() {
                        return None;
                    }
                    let number = pr["number"].as_i64()?;
                    let title = pr["title"].as_str().unwrap_or("");
                    let merged_at = pr["merged_at"].as_str().unwrap_or("");
                    let date_short = &merged_at[..merged_at.len().min(10)];
                    Some(format!("#{}: {} (merged {})", number, title, date_short))
                })
                .collect()
        }
        _ => vec![],
    };
    println!("  Merged PRs: {}", merged_prs.len());

    // 9. Phase 1: Extract concept graph + relevant files (lightweight context)
    // Keep graph + sorted alive so the repo profile stage can reference them.
    let (raw_graph, graph_context, relevant_file_paths) = match extract_concept_graph(client, &phase1_context, paper_text.as_deref(), Some(&database.pool)).await {
        Ok(graph) => {
            println!(
                "🧠 Concept graph extracted: {} concepts, {} relevant files",
                graph.concepts.len(),
                graph.relevant_files.len()
            );
            let sorted = crate::prompt_builders::topological_sort(graph.concepts.clone());
            for (i, c) in sorted.iter().enumerate() {
                println!(
                    "  {}. [{}] {} — {} (files: {})",
                    i + 1,
                    c.concept_type,
                    c.name,
                    c.description,
                    c.supporting_files.join(", ")
                );
            }
            if !graph.relevant_files.is_empty() {
                println!("  📂 Relevant files: {:?}", graph.relevant_files);
            }
            let gc = build_graph_context(&sorted);
            let rf = graph.relevant_files.clone();
            (Some((graph, sorted)), Some(gc), rf)
        }
        Err(e) => {
            println!("⚠️  Concept graph extraction failed: {} — falling back to single-phase generation", e);
            (None, None, vec![])
        }
    };

    // 10. Fetch targeted source files (Phase 1-driven or fallback)
    let files_to_fetch: Vec<String> = if relevant_file_paths.is_empty() {
        let fallback = select_fallback_files(&all_paths, 10);
        println!("  Using fallback file selection: {} files", fallback.len());
        fallback
    } else {
        // Validate that returned paths actually exist in the repo tree
        relevant_file_paths
            .into_iter()
            .filter(|p| all_paths.contains(p))
            .collect()
    };

    let source_files = fetch_relevant_files(&gh, &base, &files_to_fetch, 4000, 40000).await;
    println!("  Source files fetched: {} files", source_files.len());

    // 11. Build Phase 2 context (full: README + deps + source files + commits/issues/PRs)
    let mut context = format!("# Repository: {}/{}\n\n", owner, repo);
    if !repo_description.is_empty() {
        context.push_str(&format!("**Description:** {}\n", repo_description));
    }
    context.push_str(&format!("**Primary Language:** {}\n", primary_language));
    if !topics.is_empty() {
        context.push_str(&format!("**Topics:** {}\n", topics.join(", ")));
    }
    context.push('\n');

    if !readme_text.is_empty() {
        context.push_str(&format!("## README\n\n{}\n\n", readme_text));
    }

    for (filename, content) in &dep_contents {
        context.push_str(&format!("## {}\n\n```\n{}\n```\n\n", filename, content));
    }

    if !source_files.is_empty() {
        context.push_str("## Key Source Files\n\n");
        for (filename, content) in &source_files {
            context.push_str(&format!("### {}\n\n```\n{}\n```\n\n", filename, content));
        }
    }

    if !recent_commits.is_empty() {
        context.push_str("## Recent Commit History (newest first)\n\n");
        for commit in &recent_commits {
            context.push_str(&format!("- {}\n", commit));
        }
        context.push('\n');
    }

    if !open_issues.is_empty() {
        context.push_str("## Open Issues\n\n");
        for issue in &open_issues {
            context.push_str(&format!("- {}\n", issue));
        }
        context.push('\n');
    }

    if !merged_prs.is_empty() {
        context.push_str("## Recently Merged PRs\n\n");
        for pr in &merged_prs {
            context.push_str(&format!("- {}\n", pr));
        }
        context.push('\n');
    }

    if let Some(ref paper) = paper_text {
        // Cap at 20k chars so a long paper doesn't blow the tree-gen context budget
        let truncated = if paper.len() > 20_000 { &paper[..20_000] } else { paper.as_str() };
        context.push_str("## PAPER CONTEXT\n\n");
        context.push_str("The following reference paper describes the theoretical background this repository implements. Use it to anchor concepts, terminology, and learning progression.\n\n");
        context.push_str(truncated);
        context.push_str("\n\n");
    }

    println!("📦 Phase 2 context: {} chars", context.len());

    // 11b. Build repo profile (grounding stage) — best-effort, non-fatal
    let profile_section = if let Some((ref graph, ref sorted)) = raw_graph {
        match build_repo_profile(client, &context, graph, sorted, Some(&database.pool)).await {
            Ok(profile) => {
                let mut s = String::from("## REPO PROFILE (authoritative — use this to anchor the tree)\n\n");
                s.push_str(&format!("**Summary:** {}\n\n", profile.summary));
                s.push_str(&format!("**Stack:** {}\n\n", profile.stack.join(", ")));
                s.push_str(&format!("**Subsystems:** {}\n\n", profile.subsystems.join(", ")));
                if !profile.evidenced_concept_ids.is_empty() {
                    s.push_str(&format!("**Well-evidenced concepts:** {}\n\n", profile.evidenced_concept_ids.join(", ")));
                }
                s
            }
            Err(e) => {
                println!("⚠️  Repo profile failed (non-fatal): {}", e);
                String::new()
            }
        }
    } else {
        String::new()
    };

    // 12. Two-stage tree generation (outline then per-skill expansion)
    let outline_user_prompt = if let Some(ref gc) = graph_context {
        format!(
            "Project ID: {}\n\n{}\n\n{}\nRepository context:\n{}\n\nGenerate the skeleton outline JSON:",
            project_id, gc, profile_section, context
        )
    } else {
        format!(
            "Project ID: {}\n\n{}\nRepository context:\n{}\n\nGenerate the skeleton outline JSON:",
            project_id, profile_section, context
        )
    };

    // Compact context for skill expansion — repo profile + short context summary
    let mut expansion_context = if !profile_section.is_empty() {
        format!("{}\n\nRepository: {}/{}", profile_section, owner, repo)
    } else {
        format!(
            "Repository: {}/{}\n\n{}",
            owner, repo,
            if context.len() > 3000 { &context[..3000] } else { &context }
        )
    };

    if let Some(ref paper) = paper_text {
        // Per-skill expansion happens once per skill (often 9+ calls) — keep the
        // paper slice small so we don't multiply token cost by the fanout.
        let snippet = if paper.len() > 4_000 { &paper[..4_000] } else { paper.as_str() };
        expansion_context.push_str("\n\n## PAPER CONTEXT (reference material)\n\n");
        expansion_context.push_str(snippet);
    }

    let tree_model = std::env::var("TREE_GEN_MODEL")
        .unwrap_or_else(|_| "google/gemini-2.5-flash".to_string());
    let tree_api_key = std::env::var("TREE_GEN_API_KEY")
        .unwrap_or_else(|_| std::env::var("OPENROUTER_API_KEY").unwrap_or_default());
    let tree_base_url = std::env::var("TREE_GEN_BASE_URL")
        .unwrap_or_else(|_| "https://openrouter.ai/api/v1/chat/completions".to_string());
    // Kimi K2 does not support response_format: json_object via OpenRouter
    let json_mode = !tree_model.contains("kimi");

    println!("🌲 Tree gen (repo two-stage): model={} json_mode={}", tree_model, json_mode);

    let mut skill_tree = generate_tree_two_stage(
        client,
        &tree_model,
        &tree_base_url,
        &tree_api_key,
        &build_repo_outline_system_prompt(mastered_concepts),
        &outline_user_prompt,
        &expansion_context,
        &project_id,
        json_mode,
        Some(&database.pool),
        mastered_concepts,
    ).await?;
    skill_tree.project_id = project_id;

    println!("✅ Repo analysis complete: {} phases", skill_tree.phases.len());

    let repo_sorted_concepts = raw_graph.as_ref().map(|(_, sorted)| sorted.as_slice());
    let (tree_id, leaf_node_ids) = save_tree_to_database(&skill_tree, database, repo_sorted_concepts).await?;
    println!("💾 Saved repo tree with tree_id: {}", tree_id);

    auto_match_tree_nodes(&database.pool, client, &leaf_node_ids).await;
    crate::orchestrator::on_tree_generated(&database.pool, app, &tree_id, &skill_tree.project_id).await;

    let mut response_json = serde_json::json!(skill_tree);
    response_json["tree_id"] = serde_json::json!(tree_id);

    Ok(serde_json::to_string_pretty(&response_json).unwrap())
}

#[tauri::command]
pub async fn analyze_repo(
    project_id: String,
    github_url: String,
    paper_url: Option<String>,
    paper_pdf: Option<String>,
    app: tauri::AppHandle,
    client: State<'_, reqwest::Client>,
    database: State<'_, Database>,
) -> Result<String, String> {
    analyze_repo_inner(project_id, github_url, paper_url, paper_pdf, &[], &app, &*client, &*database).await
}

// ─── Prompt stats ─────────────────────────────────────────────────────────────

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandStats {
    pub command: String,
    pub total_calls: i64,
    pub success_rate: f64,
    pub avg_latency_ms: f64,
    pub latest_prompt_version: String,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStats {
    pub model: String,
    pub total_calls: i64,
    pub avg_latency_ms: f64,
    pub error_rate: f64,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentError {
    pub command: String,
    pub error: String,
    pub created_at: String,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptStats {
    pub by_command: Vec<CommandStats>,
    pub by_model: Vec<ModelStats>,
    pub recent_errors: Vec<RecentError>,
}

#[tauri::command]
pub async fn get_prompt_stats(
    database: tauri::State<'_, Database>,
) -> Result<PromptStats, String> {
    use sqlx::Row;

    let cmd_rows = sqlx::query(
        "SELECT command,
                COUNT(*) AS total_calls,
                COALESCE(AVG(CASE WHEN success THEN 1.0 ELSE 0.0 END), 0) AS success_rate,
                COALESCE(AVG(latency_ms), 0) AS avg_latency_ms,
                (SELECT prompt_version FROM prompt_logs pl2
                 WHERE pl2.command = pl.command
                 ORDER BY created_at DESC LIMIT 1) AS latest_version
         FROM prompt_logs pl
         GROUP BY command
         ORDER BY total_calls DESC"
    )
    .fetch_all(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    let by_command = cmd_rows.iter().map(|r| CommandStats {
        command: r.try_get("command").unwrap_or_default(),
        total_calls: r.try_get("total_calls").unwrap_or(0),
        success_rate: {
            let v: f64 = r.try_get("success_rate").unwrap_or(0.0);
            (v * 1000.0).round() / 10.0
        },
        avg_latency_ms: {
            let v: f64 = r.try_get("avg_latency_ms").unwrap_or(0.0);
            v.round()
        },
        latest_prompt_version: r.try_get("latest_version").unwrap_or_default(),
    }).collect();

    let model_rows = sqlx::query(
        "SELECT model,
                COUNT(*) AS total_calls,
                COALESCE(AVG(latency_ms), 0) AS avg_latency_ms,
                COALESCE(AVG(CASE WHEN NOT success THEN 1.0 ELSE 0.0 END), 0) AS error_rate
         FROM prompt_logs
         GROUP BY model
         ORDER BY total_calls DESC"
    )
    .fetch_all(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    let by_model = model_rows.iter().map(|r| ModelStats {
        model: r.try_get("model").unwrap_or_default(),
        total_calls: r.try_get("total_calls").unwrap_or(0),
        avg_latency_ms: {
            let v: f64 = r.try_get("avg_latency_ms").unwrap_or(0.0);
            v.round()
        },
        error_rate: {
            let v: f64 = r.try_get("error_rate").unwrap_or(0.0);
            (v * 1000.0).round() / 10.0
        },
    }).collect();

    let err_rows = sqlx::query(
        "SELECT command, error, created_at::TEXT AS created_at
         FROM prompt_logs
         WHERE success = false AND error IS NOT NULL
         ORDER BY created_at DESC
         LIMIT 5"
    )
    .fetch_all(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    let recent_errors = err_rows.iter().map(|r| RecentError {
        command: r.try_get("command").unwrap_or_default(),
        error: r.try_get("error").unwrap_or_default(),
        created_at: r.try_get("created_at").unwrap_or_default(),
    }).collect();

    Ok(PromptStats { by_command, by_model, recent_errors })
}

// ─── Tree regeneration history ────────────────────────────────────────────────

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeRegenerationRecord {
    pub id: String,
    pub old_tree_id: Option<String>,
    pub new_tree_id: Option<String>,
    pub regenerated_at: String,
    pub nodes_carried: i32,
    pub nodes_dropped: i32,
}

#[tauri::command]
pub async fn get_tree_regenerations(
    tree_id: String,
    database: tauri::State<'_, Database>,
) -> Result<Vec<TreeRegenerationRecord>, String> {
    use sqlx::Row;

    let rows = sqlx::query(
        "SELECT id, old_tree_id, new_tree_id, \
                regenerated_at::TEXT AS regenerated_at, \
                nodes_carried, nodes_dropped \
         FROM tree_regenerations \
         WHERE old_tree_id = $1 OR new_tree_id = $1 \
         ORDER BY regenerated_at DESC"
    )
    .bind(&tree_id)
    .fetch_all(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(rows.iter().map(|r| TreeRegenerationRecord {
        id: r.try_get("id").unwrap_or_default(),
        old_tree_id: r.try_get("old_tree_id").unwrap_or(None),
        new_tree_id: r.try_get("new_tree_id").unwrap_or(None),
        regenerated_at: r.try_get("regenerated_at").unwrap_or_default(),
        nodes_carried: r.try_get("nodes_carried").unwrap_or(0),
        nodes_dropped: r.try_get("nodes_dropped").unwrap_or(0),
    }).collect())
}

/// Regenerate a tree for a project, carrying over user state from the current
/// active tree. Accepts the same inputs as `generate_skill_tree` / `analyze_repo`.
#[tauri::command]
pub async fn regenerate_tree(
    project_id: String,
    use_github: bool,
    github_url: Option<String>,
    prd_text: Option<String>,
    paper_url: Option<String>,
    paper_pdf: Option<String>,
    app: tauri::AppHandle,
    client: State<'_, reqwest::Client>,
    database: State<'_, Database>,
) -> Result<serde_json::Value, String> {
    use sqlx::Row;

    // Fetch current active tree
    let active_row = sqlx::query(
        "SELECT active_tree_id FROM projects WHERE id = $1"
    )
    .bind(&project_id)
    .fetch_optional(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    let old_tree_id: Option<String> = active_row
        .and_then(|r| r.try_get::<Option<String>, _>("active_tree_id").ok().flatten());

    // Get old tree version number (if present)
    let old_version: i32 = if let Some(ref tid) = old_tree_id {
        sqlx::query("SELECT COALESCE(version, 1) AS version FROM trees WHERE id = $1")
            .bind(tid)
            .fetch_optional(&database.pool)
            .await
            .ok()
            .flatten()
            .and_then(|r| r.try_get::<i32, _>("version").ok())
            .unwrap_or(1)
    } else {
        1
    };

    // Collect mastered concepts from the old tree so V2 builds on V1 knowledge
    let mastered_concepts: Vec<String> = if let Some(ref tid) = old_tree_id {
        get_mastered_concepts(&database.pool, tid).await
    } else {
        Vec::new()
    };
    if !mastered_concepts.is_empty() {
        println!("🎓 Mastered concepts from V{}: {} slugs", old_version, mastered_concepts.len());
    }

    // Generate the new tree
    let new_tree_json_str = if use_github {
        let url = github_url.ok_or("github_url required when use_github=true")?;
        analyze_repo_inner(
            project_id.clone(),
            url,
            paper_url,
            paper_pdf,
            &mastered_concepts,
            &app,
            &*client,
            &*database,
        ).await?
    } else {
        let prd = prd_text.ok_or("prd_text required when use_github=false")?;
        generate_skill_tree_inner(
            project_id.clone(),
            prd,
            &mastered_concepts,
            &app,
            &*client,
            &*database,
        ).await?
    };

    let new_tree_parsed: serde_json::Value = serde_json::from_str(&new_tree_json_str)
        .map_err(|e| format!("Failed to parse new tree JSON: {}", e))?;

    let new_tree_id = new_tree_parsed["tree_id"]
        .as_str()
        .ok_or("New tree JSON missing tree_id")?
        .to_string();

    // Set version on new tree
    let new_version = old_version + 1;
    let _ = sqlx::query("UPDATE trees SET version = $1 WHERE id = $2")
        .bind(new_version)
        .bind(&new_tree_id)
        .execute(&database.pool)
        .await;

    // Carry over user state
    let (nodes_carried, nodes_dropped, state_losses) = if let Some(ref old_id) = old_tree_id {
        diff_and_carry_state(&database.pool, old_id, &new_tree_id).await
            .unwrap_or_else(|e| {
                println!("⚠️  diff_and_carry_state failed (non-fatal): {}", e);
                (0, 0, serde_json::Value::Array(vec![]))
            })
    } else {
        (0, 0, serde_json::Value::Array(vec![]))
    };

    // Archive old tree
    if let Some(ref old_id) = old_tree_id {
        let _ = sqlx::query("UPDATE trees SET archived_at = NOW() WHERE id = $1")
            .bind(old_id)
            .execute(&database.pool)
            .await;
    }

    // Update project's active_tree_id
    let _ = sqlx::query("UPDATE projects SET active_tree_id = $1 WHERE id = $2")
        .bind(&new_tree_id)
        .bind(&project_id)
        .execute(&database.pool)
        .await;

    // Insert tree_regenerations record
    let regen_id = uuid::Uuid::new_v4().to_string();
    let _ = sqlx::query(
        "INSERT INTO tree_regenerations \
           (id, old_tree_id, new_tree_id, nodes_carried, nodes_dropped, user_state_losses) \
         VALUES ($1, $2, $3, $4, $5, $6)"
    )
    .bind(&regen_id)
    .bind(old_tree_id.as_deref())
    .bind(&new_tree_id)
    .bind(nodes_carried as i32)
    .bind(nodes_dropped as i32)
    .bind(&state_losses)
    .execute(&database.pool)
    .await;

    println!(
        "✅ regenerate_tree: new_tree_id={} version={} carried={} dropped={}",
        new_tree_id, new_version, nodes_carried, nodes_dropped
    );

    Ok(serde_json::json!({
        "treeId": new_tree_id,
        "version": new_version,
        "nodesCarried": nodes_carried,
        "nodesDropped": nodes_dropped,
    }))
}

// ─── Concept graph retrieval ──────────────────────────────────────────────────

#[tauri::command]
pub async fn get_tree_concept_graph(
    tree_id: String,
    database: tauri::State<'_, Database>,
) -> Result<Option<serde_json::Value>, String> {
    use sqlx::Row;
    let row = sqlx::query("SELECT concept_graph FROM trees WHERE id = $1")
        .bind(&tree_id)
        .fetch_optional(&database.pool)
        .await
        .map_err(|e| e.to_string())?;

    Ok(row.and_then(|r| r.try_get::<Option<serde_json::Value>, _>("concept_graph").ok().flatten()))
}
