// src-tauri/src/brain.rs

use base64::{Engine as _, engine::general_purpose};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::json;
use tauri::State;
use uuid::Uuid;
use crate::database::Database;

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

#[derive(Debug, Serialize, Deserialize, Clone)]
struct Concept {
    id: String,
    name: String,
    description: String,
    prerequisites: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct ConceptGraph {
    concepts: Vec<Concept>,
    #[serde(default)]
    relevant_files: Vec<String>,
}

// ─── Concept graph extraction ────────────────────────────────────────────────

fn build_concept_graph_system_prompt() -> &'static str {
    r#"You are a knowledge graph architect. Extract the key concepts from this codebase/project and their prerequisite relationships.

If the user prompt includes a FILE PATHS section, also select up to 10 file paths that are most relevant to the extracted concepts. These files will be fetched for deeper analysis. Pick files that contain the core logic, not tests or configs.

Output ONLY valid JSON:
{
  "concepts": [
    {
      "id": "snake_case_id",
      "name": "Concept Name",
      "description": "One sentence — what this concept is and how it appears in this project",
      "prerequisites": ["id_of_prerequisite"]
    }
  ],
  "relevant_files": ["src/main.rs", "src/lib.rs"]
}

Rules:
- 8-20 concepts
- Concepts must be specific to this project — name the actual techniques, patterns, and algorithms the author used
- prerequisites must reference valid concept ids in this response
- No circular dependencies
- Foundational concepts have empty prerequisites array
- Every concept must be demonstrably present in the codebase
- relevant_files: up to 10 paths from the provided file list, most relevant to the concepts
- If no file paths are provided, omit relevant_files or return an empty array
Respond with ONLY the JSON."#
}

