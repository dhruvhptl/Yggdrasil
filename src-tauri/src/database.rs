use sqlx::postgres::{PgPool, PgPoolOptions};
use crate::commands::{Project, Discipline};

pub struct Database {
    pub pool: PgPool,
}

impl Database {
    pub async fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let database_url = std::env::var("DATABASE_URL")
            .expect("DATABASE_URL environment variable must be set");

        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(&database_url)
            .await?;

        sqlx::migrate!("./migrations").run(&pool).await?;
        println!("✅ Connected to Postgres and ran migrations");

        // Phase 1: one-shot backfill of concept graphs from legacy JSONB blobs.
        let _ = crate::concept_graph::backfill_concept_graphs(&pool).await;

        Ok(Database { pool })
    }

    pub async fn create_project(&self, project: &Project) -> Result<(), sqlx::Error> {
        let discipline_ids = serde_json::json!(&project.discipline_ids);
        let skill_ids = serde_json::json!(&project.skill_ids);
        let progress = project.progress as i32;

        sqlx::query!(
            r#"
            INSERT INTO projects (id, name, description, discipline_ids, skill_ids, status, created_at, progress)
            VALUES ($1, $2, $3, $4, $5, $6, NOW(), $7)
            "#,
            project.id,
            project.name,
            project.description,
            discipline_ids,
            skill_ids,
            project.status,
            progress
        ).execute(&self.pool).await?;

        Ok(())
    }

    pub async fn get_projects(&self) -> Result<Vec<Project>, sqlx::Error> {
        let rows = sqlx::query!("SELECT * FROM projects ORDER BY created_at DESC")
            .fetch_all(&self.pool)
            .await?;

        let projects = rows.into_iter().map(|row| Project {
            id: row.id,
            name: row.name,
            description: row.description,
            discipline_ids: serde_json::from_value(row.discipline_ids).unwrap_or_default(),
            skill_ids: serde_json::from_value(row.skill_ids).unwrap_or_default(),
            status: row.status,
            created_at: row.created_at.to_rfc3339(),
            progress: row.progress as u8,
        }).collect();

        Ok(projects)
    }

    pub async fn create_discipline(&self, discipline: &Discipline) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "INSERT INTO disciplines (id, name, description, color) VALUES ($1, $2, $3, $4)",
            discipline.id,
            discipline.name,
            discipline.description,
            discipline.color
        ).execute(&self.pool).await?;

        Ok(())
    }

    pub async fn get_disciplines(&self) -> Result<Vec<Discipline>, sqlx::Error> {
        let rows = sqlx::query!("SELECT * FROM disciplines ORDER BY name ASC")
            .fetch_all(&self.pool)
            .await?;

        let disciplines = rows.into_iter().map(|row| Discipline {
            id: row.id,
            name: row.name,
            description: row.description,
            color: row.color,
        }).collect();

        Ok(disciplines)
    }

    pub async fn get_discipline_by_name(&self, name: &str) -> Result<Option<Discipline>, sqlx::Error> {
        let row = sqlx::query!(
            "SELECT * FROM disciplines WHERE LOWER(name) = LOWER($1)",
            name
        ).fetch_optional(&self.pool).await?;

        Ok(row.map(|row| Discipline {
            id: row.id,
            name: row.name,
            description: row.description,
            color: row.color,
        }))
    }
}
