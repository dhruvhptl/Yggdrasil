// src-tauri/src/commands.rs - Backend commands for handling data operations with persistent storage

use tauri::State;
use uuid::Uuid;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use crate::database::Database;

// Data structures matching our TypeScript types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub description: String,
    pub discipline_ids: Vec<String>,
    pub skill_ids: Vec<String>,
    pub status: String,
    #[serde(rename = "createdAt")]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub id: String,
    pub name: String,
    pub description: String,
    pub discipline_id: String,
    pub proficiency_level: String,
    pub progress: u8,
    pub is_unlocked: bool,
    pub prerequisites: Vec<String>,
    pub project_ids: Vec<String>,
}

// Create a new project
#[tauri::command]
pub async fn create_project(
    name: String,
    description: String,
    discipline: String,
    database: State<'_, Database>
) -> Result<Project, String> {
    // Validate input
    if name.trim().is_empty() {
        return Err("Project name cannot be empty".to_string());
    }
    
    if description.trim().is_empty() {
        return Err("Project description cannot be empty".to_string());
    }

    // Create or get discipline
    let discipline_id = create_or_get_discipline(&discipline, &database).await
        .map_err(|e| format!("Failed to create discipline: {}", e))?;

    // Generate unique ID
    let project_id = Uuid::new_v4().to_string();

    // Create project
    let project = Project {
        id: project_id.clone(),
        name: name.trim().to_string(),
        description: description.trim().to_string(),
        discipline_ids: vec![discipline_id],
        skill_ids: vec![],
        status: "active".to_string(),
        created_at: Utc::now().to_rfc3339(),
        progress: 0,
    };

    // Store in database
    database.create_project(&project).await
        .map_err(|e| format!("Failed to save project: {}", e))?;

    Ok(project)
}

// Get all projects
#[tauri::command]
pub async fn get_projects(database: State<'_, Database>) -> Result<Vec<Project>, String> {
    database.get_projects().await
        .map_err(|e| format!("Failed to load projects: {}", e))
}

// Helper function to create or get discipline
async fn create_or_get_discipline(
    name: &str,
    database: &State<'_, Database>
) -> Result<String, Box<dyn std::error::Error>> {
    // Check if discipline already exists
    if let Some(existing) = database.get_discipline_by_name(name).await? {
        return Ok(existing.id);
    }
    
    // Create new discipline
    let discipline_id = Uuid::new_v4().to_string();
    let discipline = Discipline {
        id: discipline_id.clone(),
        name: name.trim().to_string(),
        description: format!("Skills related to {}", name.trim()),
        color: None,
    };
    
    database.create_discipline(&discipline).await?;
    Ok(discipline_id)
}

// Get all disciplines
#[tauri::command]
pub async fn get_disciplines(database: State<'_, Database>) -> Result<Vec<Discipline>, String> {
    database.get_disciplines().await
        .map_err(|e| format!("Failed to load disciplines: {}", e))
}

// Update project progress (new command for better functionality)
#[tauri::command]
pub async fn update_project_progress(
    _project_id: String,
    _progress: u8,
    _database: State<'_, Database>
) -> Result<(), String> {
    // This would require adding an update method to the database module
    // For now, return a placeholder
    Ok(())
}

// Delete project (new command)
#[tauri::command]
pub async fn delete_project(
    _project_id: String,
    _database: State<'_, Database>
) -> Result<(), String> {
    // This would require adding a delete method to the database module
    // For now, return a placeholder
    Ok(())
}