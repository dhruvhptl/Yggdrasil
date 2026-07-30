use sqlx::{PgPool, Row};

pub(crate) async fn get_setting(pool: &PgPool, key: &str) -> Option<String> {
    sqlx::query("SELECT value FROM settings WHERE key = $1")
        .bind(key)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .and_then(|r| r.try_get::<String, _>("value").ok())
}

pub(crate) async fn set_setting(pool: &PgPool, key: &str, value: &str) -> Result<(), String> {
    sqlx::query("INSERT INTO settings (key, value) VALUES ($1, $2) ON CONFLICT (key) DO UPDATE SET value = $2")
        .bind(key)
        .bind(value)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Returns the stored agent model id, or "" if unset (env/default takes over).
/// The frontend maps the id to a display name via openrouter-models.ts.
#[tauri::command]
pub async fn get_agent_config_cmd(
    database: tauri::State<'_, crate::database::Database>,
) -> Result<String, String> {
    Ok(get_setting(&database.pool, "agent_model").await.unwrap_or_default())
}

/// Upsert the agent model; an empty string deletes the row (→ env var wins).
#[tauri::command]
pub async fn set_agent_config_cmd(
    model: String,
    database: tauri::State<'_, crate::database::Database>,
) -> Result<(), String> {
    let m = model.trim();
    if m.is_empty() {
        sqlx::query("DELETE FROM settings WHERE key = 'agent_model'")
            .execute(&database.pool)
            .await
            .map_err(|e| e.to_string())?;
    } else {
        set_setting(&database.pool, "agent_model", m).await?;
    }
    Ok(())
}
