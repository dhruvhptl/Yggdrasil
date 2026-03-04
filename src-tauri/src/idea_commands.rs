// src-tauri/src/idea_commands.rs
// Scratchpad ideas: create, list, edit, delete, promote to project.
// All queries use sqlx::query() non-macro to avoid offline cache issues.

use serde::{Deserialize, Serialize};
use sqlx::Row;
use tauri::State;
use uuid::Uuid;

use crate::database::Database;

// ─── Struct ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Idea {
    pub id: String,
    pub content: String,
    pub tag: String,
    pub pinned: bool,
    pub created_at: String,
}

// ─── Helper ───────────────────────────────────────────────────────────────────

fn row_to_idea(r: &sqlx::postgres::PgRow) -> Result<Idea, String> {
    let created_at = r
        .try_get::<chrono::DateTime<chrono::Utc>, _>("created_at")
        .map(|d| d.to_rfc3339())
        .map_err(|e| e.to_string())?;
    Ok(Idea {
        id: r.try_get("id").map_err(|e| e.to_string())?,
        content: r.try_get("content").map_err(|e| e.to_string())?,
        tag: r.try_get("tag").map_err(|e| e.to_string())?,
        pinned: r.try_get("pinned").map_err(|e| e.to_string())?,
        created_at,
    })
}

// ─── Commands ─────────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn create_idea(
    content: String,
    tag: String,
    database: State<'_, Database>,
) -> Result<Idea, String> {
    if content.trim().is_empty() {
        return Err("Content is required".into());
    }
    let valid_tags = ["project_idea", "resource", "random", "learning"];
    let tag = if valid_tags.contains(&tag.as_str()) { tag } else { "random".to_string() };

    let id = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO ideas (id, content, tag) VALUES ($1, $2, $3)")
        .bind(&id)
        .bind(content.trim())
        .bind(&tag)
        .execute(&database.pool)
        .await
        .map_err(|e| e.to_string())?;

    let row = sqlx::query("SELECT * FROM ideas WHERE id = $1")
        .bind(&id)
        .fetch_one(&database.pool)
        .await
        .map_err(|e| e.to_string())?;
    row_to_idea(&row)
}

#[tauri::command]
pub async fn get_ideas(
    database: State<'_, Database>,
) -> Result<Vec<Idea>, String> {
    let rows = sqlx::query(
        "SELECT * FROM ideas ORDER BY pinned DESC, created_at DESC"
    )
    .fetch_all(&database.pool)
    .await
    .map_err(|e| e.to_string())?;
    rows.iter().map(row_to_idea).collect()
}

#[tauri::command]
pub async fn update_idea(
    id: String,
    content: Option<String>,
    tag: Option<String>,
    pinned: Option<bool>,
    database: State<'_, Database>,
) -> Result<Idea, String> {
    sqlx::query(
        "UPDATE ideas SET \
         content = COALESCE($2, content), \
         tag = COALESCE($3, tag), \
         pinned = COALESCE($4, pinned) \
         WHERE id = $1"
    )
    .bind(&id)
    .bind(&content)
    .bind(&tag)
    .bind(pinned)
    .execute(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    let row = sqlx::query("SELECT * FROM ideas WHERE id = $1")
        .bind(&id)
        .fetch_one(&database.pool)
        .await
        .map_err(|e| e.to_string())?;
    row_to_idea(&row)
}

#[tauri::command]
pub async fn delete_idea(
    id: String,
    database: State<'_, Database>,
) -> Result<(), String> {
    sqlx::query("DELETE FROM ideas WHERE id = $1")
        .bind(&id)
        .execute(&database.pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Promotes an idea to a project. First line of content → project name;
/// remaining lines → description. Returns the new project id so the
/// frontend can navigate to /project/:id.
#[tauri::command]
pub async fn idea_to_project(
    id: String,
    database: State<'_, Database>,
) -> Result<String, String> {
    let row = sqlx::query("SELECT content FROM ideas WHERE id = $1")
        .bind(&id)
        .fetch_one(&database.pool)
        .await
        .map_err(|e| format!("Idea not found: {}", e))?;

    let content: String = row.try_get("content").map_err(|e| e.to_string())?;

    // First non-empty line → name (capped at 120 chars).
    let mut lines = content.lines();
    let raw_name = lines.next().unwrap_or("New Project").trim().to_string();
    let name = if raw_name.len() > 120 { raw_name[..120].to_string() } else { raw_name };
    let description = lines.collect::<Vec<_>>().join("\n").trim().to_string();

    let project_id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO projects \
         (id, name, description, discipline_ids, skill_ids, status, progress) \
         VALUES ($1, $2, $3, '[]'::jsonb, '[]'::jsonb, 'active', 0)"
    )
    .bind(&project_id)
    .bind(&name)
    .bind(&description)
    .execute(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    println!("💡 Promoted idea {} → project {}", id, project_id);
    Ok(project_id)
}