async fn extract_concept_graph(api_key: &str, context: &str) -> Result<ConceptGraph, String> {
    let user_prompt = format!(
        "Extract the concept dependency graph from this project:\n\n{}\n\nGenerate the concept graph JSON:",
        context
    );

    let concept_model = std::env::var("CONCEPT_GRAPH_MODEL")
        .unwrap_or_else(|_| "llama-3.3-70b-versatile".to_string());
    let concept_base_url = std::env::var("CONCEPT_GRAPH_BASE_URL")
        .unwrap_or_else(|_| "https://api.groq.com/openai/v1/chat/completions".to_string());
    let concept_api_key = std::env::var("CONCEPT_GRAPH_API_KEY")
        .unwrap_or_else(|_| api_key.to_string());
    println!("🧠 Concept graph model: {} via {}", concept_model, concept_base_url);

    let graph_json = call_llm(
        &concept_base_url,
        &concept_api_key,
        &concept_model,
        build_concept_graph_system_prompt(),
        &user_prompt,
    )
    .await?;

    // Parse via Value first to tolerate LLM quirks like duplicate keys
    let value: serde_json::Value = serde_json::from_str(&graph_json)
        .map_err(|e| format!("Concept graph JSON is not valid JSON: {}", e))?;
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

- 3-5 phases
- 2-4 skills per phase
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

- 3-5 phases
- 2-4 skills per phase
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

    let skip_prefixes = ["node_modules/", "target/", ".git/", "dist/", "build/", ".sqlx/", "__pycache__/", ".venv/"];

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
        ("sidecar/src/", &[".ts"]),
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

// ─── LLM call helper (OpenAI-compatible API) ────────────────────────────────

async fn call_llm(
    base_url: &str,
    api_key: &str,
    model: &str,
    system_prompt: &str,
    user_prompt: &str,
) -> Result<String, String> {
    let client = reqwest::Client::new();

    let request_body = json!({
        "model": model,
        "messages": [
            { "role": "system", "content": system_prompt },
            { "role": "user",   "content": user_prompt }
        ],
        "temperature": 0.7,
        "max_tokens": 8192,
        "response_format": { "type": "json_object" }
    });

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

        if st == 429 && attempts < max_retries {
            let wait = attempts * 10;
            println!(
                "⏳ Rate limited (429), retrying in {}s (attempt {}/{})…",
                wait, attempts, max_retries
            );
            tokio::time::sleep(std::time::Duration::from_secs(wait as u64)).await;
            continue;
        }
        break (st, text);
    };

    if !status.is_success() {
        return Err(format!("LLM API returned {}: {}", status, response_text));
    }

    let parsed: GroqResponse = serde_json::from_str(&response_text)
        .map_err(|e| format!("Failed to parse LLM response: {}\nRaw: {}", e, &response_text[..response_text.len().min(500)]))?;

    Ok(parsed.choices[0].message.content.clone())
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

    let groq_key = std::env::var("GROQ_API_KEY")
        .map_err(|_| "GROQ_API_KEY environment variable not set".to_string())?;

    // Phase 1: Extract concept dependency graph
    let graph_context = match extract_concept_graph(&groq_key, &prd_text).await {
        Ok(graph) => {
            println!(
                "🧠 Concept graph extracted: {} concepts",
                graph.concepts.len()
            );
            let sorted = topological_sort(graph.concepts);
            for (i, c) in sorted.iter().enumerate() {
                println!("  {}. {} — {}", i + 1, c.name, c.description);
            }
            Some(build_graph_context(&sorted))
        }
        Err(e) => {
            println!("⚠️  Concept graph extraction failed: {} — falling back to single-phase generation", e);
            None
        }
    };

    // Phase 2: Generate tree (with or without graph context)
    let user_prompt = if let Some(ref gc) = graph_context {
        format!(
            "Project ID: {}\n\n{}\n\nPRD:\n{}\n\nGenerate the skill tree JSON:",
            project_id, gc, prd_text
        )
    } else {
        format!(
            "Project ID: {}\n\nPRD:\n{}\n\nGenerate the skill tree JSON:",
            project_id, prd_text
        )
    };

    let tree_model = std::env::var("TREE_GEN_MODEL")
        .unwrap_or_else(|_| "moonshotai/kimi-k2".to_string());
    let tree_api_key = std::env::var("TREE_GEN_API_KEY")
        .unwrap_or_else(|_| std::env::var("OPENROUTER_API_KEY").unwrap_or_default());
    let tree_base_url = std::env::var("TREE_GEN_BASE_URL")
        .unwrap_or_else(|_| "https://openrouter.ai/api/v1/chat/completions".to_string());

    println!("🌲 Tree gen model: {}", tree_model);
    let tree_json = call_llm(&tree_base_url, &tree_api_key, &tree_model, &build_system_prompt(), &user_prompt).await?;

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

    let groq_key = std::env::var("GROQ_API_KEY")
        .map_err(|_| "GROQ_API_KEY environment variable not set".to_string())?;

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
            fetch_github_file(&gh, &url, 1000).await.unwrap_or_default()
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
    let (graph_context, relevant_file_paths) = match extract_concept_graph(&groq_key, &phase1_context).await {
        Ok(graph) => {
            println!(
                "🧠 Concept graph extracted: {} concepts, {} relevant files",
                graph.concepts.len(),
                graph.relevant_files.len()
            );
            let sorted = topological_sort(graph.concepts);
            for (i, c) in sorted.iter().enumerate() {
                println!("  {}. {} — {}", i + 1, c.name, c.description);
            }
            if !graph.relevant_files.is_empty() {
                println!("  📂 Relevant files: {:?}", graph.relevant_files);
            }
            (Some(build_graph_context(&sorted)), graph.relevant_files)
        }
        Err(e) => {
            println!("⚠️  Concept graph extraction failed: {} — falling back to single-phase generation", e);
            (None, vec![])
        }
    };

    // 10. Fetch targeted source files (Phase 1-driven or fallback)
    let mut files_to_fetch: Vec<String> = if relevant_file_paths.is_empty() {
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

    // Always include pinned files (core project files) if they exist in the repo
    let pinned = vec![
        "src-tauri/src/brain.rs",
        "sidecar/src/routes/match.ts",
        "sidecar/src/routes/ingest.ts",
        "sidecar/src/index.ts",
    ];
    for p in &pinned {
        let ps = p.to_string();
        if all_paths.contains(&ps) && !files_to_fetch.contains(&ps) {
            files_to_fetch.push(ps);
        }
    }

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

    println!("📦 Phase 2 context: {} chars", context.len());

    // 12. Generate tree (with or without graph context)
    let user_prompt = if let Some(ref gc) = graph_context {
        format!(
            "Project ID: {}\n\n{}\n\nRepository context:\n{}\n\nGenerate the learning skill tree JSON:",
            project_id, gc, context
        )
    } else {
        format!(
            "Project ID: {}\n\nRepository context:\n{}\n\nGenerate the learning skill tree JSON:",
            project_id, context
        )
    };

    let tree_model = std::env::var("TREE_GEN_MODEL")
        .unwrap_or_else(|_| "moonshotai/kimi-k2".to_string());
    let tree_api_key = std::env::var("TREE_GEN_API_KEY")
        .unwrap_or_else(|_| std::env::var("OPENROUTER_API_KEY").unwrap_or_default());
    let tree_base_url = std::env::var("TREE_GEN_BASE_URL")
        .unwrap_or_else(|_| "https://openrouter.ai/api/v1/chat/completions".to_string());

    println!("🌲 Tree gen model: {}", tree_model);
    let tree_json =
        call_llm(&tree_base_url, &tree_api_key, &tree_model, &build_repo_system_prompt(), &user_prompt).await?;

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
