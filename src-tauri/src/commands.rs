use tauri::State;
use uuid::Uuid;
use serde::{Deserialize, Serialize};
use crate::database::Database;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
    pub description: String,
    pub discipline_ids: Vec<String>,
    pub skill_ids: Vec<String>,
    pub status: String,
    pub created_at: String,
    pub progress: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Discipline {
    pub id: String,
    pub name: String,
    pub description: String,
    pub color: Option<String>,
}


#[tauri::command]
pub async fn create_project(
    name: String,
    description: String,
    discipline: String,
    database: State<'_, Database>
) -> Result<Project, String> {
    if name.trim().is_empty() {
        return Err("Project name cannot be empty".to_string());
    }
    if description.trim().is_empty() {
        return Err("Project description cannot be empty".to_string());
    }

    let discipline_id = create_or_get_discipline(&discipline, &database).await
        .map_err(|e| format!("Failed to create discipline: {}", e))?;

    let project = Project {
        id: Uuid::new_v4().to_string(),
        name: name.trim().to_string(),
        description: description.trim().to_string(),
        discipline_ids: vec![discipline_id],
        skill_ids: vec![],
        status: "active".to_string(),
        created_at: String::new(), // set by DB DEFAULT NOW()
        progress: 0,
    };

    database.create_project(&project).await
        .map_err(|e| format!("Failed to save project: {}", e))?;

    // Re-fetch the project to get the DB-generated created_at
    let projects = database.get_projects().await
        .map_err(|e| format!("Failed to reload project: {}", e))?;

    projects.into_iter()
        .find(|p| p.id == project.id)
        .ok_or_else(|| "Project created but not found on reload".to_string())
}

#[tauri::command]
pub async fn get_projects(database: State<'_, Database>) -> Result<Vec<Project>, String> {
    database.get_projects().await
        .map_err(|e| format!("Failed to load projects: {}", e))
}

#[tauri::command]
pub async fn update_project(
    project_id: String,
    name: Option<String>,
    description: Option<String>,
    database: State<'_, Database>
) -> Result<(), String> {
    if let Some(ref n) = name {
        if n.trim().is_empty() {
            return Err("Project name cannot be empty".to_string());
        }
    }

    sqlx::query!(
        r#"
        UPDATE projects
        SET
            name        = COALESCE($1, name),
            description = COALESCE($2, description)
        WHERE id = $3
        "#,
        name,
        description,
        project_id
    ).execute(&database.pool).await
        .map_err(|e| format!("Failed to update project: {}", e))?;

    Ok(())
}

#[tauri::command]
pub async fn update_project_progress(
    project_id: String,
    database: State<'_, Database>
) -> Result<u8, String> {
    // Aggregate progress from all leaf nodes in all trees for this project
    let row = sqlx::query!(
        r#"
        SELECT COALESCE(AVG(n.progress), 0)::INTEGER AS avg_progress
        FROM tree_nodes n
        JOIN trees t ON n.tree_id = t.id
        WHERE t.project_id = $1 AND n.type = 'leaf'
        "#,
        project_id
    ).fetch_one(&database.pool).await
        .map_err(|e| format!("Failed to calculate progress: {}", e))?;

    let progress = row.avg_progress.unwrap_or(0) as u8;

    sqlx::query!(
        "UPDATE projects SET progress = $1 WHERE id = $2",
        progress as i32,
        project_id
    ).execute(&database.pool).await
        .map_err(|e| format!("Failed to save progress: {}", e))?;

    Ok(progress)
}

#[tauri::command]
pub async fn delete_project(
    project_id: String,
    database: State<'_, Database>
) -> Result<(), String> {
    // Cascade is handled by FK constraints:
    // projects → trees → tree_nodes → tree_edges
    sqlx::query!("DELETE FROM projects WHERE id = $1", project_id)
        .execute(&database.pool).await
        .map_err(|e| format!("Failed to delete project: {}", e))?;

    Ok(())
}

#[tauri::command]
pub async fn get_disciplines(database: State<'_, Database>) -> Result<Vec<Discipline>, String> {
    database.get_disciplines().await
        .map_err(|e| format!("Failed to load disciplines: {}", e))
}

async fn create_or_get_discipline(
    name: &str,
    database: &State<'_, Database>
) -> Result<String, Box<dyn std::error::Error>> {
    if let Some(existing) = database.get_discipline_by_name(name).await? {
        return Ok(existing.id);
    }

    let discipline = Discipline {
        id: Uuid::new_v4().to_string(),
        name: name.trim().to_string(),
        description: format!("Skills related to {}", name.trim()),
        color: None,
    };

    database.create_discipline(&discipline).await?;
    Ok(discipline.id)
}
