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
    #[serde(default)]
    pub order: i32,
    pub skills: Vec<Skill>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Skill {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub order: i32,
    pub quests: Vec<Quest>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Quest {
    pub id: String,
    pub title: String,
    pub description: String,
    #[serde(default = "default_estimated_hours")]
    pub estimated_hours: f32,
    #[serde(default = "default_difficulty")]
    pub difficulty: String,
    #[serde(default)]
    pub order: i32,
}

fn default_estimated_hours() -> f32 { 2.0 }
fn default_difficulty() -> String { "medium".to_string() }

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
}

// ─── Concept graph extraction ────────────────────────────────────────────────

fn build_concept_graph_system_prompt() -> &'static str {
    r#"You are a knowledge graph architect. Extract the key concepts from this codebase/project and their prerequisite relationships.

Output ONLY valid JSON:
{
  "concepts": [
    {
      "id": "snake_case_id",
      "name": "Concept Name",
      "description": "One sentence — what this concept is and how it appears in this project",
      "prerequisites": ["id_of_prerequisite"]
    }
  ]
}

Rules:
- 8-20 concepts
- Concepts must be specific to this project — name the actual techniques, patterns, and algorithms the author used
- prerequisites must reference valid concept ids in this response
- No circular dependencies
- Foundational concepts have empty prerequisites array
- Every concept must be demonstrably present in the codebase
Respond with ONLY the JSON."#
}

