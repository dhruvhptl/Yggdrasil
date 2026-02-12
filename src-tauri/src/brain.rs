// src-tauri/src/brain.rs

use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::State;
use uuid::Uuid;
use chrono::Utc;
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

/// Save AI-generated skill tree to database
async fn save_tree_to_database(
    skill_tree: &SkillTree,
    database: &Database
) -> Result<String, String> {
    let tree_id = Uuid::new_v4().to_string();
    let tree_name = format!("AI Generated - {}", Utc::now().format("%Y-%m-%d %H:%M"));
    let created_at = Utc::now().to_rfc3339();

    // Create tree record
    sqlx::query!(
        "INSERT INTO trees (id, project_id, name, created_at) VALUES (?1, ?2, ?3, ?4)",
        tree_id,
        skill_tree.project_id,
        tree_name,
        created_at
    ).execute(&database.pool).await.map_err(|e| format!("Failed to create tree: {}", e))?;

    println!("📦 Created tree: {}", tree_id);

    // Track node IDs for edge creation
    let mut node_ids: Vec<(String, Option<String>)> = Vec::new(); // (node_id, parent_id)
    let mut order_counter = 0;

    // Create nodes for each phase → skill → quest
    for phase in &skill_tree.phases {
        // Create trunk node for phase
        let phase_node_id = Uuid::new_v4().to_string();
        let phase_tasks = serde_json::json!([]).to_string();

        sqlx::query!(
            "INSERT INTO tree_nodes (id, tree_id, parent_id, type, title, description, progress, tasks, resources, x, y, order_index)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            phase_node_id,
            tree_id,
            None::<String>, // root level
            "trunk",
            phase.name,
            phase.description,
            0i64,
            phase_tasks,
            None::<String>,
            None::<f64>,
            None::<f64>,
            order_counter
        ).execute(&database.pool).await.map_err(|e| format!("Failed to create phase node: {}", e))?;

        node_ids.push((phase_node_id.clone(), None));
        order_counter += 1;

        println!("  🌳 Phase: {}", phase.name);

        // Create branch nodes for skills
        for skill in &phase.skills {
            let skill_node_id = Uuid::new_v4().to_string();
            let skill_tasks = serde_json::json!([]).to_string();

            sqlx::query!(
                "INSERT INTO tree_nodes (id, tree_id, parent_id, type, title, description, progress, tasks, resources, x, y, order_index)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                skill_node_id,
                tree_id,
                phase_node_id, // parent is phase
                "branch",
                skill.name,
                skill.description,
                0i64,
                skill_tasks,
                None::<String>,
                None::<f64>,
                None::<f64>,
                order_counter
            ).execute(&database.pool).await.map_err(|e| format!("Failed to create skill node: {}", e))?;

            node_ids.push((skill_node_id.clone(), Some(phase_node_id.clone())));
            order_counter += 1;

            println!("    🌿 Skill: {}", skill.name);

            // Create leaf nodes for quests
            for quest in &skill.quests {
                let quest_node_id = Uuid::new_v4().to_string();

                // Store quest metadata in tasks JSON
                let quest_metadata = serde_json::json!([{
                    "id": quest.id,
                    "title": quest.title,
                    "description": quest.description,
                    "estimated_hours": quest.estimated_hours,
                    "difficulty": quest.difficulty,
                    "completed": false
                }]).to_string();

                sqlx::query!(
                    "INSERT INTO tree_nodes (id, tree_id, parent_id, type, title, description, progress, tasks, resources, x, y, order_index)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                    quest_node_id,
                    tree_id,
                    skill_node_id, // parent is skill
                    "leaf",
                    quest.title,
                    quest.description,
                    0i64,
                    quest_metadata,
                    None::<String>,
                    None::<f64>,
                    None::<f64>,
                    order_counter
                ).execute(&database.pool).await.map_err(|e| format!("Failed to create quest node: {}", e))?;

                node_ids.push((quest_node_id.clone(), Some(skill_node_id.clone())));
                order_counter += 1;

                println!("      🍃 Quest: {} ({:.1}h, {})", quest.title, quest.estimated_hours, quest.difficulty);
            }
        }
    }

    // Create edges between parent-child nodes
    for (node_id, parent_id) in &node_ids {
        if let Some(parent) = parent_id {
            let edge_id = Uuid::new_v4().to_string();
            sqlx::query!(
                "INSERT INTO tree_edges (id, tree_id, source_node_id, target_node_id) VALUES (?1, ?2, ?3, ?4)",
                edge_id,
                tree_id,
                parent,
                node_id
            ).execute(&database.pool).await.map_err(|e| format!("Failed to create edge: {}", e))?;
        }
    }

    println!("✅ Saved tree with {} nodes and {} edges", node_ids.len(), node_ids.iter().filter(|(_, p)| p.is_some()).count());

    Ok(tree_id)
}

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
      "name": "Phase name (e.g., 'Foundations', 'Core Concepts', 'Advanced Topics')",
      "description": "What concepts this phase covers",
      "order": 1,
      "skills": [
        {
          "id": "skill_1_1",
          "name": "Skill name (specific concept or topic area)",
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

Generate quests that are LEARNING actions, such as:
- "Read Chapter X from [Resource]"
- "Watch lecture series on [Topic] from [Resource]"
- "Study [Concept] and take notes from [Resource]"
- "Work through practice problems for [Topic] from [Resource]"
- "Complete exercises 1-5 from [Resource]"
- "Review and annotate [Article/Paper]"

DO NOT generate implementation quests like:
- "Implement X feature"
- "Build Y component"
- "Write tests for Z"
- "Create a demo"

## Resource Integration

Extract resources from the PRD and reference them in quest descriptions:
- If PRD mentions "[Resource Title](URL)", use it: "Read Chapter 3 from Resource Title"
- Attach URLs to relevant quests based on context
- Prefer specific chapters/sections over vague references

## Structure Guidelines

- **Phases**: 3-5 phases representing learning progression (Foundation → Core → Advanced)
- **Skills per phase**: 2-4 skills (specific topic areas)
- **Quests per skill**: 3-6 quests (concrete learning actions)
- **Total time**: Match the timeline in the PRD

## Time Estimates

- estimated_hours should reflect LEARNING time (reading, watching, practicing)
- Easy: 0.5-2 hours (introductory material, videos)
- Medium: 2-4 hours (textbook chapters, problem sets)
- Hard: 4-8 hours (complex topics, research papers, deep practice)

## Difficulty Levels

- **easy**: Introductory material, overview lectures, light reading
- **medium**: Core concepts, textbook chapters, standard exercises
- **hard**: Advanced topics, research papers, complex problem solving

Must use exactly: "easy", "medium", or "hard"

## ID Format

- Phases: "phase_1", "phase_2", etc.
- Skills: "skill_1_1", "skill_2_1", etc. (phase_skill)
- Quests: "quest_1_1_1", "quest_2_3_2", etc. (phase_skill_quest)

IDs must be unique within the tree.

## Important Rules

1. Focus on LEARNING (reading, watching, studying), not DOING (building, implementing)
2. Reference specific resources from the PRD when possible
3. Break learning into digestible quests (avoid "Learn everything about X")
4. Respect the timeline: don't create 100 hours of quests for a 2-week project
5. Consider the user's background: don't include prerequisite material they already know
6. Include knowledge validation: problem sets, exercises, practice questions

## Response Format

Respond with ONLY the JSON object. No markdown formatting, no explanation, no additional text.
"#.to_string()
}

#[tauri::command]
pub async fn generate_skill_tree(
    project_id: String,
    prd_text: String,
    database: State<'_, Database>
) -> Result<String, String> {
    println!("\n=== Generate Skill Tree ===");
    println!("Project ID: {}", project_id);
    println!("PRD length: {} chars\n", prd_text.len());
    
    // Load API key
    dotenv::dotenv().ok();
    let api_key = std::env::var("GROQ_API_KEY")
        .map_err(|_| "GROQ_API_KEY not found in .env file".to_string())?;
    
    // Build request
    let client = reqwest::Client::new();
    let system_prompt = build_system_prompt();
    let user_prompt = format!(
        "Project ID: {}\n\nPRD:\n{}\n\nGenerate the skill tree JSON:",
        project_id, prd_text
    );
    
    let request_body = json!({
        "model": "llama-3.3-70b-versatile",
        "messages": [
            {
                "role": "system",
                "content": system_prompt
            },
            {
                "role": "user",
                "content": user_prompt
            }
        ],
        "temperature": 0.7,
        "max_tokens": 4096,
        "response_format": {"type": "json_object"}
    });
    
    println!("Calling Groq API...");
    
    let response = client
        .post("https://api.groq.com/openai/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&request_body)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {}", e))?;
    
    let status = response.status();
    let response_text = response.text().await
        .map_err(|e| format!("Failed to read response: {}", e))?;
    
    if !status.is_success() {
        println!("Groq API error: {}", response_text);
        return Err(format!("API returned {}: {}", status, response_text));
    }
    
    println!("API response received, parsing...");
    
    let groq_response: GroqResponse = serde_json::from_str(&response_text)
        .map_err(|e| format!("Failed to parse Groq response: {}", e))?;
    
    let tree_json = &groq_response.choices[0].message.content;
    
    // Validate that it parses as SkillTree
    let mut skill_tree: SkillTree = serde_json::from_str(tree_json)
        .map_err(|e| format!("Generated JSON doesn't match SkillTree schema: {}", e))?;
    
    // Ensure project_id matches
    skill_tree.project_id = project_id.clone();
    
    println!("✅ Tree generated: {} phases, {} total skills",
        skill_tree.phases.len(),
        skill_tree.phases.iter().map(|p| p.skills.len()).sum::<usize>()
    );

    // Save to database
    let tree_id = save_tree_to_database(&skill_tree, &database).await?;
    println!("💾 Saved to database with tree_id: {}", tree_id);

    // Return the tree JSON with tree_id included
    let mut response = serde_json::json!(skill_tree);
    response["tree_id"] = serde_json::json!(tree_id);

    Ok(serde_json::to_string_pretty(&response).unwrap())
}
