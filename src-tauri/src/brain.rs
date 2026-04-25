// src-tauri/src/brain.rs

use base64::{Engine as _, engine::general_purpose};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::json;
use tauri::State;
use uuid::Uuid;
use crate::constants::SCRAPER_URL;
use crate::database::Database;

/// Token budget for each per-skill checkpoint expansion call (Stage 2).
/// Every call to expand_skill_checkpoints uses this constant — never a local
/// literal — so there is one place to change and no risk of per-skill drift.
const CHECKPOINT_EXPANSION_MAX_TOKENS: u32 = 1500;

/// Accept both `"id": "abc"` and `"id": 9` from LLM output
fn string_or_int<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrInt { Str(String), Int(i64), Float(f64) }
    match StringOrInt::deserialize(deserializer)? {
        StringOrInt::Str(s) => Ok(s),
        StringOrInt::Int(n) => Ok(n.to_string()),
        StringOrInt::Float(f) => Ok(f.to_string()),
    }
}

fn string_or_int_default<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    string_or_int(deserializer).or(Ok(String::new()))
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SkillTree {
    #[serde(default, deserialize_with = "string_or_int_default")]
    pub project_id: String,
    pub phases: Vec<Phase>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Phase {
    #[serde(default, deserialize_with = "string_or_int_default")]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub order: i32,
    pub skills: Vec<Skill>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Skill {
    #[serde(default, deserialize_with = "string_or_int_default")]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub order: i32,
    /// Accept both "checkpoints" and legacy "quests" from LLM output
    #[serde(alias = "quests", default)]
    pub checkpoints: Vec<Checkpoint>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Checkpoint {
    #[serde(default, deserialize_with = "string_or_int_default")]
    pub id: String,
    #[serde(alias = "name", default)]
    pub title: String,
    #[serde(default)]
    pub mastery_criteria: String,
    #[serde(default)]
    pub exercises: Vec<String>,
    #[serde(default)]
    pub order: i32,
    /// Legacy fields — tolerated during deserialization but not used
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub estimated_hours: Option<f32>,
    #[serde(default)]
    pub difficulty: Option<String>,
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

// ─── Concept graph types ─────────────────────────────────────────────────────

/// One concept extracted from the project. Every field forces the LLM to be
/// evidence-grounded: it must name the concept type, cite files, and explain
/// why it's in this specific project — not generic knowledge.
#[derive(Debug, Serialize, Deserialize, Clone)]
struct Concept {
    id: String,
    name: String,
    description: String,
    prerequisites: Vec<String>,
    /// "algorithm" | "data_structure" | "protocol" | "pattern" | "framework" |
    /// "language_feature" | "infrastructure" | "library"
    #[serde(default)]
    concept_type: String,
    /// Source files, manifest entries, or doc sections that demonstrate this
    /// concept is actually used in the project. Empty = LLM invented it.
    #[serde(default)]
    supporting_files: Vec<String>,
    /// One sentence: how this concept manifests in this specific codebase
    /// (e.g. "Used in mimir.rs to embed chunks via pplx-embed-v1-0.6b").
    #[serde(default)]
    project_relevance: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct ConceptGraph {
    concepts: Vec<Concept>,
    #[serde(default)]
    relevant_files: Vec<String>,
}

// ─── Repo profile types ───────────────────────────────────────────────────────

/// Intermediate summary of what the repo actually is, produced after the
/// concept graph but before full tree generation. Forces a grounding step:
/// the tree generator works from this profile, not from raw repo dumps.
#[derive(Debug, Serialize, Deserialize, Clone)]
struct RepoProfile {
    /// One-paragraph plain-English summary of what the project does.
    summary: String,
    /// Detected stack: language(s), frameworks, databases, key libraries.
    stack: Vec<String>,
    /// Distinct subsystems or modules (e.g. "Mimir ingestion pipeline",
    /// "Canvas tree renderer", "GitHub repo analyser").
    subsystems: Vec<String>,
    /// List of concept ids from the ConceptGraph that are well-evidenced.
    /// The tree generator should prioritise these.
    evidenced_concept_ids: Vec<String>,
}

// ─── JSON cleanup helper ─────────────────────────────────────────────────────

/// Strip markdown fences and leading/trailing whitespace from an LLM response
/// before handing it to serde_json. Also tries to extract the first top-level
/// JSON object if the model still emitted surrounding prose.
fn clean_llm_json(raw: &str) -> String {
    let s = raw.trim();

    // Strip ```json ... ``` or ``` ... ``` fences
    let s = if let Some(inner) = s.strip_prefix("```json") {
        inner.strip_suffix("```").unwrap_or(inner).trim()
    } else if let Some(inner) = s.strip_prefix("```") {
        inner.strip_suffix("```").unwrap_or(inner).trim()
    } else {
        s
    };

    // If there's still surrounding prose, try to extract the first top-level
    // JSON object (scan for the outermost { ... } pair).
    if !s.starts_with('{') {
        if let Some(start) = s.find('{') {
            let candidate = &s[start..];
            // Walk forward to find the matching closing brace.
            // Must be string-aware: { and } inside string literals must not
            // affect depth. Uses the same idiom as repair_truncated_tree_json.
            let mut depth = 0i32;
            let mut end = None;
            let mut in_string = false;
            let mut escape_next = false;
            for (i, ch) in candidate.char_indices() {
                if escape_next { escape_next = false; continue; }
                if ch == '\\' && in_string { escape_next = true; continue; }
                if ch == '"' { in_string = !in_string; continue; }
                if in_string { continue; }
                match ch {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = Some(i + 1);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            if let Some(e) = end {
                return candidate[..e].trim().to_string();
            }
        }
    }

    s.to_string()
}

// ─── Concept graph extraction ────────────────────────────────────────────────

fn build_concept_graph_system_prompt() -> &'static str {
    r#"You are a knowledge graph architect performing evidence-grounded concept extraction.

Your job: read the provided project context (README, dependency manifests, source files, file path list) and extract the specific concepts, algorithms, and patterns that are **demonstrably present** in this codebase.

## Evidence requirement (strictly enforced)

Every concept you output MUST be supported by at least one of:
- A named dependency in package.json / Cargo.toml / requirements.txt / go.mod
- A named import, function call, or struct in a source file shown to you
- A section of the README that explicitly describes using this technique
- A file path whose name strongly implies this concept (e.g. "src/rag.rs" → RAG retrieval)

If you cannot name a file or manifest entry that proves a concept is used, do NOT include it.

## Schema

Output ONLY valid JSON:
{
  "concepts": [
    {
      "id": "snake_case_id",
      "name": "Concept Name",
      "description": "One sentence — what this concept is and how it is specifically used in this project",
      "concept_type": "algorithm|data_structure|protocol|pattern|framework|language_feature|infrastructure|library",
      "supporting_files": ["path/to/file.rs", "Cargo.toml"],
      "project_relevance": "One sentence: how this concept manifests in this specific codebase (name actual files, structs, or functions)",
      "prerequisites": ["id_of_prerequisite_concept"]
    }
  ],
  "relevant_files": ["src/main.rs", "src/lib.rs"]
}

## Rules

- 8-20 concepts
- Every concept must have at least one entry in supporting_files
- supporting_files must be paths you actually saw in the provided context — no invented paths
- project_relevance must name at least one real file, struct, function, or library from the context
- prerequisites must reference valid concept ids in this response, no circular deps
- Foundational concepts have empty prerequisites array
- relevant_files: up to 10 paths from the FILE PATHS list that contain the most core logic
- REJECT generic concepts like "Software Architecture", "Error Handling", "Testing", "Async Programming" unless they are the primary technical subject of a specific file
- concept_type must be one of the 8 values listed above
- If a PAPER / THEORY CONTEXT section is provided, prioritize concepts that appear in both the paper and the codebase. Concepts that are only in the paper (theory with no implementation) should still be included if they are prerequisites for understanding the implementation.

Respond with ONLY the JSON. No markdown, no preamble."#
}

async fn extract_concept_graph(
    client: &reqwest::Client,
    context: &str,
    paper_context: Option<&str>,
    pool: Option<&sqlx::PgPool>,
) -> Result<ConceptGraph, String> {
    let user_prompt = if let Some(paper) = paper_context {
        let truncated = if paper.len() > 20_000 { &paper[..20_000] } else { paper };
        format!(
            "## PAPER / THEORY CONTEXT\n\nThe following reference paper describes the theoretical background this project implements. Use it to decide which concepts matter and which files contain the core logic.\n\n{}\n\n---\n\nExtract the concept dependency graph from this project:\n\n{}\n\nGenerate the concept graph JSON:",
            truncated, context
        )
    } else {
        format!(
            "Extract the concept dependency graph from this project:\n\n{}\n\nGenerate the concept graph JSON:",
            context
        )
    };

    let concept_model = std::env::var("CONCEPT_GRAPH_MODEL")
        .unwrap_or_else(|_| "google/gemini-2.5-flash".to_string());
    let concept_base_url = std::env::var("CONCEPT_GRAPH_BASE_URL")
        .unwrap_or_else(|_| "https://openrouter.ai/api/v1/chat/completions".to_string());
    let concept_api_key = std::env::var("CONCEPT_GRAPH_API_KEY")
        .or_else(|_| std::env::var("OPENROUTER_API_KEY"))
        .unwrap_or_default();
    println!("🧠 Concept graph model: {} via {}", concept_model, concept_base_url);

    let (raw_graph_json, cg_latency) = call_llm(
        client,
        &concept_base_url,
        &concept_api_key,
        &concept_model,
        build_concept_graph_system_prompt(),
        &user_prompt,
        4096,
        true,
    )
    .await
    .map_err(|e| {
        if let Some(p) = pool {
            log_prompt_call(p.clone(), "concept_graph", &concept_model, "concept_graph_v1", 0, false, Some(e.clone()), None);
        }
        e
    })?;
    if let Some(p) = pool {
        log_prompt_call(p.clone(), "concept_graph", &concept_model, "concept_graph_v1", cg_latency, true, None, None);
    }

    // Log a preview before attempting to parse
    let preview_len = raw_graph_json.len().min(600);
    println!("🔎 Concept graph raw response ({} chars): {}{}",
        raw_graph_json.len(),
        &raw_graph_json[..preview_len],
        if raw_graph_json.len() > preview_len { "…" } else { "" }
    );

    let graph_json = clean_llm_json(&raw_graph_json);

    // Parse via Value first to tolerate LLM quirks like duplicate keys
    let value: serde_json::Value = serde_json::from_str(&graph_json)
        .map_err(|e| format!("Concept graph JSON is not valid JSON: {} (raw preview: {}…)", e, &raw_graph_json[..raw_graph_json.len().min(200)]))?;
    let graph: ConceptGraph = serde_json::from_value(value)
        .map_err(|e| format!("Concept graph JSON doesn't match schema: {}", e))?;

    if graph.concepts.is_empty() {
        return Err("Concept graph returned zero concepts".to_string());
    }

    // Validate: all prerequisite ids reference existing concepts
    let valid_ids: std::collections::HashSet<&str> =
        graph.concepts.iter().map(|c| c.id.as_str()).collect();
    for concept in &graph.concepts {
        for prereq in &concept.prerequisites {
            if !valid_ids.contains(prereq.as_str()) {
                println!(
                    "⚠️  Concept '{}' references unknown prerequisite '{}' — ignoring",
                    concept.id, prereq
                );
            }
        }
    }

    Ok(graph)
}

// ─── Topological sort (Kahn's algorithm) ─────────────────────────────────────

fn topological_sort(concepts: Vec<Concept>) -> Vec<Concept> {
    use std::collections::{HashMap, VecDeque};

    let id_set: std::collections::HashSet<String> =
        concepts.iter().map(|c| c.id.clone()).collect();

    // Build in-degree map and adjacency list (concept -> list of dependents)
    let mut in_degree: HashMap<String, usize> = HashMap::new();
    let mut dependents: HashMap<String, Vec<String>> = HashMap::new();

    for c in &concepts {
        in_degree.entry(c.id.clone()).or_insert(0);
        // Only count prerequisites that actually exist in the graph
        let valid_prereqs: Vec<&String> = c.prerequisites.iter().filter(|p| id_set.contains(*p)).collect();
        *in_degree.entry(c.id.clone()).or_insert(0) = valid_prereqs.len();
        for prereq in valid_prereqs {
            dependents.entry(prereq.clone()).or_default().push(c.id.clone());
        }
    }

    // Initialize queue with zero-prerequisite concepts
    let mut queue: VecDeque<String> = VecDeque::new();
    for c in &concepts {
        if *in_degree.get(&c.id).unwrap_or(&0) == 0 {
            queue.push_back(c.id.clone());
        }
    }

    let concept_map: HashMap<String, Concept> =
        concepts.iter().cloned().map(|c| (c.id.clone(), c)).collect();

    let mut sorted: Vec<Concept> = Vec::new();

    while let Some(id) = queue.pop_front() {
        if let Some(concept) = concept_map.get(&id) {
            sorted.push(concept.clone());
        }
        if let Some(deps) = dependents.get(&id) {
            for dep_id in deps {
                if let Some(deg) = in_degree.get_mut(dep_id) {
                    *deg = deg.saturating_sub(1);
                    if *deg == 0 {
                        queue.push_back(dep_id.clone());
                    }
                }
            }
        }
    }

    // Cycle detection: if sorted has fewer items than input, there's a cycle
    if sorted.len() < concept_map.len() {
        println!("⚠️  Cycle detected in concept graph — falling back to original order");
        return concept_map.into_values().collect();
    }

    sorted
}

// ─── Build graph context string ──────────────────────────────────────────────

fn build_graph_context(sorted_concepts: &[Concept]) -> String {
    let mut out = String::from("CONCEPT DEPENDENCY ORDER (foundational -> advanced):\n");

    // Build a name lookup for prerequisite display
    let name_map: std::collections::HashMap<&str, &str> = sorted_concepts
        .iter()
        .map(|c| (c.id.as_str(), c.name.as_str()))
        .collect();

    for (i, concept) in sorted_concepts.iter().enumerate() {
        let prereq_display = if concept.prerequisites.is_empty() {
            "none".to_string()
        } else {
            concept
                .prerequisites
                .iter()
                .filter_map(|p| name_map.get(p.as_str()).copied())
                .collect::<Vec<_>>()
                .join(", ")
        };

        out.push_str(&format!(
            "{}. {} — {}\n   Prerequisites: {}\n",
            i + 1,
            concept.name,
            concept.description,
            prereq_display
        ));
    }

    out
}

// ─── Repo profile (intermediate grounding stage) ─────────────────────────────

fn build_repo_profile_system_prompt() -> &'static str {
    r#"You are a codebase analyst. Given repository context (README, dependency manifests, source files) and an already-extracted concept graph, produce a concise structured profile of the project.

Output ONLY valid JSON:
{
  "summary": "One paragraph — what this project does, who it is for, and the key technical approach",
  "stack": ["language or framework or library, one per entry, e.g. 'Rust', 'React 19', 'pgvector', 'Groq LLaMA 3.3-70b'"],
  "subsystems": ["Each distinct module or subsystem as a short label, e.g. 'Mimir ingestion pipeline', 'Canvas tree renderer'"],
  "evidenced_concept_ids": ["concept_id_1", "concept_id_2"]
}

Rules:
- summary: one dense paragraph, no bullet points. Name the actual technologies.
- stack: list every distinct technology visible in the manifest and source files. Include specific versions or model names where shown.
- subsystems: identify 3-8 distinct functional modules. Name them after what they DO, not what they ARE.
- evidenced_concept_ids: include only concept ids from the provided concept graph that are directly confirmed by manifest entries or source file content. Omit any concept whose only evidence is the README description.
- Respond with ONLY the JSON. No markdown, no preamble."#
}

async fn build_repo_profile(
    client: &reqwest::Client,
    repo_context: &str,
    concept_graph: &ConceptGraph,
    sorted_concepts: &[Concept],
    pool: Option<&sqlx::PgPool>,
) -> Result<RepoProfile, String> {
    // Build a compact concept list for the prompt
    let concept_summary: String = sorted_concepts
        .iter()
        .map(|c| format!("  id={} name=\"{}\" files={:?}", c.id, c.name, c.supporting_files))
        .collect::<Vec<_>>()
        .join("\n");

    let user_prompt = format!(
        "Repository context:\n{}\n\nExtracted concept graph ({} concepts):\n{}\n\nGenerate the repo profile JSON:",
        repo_context,
        concept_graph.concepts.len(),
        concept_summary
    );

    let concept_model = std::env::var("CONCEPT_GRAPH_MODEL")
        .unwrap_or_else(|_| "google/gemini-2.5-flash".to_string());
    let concept_base_url = std::env::var("CONCEPT_GRAPH_BASE_URL")
        .unwrap_or_else(|_| "https://openrouter.ai/api/v1/chat/completions".to_string());
    let concept_api_key = std::env::var("CONCEPT_GRAPH_API_KEY")
        .or_else(|_| std::env::var("OPENROUTER_API_KEY"))
        .unwrap_or_default();

    println!("🔍 Building repo profile…");
    let (profile_json, rp_latency) = call_llm(
        client,
        &concept_base_url,
        &concept_api_key,
        &concept_model,
        build_repo_profile_system_prompt(),
        &user_prompt,
        2048,
        true,
    )
    .await
    .map_err(|e| {
        if let Some(p) = pool {
            log_prompt_call(p.clone(), "repo_profile", &concept_model, "repo_profile_v1", 0, false, Some(e.clone()), None);
        }
        e
    })?;
    if let Some(p) = pool {
        log_prompt_call(p.clone(), "repo_profile", &concept_model, "repo_profile_v1", rp_latency, true, None, None);
    }

    let profile_json_clean = clean_llm_json(&profile_json);
    let value: serde_json::Value = serde_json::from_str(&profile_json_clean)
        .map_err(|e| format!("Repo profile JSON invalid: {}", e))?;
    let profile: RepoProfile = serde_json::from_value(value)
        .map_err(|e| format!("Repo profile doesn't match schema: {}", e))?;

    println!(
        "  📋 Stack: {}, Subsystems: {}, Evidenced concepts: {}",
        profile.stack.len(),
        profile.subsystems.len(),
        profile.evidenced_concept_ids.len()
    );
    Ok(profile)
}

// ─── PRD profile (grounding stage for PRD-based tree gen) ────────────────────

fn build_prd_profile_system_prompt() -> &'static str {
    r#"You are a learning-path analyst. Given a project PRD and an already-extracted concept dependency graph, produce a concise structured profile for use in skill tree generation.

Output ONLY valid JSON:
{
  "goal": "One sentence — what this project does and who it is for",
  "stack": ["technology or library, one per entry, e.g. 'Rust', 'React 19', 'pgvector', 'Groq LLaMA 3.3-70b'"],
  "subsystems": ["Each distinct functional module or subsystem, e.g. 'RAG ingestion pipeline', 'Canvas tree renderer'"],
  "key_concepts": [
    {
      "name": "Concept name",
      "depends_on": ["prerequisite concept name"]
    }
  ]
}

Rules:
- goal: one tight sentence. Name the specific domain or technical niche.
- stack: extract every technology named in the PRD. Include model names, database types, protocol names.
- subsystems: 3-8 modules named after what they DO (verbs), not what they ARE (nouns).
- key_concepts: 6-12 entries drawn from the concept graph. Order from foundational to advanced. depends_on lists direct prerequisites by name.
- Respond with ONLY the JSON. No markdown, no preamble."#
}

/// Lightweight PRD profile used to anchor skill expansion calls.
/// Mirrors build_repo_profile — produces a structured summary instead of
/// passing raw PRD text into each of the 9 checkpoint expansion calls.
async fn build_prd_profile(
    client: &reqwest::Client,
    prd_text: &str,
    sorted_concepts: &[Concept],
    pool: Option<&sqlx::PgPool>,
) -> Result<String, String> {
    let concept_summary: String = sorted_concepts
        .iter()
        .map(|c| {
            if c.prerequisites.is_empty() {
                format!("  {} — {} [no prerequisites]", c.name, c.description)
            } else {
                format!("  {} — {} [requires: {}]", c.name, c.description, c.prerequisites.join(", "))
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    let prd_excerpt = if prd_text.len() > 4000 { &prd_text[..4000] } else { prd_text };

    let user_prompt = format!(
        "PRD:\n{}\n\nConcept graph ({} concepts, topologically sorted):\n{}\n\nGenerate the project profile JSON:",
        prd_excerpt,
        sorted_concepts.len(),
        concept_summary
    );

    let concept_model = std::env::var("CONCEPT_GRAPH_MODEL")
        .unwrap_or_else(|_| "google/gemini-2.5-flash".to_string());
    let concept_base_url = std::env::var("CONCEPT_GRAPH_BASE_URL")
        .unwrap_or_else(|_| "https://openrouter.ai/api/v1/chat/completions".to_string());
    let concept_api_key = std::env::var("CONCEPT_GRAPH_API_KEY")
        .or_else(|_| std::env::var("OPENROUTER_API_KEY"))
        .unwrap_or_default();

    println!("🔍 Building PRD profile…");
    let (raw, pp_latency) = call_llm(
        client,
        &concept_base_url,
        &concept_api_key,
        &concept_model,
        build_prd_profile_system_prompt(),
        &user_prompt,
        1024,
        true,
    )
    .await
    .map_err(|e| {
        if let Some(p) = pool {
            log_prompt_call(p.clone(), "prd_profile", &concept_model, "prd_profile_v1", 0, false, Some(e.clone()), None);
        }
        e
    })?;
    if let Some(p) = pool {
        log_prompt_call(p.clone(), "prd_profile", &concept_model, "prd_profile_v1", pp_latency, true, None, None);
    }

    let clean = clean_llm_json(&raw);
    // Validate it's parseable JSON — if not, return error so caller can fall back
    serde_json::from_str::<serde_json::Value>(&clean)
        .map_err(|e| format!("PRD profile JSON invalid: {}", e))?;

    println!("  ✅ PRD profile built ({} chars)", clean.len());
    Ok(clean)
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

            for checkpoint in &skill.checkpoints {
                let cp_node_id = Uuid::new_v4().to_string();
                let cp_tasks = serde_json::json!({
                    "mastery_criteria": checkpoint.mastery_criteria,
                    "exercises": checkpoint.exercises,
                    "notes": "",
                    "completed": false
                });

                sqlx::query(
                    "INSERT INTO tree_nodes (id, tree_id, parent_id, type, title, description, progress, tasks, resources, x, y, order_index) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)"
                )
                .bind(&cp_node_id).bind(&tree_id).bind(&skill_node_id).bind("leaf")
                .bind(&checkpoint.title).bind(&checkpoint.mastery_criteria).bind(0i32)
                .bind(&cp_tasks).bind(None::<serde_json::Value>)
                .bind(None::<f64>).bind(None::<f64>).bind(order_counter)
                .execute(&database.pool)
                .await
                .map_err(|e| format!("Failed to create checkpoint node: {}", e))?;

                leaf_node_ids.push(cp_node_id.clone());
                node_ids.push((cp_node_id.clone(), Some(skill_node_id.clone())));
                order_counter += 1;
                println!("      🍃 Checkpoint: {}", checkpoint.title);
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

/// Semantically link library resources to each leaf node using native Mimir.
/// Best-effort: failures are logged but don't prevent tree generation from succeeding.
async fn auto_match_tree_nodes(pool: &sqlx::PgPool, leaf_node_ids: &[String]) {
    if leaf_node_ids.is_empty() {
        return;
    }

    let client = reqwest::Client::new();
    let mut matched = 0usize;

    for node_id in leaf_node_ids {
        match crate::mimir::match_node_impl(pool, &client, node_id).await {
            Ok(resources) if !resources.is_empty() => matched += 1,
            Ok(_) => {} // no matching resources — fine
            Err(e) => println!("⚠️  Mimir auto-match error for node {}: {}", node_id, e),
        }
    }

    if matched > 0 {
        println!("🔗 Auto-matched {} leaf nodes to Mimir resources", matched);
    }
}

// ─── Legacy monolithic system prompts (kept for reference, not currently used) ─

#[allow(dead_code)]
fn build_system_prompt() -> String {
    r#"You are a learning path architect for Yggdrasil, a skill tree app that helps developers deeply understand projects they've built.

Given a PRD (Product Requirements Document), generate a learning skill tree — a structured path through every concept behind this project.

## Your Role

You generate LEARNING trees, not implementation plans. The user already built or is building the project. They need to understand the concepts behind it at a deep level.

## The Checkpoint Model

Every leaf node is a **concept checkpoint** — a specific concept the learner must genuinely understand. Checkpoints are not tasks. They are concepts you reach, not chores you complete.

Each checkpoint has:
- **title**: A noun phrase naming the concept (e.g. "Velocity-Verlet Symplectic Integration")
- **mastery_criteria**: What understanding this concept looks like — how you know you've reached it
- **exercises**: 2-4 specific things to work through to confirm understanding

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
          "description": "What you'll learn and why it matters for this project",
          "order": 1,
          "checkpoints": [
            {
              "id": "checkpoint_1_1_1",
              "title": "Concept name as a noun phrase",
              "mastery_criteria": "You understand this when you can explain...",
              "exercises": [
                "Work through X to see how...",
                "Implement a minimal version of...",
                "Compare X and Y to understand why..."
              ],
              "order": 1
            }
          ]
        }
      ]
    }
  ]
}

## Checkpoint Title Rules — ENFORCE STRICTLY

### Titles must be CONCEPT NOUN PHRASES, not imperatives:
- GOOD: "PostgreSQL MVCC Concurrency Model"
- GOOD: "React Fiber Reconciliation Algorithm"
- GOOD: "Velocity-Verlet Symplectic Integration"
- GOOD: "pgvector HNSW Approximate Nearest Neighbor Search"

### BANNED title patterns (NEVER generate these):
- Imperative verbs: "Learn X", "Study X", "Understand X", "Explore X", "Read X", "Watch X"
- "Introduction to X", "Overview of X", "Basics of X"
- Platform names: "Watch X on YouTube", "Read X on Stack Overflow"
- Two libraries without a concept: "NumPy and Matplotlib"
- Generic filler: "Best Practices", "Code Quality", "Testing and Debugging"

### Mastery criteria rules:
- Describe what genuine understanding looks like in concrete terms
- Use phrases like "You can explain why...", "You can predict what happens when...", "You can implement... from scratch"
- Never use "You have learned..." or "You have read..."

### Exercise rules:
- 2-4 exercises per checkpoint
- Must be specific, actionable activities
- Good: "Implement a minimal Velocity-Verlet integrator and compare energy drift vs Euler over 1000 steps"
- Good: "Write a SQL query that demonstrates MVCC behavior using two concurrent transactions"
- Bad: "Read the documentation" — too vague
- Bad: "Practice using X" — not specific

## Checkpoint Count: Exactly 3 per skill

The three checkpoints within each skill must form a progression from foundational to advanced:

**Checkpoint 1 — Core property of this concept**
Name a specific, interesting property. "Basics of X" is ABSOLUTELY FORBIDDEN.

**Checkpoint 2 — Internal mechanism**
Name the algorithm, data structure, or mechanism that makes this work.

**Checkpoint 3 — Application in this project**
Connect to a specific feature, design choice, or constraint from the PRD.

## Structure

- Exactly 3 phases (no more, no fewer)
- Exactly 3 skills per phase
- Exactly 3 checkpoints per skill
- Every phase must map to real concepts in the PRD — no filler phases

## Phase Design

**Phase 1 — Technology Foundations (MANDATORY, always first)**

This phase MUST exist in every tree. Its purpose is to teach the underlying technologies
before any project-specific patterns. Every major library and framework from the dependency
files (package.json, Cargo.toml, requirements.txt, etc.) gets covered here.

The developer vibe-coded this project — they used these technologies without understanding
them. Phase 1 fixes that. It teaches what each technology IS and how it works internally,
not how this specific project uses it.

AI groups libraries into skills based on conceptual relatedness. Examples:
- "Rust Ownership and Memory Model" — covers Rust's ownership, borrowing, lifetimes
- "React Component Model and Hooks" — covers reconciliation, useState, useEffect
- "PostgreSQL and SQLx" — covers relational model, connection pools, query execution

Every library in the dependency files gets at least one checkpoint in Phase 1.
Do not skip small libraries — even utility packages have concepts worth understanding.

**Phases 2+ — Project Patterns**

These phases cover how THIS project uses technologies. Skills are named after project
concepts: "Yggdrasil's Tree Layout Algorithm", "Tauri IPC in this App", etc.

**CHECKPOINT PATTERN — enforced in every skill across all phases:**

Every skill must begin with at least one FOUNDATIONAL checkpoint before any
PROJECT-SPECIFIC checkpoints.

FOUNDATIONAL checkpoint = explains what the technology/concept IS, how it works
internally, at a conceptual level. Does NOT reference any file, function, or
behavior from this specific codebase.

PROJECT-SPECIFIC checkpoint = names a real file, function, struct, or behavior
from this codebase. The mastery_criteria references actual code.

EXAMPLE — skill: "SQLx and the Database Layer"
  ✅ FOUNDATIONAL: "How SQLx compile-time query checking works"
     mastery_criteria: "You can explain how sqlx::query! macro sends SQL to a
     real database at compile time to verify correctness, and why this catches
     errors before runtime."
  ✅ PROJECT-SPECIFIC: "Database::new() and connection pool setup in this app"
     mastery_criteria: "You can trace how Database::new() in commands.rs reads
     DATABASE_URL, creates a PgPoolOptions pool, and makes it available via
     Tauri managed state."

EXAMPLE — skill: "React Router in Tauri"
  ✅ FOUNDATIONAL: "How React Router's BrowserRouter manages navigation state"
     mastery_criteria: "You can explain how BrowserRouter uses the HTML5 History
     API to track routes without a server, and why Tauri apps can use it without
     a real URL."
  ✅ PROJECT-SPECIFIC: "Route structure in App.tsx"
     mastery_criteria: "You can trace all routes defined in App.tsx, identify
     which use the MainLayout wrapper, and explain what renders at each path."

WRONG — project-specific from checkpoint 1:
  ❌ "Type-Safe API Contract Design" — this is a pattern, not a foundation
  ❌ "Serde JSON Serialization Patterns" — assumes you know what Serde is
  ❌ "Tauri IPC Command Registration" — assumes you know what Tauri is

The number of foundational checkpoints per skill depends on complexity:
- Simple/familiar tech (e.g. a utility library): 1 foundation → 1-2 project
- Complex tech (e.g. Rust, Tauri, pgvector): 2-3 foundations → 1-2 project

**Ordering rules — enforce strictly:**
1. Phase 1 foundations always before any project-specific phases
2. State and data flow before features that consume them
3. IPC and communication layer before either side that uses it
4. Core data structures before algorithms that operate on them
5. Follow the CONCEPT DEPENDENCY ORDER graph — if concept A precedes B in the
   graph, the skill covering A must be in an equal or earlier phase than B

**Adapting to learner level (if LEARNER'S EXISTING SKILLS is provided):**
- Level 2+ (Familiar): skip foundational checkpoints for that technology entirely
- Level 1 (Aware): one brief refresher checkpoint, then project-specific
- Not listed: full foundational coverage as normal
Phase 1 may be shortened or skipped if the learner already knows all the tech.

## Response Format

Respond with ONLY the JSON object. No markdown, no explanation.
"#.to_string()
}

#[allow(dead_code)]
fn build_repo_system_prompt() -> String {
    r#"You are a learning path architect for Yggdrasil, a skill tree app that helps developers deeply understand projects they've built.

Given rich information about a GitHub repository — README, dependency files, actual source files, commit history, open issues, and merged PRs — generate a learning tree of all the concepts, algorithms, and patterns the developer used but may not fully understand.

## Your Role

Focus ENTIRELY on learning — what concepts does this codebase exemplify? What should the developer study to deeply understand what they built?

Use the repository data as evidence:
- **Dependency files**: Every listed library is a skill candidate
- **Source files**: See exactly which patterns, algorithms, and APIs are used
- **README + Description**: Understand the project's purpose
- **Commit history**: Where the developer spent effort — these need the deepest checkpoints
- **Open issues / Merged PRs**: Known pain points and solved problems worth studying

## The Checkpoint Model

Every leaf node is a **concept checkpoint** — a specific concept the learner must genuinely understand. Checkpoints are not tasks. They are concepts you reach, not chores you complete.

Each checkpoint has:
- **title**: A noun phrase naming the concept (e.g. "Velocity-Verlet Symplectic Integration")
- **mastery_criteria**: What understanding this concept looks like — how you know you've reached it
- **exercises**: 2-4 specific things to work through to confirm understanding

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
          "name": "Skill name — specific to this repo",
          "description": "What you'll learn and why it matters for understanding this codebase",
          "order": 1,
          "checkpoints": [
            {
              "id": "checkpoint_1_1_1",
              "title": "Concept name as a noun phrase",
              "mastery_criteria": "You understand this when you can explain...",
              "exercises": [
                "Work through X to see how...",
                "Implement a minimal version of...",
                "Compare X and Y to understand why..."
              ],
              "order": 1
            }
          ]
        }
      ]
    }
  ]
}

## Checkpoint Title Rules — ENFORCE STRICTLY

### Titles must be CONCEPT NOUN PHRASES derived from the actual codebase:
- GOOD: "NumPy Contiguous Memory Layout and Vectorized Operations"
- GOOD: "Numba JIT Compilation Pipeline"
- GOOD: "Velocity-Verlet Symplectic Integration"
- GOOD: "Julia Multiple Dispatch Method Resolution"

### BANNED title patterns (NEVER generate these):
- Imperative verbs: "Learn X", "Study X", "Understand X", "Explore X", "Read X", "Watch X"
- "Introduction to X", "Overview of X", "Basics of X"
- Platform names: "Watch X on YouTube", "Read X on Stack Overflow"
- Two libraries without a concept: "NumPy and Matplotlib"
- Generic filler: "Best Practices", "Code Quality", "Testing and Debugging" (unless repo has test files)

### How to derive checkpoints from source code:
- If the repo uses OOP heavily → checkpoint on that specific OOP pattern
- If it uses numerical methods → name the method explicitly
- If it imports a visualization library and uses 3D plotting → checkpoint for 3D projection mechanics
- If it uses JIT compilation → checkpoint for how JIT works
- If it implements an algorithm → name the algorithm

### Mastery criteria rules:
- Describe what genuine understanding looks like in concrete terms
- Reference specific classes, functions, or files from THIS codebase
- Good: "You can explain why the Body class in common.py stores position as a NumPy array instead of three separate floats"
- Bad: "You have learned about NumPy" — too vague

### Exercise rules:
- 2-4 exercises per checkpoint
- Must be specific, actionable activities referencing this project's code
- Good: "Modify the Body class to use a different integrator and compare energy conservation"
- Bad: "Read the documentation" — too vague

## Checkpoint Count: Exactly 3 per skill

The three checkpoints within each skill must form a progression:

**Checkpoint 1 — Core property**
Name a specific, interesting property. "Basics of X" is ABSOLUTELY FORBIDDEN.

**Checkpoint 2 — Internal mechanism**
Name the algorithm, data structure, or mechanism that makes this work.

**Checkpoint 3 — Application in this codebase**
Name a specific class, function, file, or behavior from THIS codebase.

## Structure

- Exactly 3 phases (no more, no fewer)
- Exactly 3 skills per phase
- Exactly 3 checkpoints per skill
- Every skill must be derived from actual evidence in the repository data

## Phase Design

Phase 1 MUST begin with at least one skill covering raw language fundamentals
for each primary language detected in the repo — e.g. Rust ownership/borrowing/lifetimes,
TypeScript type system, Python data model. These skills come before any framework or
library skills. Even if the source code looks sophisticated, assume the developer does
not deeply understand the language itself.

**Phase 1 — Technology Foundations (MANDATORY, always first)**

This phase MUST exist in every tree. Its purpose is to teach the underlying technologies
before any project-specific patterns. Every major library and framework from the dependency
files (package.json, Cargo.toml, requirements.txt, etc.) gets covered here.

The developer vibe-coded this project — they used these technologies without understanding
them. Phase 1 fixes that. It teaches what each technology IS and how it works internally,
not how this specific project uses it.

AI groups libraries into skills based on conceptual relatedness. Examples:
- "Rust Ownership and Memory Model" — covers Rust's ownership, borrowing, lifetimes
- "React Component Model and Hooks" — covers reconciliation, useState, useEffect
- "PostgreSQL and SQLx" — covers relational model, connection pools, query execution

Every library in the dependency files gets at least one checkpoint in Phase 1.
Do not skip small libraries — even utility packages have concepts worth understanding.

**Phases 2+ — Project Patterns**

These phases cover how THIS project uses technologies. Skills are named after project
concepts: "Yggdrasil's Tree Layout Algorithm", "Tauri IPC in this App", etc.

**CHECKPOINT PATTERN — enforced in every skill across all phases:**

Every skill must begin with at least one FOUNDATIONAL checkpoint before any
PROJECT-SPECIFIC checkpoints.

FOUNDATIONAL checkpoint = explains what the technology/concept IS, how it works
internally, at a conceptual level. Does NOT reference any file, function, or
behavior from this specific codebase.

PROJECT-SPECIFIC checkpoint = names a real file, function, struct, or behavior
from this codebase. The mastery_criteria references actual code.

EXAMPLE — skill: "SQLx and the Database Layer"
  ✅ FOUNDATIONAL: "How SQLx compile-time query checking works"
     mastery_criteria: "You can explain how sqlx::query! macro sends SQL to a
     real database at compile time to verify correctness, and why this catches
     errors before runtime."
  ✅ PROJECT-SPECIFIC: "Database::new() and connection pool setup in this app"
     mastery_criteria: "You can trace how Database::new() in commands.rs reads
     DATABASE_URL, creates a PgPoolOptions pool, and makes it available via
     Tauri managed state."

EXAMPLE — skill: "React Router in Tauri"
  ✅ FOUNDATIONAL: "How React Router's BrowserRouter manages navigation state"
     mastery_criteria: "You can explain how BrowserRouter uses the HTML5 History
     API to track routes without a server, and why Tauri apps can use it without
     a real URL."
  ✅ PROJECT-SPECIFIC: "Route structure in App.tsx"
     mastery_criteria: "You can trace all routes defined in App.tsx, identify
     which use the MainLayout wrapper, and explain what renders at each path."

WRONG — project-specific from checkpoint 1:
  ❌ "Type-Safe API Contract Design" — this is a pattern, not a foundation
  ❌ "Serde JSON Serialization Patterns" — assumes you know what Serde is
  ❌ "Tauri IPC Command Registration" — assumes you know what Tauri is

The number of foundational checkpoints per skill depends on complexity:
- Simple/familiar tech (e.g. a utility library): 1 foundation → 1-2 project
- Complex tech (e.g. Rust, Tauri, pgvector): 2-3 foundations → 1-2 project

**Ordering rules — enforce strictly:**
1. Phase 1 foundations always before any project-specific phases
2. State and data flow before features that consume them
3. IPC and communication layer before either side that uses it
4. Core data structures before algorithms that operate on them
5. Follow the CONCEPT DEPENDENCY ORDER graph — if concept A precedes B in the
   graph, the skill covering A must be in an equal or earlier phase than B

**Adapting to learner level (if LEARNER'S EXISTING SKILLS is provided):**
- Level 2+ (Familiar): skip foundational checkpoints for that technology entirely
- Level 1 (Aware): one brief refresher checkpoint, then project-specific
- Not listed: full foundational coverage as normal
Phase 1 may be shortened or skipped if the learner already knows all the tech.

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

// ─── GitHub file helper ──────────────────────────────────────────────────────

async fn fetch_github_file(
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
async fn fetch_repo_tree(gh: &reqwest::Client, base: &str) -> Result<Vec<String>, String> {
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
async fn fetch_relevant_files(
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
fn select_fallback_files(all_paths: &[String], max: usize) -> Vec<String> {
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

// ─── Two-stage tree generation types ─────────────────────────────────────────

/// Outline-only skill (no checkpoints) — Stage 1 output
#[derive(Debug, Serialize, Deserialize, Clone)]
struct OutlineSkill {
    #[serde(default, deserialize_with = "string_or_int_default")]
    id: String,
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    order: i32,
}

/// Outline-only phase — Stage 1 output
#[derive(Debug, Serialize, Deserialize, Clone)]
struct OutlinePhase {
    #[serde(default, deserialize_with = "string_or_int_default")]
    id: String,
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    order: i32,
    skills: Vec<OutlineSkill>,
}

/// Full outline — Stage 1 output
#[derive(Debug, Serialize, Deserialize, Clone)]
struct OutlineTree {
    #[serde(default, deserialize_with = "string_or_int_default")]
    project_id: String,
    phases: Vec<OutlinePhase>,
}

/// Checkpoint expansion — Stage 2 output (one call per skill)
#[derive(Debug, Serialize, Deserialize, Clone)]
struct CheckpointExpansion {
    checkpoints: Vec<Checkpoint>,
}

// ─── Outline system prompts ───────────────────────────────────────────────────

fn build_outline_system_prompt() -> &'static str {
    r#"You are a learning path architect for Yggdrasil.

Given a project description, generate a SKELETON learning tree — phases and skills only, no checkpoints.

## Structure (strictly enforced)
- Exactly 3 phases
- Exactly 3 skills per phase (9 skills total)
- NO checkpoints field

## Phase design
Phase 1 — Technology Foundations: one skill per major technology group. Every main library/framework must appear.
Phases 2–3 — Project Patterns: skills named after this project's specific concepts, algorithms, and subsystems.

## Output format — ONLY valid JSON, no markdown

{
  "project_id": "provided_below",
  "phases": [
    {
      "id": "phase_1",
      "name": "Phase name",
      "description": "What this phase covers (1 sentence)",
      "order": 1,
      "skills": [
        {
          "id": "skill_1_1",
          "name": "Skill name",
          "description": "What you'll learn (1 sentence)",
          "order": 1
        }
      ]
    }
  ]
}

Respond with ONLY the JSON. No markdown, no explanation."#
}

fn build_repo_outline_system_prompt() -> &'static str {
    r#"You are a learning path architect for Yggdrasil.

Given GitHub repository data (README, dependencies, source files, commit history), generate a SKELETON learning tree — phases and skills only, no checkpoints.

## Structure (strictly enforced)
- Exactly 3 phases
- Exactly 3 skills per phase (9 skills total)
- NO checkpoints field

## Phase design
Phase 1 — Technology Foundations: group major libraries and language features into skill clusters. Every main dependency must appear.
Phases 2–3 — Project Patterns: skills named after actual patterns, algorithms, and subsystems visible in the source files.

## Output format — ONLY valid JSON, no markdown

{
  "project_id": "provided_below",
  "phases": [
    {
      "id": "phase_1",
      "name": "Phase name",
      "description": "What this phase covers (1 sentence)",
      "order": 1,
      "skills": [
        {
          "id": "skill_1_1",
          "name": "Skill name — specific to this repo",
          "description": "What you'll learn from this codebase (1 sentence)",
          "order": 1
        }
      ]
    }
  ]
}

Respond with ONLY the JSON. No markdown, no explanation."#
}

// ─── Checkpoint expansion prompt ──────────────────────────────────────────────

fn build_checkpoint_expansion_system_prompt() -> &'static str {
    r#"You are a learning path architect for Yggdrasil.

Given one skill from a learning tree, generate EXACTLY 3 concept checkpoints for it.

## The Checkpoint Model

A checkpoint is a CONCEPT, not a task. The learner reaches it by genuinely understanding the concept — not by completing a chore.

Each checkpoint:
- **title**: A noun phrase naming the concept (e.g. "PostgreSQL MVCC Concurrency Model")
- **mastery_criteria**: What genuine understanding looks like (start with "You can explain..." or "You can predict...")
- **exercises**: Exactly 2 specific, actionable activities to confirm understanding
- **order**: 1, 2, or 3

## BANNED title patterns
- Imperative verbs: "Learn X", "Study X", "Read X", "Watch X"
- "Introduction to X", "Overview of X", "Basics of X"
- Two libraries without a concept: "NumPy and Matplotlib"

## Checkpoint progression (strictly enforced)
Checkpoint 1 — Core property: a specific, interesting property of the concept. NEVER "Basics of X".
Checkpoint 2 — Internal mechanism: the algorithm, data structure, or mechanism that makes it work.
Checkpoint 3 — Application in this project: connect to a specific file, function, struct, or behavior from this codebase.

## Output format — ONLY valid JSON, no markdown

{
  "checkpoints": [
    {
      "id": "checkpoint_X_Y_1",
      "title": "Concept name as a noun phrase",
      "mastery_criteria": "You can explain why...",
      "exercises": [
        "Specific actionable activity 1",
        "Specific actionable activity 2"
      ],
      "order": 1
    }
  ]
}

Respond with ONLY the JSON. No markdown, no explanation."#
}

// ─── Checkpoint expansion call ────────────────────────────────────────────────

async fn expand_skill_checkpoints(
    client: &reqwest::Client,
    tree_model: &str,
    tree_base_url: &str,
    tree_api_key: &str,
    phase_name: &str,
    skill: &OutlineSkill,
    project_context: &str,
    skill_index: usize,
    total_skills: usize,
    json_mode: bool,
    pool: Option<&sqlx::PgPool>,
    log_metadata: Option<serde_json::Value>,
) -> Result<Vec<Checkpoint>, String> {
    let expansion_tokens = CHECKPOINT_EXPANSION_MAX_TOKENS;

    // Defensive guard: if the constant ever becomes unreasonably small (e.g.
    // due to a future edit mistake), abort before wasting an API call.
    if expansion_tokens < 200 {
        return Err(format!(
            "CHECKPOINT_EXPANSION_MAX_TOKENS={} is too small (< 200) — refusing to call model for skill '{}'",
            expansion_tokens, skill.name
        ));
    }

    println!(
        "  🔧 Expanding skill {}/{}: '{}' (max_tokens={})",
        skill_index, total_skills, skill.name, expansion_tokens
    );

    let user_prompt = format!(
        "Project context:\n{}\n\nPhase: {}\nSkill: {}\nDescription: {}\n\nGenerate exactly 3 checkpoints for this skill.",
        project_context, phase_name, skill.name, skill.description
    );

    let (raw, exp_latency) = call_llm(
        client,
        tree_base_url,
        tree_api_key,
        tree_model,
        build_checkpoint_expansion_system_prompt(),
        &user_prompt,
        expansion_tokens,
        json_mode,
    )
    .await
    .map_err(|e| {
        if let Some(p) = pool {
            log_prompt_call(p.clone(), "skill_expansion", tree_model, "skill_expansion_v1", 0, false, Some(e.clone()), log_metadata.clone());
        }
        e
    })?;
    if let Some(p) = pool {
        log_prompt_call(p.clone(), "skill_expansion", tree_model, "skill_expansion_v1", exp_latency, true, None, log_metadata.clone());
    }

    let cleaned = clean_llm_json(&raw);
    let (repaired, was_repaired) = repair_truncated_tree_json(&cleaned);
    if was_repaired {
        println!("    ⚠️  Checkpoint JSON repaired for skill: {}", skill.name);
    }

    let expansion: CheckpointExpansion = serde_json::from_str(&repaired)
        .map_err(|e| format!("Checkpoint expansion parse failed for '{}': {} (raw: {}…)", skill.name, e, &raw[..raw.len().min(200)]))?;

    if expansion.checkpoints.is_empty() {
        return Err(format!("Skill '{}' expansion returned zero checkpoints", skill.name));
    }

    println!("    ✅ {} checkpoints generated", expansion.checkpoints.len());
    Ok(expansion.checkpoints)
}

// ─── Two-stage tree assembly ──────────────────────────────────────────────────

/// Run Stage 1 (outline) then Stage 2 (per-skill checkpoint expansion) and
/// assemble into a SkillTree. Used by both generate_skill_tree and analyze_repo.
async fn generate_tree_two_stage(
    client: &reqwest::Client,
    tree_model: &str,
    tree_base_url: &str,
    tree_api_key: &str,
    outline_system_prompt: &str,
    outline_user_prompt: &str,
    project_context: &str, // compact summary passed to each expansion call
    project_id: &str,
    json_mode: bool,
    pool: Option<&sqlx::PgPool>,
) -> Result<SkillTree, String> {
    // ── Stage 1: Outline ──────────────────────────────────────────────────────
    println!("🌿 Stage 1: generating outline (3×3 skeleton)…");
    let (raw_outline, outline_latency) = call_llm(
        client,
        tree_base_url,
        tree_api_key,
        tree_model,
        outline_system_prompt,
        outline_user_prompt,
        3000,
        json_mode,
    )
    .await
    .map_err(|e| {
        if let Some(p) = pool {
            log_prompt_call(p.clone(), "tree_outline", tree_model, "tree_outline_v1", 0, false, Some(e.clone()), Some(serde_json::json!({ "project_id": project_id })));
        }
        e
    })?;
    if let Some(p) = pool {
        log_prompt_call(p.clone(), "tree_outline", tree_model, "tree_outline_v1", outline_latency, true, None, Some(serde_json::json!({ "project_id": project_id })));
    }

    // Log raw preview before any cleanup
    let raw_preview_len = raw_outline.len().min(600);
    println!("🔎 Outline raw ({} chars): {}{}",
        raw_outline.len(),
        &raw_outline[..raw_preview_len],
        if raw_outline.len() > raw_preview_len { "…" } else { "" }
    );

    if raw_outline.trim().is_empty() {
        return Err("Outline JSON is empty — model returned no content".to_string());
    }

    // Step 1: strip fences / extract first JSON object
    let cleaned_outline = clean_llm_json(&raw_outline);

    // Step 2: fix `[ { {` duplicate-open-brace pattern
    let (deduped_outline, was_deduped) = repair_duplicate_open_braces(&cleaned_outline);
    if was_deduped {
        let deduped_preview = &deduped_outline[..deduped_outline.len().min(400)];
        println!("⚠️  Outline: duplicate-brace repair applied. After repair: {}…", deduped_preview);
    }

    // Step 3: close any truncation (unmatched braces/brackets)
    let (repaired_outline, was_truncation_repaired) = repair_truncated_tree_json(&deduped_outline);
    if was_truncation_repaired {
        println!("⚠️  Outline: truncation repair applied");
    }

    let mut outline: OutlineTree = serde_json::from_str(&repaired_outline)
        .map_err(|e| format!(
            "Outline JSON malformed after cleanup/repair: {} (raw preview: {}…)",
            e,
            &raw_outline[..raw_outline.len().min(300)]
        ))?;
    outline.project_id = project_id.to_string();

    println!(
        "✅ Outline complete: {} phases, {} total skills",
        outline.phases.len(),
        outline.phases.iter().map(|p| p.skills.len()).sum::<usize>()
    );

    // Fix 4: validate 3×3 structure — warn but continue
    if outline.phases.len() != 3 {
        println!(
            "⚠️  Outline has {} phases (expected 3) — continuing with what was generated",
            outline.phases.len()
        );
    }
    for (pi, phase) in outline.phases.iter().enumerate() {
        if phase.skills.len() != 3 {
            println!(
                "⚠️  Phase {} ('{}') has {} skills (expected 3) — continuing",
                pi + 1, phase.name, phase.skills.len()
            );
        }
    }

    // ── Stage 2: Per-skill checkpoint expansion ───────────────────────────────
    let total_skills: usize = outline.phases.iter().map(|p| p.skills.len()).sum();
    println!("🌲 Stage 2: expanding {} skills…", total_skills);

    let mut skill_index = 0usize;
    let mut assembled_phases: Vec<Phase> = Vec::new();

    for outline_phase in &outline.phases {
        let mut assembled_skills: Vec<Skill> = Vec::new();

        for outline_skill in &outline_phase.skills {
            skill_index += 1;
            let skill_meta = Some(serde_json::json!({ "project_id": project_id, "skill_name": outline_skill.name }));
            // Fix 5: single retry with 5s delay before aborting the whole tree
            let checkpoints = match expand_skill_checkpoints(
                client,
                tree_model,
                tree_base_url,
                tree_api_key,
                &outline_phase.name,
                outline_skill,
                project_context,
                skill_index,
                total_skills,
                json_mode,
                pool,
                skill_meta.clone(),
            )
            .await
            {
                Ok(cps) => cps,
                Err(first_err) => {
                    println!(
                        "  ⚠️  Skill '{}' expansion failed (attempt 1/2): {} — retrying in 5s…",
                        outline_skill.name, first_err
                    );
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    expand_skill_checkpoints(
                        client,
                        tree_model,
                        tree_base_url,
                        tree_api_key,
                        &outline_phase.name,
                        outline_skill,
                        project_context,
                        skill_index,
                        total_skills,
                        json_mode,
                        pool,
                        skill_meta,
                    )
                    .await
                    .map_err(|e| format!("Stage 2 failed at skill '{}' after 2 attempts: {}", outline_skill.name, e))?
                }
            };

            assembled_skills.push(Skill {
                id: outline_skill.id.clone(),
                name: outline_skill.name.clone(),
                description: outline_skill.description.clone(),
                order: outline_skill.order,
                checkpoints,
            });
        }

        assembled_phases.push(Phase {
            id: outline_phase.id.clone(),
            name: outline_phase.name.clone(),
            description: outline_phase.description.clone(),
            order: outline_phase.order,
            skills: assembled_skills,
        });
    }

    Ok(SkillTree {
        project_id: project_id.to_string(),
        phases: assembled_phases,
    })
}

// ─── Outline structure repair ────────────────────────────────────────────────

/// Fix the LLM habit of doubling the opening brace after an array bracket:
///   `"phases": [ { { "id": ...`  →  `"phases": [ { "id": ...`
///   `"skills": [ { { "id": ...`  →  `"skills": [ { "id": ...`
///
/// The model emits `[ {` (correct) then immediately emits another `{` before
/// the first key. This removes the spurious second `{` (and a matching extra
/// closing `}`) so the resulting JSON is structurally valid.
///
/// Strategy: scan for the byte sequence `[ { {` (with any whitespace) and
/// collapse the two consecutive `{` tokens into one. We also remove the
/// matching surplus closing `}` by decrementing the brace count on the way
/// back — but that's handled naturally by the `repair_truncated_tree_json`
/// helper if needed; here we only fix the OPEN side, which is what prevents
/// serde_json from parsing at all.
fn repair_duplicate_open_braces(s: &str) -> (String, bool) {
    // We use a simple state machine rather than regex to avoid a dependency.
    // Look for the pattern: '[' → optional-whitespace → '{' → optional-whitespace → '{'
    // and collapse the two '{' into one.
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    let mut repaired = false;

    while i < bytes.len() {
        // Detect '[' followed (through whitespace) by '{' followed (through whitespace) by '{'
        if bytes[i] == b'[' {
            out.push(bytes[i]);
            i += 1;
            // Skip whitespace
            let ws_start = i;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b'{' {
                // Emit everything up to and including this first '{'
                out.extend_from_slice(&bytes[ws_start..i]);
                out.push(b'{');
                i += 1;
                // Skip whitespace after first '{'
                let ws2_start = i;
                while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                if i < bytes.len() && bytes[i] == b'{' {
                    // This is the spurious duplicate — skip it (and any trailing whitespace)
                    i += 1;
                    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                        i += 1;
                    }
                    // Also drop one surplus closing '}' from the end to keep balance.
                    // We do this by marking that we need to remove the last bare '}'.
                    repaired = true;
                    // Don't emit ws2 — restart loop with i pointing at first key
                    continue;
                } else {
                    // No duplicate — emit the whitespace we consumed and continue
                    out.extend_from_slice(&bytes[ws2_start..i]);
                    continue;
                }
            } else {
                // Not a '{ {' pattern — emit the whitespace and continue
                out.extend_from_slice(&bytes[ws_start..i]);
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }

    if !repaired {
        return (s.to_string(), false);
    }

    // We removed N opening braces without removing matching closing braces,
    // so the JSON may now be unbalanced (extra '}'). Remove the last bare '}'
    // for each repair we made. A single scan from the end is sufficient for
    // the common case (1–3 duplicate pairs).
    //
    // Count how many extra '}' we need to drop: we removed exactly one '{' per
    // repair event, so we need to drop one '}' from the end.
    // We already set repaired=true once per '[{ {' occurrence; but the loop
    // above only sets it once (the flag, not a counter). Count properly:
    let removals_needed = {
        // Re-count occurrences in original string
        let orig = s.as_bytes();
        let mut count = 0usize;
        let mut j = 0usize;
        while j < orig.len() {
            if orig[j] == b'[' {
                let mut k = j + 1;
                while k < orig.len() && orig[k].is_ascii_whitespace() { k += 1; }
                if k < orig.len() && orig[k] == b'{' {
                    k += 1;
                    while k < orig.len() && orig[k].is_ascii_whitespace() { k += 1; }
                    if k < orig.len() && orig[k] == b'{' {
                        count += 1;
                    }
                }
            }
            j += 1;
        }
        count
    };

    // Remove `removals_needed` closing '}' from the end of `out`
    let mut removed = 0usize;
    let mut end = out.len();
    while removed < removals_needed && end > 0 {
        end -= 1;
        while end > 0 && out[end].is_ascii_whitespace() { end -= 1; }
        if out[end] == b'}' {
            out.remove(end);
            removed += 1;
            end = out.len();
        } else {
            break; // nothing left to safely remove
        }
    }

    match String::from_utf8(out) {
        Ok(fixed) => (fixed, true),
        Err(_) => (s.to_string(), false), // shouldn't happen — input was valid UTF-8
    }
}

// ─── Tree JSON repair helper ─────────────────────────────────────────────────

/// Attempt minimal repair of a truncated tree JSON string.
/// Handles the common case where the model output is cut off near the end and
/// is only missing the final closing brackets.
/// Returns (repaired_string, was_repaired).
fn repair_truncated_tree_json(s: &str) -> (String, bool) {
    if s.ends_with('}') {
        return (s.to_string(), false);
    }

    // Count brace/bracket imbalance
    let mut brace_depth = 0i32;
    let mut bracket_depth = 0i32;
    let mut in_string = false;
    let mut escape_next = false;
    for ch in s.chars() {
        if escape_next { escape_next = false; continue; }
        if ch == '\\' && in_string { escape_next = true; continue; }
        if ch == '"' { in_string = !in_string; continue; }
        if in_string { continue; }
        match ch {
            '{' => brace_depth += 1,
            '}' => brace_depth -= 1,
            '[' => bracket_depth += 1,
            ']' => bracket_depth -= 1,
            _ => {}
        }
    }

    if brace_depth <= 0 && bracket_depth <= 0 {
        // Already balanced — no repair needed
        return (s.to_string(), false);
    }

    // Build closing suffix: close any open string, then close arrays and objects
    let mut suffix = String::new();
    // If we're mid-string (odd number of unescaped quotes), close it
    if in_string {
        suffix.push('"');
    }
    // Close open arrays
    for _ in 0..bracket_depth.max(0) {
        suffix.push(']');
    }
    // Close open objects
    for _ in 0..brace_depth.max(0) {
        suffix.push('}');
    }

    let repaired = format!("{}{}", s.trim_end_matches(',').trim_end(), suffix);
    (repaired, true)
}

// ─── Prompt logging ──────────────────────────────────────────────────────────

/// Fire-and-forget insert into prompt_logs. Spawns its own task so it never
/// blocks the caller. All errors are silently swallowed (debug tool only).
pub(crate) fn log_prompt_call(
    pool: sqlx::PgPool,
    command: &str,
    model: &str,
    prompt_version: &str,
    latency_ms: i64,
    success: bool,
    error: Option<String>,
    metadata: Option<serde_json::Value>,
) {
    let command = command.to_string();
    let model = model.to_string();
    let prompt_version = prompt_version.to_string();
    tokio::spawn(async move {
        let id = uuid::Uuid::new_v4().to_string();
        let _ = sqlx::query(
            "INSERT INTO prompt_logs \
               (id, command, model, prompt_version, latency_ms, success, error, metadata) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"
        )
        .bind(&id)
        .bind(&command)
        .bind(&model)
        .bind(&prompt_version)
        .bind(latency_ms as i32)
        .bind(success)
        .bind(error.as_deref())
        .bind(&metadata)
        .execute(&pool)
        .await;
    });
}

// ─── LLM call helper (OpenAI-compatible API) ────────────────────────────────

/// Returns (content, latency_ms).
async fn call_llm(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    model: &str,
    system_prompt: &str,
    user_prompt: &str,
    max_tokens: u32,
    json_mode: bool,
) -> Result<(String, i64), String> {
    println!("📡 call_llm: model={} max_tokens={} json_mode={} url={}", model, max_tokens, json_mode, base_url);
    let mut request_body = json!({
        "model": model,
        "messages": [
            { "role": "system", "content": system_prompt },
            { "role": "user",   "content": user_prompt }
        ],
        "temperature": 0.7,
        "max_tokens": max_tokens,
    });
    if json_mode {
        request_body["response_format"] = json!({ "type": "json_object" });
    }

    let t0 = std::time::Instant::now();
    let mut attempts = 0u32;
    let max_retries = 3u32;
    let (status, response_text) = loop {
        attempts += 1;
        let resp = client
            .post(base_url)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await
            .map_err(|e| format!("HTTP request failed: {}", e))?;

        let st = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| format!("Failed to read response: {}", e))?;

        let retryable = (st == 429 || st.is_server_error()) && attempts < max_retries;
        if retryable {
            let wait = attempts * 10;
            println!(
                "⏳ API returned {} — retrying in {}s (attempt {}/{})…",
                st, wait, attempts, max_retries
            );
            tokio::time::sleep(std::time::Duration::from_secs(wait as u64)).await;
            continue;
        }
        break (st, text);
    };
    let latency_ms = t0.elapsed().as_millis() as i64;

    if !status.is_success() {
        return Err(format!("LLM API returned {}: {}", status, response_text));
    }

    let parsed: GroqResponse = serde_json::from_str(&response_text)
        .map_err(|e| format!("Failed to parse LLM response: {}\nRaw: {}", e, &response_text[..response_text.len().min(500)]))?;

    Ok((parsed.choices[0].message.content.clone(), latency_ms))
}

// ─── Commands ────────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn generate_skill_tree(
    project_id: String,
    prd_text: String,
    app: tauri::AppHandle,
    database: State<'_, Database>,
) -> Result<String, String> {
    println!("\n=== Generate Skill Tree (PRD) ===");
    println!("Project ID: {}", project_id);
    println!("PRD length: {} chars\n", prd_text.len());

    let client = reqwest::Client::new();

    // Phase 1: Extract concept dependency graph
    // Keep sorted concepts alive so we can build a PRD profile in phase 1b.
    let (graph_context, prd_profile_section) = match extract_concept_graph(&client, &prd_text, None, Some(&database.pool)).await {
        Ok(graph) => {
            println!(
                "🧠 Concept graph extracted: {} concepts",
                graph.concepts.len()
            );
            let sorted = topological_sort(graph.concepts);
            for (i, c) in sorted.iter().enumerate() {
                println!("  {}. {} — {}", i + 1, c.name, c.description);
            }
            let gc = build_graph_context(&sorted);

            // Fix 2: PRD profile — grounding step identical to repo path's build_repo_profile
            let profile = match build_prd_profile(&client, &prd_text, &sorted, Some(&database.pool)).await {
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

            (Some(gc), profile)
        }
        Err(e) => {
            println!("⚠️  Concept graph extraction failed: {} — falling back to single-phase generation", e);
            (None, String::new())
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
        &client,
        &tree_model,
        &tree_base_url,
        &tree_api_key,
        build_outline_system_prompt(),
        &outline_user_prompt,
        &expansion_context,
        &project_id,
        json_mode,
        Some(&database.pool),
    ).await?;
    skill_tree.project_id = project_id;

    println!(
        "✅ Tree generated: {} phases, {} total skills",
        skill_tree.phases.len(),
        skill_tree.phases.iter().map(|p| p.skills.len()).sum::<usize>()
    );

    let (tree_id, leaf_node_ids) = save_tree_to_database(&skill_tree, &database).await?;
    println!("💾 Saved to database with tree_id: {}", tree_id);

    auto_match_tree_nodes(&database.pool, &leaf_node_ids).await;
    crate::orchestrator::on_tree_generated(&database.pool, &app, &tree_id, &skill_tree.project_id).await;

    let mut response_json = serde_json::json!(skill_tree);
    response_json["tree_id"] = serde_json::json!(tree_id);

    Ok(serde_json::to_string_pretty(&response_json).unwrap())
}

async fn fetch_paper_text(
    paper_url: Option<&str>,
    paper_pdf_base64: Option<&str>,
) -> Result<Option<String>, String> {
    let client = reqwest::Client::new();

    if let Some(base64_data) = paper_pdf_base64 {
        println!("📄 Paper PDF upload: {} base64 chars", base64_data.len());
        let resp = client
            .post(format!("{}/fetch-pdf", SCRAPER_URL))
            .json(&json!({ "pdf_base64": base64_data, "filename": "paper.pdf" }))
            .timeout(std::time::Duration::from_secs(120))
            .send()
            .await
            .map_err(|e| format!("Paper PDF scraper request failed: {}", e))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("Paper PDF scraper returned error: {}", body));
        }
        let result: serde_json::Value = resp.json().await
            .map_err(|e| format!("Paper PDF scraper response parse failed: {}", e))?;
        let text = result["text"].as_str().unwrap_or("").to_string();
        if text.trim().is_empty() {
            return Err("Paper PDF produced no text (scanned/image-based?)".to_string());
        }
        println!("  ✅ Paper PDF extracted: {} chars", text.len());
        return Ok(Some(text));
    }

    if let Some(url) = paper_url {
        let url = url.trim();
        if url.is_empty() {
            return Ok(None);
        }

        if url.contains("arxiv.org/abs/") {
            let pdf_url = url.replace("/abs/", "/pdf/");
            println!("📄 arXiv paper: {} → {}", url, pdf_url);

            let pdf_bytes = client
                .get(&pdf_url)
                .header(reqwest::header::USER_AGENT, "Yggdrasil")
                .timeout(std::time::Duration::from_secs(60))
                .send()
                .await
                .map_err(|e| format!("arXiv PDF fetch failed: {}", e))?
                .error_for_status()
                .map_err(|e| format!("arXiv PDF returned error status: {}", e))?
                .bytes()
                .await
                .map_err(|e| format!("arXiv PDF body read failed: {}", e))?;

            let encoded = general_purpose::STANDARD.encode(&pdf_bytes);
            println!("  Downloaded {} bytes, forwarding to scraper", pdf_bytes.len());

            let resp = client
                .post(format!("{}/fetch-pdf", SCRAPER_URL))
                .json(&json!({ "pdf_base64": encoded, "filename": "arxiv.pdf" }))
                .timeout(std::time::Duration::from_secs(120))
                .send()
                .await
                .map_err(|e| format!("arXiv scraper request failed: {}", e))?;
            if !resp.status().is_success() {
                let body = resp.text().await.unwrap_or_default();
                return Err(format!("arXiv scraper returned error: {}", body));
            }
            let result: serde_json::Value = resp.json().await
                .map_err(|e| format!("arXiv scraper response parse failed: {}", e))?;
            let text = result["text"].as_str().unwrap_or("").to_string();
            if text.trim().is_empty() {
                return Err("arXiv PDF produced no text".to_string());
            }
            println!("  ✅ arXiv paper extracted: {} chars", text.len());
            return Ok(Some(text));
        }

        println!("📄 Paper URL (webpage): {}", url);
        let resp = client
            .post(format!("{}/fetch", SCRAPER_URL))
            .json(&json!({ "url": url }))
            .timeout(std::time::Duration::from_secs(60))
            .send()
            .await
            .map_err(|e| format!("Paper web scraper request failed: {}", e))?;
        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("Paper web scraper returned error: {}", body));
        }
        let result: serde_json::Value = resp.json().await
            .map_err(|e| format!("Paper web scraper response parse failed: {}", e))?;
        let text = result["text"]
            .as_str()
            .or_else(|| result["content"].as_str())
            .unwrap_or("")
            .to_string();
        if text.trim().is_empty() {
            return Err("Paper webpage produced no text".to_string());
        }
        println!("  ✅ Paper webpage extracted: {} chars", text.len());
        return Ok(Some(text));
    }

    Ok(None)
}

