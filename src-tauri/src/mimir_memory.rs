// src-tauri/src/mimir_memory.rs
//
// Mimir's persistent memory.
//   Tier 3 (long-term): mimir_memory_facts — durable, scoped facts.
//   Tier 2 (short-term): mimir_memory_shortterm — one rolling summary per session.
// Facts are written by orchestrator events, by the agent's set_fact tool, and
// by session consolidation. Read via get_memory_context / the recall seed.

use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use sqlx::Row;
use tauri::State;

use crate::database::Database;

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MemoryFact {
    pub id: String,
    pub scope: String,
    pub tree_id: Option<String>,
    pub project_id: Option<String>,
    pub fact_key: String,
    pub entity_id: Option<String>,
    pub fact_value: serde_json::Value,
    pub confidence: f64,
    pub source: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MemoryContext {
    pub facts: Vec<MemoryFact>,
    pub user_facts: Vec<MemoryFact>,
    pub recent_topics: Vec<String>,
}

// ─── Recall seed formatting ──────────────────────────────────────────────────

/// Render a JSON fact_value compactly: bare strings stay bare, objects with a
/// "title" key collapse to the title, everything else serializes.
fn compact_value(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Object(o) => o
            .get("title")
            .and_then(|t| t.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| v.to_string()),
        _ => v.to_string(),
    }
}

/// Build the "[Known facts...]" block injected into the chat system prompt.
/// Returns an empty string when there is nothing to say.
pub(crate) fn format_memory_block(ctx: &MemoryContext, max_facts: usize) -> String {
    let mut lines: Vec<String> = Vec::new();
    for f in ctx.facts.iter().take(max_facts) {
        lines.push(format!(
            "- {}: {} (confidence {:.1})",
            f.fact_key,
            compact_value(&f.fact_value),
            f.confidence
        ));
    }
    let remaining = max_facts.saturating_sub(lines.len());
    for f in ctx.user_facts.iter().take(remaining) {
        lines.push(format!(
            "- [user] {}: {} (confidence {:.1})",
            f.fact_key,
            compact_value(&f.fact_value),
            f.confidence
        ));
    }
    if !ctx.recent_topics.is_empty() {
        lines.push(format!(
            "- Recent topics discussed: {}",
            ctx.recent_topics.join(", ")
        ));
    }
    if lines.is_empty() {
        return String::new();
    }
    format!(
        "\n\n[Known facts about the user and their progress]\n{}\n",
        lines.join("\n")
    )
}

// ─── Tier 3: fact CRUD ───────────────────────────────────────────────────────

const ALLOWED_SOURCES: [&str; 6] = [
    "agent_extraction", "checkpoint_complete", "resource_complete",
    "user_said", "manual", "session_consolidation",
];

/// Upsert a fact. Same (scope, owner, key, entity) updates in place;
/// distinct entity_ids accumulate as separate rows. Returns the fact id.
pub(crate) async fn set_memory_fact(
    pool: &PgPool,
    scope: &str,
    tree_id: Option<&str>,
    project_id: Option<&str>,
    fact_key: &str,
    entity_id: Option<&str>,
    fact_value: serde_json::Value,
    confidence: f64,
    source: &str,
) -> Result<String, String> {
    if fact_key.trim().is_empty() {
        return Err("fact_key must not be empty".into());
    }
    if !ALLOWED_SOURCES.contains(&source) {
        return Err(format!("invalid source '{}'", source));
    }
    let id = uuid::Uuid::new_v4().to_string();
    let row = sqlx::query(
        "INSERT INTO mimir_memory_facts \
           (id, scope, tree_id, project_id, fact_key, entity_id, fact_value, confidence, source) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) \
         ON CONFLICT (scope, COALESCE(tree_id,''), COALESCE(project_id,''), fact_key, COALESCE(entity_id,'')) \
         DO UPDATE SET fact_value = EXCLUDED.fact_value, \
                       confidence = EXCLUDED.confidence, \
                       source     = EXCLUDED.source, \
                       updated_at = NOW() \
         RETURNING id"
    )
    .bind(&id)
    .bind(scope)
    .bind(tree_id)
    .bind(project_id)
    .bind(fact_key)
    .bind(entity_id)
    .bind(&fact_value)
    .bind(confidence.clamp(0.0, 1.0))
    .bind(source)
    .fetch_one(pool)
    .await
    .map_err(|e| e.to_string())?;
    row.try_get("id").map_err(|e| e.to_string())
}

fn row_to_fact(row: &sqlx::postgres::PgRow) -> Option<MemoryFact> {
    Some(MemoryFact {
        id: row.try_get("id").ok()?,
        scope: row.try_get("scope").ok()?,
        tree_id: row.try_get("tree_id").ok().flatten(),
        project_id: row.try_get("project_id").ok().flatten(),
        fact_key: row.try_get("fact_key").ok()?,
        entity_id: row.try_get("entity_id").ok().flatten(),
        fact_value: row.try_get("fact_value").ok()?,
        confidence: row.try_get("confidence").ok()?,
        source: row.try_get("source").ok()?,
        created_at: row.try_get("created_at").unwrap_or_default(),
        updated_at: row.try_get("updated_at").unwrap_or_default(),
    })
}

