// src-tauri/src/brain.rs

use base64::{Engine as _, engine::general_purpose};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::State;
use uuid::Uuid;
use crate::database::Database;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SkillTree {
    pub project_id: String,
    pub phases: Vec<Phase>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Phase {
    pub id: String,
    pub name: String,
    pub description: String,
    pub order: i32,
    pub skills: Vec<Skill>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Skill {
    pub id: String,
    pub name: String,
    pub description: String,
    pub order: i32,
    pub quests: Vec<Quest>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Quest {
    pub id: String,
    pub title: String,
    pub description: String,
    pub estimated_hours: f32,
    pub difficulty: String,
    pub order: i32,
}

#[derive(Debug, Deserialize)]
struct GroqResponse {
    choices: Vec<GroqChoice>,
}

#[derive(Debug, Deserialize)]
struct GroqChoice {
    message: GroqMessage,
}

#[derive(Debug, Deserialize)]
struct GroqMessage {
    content: String,
}

// ─── Database helpers ────────────────────────────────────────────────────────

/// Save AI-generated skill tree to database.
/// Returns (tree_id, leaf_node_ids) so callers can trigger auto-matching.
async fn save_tree_to_database(
    skill_tree: &SkillTree,
    database: &Database,
) -> Result<(String, Vec<String>), String> {
    let tree_id = Uuid::new_v4().to_string();
    let tree_name = format!("AI Generated - {}", chrono::Utc::now().format("%Y-%m-%d %H:%M"));

    sqlx::query!(
        "INSERT INTO trees (id, project_id, name) VALUES ($1, $2, $3)",
        tree_id,
        skill_tree.project_id,
        tree_name
    )
    .execute(&database.pool)
    .await
    .map_err(|e| format!("Failed to create tree: {}", e))?;

    println!("📦 Created tree: {}", tree_id);

    let mut node_ids: Vec<(String, Option<String>)> = Vec::new();
    let mut leaf_node_ids: Vec<String> = Vec::new();
    let mut order_counter: i32 = 0;

    for phase in &skill_tree.phases {
        let phase_node_id = Uuid::new_v4().to_string();
        let phase_tasks = serde_json::json!([]);

        sqlx::query(
            "INSERT INTO tree_nodes (id, tree_id, parent_id, type, title, description, progress, tasks, resources, x, y, order_index) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)"
        )
        .bind(&phase_node_id).bind(&tree_id).bind(None::<&str>).bind("trunk")
        .bind(&phase.name).bind(&phase.description).bind(0i32)
        .bind(&phase_tasks).bind(None::<serde_json::Value>)
        .bind(None::<f64>).bind(None::<f64>).bind(order_counter)
        .execute(&database.pool)
        .await
        .map_err(|e| format!("Failed to create phase node: {}", e))?;

        node_ids.push((phase_node_id.clone(), None));
        order_counter += 1;
        println!("  🌳 Phase: {}", phase.name);

        for (skill_index, skill) in phase.skills.iter().enumerate() {
            let skill_node_id = Uuid::new_v4().to_string();
            let skill_tasks = serde_json::json!([]);
            // First skill of each phase is unlocked; subsequent skills start locked
            let is_locked = skill_index > 0;

            sqlx::query(
                "INSERT INTO tree_nodes (id, tree_id, parent_id, type, title, description, progress, tasks, resources, x, y, order_index, is_locked) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)"
            )
            .bind(&skill_node_id).bind(&tree_id).bind(&phase_node_id).bind("branch")
            .bind(&skill.name).bind(&skill.description).bind(0i32)
            .bind(&skill_tasks).bind(None::<serde_json::Value>)
            .bind(None::<f64>).bind(None::<f64>).bind(order_counter)
            .bind(is_locked)
            .execute(&database.pool)
            .await
            .map_err(|e| format!("Failed to create skill node: {}", e))?;

            node_ids.push((skill_node_id.clone(), Some(phase_node_id.clone())));
            order_counter += 1;
            println!("    🌿 Skill: {}", skill.name);

            for quest in &skill.quests {
                let quest_node_id = Uuid::new_v4().to_string();
                let quest_tasks = serde_json::json!([{
                    "id": quest.id,
                    "title": quest.title,
                    "description": quest.description,
                    "estimated_hours": quest.estimated_hours,
                    "difficulty": quest.difficulty,
                    "completed": false
                }]);

                sqlx::query(
                    "INSERT INTO tree_nodes (id, tree_id, parent_id, type, title, description, progress, tasks, resources, x, y, order_index) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)"
                )
                .bind(&quest_node_id).bind(&tree_id).bind(&skill_node_id).bind("leaf")
                .bind(&quest.title).bind(&quest.description).bind(0i32)
                .bind(&quest_tasks).bind(None::<serde_json::Value>)
                .bind(None::<f64>).bind(None::<f64>).bind(order_counter)
                .execute(&database.pool)
                .await
                .map_err(|e| format!("Failed to create quest node: {}", e))?;

                leaf_node_ids.push(quest_node_id.clone());
                node_ids.push((quest_node_id.clone(), Some(skill_node_id.clone())));
                order_counter += 1;
                println!("      🍃 Quest: {} ({:.1}h, {})", quest.title, quest.estimated_hours, quest.difficulty);
            }
        }
    }

    // Create edges
    for (node_id, parent_id) in &node_ids {
        if let Some(parent) = parent_id {
            let edge_id = Uuid::new_v4().to_string();
            sqlx::query!(
                "INSERT INTO tree_edges (id, tree_id, source_node_id, target_node_id) VALUES ($1, $2, $3, $4)",
                edge_id,
                tree_id,
                parent,
                node_id
            )
            .execute(&database.pool)
            .await
            .map_err(|e| format!("Failed to create edge: {}", e))?;
        }
    }

    let edge_count = node_ids.iter().filter(|(_, p)| p.is_some()).count();
    println!(
        "✅ Saved tree: {} nodes, {} edges, {} leaf nodes",
        node_ids.len(),
        edge_count,
        leaf_node_ids.len()
    );

    Ok((tree_id, leaf_node_ids))
}

// ─── Mimir auto-matching ─────────────────────────────────────────────────────

/// Call Mimir sidecar to semantically link library resources to each leaf node.
/// Best-effort: if the sidecar is not running, tree generation still succeeds.
async fn auto_match_tree_nodes(leaf_node_ids: &[String]) {
    if leaf_node_ids.is_empty() {
        return;
    }

    let client = reqwest::Client::new();

    // Quick health check first
    let alive = client
        .get("http://localhost:3001/health")
        .timeout(std::time::Duration::from_secs(2))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false);

    if !alive {
        println!("⚠️  Mimir sidecar not running — skipping auto-match");
        return;
    }

    let mut matched = 0usize;
    for node_id in leaf_node_ids {
        match client
            .post(format!("http://localhost:3001/match/{}", node_id))
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => matched += 1,
            Ok(r) => println!("⚠️  Mimir match returned {} for node {}", r.status(), node_id),
            Err(e) => {
                println!("⚠️  Mimir match error: {}", e);
                break;
            }
        }
    }

    if matched > 0 {
        println!("🔗 Auto-matched {} leaf nodes to Mimir resources", matched);
    }
}

// ─── System prompts ──────────────────────────────────────────────────────────

fn build_system_prompt() -> String {
    r#"You are a learning path architect for Yggdrasil, a skill tree app for project-based learning.

Given a PRD (Product Requirements Document) for a learning project, generate a structured skill tree in JSON format.

## Your Role

You generate LEARNING trees, not implementation plans. Focus on:
- Concepts to study and understand
- Resources to read, watch, or practice with
- Knowledge validation, not code implementation

## Output Format

Generate valid JSON matching this exact schema:

{
  "project_id": "will be provided",
  "phases": [
    {
      "id": "phase_1",
      "name": "Phase name",
      "description": "What concepts this phase covers",
      "order": 1,
      "skills": [
        {
          "id": "skill_1_1",
          "name": "Skill name",
          "description": "What you'll learn and why it matters",
          "order": 1,
          "quests": [
            {
              "id": "quest_1_1_1",
              "title": "Learning quest title",
              "description": "Specific learning action with resource reference",
              "estimated_hours": 2.5,
              "difficulty": "easy",
              "order": 1
            }
          ]
        }
      ]
    }
  ]
}

## Quest Types (Learning-Focused)

ALLOWED:
- "Read Chapter X from [Resource]"
- "Watch lecture series on [Topic]"
- "Study [Concept] and take notes"
- "Work through practice problems for [Topic]"

FORBIDDEN:
- "Implement X feature"
- "Build Y component"
- "Write tests for Z"
- "Create a demo"

## Structure Guidelines

- 3-5 phases (Foundation → Core → Advanced)
- 2-4 skills per phase
- 3-6 quests per skill

## Difficulty

Must use exactly: "easy", "medium", or "hard"

## Response Format

Respond with ONLY the JSON object. No markdown, no explanation.
"#.to_string()
}

fn build_repo_system_prompt() -> String {
    r#"You are a learning path architect for Yggdrasil, a skill tree app for project-based learning.

Given information about a GitHub repository (README, dependency files, language, topics), generate a learning tree of all the concepts, patterns, and technologies the developer used but may not fully understand.

## Your Role

Focus ENTIRELY on learning — what concepts does this codebase exemplify? What should the developer study to deeply understand what they built?

Examples:
- A React app → learn component lifecycle, hooks, reconciliation, bundling
- A Rust CLI → learn ownership, lifetimes, traits, error handling, async Rust
- A Python ML script → learn numpy internals, gradient descent, regularization
- A PostgreSQL schema → learn normalization, indexing, query planning, transactions

## Output Format

Generate valid JSON:

{
  "project_id": "will be provided",
  "phases": [
    {
      "id": "phase_1",
      "name": "Phase name",
      "description": "What concepts this phase covers",
      "order": 1,
      "skills": [
        {
          "id": "skill_1_1",
          "name": "Skill name",
          "description": "What you'll learn and why",
          "order": 1,
          "quests": [
            {
              "id": "quest_1_1_1",
              "title": "Learning quest title",
              "description": "Specific learning action",
              "estimated_hours": 2.5,
              "difficulty": "easy",
              "order": 1
            }
          ]
        }
      ]
    }
  ]
}

## Quest Types — Learning ONLY

ALLOWED: "Read the official docs for X", "Watch Y lecture on Z", "Study X with worked examples"
FORBIDDEN: "Implement X", "Build Y", "Write tests for Z", "Add feature W"

## Structure

- 3-5 phases (Foundation → Core → Advanced)
- 2-4 skills per phase, 3-6 quests per skill

## Difficulty

Must use exactly: "easy", "medium", or "hard"

## Response Format

Respond with ONLY the JSON object. No markdown, no explanation.
"#.to_string()
}

// ─── GitHub helpers ──────────────────────────────────────────────────────────

fn parse_github_url(url: &str) -> Option<(String, String)> {
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

// ─── Groq call helper ────────────────────────────────────────────────────────

async fn call_groq(
    api_key: &str,
    system_prompt: &str,
    user_prompt: &str,
) -> Result<String, String> {
    let client = reqwest::Client::new();

    let request_body = json!({
        "model": "llama-3.3-70b-versatile",
        "messages": [
            { "role": "system", "content": system_prompt },
            { "role": "user",   "content": user_prompt }
        ],
        "temperature": 0.7,
        "max_tokens": 4096,
        "response_format": { "type": "json_object" }
    });

    let response = client
        .post("https://api.groq.com/openai/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&request_body)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {}", e))?;

    let status = response.status();
    let response_text = response
        .text()
        .await
        .map_err(|e| format!("Failed to read response: {}", e))?;

    if !status.is_success() {
        return Err(format!("Groq API returned {}: {}", status, response_text));
    }

    let groq_response: GroqResponse = serde_json::from_str(&response_text)
        .map_err(|e| format!("Failed to parse Groq response: {}", e))?;

    Ok(groq_response.choices[0].message.content.clone())
}

// ─── Commands ────────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn generate_skill_tree(
    project_id: String,
    prd_text: String,
    database: State<'_, Database>,
) -> Result<String, String> {
    println!("\n=== Generate Skill Tree (PRD) ===");
    println!("Project ID: {}", project_id);
    println!("PRD length: {} chars\n", prd_text.len());

    let env_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".env");
    dotenv::from_path(&env_path).ok();
    dotenv::dotenv().ok();
    let api_key = std::env::var("GROQ_API_KEY")
        .map_err(|_| "GROQ_API_KEY not found in .env file".to_string())?;

    let user_prompt = format!(
        "Project ID: {}\n\nPRD:\n{}\n\nGenerate the skill tree JSON:",
        project_id, prd_text
    );

    println!("Calling Groq API…");
    let tree_json = call_groq(&api_key, &build_system_prompt(), &user_prompt).await?;

    let mut skill_tree: SkillTree = serde_json::from_str(&tree_json)
        .map_err(|e| format!("Generated JSON doesn't match SkillTree schema: {}", e))?;
    skill_tree.project_id = project_id;

    println!(
        "✅ Tree generated: {} phases, {} total skills",
        skill_tree.phases.len(),
        skill_tree.phases.iter().map(|p| p.skills.len()).sum::<usize>()
    );

    let (tree_id, leaf_node_ids) = save_tree_to_database(&skill_tree, &database).await?;
    println!("💾 Saved to database with tree_id: {}", tree_id);

    auto_match_tree_nodes(&leaf_node_ids).await;

    let mut response_json = serde_json::json!(skill_tree);
    response_json["tree_id"] = serde_json::json!(tree_id);

    Ok(serde_json::to_string_pretty(&response_json).unwrap())
}