async fn extract_concept_graph(api_key: &str, context: &str) -> Result<ConceptGraph, String> {
    let user_prompt = format!(
        "Extract the concept dependency graph from this project:\n\n{}\n\nGenerate the concept graph JSON:",
        context
    );

    let graph_json = call_openrouter(
        api_key,
        build_concept_graph_system_prompt(),
        &user_prompt,
    )
    .await?;

    let graph: ConceptGraph = serde_json::from_str(&graph_json)
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
    r#"You are a learning path architect for Yggdrasil, a skill tree app that helps developers deeply understand projects they've built.

Given a PRD (Product Requirements Document), generate a learning skill tree — a structured path through every concept behind this project.

## Your Role

You generate LEARNING trees, not implementation plans. The user already built or is building the project. They need to understand the concepts behind it at a deep level.

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
          "quests": [
            {
              "id": "quest_1_1_1",
              "title": "Specific concept-focused quest title",
              "description": "What to study and what understanding to gain",
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

## Quest Quality Rules — ENFORCE STRICTLY

### BANNED quest title patterns (NEVER generate these):
- ANY title starting with "Learn" — e.g. "Learn Python Basics", "Learn about X", "Learn X and Y"
- "Explore X", "Introduction to X", "Overview of X"
- "Watch X on YouTube", "Read X on Stack Overflow", "Study X on GitHub"
- "Understanding X" as a complete title
- "Understand the basics of X" — replace with the specific concept
- Any title that names a website or platform instead of a concept
- Titles that name two libraries without a specific concept

### REQUIRED quest format:
- Every quest title must describe a SPECIFIC concept, mechanism, or algorithm to understand
- Every quest must answer: what will the learner understand or be able to do after completing it?
- Reference the actual technology, algorithm, or pattern by name
- Quests within a skill must form a PROGRESSION (each builds on the previous)
- The title should be a complete thought: what to study AND what insight is gained

### Good quest title examples:
- "Understand how Python classes use __init__ and self to encapsulate state"
- "Study how NumPy's vectorized operations eliminate Python for-loops for performance"
- "Work through the velocity-Verlet integration algorithm and why it conserves energy better than Euler"
- "Understand Matplotlib's Figure/Axes architecture and the difference between pyplot and OOP interface"
- "Study how React's reconciliation algorithm diffs virtual DOM trees to minimize repaints"
- "Understand how PostgreSQL's query planner chooses between sequential and index scans"

### Bad quest title examples (NEVER generate):
- "Learn Python Basics" — starts with Learn, too vague
- "Learn about NumPy arrays" — starts with Learn
- "Explore NumPy and Matplotlib" — names two libraries with no concept
- "Watch Python tutorials on YouTube" — names a platform, not a concept
- "Introduction to Quantum Mechanics" — generic, no specific mechanism
- "Understand the basics of X" — too vague, name the specific concept

## Quest Count: Exactly 3 per skill

The three quests within each skill must form a progression. Write the titles as specific, complete statements of what the learner will understand.

**Quest 1 — A specific property (difficulty: "easy")**
State one interesting, named property of this concept. The phrase "basics of" is ABSOLUTELY FORBIDDEN.
- "Understand the basics of React" → BANNED. Replace with: "Understand how React's virtual DOM lets the reconciliation algorithm batch DOM writes into a single paint cycle"
- "Understand the basics of PostgreSQL" → BANNED. Replace with: "Understand how PostgreSQL's MVCC model allows readers and writers to proceed concurrently without locking"
- "Understand the basics of NumPy" → BANNED. Replace with: "Understand how NumPy arrays store data in contiguous memory blocks and why this enables vectorized operations"

**Quest 2 — Internal mechanism (difficulty: "medium")**
Describe the algorithm, data structure, or internal mechanism by name.
CORRECT: "Study how React's reconciliation algorithm uses the fiber tree to diff component output and batch DOM updates"
CORRECT: "Work through the velocity-Verlet integration algorithm and why it conserves energy better than Euler"

**Quest 3 — This specific project (difficulty: "medium" or "hard")**
Connect to a specific feature, design choice, or constraint from the PRD. "Apply X to a real-world problem" is BANNED.
CORRECT: "Understand why Yggdrasil's skill tree uses pgvector cosine distance rather than exact search for semantic matching"
WRONG: "Apply React to a real-world problem" — BANNED, generic
WRONG: "Use X to solve a real-world problem" — BANNED, names nothing specific

## Structure

- 3-5 phases (Foundation → Core → Advanced)
- 2-4 skills per phase
- Exactly 3 quests per skill
- Every phase must map to real concepts in the PRD — no filler phases
- Never add generic "Best Practices", "Code Quality", or "Testing and Debugging" skills unless the PRD specifically requires them

## Phase Design

- **Foundation**: Truly foundational concepts needed before anything else
- **Core**: The main technical skills the project actually uses
- **Advanced**: Deeper understanding of the hardest or most interesting parts

## Concept Dependency Graph

You will receive a CONCEPT DEPENDENCY ORDER section in the user prompt. This is a topologically sorted list of concepts extracted from this project — foundational concepts first, advanced last.

Use this graph to:
- Determine phase ordering: earlier concepts belong in earlier phases
- Determine skill sequencing within phases
- Ensure no skill assumes knowledge of a concept listed after it
- Generate quest progressions that follow the dependency order

The graph represents actual techniques and patterns present in this codebase. Every quest should connect back to a concept in this graph.

## Difficulty

Must use exactly: "easy", "medium", or "hard"

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
- **Dependency files** (requirements.txt, Cargo.toml, package.json, etc.): Every listed library is a skill candidate. What does it actually do? How does it work?
- **Source files**: See exactly which patterns, algorithms, and APIs are used in practice — generate quests for these specific mechanisms, not just the library names
- **README + Description**: Understand the project's purpose to make quests relevant to this specific context
- **Commit history**: Understand where the developer spent effort — these areas need the deepest quests
- **Open issues**: Known pain points and areas of complexity worth studying
- **Merged PRs**: What problems were actively solved — these mechanics are worth a quest

## How to Generate Skills from Source Code

Look at the actual code:
- If the repo uses OOP heavily → add a skill on that OOP pattern
- If it uses specific numerical methods → name the method explicitly in a quest
- If it imports a visualization library and uses 3D plotting → generate quests for 3D projection mechanics
- If it uses JIT compilation → generate quests for how JIT works
- If it implements a specific algorithm explicitly → name the algorithm in the quest

Do NOT add generic skill names. Every skill must name a specific technology, library, or concept from this actual codebase.

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
          "quests": [
            {
              "id": "quest_1_1_1",
              "title": "Specific concept-focused quest title",
              "description": "What to study and what understanding to gain",
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

## Quest Quality Rules — ENFORCE STRICTLY

### BANNED quest title patterns (NEVER generate these):
- ANY title starting with "Learn" — e.g. "Learn Python Basics", "Learn about NumPy", "Learn X and Y"
- "Explore X", "Introduction to X", "Overview of X"
- "Watch X on YouTube", "Read X on Stack Overflow", "Study X on GitHub"
- "Understanding X" as a complete title (e.g. "Understanding NumPy")
- "Understand the basics of X" — too vague, replace with the specific concept
- "X and its applications" — too generic
- Any title that names a website or platform instead of a concept
- Titles that name two libraries without naming a specific concept: "Explore NumPy and Matplotlib"

### REQUIRED quest format:
- Every quest title must describe a SPECIFIC concept, mechanism, or algorithm to understand
- Every quest must answer: what will the learner understand or be able to do after completing it?
- Reference the actual technology, algorithm, or data structure by name — use the exact names from the source code
- Quests within a skill must form a PROGRESSION (each builds on the previous)
- The title should be a complete thought: what to study AND what insight is gained

### Good quest title examples:
- "Understand how Python classes use __init__ and self to encapsulate state"
- "Study how NumPy's vectorized operations eliminate Python for-loops for performance"
- "Work through the velocity-Verlet integration algorithm and why it conserves energy better than Euler"
- "Understand Matplotlib's Figure/Axes architecture and the difference between pyplot and OOP interface"
- "Study how Numba's @jit decorator compiles Python to machine code at runtime"
- "Understand 3D plotting with Matplotlib's Axes3D — projections, viewing angles, and camera transforms"
- "Study how pgvector's HNSW index approximates nearest-neighbor search without scanning all vectors"
- "Work through how Julia's multiple dispatch selects method implementations at compile time"

### Bad quest title examples (NEVER generate):
- "Learn Python Basics" — starts with Learn, too vague
- "Learn about NumPy's array data structure" — starts with Learn
- "Learn about the Velocity-Verlet integrator" — starts with Learn
- "Explore NumPy and Matplotlib" — names two libraries with no concept
- "Watch Python tutorials on YouTube" — names a platform, not a concept
- "Introduction to Quantum Mechanics" — generic, no specific mechanism
- "Understand the basics of N-body simulation" — "basics of" is too vague
- "Best Practices for Code Quality" — generic filler skill
- "Testing and Debugging" — generic unless the repo has actual test files

## Quest Count: Exactly 3 per skill

The three quests within each skill must form a progression. Write the titles as specific, complete statements of what the learner will understand.

**Quest 1 — A specific property (difficulty: "easy")**
State one interesting, named property of this concept. Template: "Understand how [SPECIFIC THING] works" or "Study why [SPECIFIC THING] matters".
The phrase "basics of" is ABSOLUTELY FORBIDDEN in Quest 1. Replace it with the actual property.
- "Understand the basics of Velocity-Verlet" → BANNED. Replace with: "Understand why Velocity-Verlet conserves time-reversal symmetry while Euler integration does not"
- "Understand the basics of Julia" → BANNED. Replace with: "Understand how Julia's type inference lets the JIT compiler generate C-speed machine code without explicit type annotations"
- "Understand the basics of Makie.jl" → BANNED. Replace with: "Understand how Makie.jl's Observable system allows animated plots to update reactively when data changes"
- "Understand the basics of NumPy" → BANNED. Replace with: "Understand how NumPy arrays store data in contiguous memory blocks and why this enables vectorized operations"

**Quest 2 — Internal mechanism (difficulty: "medium")**
Describe the algorithm, data structure, or internal mechanism by name.
CORRECT: "Work through the velocity-Verlet integration algorithm and why it conserves energy better than Euler"
CORRECT: "Study how Numba's @jit decorator compiles Python to machine code at runtime"

**Quest 3 — This specific project (difficulty: "medium" or "hard")**
Name a specific class, function, file, or behavior from THIS codebase. "Apply X to a real-world problem" and "Study how the project uses X" are BANNED — too generic.
CORRECT: "Study how the Body class in common.py uses NumPy arrays to represent position and velocity vectors for each planet"
CORRECT: "Understand why the benchmark in benchmark_python.py measures Numba's first-call JIT cost separately from subsequent calls"
WRONG: "Apply NumPy to a real-world problem" — BANNED, generic
WRONG: "Study how the project uses Numba to improve performance" — BANNED, says nothing specific

## Structure

- 3-5 phases (Foundation → Core → Advanced)
- 2-4 skills per phase
- Exactly 3 quests per skill
- Every skill must be derived from actual evidence in the repository data
- Never add "Best Practices", "Code Quality", or "Testing and Debugging" unless the repo has test files

## Phase Design

- **Foundation**: Language fundamentals and core data structures the codebase depends on
- **Core**: The main libraries, algorithms, and patterns the project actively uses
- **Advanced**: The deepest or most complex concepts — the things that make this project actually hard

## Concept Dependency Graph

You will receive a CONCEPT DEPENDENCY ORDER section in the user prompt. This is a topologically sorted list of concepts extracted from this project — foundational concepts first, advanced last.

Use this graph to:
- Determine phase ordering: earlier concepts belong in earlier phases
- Determine skill sequencing within phases
- Ensure no skill assumes knowledge of a concept listed after it
- Generate quest progressions that follow the dependency order

The graph represents actual techniques and patterns present in this codebase. Every quest should connect back to a concept in this graph.

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
        format!("{}…", &content[..max_chars])
    } else {
        content
    })
}

// ─── OpenRouter call helper (tree generation only) ───────────────────────────

async fn call_openrouter(
    api_key: &str,
    system_prompt: &str,
    user_prompt: &str,
) -> Result<String, String> {
    let client = reqwest::Client::new();

    let request_body = json!({
        "model": "moonshotai/kimi-k2",
        "messages": [
            { "role": "system", "content": system_prompt },
            { "role": "user",   "content": user_prompt }
        ],
        "temperature": 0.7,
        "max_tokens": 4096,
        "response_format": { "type": "json_object" }
    });

    let response = client
        .post("https://openrouter.ai/api/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .header("HTTP-Referer", "http://localhost")
        .header("X-Title", "Yggdrasil")
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
        return Err(format!("OpenRouter API returned {}: {}", status, response_text));
    }

    let parsed: GroqResponse = serde_json::from_str(&response_text)
        .map_err(|e| format!("Failed to parse OpenRouter response: {}\nRaw: {}", e, &response_text[..response_text.len().min(500)]))?;

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

    let env_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".env");
    dotenv::from_path(&env_path).ok();
    dotenv::dotenv().ok();
    let api_key = std::env::var("OPENROUTER_API_KEY")
        .map_err(|_| "OPENROUTER_API_KEY not found in .env file".to_string())?;

    // Phase 1: Extract concept dependency graph
    let graph_context = match extract_concept_graph(&api_key, &prd_text).await {
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

    println!("Calling OpenRouter (kimi-k2)…");
    let tree_json = call_openrouter(&api_key, &build_system_prompt(), &user_prompt).await?;

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
    let api_key = std::env::var("OPENROUTER_API_KEY")
        .map_err(|_| "OPENROUTER_API_KEY not found in .env file".to_string())?;

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

    // 2. Root directory listing (needed for all subsequent fetches)
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

    // 3. README — find it in root listing, fetch via /contents/{name}
    let readme_text = {
        let readme_name = root_files.iter().find(|f| {
            let lower = f.to_lowercase();
            lower == "readme.md"
                || lower == "readme.rst"
                || lower == "readme.txt"
                || lower == "readme"
        });
        if let Some(name) = readme_name {
            let url = format!("{}/contents/{}", base, name);
            fetch_github_file(&gh, &url, 3000).await.unwrap_or_default()
        } else {
            String::new()
        }
    };
    println!("  README: {} chars", readme_text.len());

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
            let url = format!("{}/contents/{}", base, dep_file);
            if let Some(content) = fetch_github_file(&gh, &url, 2000).await {
                dep_contents.push((dep_file.to_string(), content));
                println!("  Fetched: {}", dep_file);
            }
        }
    }

    // 5. Key source files
    let entry_point_candidates = [
        "main.rs", "lib.rs", "main.py", "app.py", "server.py",
        "index.ts", "index.js", "app.ts", "app.js", "main.ts",
        "main.go", "app.go", "server.go",
        "Main.java", "App.java",
        "server.js", "server.ts",
        "index.py",
    ];
    let mut source_files: Vec<(String, String)> = Vec::new();

    for candidate in &entry_point_candidates {
        if source_files.len() >= 3 { break; }
        if root_files.iter().any(|f| f.as_str() == *candidate) {
            let url = format!("{}/contents/{}", base, candidate);
            if let Some(content) = fetch_github_file(&gh, &url, 1500).await {
                source_files.push((candidate.to_string(), content));
                println!("  Source: {}", candidate);
            }
        }
    }

    // Check common source subdirectories: src/, python/, julia/, lib/, core/, cmd/
    let source_subdirs = ["src", "python", "julia", "lib", "core", "cmd"];
    for subdir in &source_subdirs {
        if source_files.len() >= 5 { break; }
        if root_files.iter().any(|f| f.as_str() == *subdir) {
            if let Ok(r) = gh.get(format!("{}/contents/{}", base, subdir)).send().await {
                if r.status().is_success() {
                    let dir_list: Vec<serde_json::Value> = r.json().await.unwrap_or_default();
                    let interesting: Vec<String> = dir_list
                        .iter()
                        .filter_map(|f| {
                            let name = f["name"].as_str()?;
                            let ftype = f["type"].as_str().unwrap_or("");
                            if ftype == "file"
                                && !name.contains("test")
                                && !name.contains("spec")
                                && !name.ends_with(".lock")
                            {
                                Some(name.to_string())
                            } else {
                                None
                            }
                        })
                        .collect();
                    for src_file in interesting.iter().take(5 - source_files.len()) {
                        let url = format!("{}/contents/{}/{}", base, subdir, src_file);
                        if let Some(content) = fetch_github_file(&gh, &url, 1500).await {
                            source_files.push((format!("{}/{}", subdir, src_file), content));
                            println!("  Source: {}/{}", subdir, src_file);
                        }
                    }
                }
            }
        }
    }
    // Fallback: if no known entry points matched, pick source files from root by extension
    if source_files.is_empty() {
        let src_extensions = [".py", ".js", ".ts", ".rs", ".go", ".java", ".cpp", ".c", ".rb"];
        let skip_prefixes = ["setup", "conftest", "manage", "wsgi", "asgi"];
        let root_sources: Vec<String> = root_files
            .iter()
            .filter(|f| {
                let lower = f.to_lowercase();
                src_extensions.iter().any(|ext| lower.ends_with(ext))
                    && !skip_prefixes.iter().any(|s| lower.starts_with(s))
                    && !lower.contains("test")
                    && !lower.contains("spec")
            })
            .cloned()
            .collect();
        for src_file in root_sources.iter().take(3) {
            let url = format!("{}/contents/{}", base, src_file);
            if let Some(content) = fetch_github_file(&gh, &url, 1500).await {
                source_files.push((src_file.to_string(), content));
                println!("  Source (fallback): {}", src_file);
            }
        }
    }
    println!("  Source files fetched: {}", source_files.len());

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
                    let msg = if first_line.len() > 80 { &first_line[..80] } else { first_line };
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

    // 9. Build context
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

    println!("  Context built: {} chars", context.len());

    // 10. Phase 1: Extract concept dependency graph
    let graph_context = match extract_concept_graph(&api_key, &context).await {
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

    // 11. Phase 2: Generate tree (with or without graph context)
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

    println!("Calling OpenRouter (kimi-k2) for repo analysis…");
    let tree_json =
        call_openrouter(&api_key, &build_repo_system_prompt(), &user_prompt).await?;

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
