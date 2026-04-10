// src-tauri/src/daily_commands.rs
// Daily Eisenhower Matrix: triage quests + free-form tasks into quadrants per day.

use serde::{Deserialize, Serialize};
use sqlx::Row;
use tauri::State;
use uuid::Uuid;

use crate::database::Database;

// ─── Structs ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyQuestLink {
    pub id: String,
    pub date: String,
    pub node_id: Option<String>,
    pub free_text: Option<String>,
    pub quadrant: String,
    pub sort_order: i32,
    pub added_at: String,
    // Joined from tree_nodes (null for free-text tasks)
    pub node_title: Option<String>,
    pub node_progress: Option<i32>,
    pub node_is_locked: Option<bool>,
    // Persisted completion flag (for both free-text and quest tasks on this day)
    pub completed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyLog {
    pub date: String,
    pub notes: Option<String>,
    pub links: Vec<DailyQuestLink>,
}

fn row_to_link(r: &sqlx::postgres::PgRow) -> Result<DailyQuestLink, String> {
    let added_at = r
        .try_get::<chrono::DateTime<chrono::Utc>, _>("added_at")
        .map(|d| d.to_rfc3339())
        .map_err(|e| e.to_string())?;
    let date = r
        .try_get::<chrono::NaiveDate, _>("date")
        .map(|d| d.to_string())
        .map_err(|e| e.to_string())?;
    Ok(DailyQuestLink {
        id: r.try_get("id").map_err(|e| e.to_string())?,
        date,
        node_id: r.try_get("node_id").map_err(|e| e.to_string())?,
        free_text: r.try_get("free_text").map_err(|e| e.to_string())?,
        quadrant: r.try_get("quadrant").map_err(|e| e.to_string())?,
        sort_order: r.try_get("sort_order").map_err(|e| e.to_string())?,
        added_at,
        node_title: r.try_get("node_title").unwrap_or(None),
        node_progress: r.try_get("node_progress").unwrap_or(None),
        node_is_locked: r.try_get("node_is_locked").unwrap_or(None),
        completed: r.try_get("completed").unwrap_or(false),
    })
}

// ─── Commands ───────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_daily_log(
    date: String,
    database: State<'_, Database>,
) -> Result<DailyLog, String> {
    // Only auto-create the daily_logs row for today — don't pollute the DB
    // with empty rows just because the user browsed to a past date.
    let today = chrono::Utc::now().date_naive().to_string();
    if date >= today {
        sqlx::query(
            "INSERT INTO daily_logs (id, date) VALUES ($1, $2::date) ON CONFLICT (date) DO NOTHING"
        )
        .bind(Uuid::new_v4().to_string())
        .bind(&date)
        .execute(&database.pool)
        .await
        .map_err(|e| e.to_string())?;
    }

    // Fetch notes — row may not exist for past dates that were never written to
    let notes = sqlx::query("SELECT notes FROM daily_logs WHERE date = $1::date")
        .bind(&date)
        .fetch_optional(&database.pool)
        .await
        .map_err(|e| e.to_string())?
        .and_then(|r| r.try_get::<Option<String>, _>("notes").ok().flatten());

    // Fetch links with joined node data
    let link_rows = sqlx::query(
        "SELECT dql.id, dql.date, dql.node_id, dql.free_text, dql.quadrant, \
                dql.sort_order, dql.added_at, dql.completed, \
                tn.title AS node_title, tn.progress AS node_progress, tn.is_locked AS node_is_locked \
         FROM daily_quest_links dql \
         LEFT JOIN tree_nodes tn ON tn.id = dql.node_id \
         WHERE dql.date = $1::date \
         ORDER BY dql.sort_order ASC, dql.added_at ASC"
    )
    .bind(&date)
    .fetch_all(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    let links = link_rows.iter().map(row_to_link).collect::<Result<Vec<_>, _>>()?;

    Ok(DailyLog { date, notes, links })
}

#[tauri::command]
pub async fn upsert_daily_notes(
    date: String,
    notes: String,
    database: State<'_, Database>,
) -> Result<(), String> {
    sqlx::query(
        "INSERT INTO daily_logs (id, date, notes) VALUES ($1, $2::date, $3) \
         ON CONFLICT (date) DO UPDATE SET notes = $3"
    )
    .bind(Uuid::new_v4().to_string())
    .bind(&date)
    .bind(&notes)
    .execute(&database.pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn add_quest_to_day(
    date: String,
    node_id: String,
    quadrant: String,
    database: State<'_, Database>,
) -> Result<DailyQuestLink, String> {
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO daily_quest_links (id, date, node_id, quadrant) \
         VALUES ($1, $2::date, $3, $4) \
         ON CONFLICT (date, node_id) DO UPDATE SET quadrant = $4"
    )
    .bind(&id)
    .bind(&date)
    .bind(&node_id)
    .bind(&quadrant)
    .execute(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    // Return the link with joined node data
    let row = sqlx::query(
        "SELECT dql.id, dql.date, dql.node_id, dql.free_text, dql.quadrant, \
                dql.sort_order, dql.added_at, dql.completed, \
                tn.title AS node_title, tn.progress AS node_progress, tn.is_locked AS node_is_locked \
         FROM daily_quest_links dql \
         LEFT JOIN tree_nodes tn ON tn.id = dql.node_id \
         WHERE dql.date = $1::date AND dql.node_id = $2"
    )
    .bind(&date)
    .bind(&node_id)
    .fetch_one(&database.pool)
    .await
    .map_err(|e| e.to_string())?;
    row_to_link(&row)
}

#[tauri::command]
pub async fn add_free_task_to_day(
    date: String,
    text: String,
    quadrant: String,
    database: State<'_, Database>,
) -> Result<DailyQuestLink, String> {
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO daily_quest_links (id, date, free_text, quadrant) \
         VALUES ($1, $2::date, $3, $4)"
    )
    .bind(&id)
    .bind(&date)
    .bind(text.trim())
    .bind(&quadrant)
    .execute(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    let row = sqlx::query(
        "SELECT dql.id, dql.date, dql.node_id, dql.free_text, dql.quadrant, \
                dql.sort_order, dql.added_at, dql.completed, \
                NULL::text AS node_title, NULL::int AS node_progress, NULL::bool AS node_is_locked \
         FROM daily_quest_links dql \
         WHERE dql.id = $1"
    )
    .bind(&id)
    .fetch_one(&database.pool)
    .await
    .map_err(|e| e.to_string())?;
    row_to_link(&row)
}

#[tauri::command]
pub async fn move_to_quadrant(
    link_id: String,
    quadrant: String,
    database: State<'_, Database>,
) -> Result<(), String> {
    sqlx::query("UPDATE daily_quest_links SET quadrant = $2 WHERE id = $1")
        .bind(&link_id)
        .bind(&quadrant)
        .execute(&database.pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn remove_from_day(
    link_id: String,
    database: State<'_, Database>,
) -> Result<(), String> {
    sqlx::query("DELETE FROM daily_quest_links WHERE id = $1")
        .bind(&link_id)
        .execute(&database.pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn toggle_task_complete(
    link_id: String,
    database: State<'_, Database>,
) -> Result<bool, String> {
    let row = sqlx::query(
        "UPDATE daily_quest_links SET completed = NOT completed WHERE id = $1 RETURNING completed"
    )
    .bind(&link_id)
    .fetch_one(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    row.try_get::<bool, _>("completed").map_err(|e| e.to_string())
}