#[tauri::command]
pub async fn analyze_repo(
    project_id: String,
    github_url: String,
    paper_url: Option<String>,
    paper_pdf: Option<String>,
    app: tauri::AppHandle,
    database: State<'_, Database>,
) -> Result<String, String> {
    let (owner, repo) = parse_github_url(&github_url).ok_or_else(|| {
        "Invalid GitHub URL. Expected: https://github.com/owner/repo".to_string()
    })?;

    println!("\n=== Analyze Repo: {}/{} ===", owner, repo);

    // Optional reference paper — non-fatal if it fails
    let paper_text: Option<String> = match fetch_paper_text(
        paper_url.as_deref(),
        paper_pdf.as_deref(),
    ).await {
        Ok(t) => t,
        Err(e) => {
            println!("⚠️  Paper fetch failed (non-fatal): {}", e);
            None
        }
    };

    // Shared LLM client (reused for concept graph + tree gen)
    let llm_client = reqwest::Client::new();

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
    let (raw_graph, graph_context, relevant_file_paths) = match extract_concept_graph(&llm_client, &phase1_context, paper_text.as_deref(), Some(&database.pool)).await {
        Ok(graph) => {
            println!(
                "🧠 Concept graph extracted: {} concepts, {} relevant files",
                graph.concepts.len(),
                graph.relevant_files.len()
            );
            let sorted = topological_sort(graph.concepts.clone());
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
        match build_repo_profile(&llm_client, &context, graph, sorted, Some(&database.pool)).await {
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
        &llm_client,
        &tree_model,
        &tree_base_url,
        &tree_api_key,
        build_repo_outline_system_prompt(),
        &outline_user_prompt,
        &expansion_context,
        &project_id,
        json_mode,
        Some(&database.pool),
    ).await?;
    skill_tree.project_id = project_id;

    println!("✅ Repo analysis complete: {} phases", skill_tree.phases.len());

    let (tree_id, leaf_node_ids) = save_tree_to_database(&skill_tree, &database).await?;
    println!("💾 Saved repo tree with tree_id: {}", tree_id);

    auto_match_tree_nodes(&database.pool, &leaf_node_ids).await;
    crate::orchestrator::on_tree_generated(&database.pool, &app, &tree_id, &skill_tree.project_id).await;

    let mut response_json = serde_json::json!(skill_tree);
    response_json["tree_id"] = serde_json::json!(tree_id);

    Ok(serde_json::to_string_pretty(&response_json).unwrap())
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