const FACT_COLUMNS: &str =
    "id, scope, tree_id, project_id, fact_key, entity_id, fact_value, confidence, source, \
     created_at::text AS created_at, updated_at::text AS updated_at";

/// Read memory for a chat context: tree-scoped facts (when tree_id given),
/// user-level facts, and recent topics from short-term summaries.
pub(crate) async fn get_memory_context(
    pool: &PgPool,
    tree_id: Option<&str>,
) -> Result<MemoryContext, String> {
    let mut ctx = MemoryContext::default();

    if let Some(tid) = tree_id {
        let rows = sqlx::query(&format!(
            "SELECT {} FROM mimir_memory_facts \
             WHERE scope = 'tree' AND tree_id = $1 \
             ORDER BY confidence DESC, updated_at DESC LIMIT 50",
            FACT_COLUMNS
        ))
        .bind(tid)
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;
        ctx.facts = rows.iter().filter_map(row_to_fact).collect();

        let topic_rows = sqlx::query(
            "SELECT ms.key_topics FROM mimir_memory_shortterm ms \
             JOIN mimir_chat_sessions cs ON cs.id = ms.session_id \
             WHERE cs.tree_id = $1 \
             ORDER BY ms.updated_at DESC LIMIT 5"
        )
        .bind(tid)
        .fetch_all(pool)
        .await
        .unwrap_or_default();
        for row in &topic_rows {
            let topics: Vec<String> = row.try_get("key_topics").unwrap_or_default();
            for t in topics {
                if !ctx.recent_topics.iter().any(|x| x.eq_ignore_ascii_case(&t)) {
                    ctx.recent_topics.push(t);
                }
            }
        }
        ctx.recent_topics.truncate(10);
    }

    let user_rows = sqlx::query(&format!(
        "SELECT {} FROM mimir_memory_facts \
         WHERE scope = 'user' \
         ORDER BY confidence DESC, updated_at DESC LIMIT 25",
        FACT_COLUMNS
    ))
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    ctx.user_facts = user_rows.iter().filter_map(row_to_fact).collect();

    Ok(ctx)
}

pub(crate) async fn delete_memory_fact(pool: &PgPool, fact_id: &str) -> Result<(), String> {
    sqlx::query("DELETE FROM mimir_memory_facts WHERE id = $1")
        .bind(fact_id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ─── Tauri commands ──────────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_memory_context_cmd(
    tree_id: String,
    database: State<'_, Database>,
) -> Result<MemoryContext, String> {
    get_memory_context(&database.pool, Some(&tree_id)).await
}

#[tauri::command]
pub async fn set_memory_fact_cmd(
    tree_id: Option<String>,
    fact_key: String,
    fact_value: serde_json::Value,
    entity_id: Option<String>,
    confidence: Option<f64>,
    source: Option<String>,
    database: State<'_, Database>,
) -> Result<String, String> {
    let scope = if tree_id.is_some() { "tree" } else { "user" };
    let src = source.unwrap_or_else(|| "manual".to_string());
    let src = if ALLOWED_SOURCES.contains(&src.as_str()) { src } else { "manual".to_string() };
    set_memory_fact(
        &database.pool,
        scope,
        tree_id.as_deref(),
        None,
        &fact_key,
        entity_id.as_deref(),
        fact_value,
        confidence.unwrap_or(0.7),
        &src,
    )
    .await
}

#[tauri::command]
pub async fn delete_memory_fact_cmd(
    fact_id: String,
    database: State<'_, Database>,
) -> Result<(), String> {
    delete_memory_fact(&database.pool, &fact_id).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fact(key: &str, value: serde_json::Value, conf: f64) -> MemoryFact {
        MemoryFact {
            id: "f1".into(),
            scope: "tree".into(),
            tree_id: Some("t1".into()),
            project_id: None,
            fact_key: key.into(),
            entity_id: None,
            fact_value: value,
            confidence: conf,
            source: "manual".into(),
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    #[test]
    fn empty_context_renders_empty_string() {
        let ctx = MemoryContext::default();
        assert_eq!(format_memory_block(&ctx, 12), "");
    }

    #[test]
    fn facts_render_with_title_collapse_and_cap() {
        let mut ctx = MemoryContext::default();
        ctx.facts.push(fact(
            "mastered_concept",
            json!({ "title": "RRF Fusion", "node_id": "n1" }),
            0.9,
        ));
        ctx.facts.push(fact("prefers_format", json!("video"), 0.7));
        ctx.recent_topics = vec!["pgvector".into(), "hybrid search".into()];
        let block = format_memory_block(&ctx, 12);
        assert!(block.contains("[Known facts about the user and their progress]"));
        assert!(block.contains("- mastered_concept: RRF Fusion (confidence 0.9)"));
        assert!(block.contains("- prefers_format: video (confidence 0.7)"));
        assert!(block.contains("- Recent topics discussed: pgvector, hybrid search"));

        // Cap: max_facts=1 keeps only the first fact line (+ topics line)
        let capped = format_memory_block(&ctx, 1);
        assert!(capped.contains("mastered_concept"));
        assert!(!capped.contains("prefers_format"));
    }
}
