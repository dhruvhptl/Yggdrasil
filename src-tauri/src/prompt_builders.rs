// src-tauri/src/prompt_builders.rs

use serde::{Deserialize, Serialize};
use serde_json::json;
use crate::constants::SCRAPER_URL;
use crate::llm_client::{
    call_llm, log_prompt_call, clean_llm_json, repair_duplicate_open_braces,
    repair_truncated_tree_json, string_or_int_default,
    SkillTree, Phase, Skill, Checkpoint, CHECKPOINT_EXPANSION_MAX_TOKENS,
};

// ─── Concept graph types ─────────────────────────────────────────────────────

/// One concept extracted from the project. Every field forces the LLM to be
/// evidence-grounded: it must name the concept type, cite files, and explain
/// why it's in this specific project — not generic knowledge.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct Concept {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) prerequisites: Vec<String>,
    /// "algorithm" | "data_structure" | "protocol" | "pattern" | "framework" |
    /// "language_feature" | "infrastructure" | "library"
    #[serde(default)]
    pub(crate) concept_type: String,
    /// Source files, manifest entries, or doc sections that demonstrate this
    /// concept is actually used in the project. Empty = LLM invented it.
    #[serde(default)]
    pub(crate) supporting_files: Vec<String>,
    /// One sentence: how this concept manifests in this specific codebase
    /// (e.g. "Used in mimir.rs to embed chunks via pplx-embed-v1-0.6b").
    #[serde(default)]
    pub(crate) project_relevance: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct ConceptGraph {
    pub(crate) concepts: Vec<Concept>,
    #[serde(default)]
    pub(crate) relevant_files: Vec<String>,
}

// ─── Repo profile types ───────────────────────────────────────────────────────

/// Intermediate summary of what the repo actually is, produced after the
/// concept graph but before full tree generation. Forces a grounding step:
/// the tree generator works from this profile, not from raw repo dumps.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct RepoProfile {
    /// One-paragraph plain-English summary of what the project does.
    pub(crate) summary: String,
    /// Detected stack: language(s), frameworks, databases, key libraries.
    pub(crate) stack: Vec<String>,
    /// Distinct subsystems or modules (e.g. "Mimir ingestion pipeline",
    /// "Canvas tree renderer", "GitHub repo analyser").
    pub(crate) subsystems: Vec<String>,
    /// List of concept ids from the ConceptGraph that are well-evidenced.
    /// The tree generator should prioritise these.
    pub(crate) evidenced_concept_ids: Vec<String>,
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

pub(crate) async fn extract_concept_graph(
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
        0.7,
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

pub(crate) fn topological_sort(concepts: Vec<Concept>) -> Vec<Concept> {
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

pub(crate) fn build_graph_context(sorted_concepts: &[Concept]) -> String {
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

pub(crate) async fn build_repo_profile(
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
        0.7,
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
pub(crate) async fn build_prd_profile(
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
        0.7,
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

// ─── Legacy monolithic system prompts (kept for reference, not currently used) ─

#[allow(dead_code)]
pub(crate) fn build_system_prompt(mastered_concepts: &[String]) -> String {
    let base = r#"You are a learning path architect for Yggdrasil, a skill tree app that helps developers deeply understand projects they've built.

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
"#;

    if mastered_concepts.is_empty() {
        base.to_string()
    } else {
        format!(
            "{}\n## ALREADY MASTERED\nThe user has already completed these concepts — do not generate checkpoints for them.\nBuild on this knowledge: go deeper, explore adjacent concepts, or cover advanced applications.\n\nMastered concepts: {}",
            base,
            mastered_concepts.join(", ")
        )
    }
}

#[allow(dead_code)]
pub(crate) fn build_repo_system_prompt(mastered_concepts: &[String]) -> String {
    let base = r#"You are a learning path architect for Yggdrasil, a skill tree app that helps developers deeply understand projects they've built.

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
"#;

    if mastered_concepts.is_empty() {
        base.to_string()
    } else {
        format!(
            "{}\n## ALREADY MASTERED\nThe user has already completed these concepts — do not generate checkpoints for them.\nBuild on this knowledge: go deeper, explore adjacent concepts, or cover advanced applications.\n\nMastered concepts: {}",
            base,
            mastered_concepts.join(", ")
        )
    }
}

// ─── Outline system prompts ───────────────────────────────────────────────────

pub(crate) fn build_outline_system_prompt(mastered_concepts: &[String]) -> String {
    let base = r#"You are a learning path architect for Yggdrasil.

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

Respond with ONLY the JSON. No markdown, no explanation."#;

    if mastered_concepts.is_empty() {
        base.to_string()
    } else {
        format!(
            "{}\n\n## ALREADY MASTERED\nThe user has already completed these concepts — do not generate checkpoints for them.\nBuild on this knowledge: go deeper, explore adjacent concepts, or cover advanced applications.\n\nMastered concepts: {}",
            base,
            mastered_concepts.join(", ")
        )
    }
}

pub(crate) fn build_repo_outline_system_prompt(mastered_concepts: &[String]) -> String {
    let base = r#"You are a learning path architect for Yggdrasil.

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

Respond with ONLY the JSON. No markdown, no explanation."#;

    if mastered_concepts.is_empty() {
        base.to_string()
    } else {
        format!(
            "{}\n\n## ALREADY MASTERED\nThe user has already completed these concepts — do not generate checkpoints for them.\nBuild on this knowledge: go deeper, explore adjacent concepts, or cover advanced applications.\n\nMastered concepts: {}",
            base,
            mastered_concepts.join(", ")
        )
    }
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
        0.7,
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
pub(crate) async fn generate_tree_two_stage(
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
    mastered_concepts: &[String],
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
        0.7,
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

    let prereq_line = if mastered_concepts.is_empty() {
        String::new()
    } else {
        format!("\n\nPreviously mastered: {}", mastered_concepts.join(", "))
    };
    let expansion_context_with_prereqs = if prereq_line.is_empty() {
        project_context.to_string()
    } else {
        format!("{}{}", project_context, prereq_line)
    };

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
                &expansion_context_with_prereqs,
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
                        &expansion_context_with_prereqs,
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

// ─── fetch_paper_text ─────────────────────────────────────────────────────────

pub(crate) async fn fetch_paper_text(
    client: &reqwest::Client,
    paper_url: Option<&str>,
    paper_pdf_base64: Option<&str>,
) -> Result<Option<String>, String> {
    use base64::{Engine as _, engine::general_purpose};

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
