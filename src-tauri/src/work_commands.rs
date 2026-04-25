// src-tauri/src/work_commands.rs
// Work graph: co-op terms → research topics → resources → AI-extracted skill tags.
// All queries use sqlx::query() non-macro to avoid offline cache issues with new tables.

use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::collections::HashMap;
use tauri::State;
use uuid::Uuid;
use crate::constants::GROQ_API_URL;

use crate::database::Database;

// ─── Structs ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoopTerm {
    pub id: String,
    pub company: String,
    pub role: String,
    pub start_date: String,
    pub end_date: String,
    pub color: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchTopic {
    pub id: String,
    pub coop_id: String,
    pub name: String,
    pub track: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkResource {
    pub id: String,
    pub topic_id: String,
    pub title: String,
    pub url: Option<String>,
    pub notes: Option<String>,
    pub completed: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkResourceSkill {
    pub id: String,
    pub resource_id: String,
    pub skill_name: String,
    pub tree_id: Option<String>,
}

// Nested types for get_full_work_graph

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceWithSkills {
    pub id: String,
    pub topic_id: String,
    pub title: String,
    pub url: Option<String>,
    pub notes: Option<String>,
    pub completed: bool,
    pub created_at: String,
    pub skills: Vec<WorkResourceSkill>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TopicWithResources {
    pub id: String,
    pub coop_id: String,
    pub name: String,
    pub track: String,
    pub created_at: String,
    pub resources: Vec<ResourceWithSkills>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoopWithTopics {
    pub id: String,
    pub company: String,
    pub role: String,
    pub start_date: String,
    pub end_date: String,
    pub color: String,
    pub created_at: String,
    pub topics: Vec<TopicWithResources>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkGraph {
    pub coops: Vec<CoopWithTopics>,
}

// ─── Commands ─────────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn create_coop(
    company: String,
    role: String,
    start_date: String,
    end_date: String,
    color: String,
    database: State<'_, Database>,
) -> Result<CoopTerm, String> {
    if company.trim().is_empty() || role.trim().is_empty() {
        return Err("Company and role are required".into());
    }

    let id = Uuid::new_v4().to_string();

    sqlx::query(
        "INSERT INTO coop_terms (id, company, role, start_date, end_date, color) \
         VALUES ($1, $2, $3, $4, $5, $6)"
    )
    .bind(&id)
    .bind(company.trim())
    .bind(role.trim())
    .bind(&start_date)
    .bind(&end_date)
    .bind(&color)
    .execute(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    let row = sqlx::query(
        "SELECT id, company, role, start_date, end_date, color, created_at \
         FROM coop_terms WHERE id = $1"
    )
    .bind(&id)
    .fetch_one(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(CoopTerm {
        id: row.try_get("id").map_err(|e| e.to_string())?,
        company: row.try_get("company").map_err(|e| e.to_string())?,
        role: row.try_get("role").map_err(|e| e.to_string())?,
        start_date: row.try_get("start_date").map_err(|e| e.to_string())?,
        end_date: row.try_get("end_date").map_err(|e| e.to_string())?,
        color: row.try_get("color").map_err(|e| e.to_string())?,
        created_at: row.try_get::<chrono::DateTime<chrono::Utc>, _>("created_at")
            .map(|dt| dt.to_rfc3339())
            .map_err(|e| e.to_string())?,
    })
}

#[tauri::command]
pub async fn get_coops(database: State<'_, Database>) -> Result<Vec<CoopTerm>, String> {
    let rows = sqlx::query(
        "SELECT id, company, role, start_date, end_date, color, created_at \
         FROM coop_terms ORDER BY created_at DESC"
    )
    .fetch_all(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    rows.into_iter()
        .map(|r| -> Result<CoopTerm, String> {
            Ok(CoopTerm {
                id: r.try_get("id").map_err(|e: sqlx::Error| e.to_string())?,
                company: r.try_get("company").map_err(|e: sqlx::Error| e.to_string())?,
                role: r.try_get("role").map_err(|e: sqlx::Error| e.to_string())?,
                start_date: r.try_get("start_date").map_err(|e: sqlx::Error| e.to_string())?,
                end_date: r.try_get("end_date").map_err(|e: sqlx::Error| e.to_string())?,
                color: r.try_get("color").map_err(|e: sqlx::Error| e.to_string())?,
                created_at: r.try_get::<chrono::DateTime<chrono::Utc>, _>("created_at")
                    .map(|dt| dt.to_rfc3339())
                    .map_err(|e| e.to_string())?,
            })
        })
        .collect()
}

#[tauri::command]
pub async fn create_topic(
    coop_id: String,
    name: String,
    track: Option<String>,
    database: State<'_, Database>,
) -> Result<ResearchTopic, String> {
    if name.trim().is_empty() {
        return Err("Topic name is required".into());
    }

    let track_val = track
        .as_deref()
        .filter(|t| matches!(*t, "hardware" | "software" | "general"))
        .unwrap_or("general")
        .to_string();

    let id = Uuid::new_v4().to_string();

    sqlx::query("INSERT INTO research_topics (id, coop_id, name, track) VALUES ($1, $2, $3, $4)")
        .bind(&id)
        .bind(&coop_id)
        .bind(name.trim())
        .bind(&track_val)
        .execute(&database.pool)
        .await
        .map_err(|e| e.to_string())?;

    let row = sqlx::query(
        "SELECT id, coop_id, name, track, created_at FROM research_topics WHERE id = $1"
    )
    .bind(&id)
    .fetch_one(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(ResearchTopic {
        id: row.try_get("id").map_err(|e| e.to_string())?,
        coop_id: row.try_get("coop_id").map_err(|e| e.to_string())?,
        name: row.try_get("name").map_err(|e| e.to_string())?,
        track: row.try_get("track").unwrap_or_else(|_| "general".to_string()),
        created_at: row.try_get::<chrono::DateTime<chrono::Utc>, _>("created_at")
            .map(|dt| dt.to_rfc3339())
            .map_err(|e| e.to_string())?,
    })
}

#[tauri::command]
pub async fn add_resource(
    topic_id: String,
    title: String,
    url: Option<String>,
    notes: Option<String>,
    completed: bool,
    database: State<'_, Database>,
) -> Result<WorkResource, String> {
    if title.trim().is_empty() {
        return Err("Resource title is required".into());
    }

    let id = Uuid::new_v4().to_string();
    let clean_url = url.filter(|s| !s.trim().is_empty());
    let clean_notes = notes.filter(|s| !s.trim().is_empty());

    sqlx::query(
        "INSERT INTO work_resources (id, topic_id, title, url, notes, completed) \
         VALUES ($1, $2, $3, $4, $5, $6)"
    )
    .bind(&id)
    .bind(&topic_id)
    .bind(title.trim())
    .bind(&clean_url)
    .bind(&clean_notes)
    .bind(completed)
    .execute(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    let row = sqlx::query(
        "SELECT id, topic_id, title, url, notes, completed, created_at \
         FROM work_resources WHERE id = $1"
    )
    .bind(&id)
    .fetch_one(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(WorkResource {
        id: row.try_get("id").map_err(|e| e.to_string())?,
        topic_id: row.try_get("topic_id").map_err(|e| e.to_string())?,
        title: row.try_get("title").map_err(|e| e.to_string())?,
        url: row.try_get("url").map_err(|e| e.to_string())?,
        notes: row.try_get("notes").map_err(|e| e.to_string())?,
        completed: row.try_get("completed").map_err(|e| e.to_string())?,
        created_at: row.try_get::<chrono::DateTime<chrono::Utc>, _>("created_at")
            .map(|dt| dt.to_rfc3339())
            .map_err(|e| e.to_string())?,
    })
}

#[tauri::command]
pub async fn toggle_resource_completed(
    resource_id: String,
    database: State<'_, Database>,
) -> Result<bool, String> {
    sqlx::query(
        "UPDATE work_resources SET completed = NOT completed WHERE id = $1"
    )
    .bind(&resource_id)
    .execute(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    let row = sqlx::query("SELECT completed FROM work_resources WHERE id = $1")
        .bind(&resource_id)
        .fetch_one(&database.pool)
        .await
        .map_err(|e| e.to_string())?;

    row.try_get("completed").map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn extract_skills(
    resource_id: String,
    app: tauri::AppHandle,
    database: State<'_, Database>,
) -> Result<Vec<WorkResourceSkill>, String> {
    // Load resource
    let row = sqlx::query(
        "SELECT title, notes FROM work_resources WHERE id = $1"
    )
    .bind(&resource_id)
    .fetch_one(&database.pool)
    .await
    .map_err(|e| format!("Resource not found: {}", e))?;

    let title: String = row.try_get("title").map_err(|e| e.to_string())?;
    let notes: Option<String> = row.try_get("notes").map_err(|e| e.to_string())?;

    // Load API key
    let api_key = std::env::var("GROQ_API_KEY")
        .map_err(|_| "GROQ_API_KEY environment variable not set".to_string())?;

    let system_prompt = "You are a skill extractor. Given a research resource title and notes, \
        return a JSON array of up to 6 skill tags. Prefer broader domain terms over hyper-specific \
        ones — e.g. prefer 'NLP' over 'tokenization', 'Deep Learning' over 'dropout regularization', \
        'Cloud Computing' over 'S3 bucket configuration'. Include the specific technique only if it \
        is well-known in its own right (e.g. 'BERT', 'Transformer', 'LDA', 'RLHF'). \
        Return ONLY a valid JSON array of strings, nothing else. No markdown fences.";

    let user_prompt = format!(
        "Title: {}\nNotes: {}",
        title,
        notes.as_deref().unwrap_or("")
    );

    let client = reqwest::Client::new();
    let response = client
        .post(GROQ_API_URL)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "model": "llama-3.3-70b-versatile",
            "messages": [
                { "role": "system", "content": system_prompt },
                { "role": "user",   "content": user_prompt }
            ],
            "temperature": 0.2,
            "max_tokens": 150
        }))
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| format!("Groq request failed: {}", e))?;

    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Groq response parse error: {}", e))?;

    let content = body["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("[]");

    // Strip markdown fences if the model wrapped the response
    let clean = content
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    let raw_names: Vec<String> = serde_json::from_str(clean).unwrap_or_default();

    // Normalize and deduplicate: lowercase comparison, prefer longer form
    // e.g. "Transformer" + "Transformers" → keep "Transformers"
    //      "NLP" + "Natural Language Processing" → keep "Natural Language Processing"
    let mut normalized: Vec<String> = Vec::new();
    'outer: for name in raw_names {
        let trimmed = name.trim().to_string();
        if trimmed.is_empty() { continue; }
        let lower = trimmed.to_lowercase();
        // Check if any already-accepted skill subsumes or is subsumed by this one
        for existing in &mut normalized {
            let el = existing.to_lowercase();
            if el == lower { continue 'outer; } // exact duplicate
            // One is a prefix/suffix of the other — keep the longer form
            if el.contains(&lower) || lower.contains(&el) {
                if trimmed.len() > existing.len() {
                    *existing = trimmed.clone();
                }
                continue 'outer;
            }
        }
        normalized.push(trimmed);
        if normalized.len() >= 6 { break; }
    }

    // Idempotent: delete existing skills for this resource
    sqlx::query("DELETE FROM work_resource_skills WHERE resource_id = $1")
        .bind(&resource_id)
        .execute(&database.pool)
        .await
        .map_err(|e| e.to_string())?;

    // Insert normalized skills
    let mut result = Vec::new();
    for skill_name in normalized {
        let skill_id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO work_resource_skills (id, resource_id, skill_name) VALUES ($1, $2, $3)"
        )
        .bind(&skill_id)
        .bind(&resource_id)
        .bind(&skill_name)
        .execute(&database.pool)
        .await
        .map_err(|e| e.to_string())?;

        result.push(WorkResourceSkill {
            id: skill_id,
            resource_id: resource_id.clone(),
            skill_name,
            tree_id: None,
        });
    }

    println!("🏷️  Extracted {} skills for resource {}", result.len(), resource_id);

    crate::orchestrator::on_work_skills_extracted(&database.pool, &app).await;

    Ok(result)
}

#[tauri::command]
pub async fn get_full_work_graph(
    database: State<'_, Database>,
) -> Result<WorkGraph, String> {
    // 4 queries, no N+1
    let coop_rows = sqlx::query(
        "SELECT id, company, role, start_date, end_date, color, created_at \
         FROM coop_terms ORDER BY created_at ASC"
    )
    .fetch_all(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    let topic_rows = sqlx::query(
        "SELECT id, coop_id, name, track, created_at FROM research_topics ORDER BY created_at ASC"
    )
    .fetch_all(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    let resource_rows = sqlx::query(
        "SELECT id, topic_id, title, url, notes, completed, created_at \
         FROM work_resources ORDER BY created_at ASC"
    )
    .fetch_all(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    let skill_rows = sqlx::query(
        "SELECT id, resource_id, skill_name, tree_id FROM work_resource_skills"
    )
    .fetch_all(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    // Group skills by resource_id
    let mut skills_by_resource: HashMap<String, Vec<WorkResourceSkill>> = HashMap::new();
    for r in skill_rows {
        let resource_id: String = r.try_get("resource_id").map_err(|e| e.to_string())?;
        let skill = WorkResourceSkill {
            id: r.try_get("id").map_err(|e| e.to_string())?,
            resource_id: resource_id.clone(),
            skill_name: r.try_get("skill_name").map_err(|e| e.to_string())?,
            tree_id: r.try_get("tree_id").map_err(|e| e.to_string())?,
        };
        skills_by_resource.entry(resource_id).or_default().push(skill);
    }

    // Group resources by topic_id
    let mut resources_by_topic: HashMap<String, Vec<ResourceWithSkills>> = HashMap::new();
    for r in resource_rows {
        let topic_id: String = r.try_get("topic_id").map_err(|e| e.to_string())?;
        let resource_id: String = r.try_get("id").map_err(|e| e.to_string())?;
        let skills = skills_by_resource.remove(&resource_id).unwrap_or_default();
        let resource = ResourceWithSkills {
            id: resource_id.clone(),
            topic_id: topic_id.clone(),
            title: r.try_get("title").map_err(|e| e.to_string())?,
            url: r.try_get("url").map_err(|e| e.to_string())?,
            notes: r.try_get("notes").map_err(|e| e.to_string())?,
            completed: r.try_get("completed").map_err(|e| e.to_string())?,
            created_at: r.try_get::<chrono::DateTime<chrono::Utc>, _>("created_at")
                .map(|dt| dt.to_rfc3339())
                .map_err(|e| e.to_string())?,
            skills,
        };
        resources_by_topic.entry(topic_id).or_default().push(resource);
    }

    // Group topics by coop_id
    let mut topics_by_coop: HashMap<String, Vec<TopicWithResources>> = HashMap::new();
    for r in topic_rows {
        let coop_id: String = r.try_get("coop_id").map_err(|e| e.to_string())?;
        let topic_id: String = r.try_get("id").map_err(|e| e.to_string())?;
        let resources = resources_by_topic.remove(&topic_id).unwrap_or_default();
        let topic = TopicWithResources {
            id: topic_id.clone(),
            coop_id: coop_id.clone(),
            name: r.try_get("name").map_err(|e| e.to_string())?,
            track: r.try_get("track").unwrap_or_else(|_| "general".to_string()),
            created_at: r.try_get::<chrono::DateTime<chrono::Utc>, _>("created_at")
                .map(|dt| dt.to_rfc3339())
                .map_err(|e| e.to_string())?,
            resources,
        };
        topics_by_coop.entry(coop_id).or_default().push(topic);
    }

    // Assemble coops
    let coops = coop_rows
        .into_iter()
        .map(|r| -> Result<CoopWithTopics, String> {
            let coop_id: String = r.try_get("id").map_err(|e| e.to_string())?;
            let topics = topics_by_coop.remove(&coop_id).unwrap_or_default();
            Ok(CoopWithTopics {
                id: coop_id.clone(),
                company: r.try_get("company").map_err(|e| e.to_string())?,
                role: r.try_get("role").map_err(|e| e.to_string())?,
                start_date: r.try_get("start_date").map_err(|e| e.to_string())?,
                end_date: r.try_get("end_date").map_err(|e| e.to_string())?,
                color: r.try_get("color").map_err(|e| e.to_string())?,
                created_at: r.try_get::<chrono::DateTime<chrono::Utc>, _>("created_at")
                    .map(|dt| dt.to_rfc3339())
                    .map_err(|e| e.to_string())?,
                topics,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok(WorkGraph { coops })
}