#[tauri::command]
pub async fn analyze_repo(
    project_id: String,
    github_url: String,
    database: State<'_, Database>,
) -> Result<String, String> {
    let (owner, repo) = parse_github_url(&github_url).ok_or_else(|| {
        "Invalid GitHub URL. Expected: https://github.com/owner/repo".to_string()
    })?;

    println!("\n=== Analyze Repo: {}/{} ===", owner, repo);

    let env_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".env");
    dotenv::from_path(&env_path).ok();
    dotenv::dotenv().ok();
    let api_key = std::env::var("GROQ_API_KEY")
        .map_err(|_| "GROQ_API_KEY not found in .env file".to_string())?;

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

    // 2. README
    let readme_text = {
        match gh.get(format!("{}/readme", base)).send().await {
            Ok(r) if r.status().is_success() => {
                let data: serde_json::Value = r.json().await.unwrap_or_default();
                let encoded = data["content"].as_str().unwrap_or("").replace('\n', "");
                if encoded.is_empty() {
                    String::new()
                } else {
                    general_purpose::STANDARD
                        .decode(&encoded)
                        .map(|b| String::from_utf8_lossy(&b).to_string())
                        .unwrap_or_default()
                }
            }
            _ => String::new(),
        }
    };
    println!("  README: {} chars", readme_text.len());

    // 3. Root directory listing
    let root_files: Vec<String> = match gh
        .get(format!("{}/contents/", base))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => {
            let arr: Vec<serde_json::Value> = r.json().await.unwrap_or_default();
            arr.iter()
                .filter_map(|f| f["name"].as_str().map(String::from))
                .collect()
        }
        _ => vec![],
    };
    println!("  Root files: {:?}", root_files);

    // 4. Dependency files
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
        if root_files.iter().any(|f| f.as_str() == *dep_file) {
            if let Ok(r) = gh
                .get(format!("{}/contents/{}", base, dep_file))
                .send()
                .await
            {
                if r.status().is_success() {
                    let data: serde_json::Value = r.json().await.unwrap_or_default();
                    let encoded = data["content"].as_str().unwrap_or("").replace('\n', "");
                    if !encoded.is_empty() {
                        if let Ok(decoded) = general_purpose::STANDARD.decode(&encoded) {
                            let content = String::from_utf8_lossy(&decoded).to_string();
                            dep_contents.push((dep_file.to_string(), content));
                            println!("  Fetched: {}", dep_file);
                        }
                    }
                }
            }
        }
    }

    // 5. Build context
    let mut context = format!("# Repository: {}/{}\n\n", owner, repo);
    if !repo_description.is_empty() {
        context.push_str(&format!("**Description:** {}\n", repo_description));
    }
    context.push_str(&format!("**Primary Language:** {}\n", primary_language));
    if !topics.is_empty() {
        context.push_str(&format!("**Topics:** {}\n", topics.join(", ")));
    }
    context.push_str(&format!("**Root files:** {}\n\n", root_files.join(", ")));

    if !readme_text.is_empty() {
        let preview = if readme_text.len() > 3000 {
            format!("{}…", &readme_text[..3000])
        } else {
            readme_text.clone()
        };
        context.push_str(&format!("## README\n\n{}\n\n", preview));
    }

    for (filename, content) in &dep_contents {
        let preview = if content.len() > 2000 {
            format!("{}…", &content[..2000])
        } else {
            content.clone()
        };
        context.push_str(&format!("## {}\n\n```\n{}\n```\n\n", filename, preview));
    }

    println!("  Context built: {} chars", context.len());

    // 6. Call Groq
    let user_prompt = format!(
        "Project ID: {}\n\nRepository context:\n{}\n\nGenerate the learning skill tree JSON:",
        project_id, context
    );

    println!("Calling Groq API for repo analysis…");
    let tree_json =
        call_groq(&api_key, &build_repo_system_prompt(), &user_prompt).await?;

    let mut skill_tree: SkillTree = serde_json::from_str(&tree_json)
        .map_err(|e| format!("Generated JSON doesn't match SkillTree schema: {}", e))?;
    skill_tree.project_id = project_id;

    println!("✅ Repo analysis complete: {} phases", skill_tree.phases.len());

    let (tree_id, leaf_node_ids) = save_tree_to_database(&skill_tree, &database).await?;
    println!("💾 Saved repo tree with tree_id: {}", tree_id);

    auto_match_tree_nodes(&leaf_node_ids).await;

    let mut response_json = serde_json::json!(skill_tree);
    response_json["tree_id"] = serde_json::json!(tree_id);

    Ok(serde_json::to_string_pretty(&response_json).unwrap())
}
