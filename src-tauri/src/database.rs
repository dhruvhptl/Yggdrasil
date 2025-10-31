// src-tauri/src/database.rs - Database operations for persistent storage

use sqlx::{migrate::MigrateDatabase, Sqlite, SqlitePool};
use tauri::AppHandle;
use std::fs;
use crate::commands::{Project, Discipline};

pub struct Database {
    pub pool: SqlitePool,
}

impl Database {
    pub async fn new(app_handle: &AppHandle) -> Result<Self, Box<dyn std::error::Error>> {
        let app_dir = app_handle.path().app_data_dir()
            .expect("Failed to get app data directory");
        
        // Create app data directory if it doesn't exist
        fs::create_dir_all(&app_dir)?;
        
        let database_path = app_dir.join("yggdrasil.db");
        let database_url = format!("sqlite:{}", database_path.to_string_lossy());
        
        // Create database if it doesn't exist
        if !Sqlite::database_exists(&database_url).await.unwrap_or(false) {
            Sqlite::create_database(&database_url).await?;
        }
        
        let pool = SqlitePool::connect(&database_url).await?;
        
        // Run migrations
        sqlx::migrate!("./migrations").run(&pool).await?;
        
        Ok(Database { pool })
    }
    
    // Project operations
    pub async fn create_project(&self, project: &Project) -> Result<(), sqlx::Error> {
        sqlx::query!(
            r#"
            INSERT INTO projects (id, name, description, discipline_ids, skill_ids, status, created_at, progress)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
            project.id,
            project.name,
            project.description,
            serde_json::to_string(&project.discipline_ids).unwrap(),
            serde_json::to_string(&project.skill_ids).unwrap(),
            project.status,
            project.created_at,
            project.progress
        ).execute(&self.pool).await?;
        
        Ok(())
    }
    
    pub async fn get_projects(&self) -> Result<Vec<Project>, sqlx::Error> {
        let rows = sqlx::query!("SELECT * FROM projects")
            .fetch_all(&self.pool)
            .await?;
        
        let mut projects = Vec::new();
        for row in rows {
            let project = Project {
                id: row.id,
                name: row.name,
                description: row.description,
                discipline_ids: serde_json::from_str(&row.discipline_ids).unwrap_or_default(),
                skill_ids: serde_json::from_str(&row.skill_ids).unwrap_or_default(),
                status: row.status,
                created_at: row.created_at,
                progress: row.progress as u8,
            };
            projects.push(project);
        }
        
        Ok(projects)
    }
    
    // Discipline operations
    pub async fn create_discipline(&self, discipline: &Discipline) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "INSERT INTO disciplines (id, name, description, color) VALUES (?1, ?2, ?3, ?4)",
            discipline.id,
            discipline.name,
            discipline.description,
            discipline.color
        ).execute(&self.pool).await?;
        
        Ok(())
    }
    
    pub async fn get_disciplines(&self) -> Result<Vec<Discipline>, sqlx::Error> {
        let rows = sqlx::query!("SELECT * FROM disciplines")
            .fetch_all(&self.pool)
            .await?;
        
        let mut disciplines = Vec::new();
        for row in rows {
            let discipline = Discipline {
                id: row.id,
                name: row.name,
                description: row.description,
                color: row.color,
            };
            disciplines.push(discipline);
        }
        
        Ok(disciplines)
    }
    
    pub async fn get_discipline_by_name(&self, name: &str) -> Result<Option<Discipline>, sqlx::Error> {
        let row = sqlx::query!("SELECT * FROM disciplines WHERE LOWER(name) = LOWER(?1)", name)
            .fetch_optional(&self.pool)
            .await?;
        
        if let Some(row) = row {
            Ok(Some(Discipline {
                id: row.id,
                name: row.name,
                description: row.description,
                color: row.color,
            }))
        } else {
            Ok(None)
        }
    }
}