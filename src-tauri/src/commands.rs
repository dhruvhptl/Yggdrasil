// src-tauri/src/commands.rs - Backend commands for handling data operations

use tauri::State;
use uuid::Uuid;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

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

// Simple in-memory storage for MVP (we'll upgrade to SQLite later)
#[derive(Debug, Default)]
pub struct AppState {
    pub projects: Mutex<HashMap<String, Project>>,
    pub disciplines: Mutex<HashMap<String, Discipline>>,
}

// Create a new project
#[tauri::command]
pub async fn create_project(
    name: String,
    description: String,
    discipline: String,
    state: State<'_, AppState>
) -> Result<Project, String> {
    // Validate input
    if name.trim().is_empty() {
        return Err("Project name cannot be empty".to_string());
    }
    
    if description.trim().is_empty() {
        return Err("Project description cannot be empty".to_string());
    }

    // Create or get discipline
    let discipline_id = create_or_get_discipline(&discipline, &state).await?;

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

    // Store in memory
    let mut projects = state.projects.lock().unwrap();
    projects.insert(project_id, project.clone());

    Ok(project)
}

// Get all projects
#[tauri::command]
pub async fn get_projects(state: State<'_, AppState>) -> Result<Vec<Project>, String> {
    let projects = state.projects.lock().unwrap();
    Ok(projects.values().cloned().collect())
}

// Helper function to create or get discipline
async fn create_or_get_discipline(
    name: &str,
    state: &State<'_, AppState>
) -> Result<String, String> {
    let mut disciplines = state.disciplines.lock().unwrap();
    
    // Check if discipline already exists
    for (id, discipline) in disciplines.iter() {
        if discipline.name.to_lowercase() == name.trim().to_lowercase() {
            return Ok(id.clone());
        }
    }
    
    // Create new discipline
    let discipline_id = Uuid::new_v4().to_string();
    let discipline = Discipline {
        id: discipline_id.clone(),
        name: name.trim().to_string(),
        description: format!("Skills related to {}", name.trim()),
        color: None,
    };
    
    disciplines.insert(discipline_id.clone(), discipline);
    Ok(discipline_id)
}

// Get all disciplines
#[tauri::command]
pub async fn get_disciplines(state: State<'_, AppState>) -> Result<Vec<Discipline>, String> {
    let disciplines = state.disciplines.lock().unwrap();
    Ok(disciplines.values().cloned().collect())
}
