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
    // Normalize empty strings to None — '' would alias with NULL in the
    // COALESCE unique index and read back inconsistently.
    let tree_id = tree_id.filter(|s| !s.is_empty());
    let project_id = project_id.filter(|s| !s.is_empty());
    let entity_id = entity_id.filter(|s| !s.is_empty());
    if scope == "tree" && tree_id.is_none() {
        return Err("scope 'tree' requires a non-empty tree_id".into());
    }
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

// ─── Tier 2: session consolidation ───────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(crate) struct ConsolidationOut {
    pub summary: String,
    #[serde(default)]
    pub key_topics: Vec<String>,
}

pub(crate) fn parse_consolidation(raw: &str) -> Result<ConsolidationOut, String> {
    serde_json::from_str(&crate::llm_client::clean_llm_json(raw))
        .map_err(|e| format!("consolidation parse failed: {}", e))
}

fn default_extract_confidence() -> f64 { 0.6 }

#[derive(Debug, Deserialize)]
pub(crate) struct ExtractedFact {
    pub fact_key: String,
    pub fact_value: serde_json::Value,
    #[serde(default = "default_extract_confidence")]
    pub confidence: f64,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct ExtractedFacts {
    #[serde(default)]
    pub facts: Vec<ExtractedFact>,
}

pub(crate) fn parse_extracted_facts(raw: &str) -> ExtractedFacts {
    serde_json::from_str(&crate::llm_client::clean_llm_json(raw)).unwrap_or_default()
}

/// Incrementally update the session's ONE rolling summary row.
/// Only messages newer than covered_through are read; the previous summary is
/// passed as context. Returns Ok(false) when there was too little new material.
pub(crate) async fn consolidate_session(
    pool: &PgPool,
    client: &reqwest::Client,
    session_id: &str,
) -> Result<bool, String> {
    let api_key = crate::mimir::groq_api_key()?;

    let existing = sqlx::query(
        "SELECT summary, key_topics, covered_through::text AS covered \
         FROM mimir_memory_shortterm WHERE session_id = $1"
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;

    let (prev_summary, prev_topics, covered): (String, Vec<String>, Option<String>) = match &existing {
        Some(r) => (
            r.try_get("summary").unwrap_or_default(),
            r.try_get("key_topics").unwrap_or_default(),
            r.try_get::<Option<String>, _>("covered").unwrap_or(None),
        ),
        None => (String::new(), Vec::new(), None),
    };

    let rows = sqlx::query(
        "SELECT role, content, created_at::text AS created_at \
         FROM mimir_chat_messages \
         WHERE session_id = $1 \
           AND ($2::timestamptz IS NULL OR created_at > $2::timestamptz) \
         ORDER BY created_at ASC LIMIT 40"
    )
    .bind(session_id)
    .bind(&covered)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    if rows.len() < 4 {
        return Ok(false);
    }

    let mut transcript = String::new();
    let mut last_ts = String::new();
    for row in &rows {
        let role: String = row.try_get("role").unwrap_or_default();
        let content: String = row.try_get("content").unwrap_or_default();
        last_ts = row.try_get("created_at").unwrap_or_default();
        // User messages in full-ish; assistant answers trimmed — the gist is
        // what a summary needs, not the retrieved passages.
        let cap = if role == "assistant" { 600 } else { 1200 };
        let trimmed: String = content.chars().take(cap).collect();
        transcript.push_str(&format!("{}: {}\n", role, trimmed));
    }

    let system = "You maintain a rolling summary of a tutoring chat session. Merge the previous summary \
                  with the new messages into one updated summary. Return ONLY JSON: \
                  {\"summary\": \"<= 200 words\", \"key_topics\": [\"lowercase topic\", ...]} \
                  with at most 12 topics.";
    let prev_for_prompt = if prev_summary.is_empty() { "(none)".to_string() } else { prev_summary };
    let user = format!("Previous summary:\n{}\n\nNew messages:\n{}", prev_for_prompt, transcript);

    let (raw, latency) = crate::llm_client::call_llm(
        client,
        crate::constants::GROQ_API_URL,
        &api_key,
        "llama-3.3-70b-versatile",
        system,
        &user,
        400,
        true,
        0.2,
    )
    .await?;
    crate::brain::log_prompt_call(
        pool.clone(), "mimir_consolidate", "llama-3.3-70b-versatile", "mimir_consolidate_v1",
        latency, true, None, None,
    );

    let parsed = parse_consolidation(&raw)?;

    // Union topics (case-insensitive), cap 12.
    let mut topics = prev_topics;
    for t in parsed.key_topics {
        let tl = t.trim().to_lowercase();
        if !tl.is_empty() && !topics.iter().any(|x| x.eq_ignore_ascii_case(&tl)) {
            topics.push(tl);
        }
    }
    topics.truncate(12);

    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO mimir_memory_shortterm (id, session_id, summary, key_topics, covered_through) \
         VALUES ($1, $2, $3, $4, $5::timestamptz) \
         ON CONFLICT (session_id) DO UPDATE SET \
           summary = EXCLUDED.summary, \
           key_topics = EXCLUDED.key_topics, \
           covered_through = EXCLUDED.covered_through, \
           updated_at = NOW()"
    )
    .bind(&id)
    .bind(session_id)
    .bind(&parsed.summary)
    .bind(&topics)
    .bind(&last_ts)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    println!("🧠 [memory] consolidated session {} ({} new msgs, {} topics)", session_id, rows.len(), topics.len());
    Ok(true)
}

// ─── Tier 2→3: fact extraction from summaries ────────────────────────────────

/// Distill durable facts from recent short-term summaries for the session's
/// tree. Works off summaries, not raw messages. Returns number written.
pub(crate) async fn extract_longterm_facts(
    pool: &PgPool,
    client: &reqwest::Client,
    session_id: &str,
) -> Result<usize, String> {
    let api_key = crate::mimir::groq_api_key()?;

    let tree_row = sqlx::query("SELECT tree_id FROM mimir_chat_sessions WHERE id = $1")
        .bind(session_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?;
    let tree_id: Option<String> = tree_row
        .and_then(|r| r.try_get::<Option<String>, _>("tree_id").ok())
        .flatten();
    let Some(tree_id) = tree_id else { return Ok(0) };

    let rows = sqlx::query(
        "SELECT ms.summary, ms.key_topics \
         FROM mimir_memory_shortterm ms \
         JOIN mimir_chat_sessions cs ON cs.id = ms.session_id \
         WHERE cs.tree_id = $1 AND ms.updated_at > NOW() - INTERVAL '24 hours' \
         ORDER BY ms.updated_at DESC LIMIT 5"
    )
    .bind(&tree_id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    if rows.is_empty() {
        return Ok(0);
    }

    let mut digest = String::new();
    for row in &rows {
        let summary: String = row.try_get("summary").unwrap_or_default();
        let topics: Vec<String> = row.try_get("key_topics").unwrap_or_default();
        digest.push_str(&format!("Session summary: {}\nTopics: {}\n\n", summary, topics.join(", ")));
    }

    let system = "Extract durable facts about the LEARNER from these tutoring-session summaries. \
                  Only facts about the user themself: their knowledge level, misconceptions, \
                  preferences, goals, background. NOT facts about the subject matter. \
                  Return ONLY JSON: {\"facts\": [{\"fact_key\": \"snake_case_key\", \
                  \"fact_value\": \"short string\", \"confidence\": 0.0}]} \
                  — at most 8 facts, empty array if none.";

    let (raw, latency) = crate::llm_client::call_llm(
        client,
        crate::constants::GROQ_API_URL,
        &api_key,
        "llama-3.3-70b-versatile",
        system,
        &digest,
        500,
        true,
        0.2,
    )
    .await?;
    crate::brain::log_prompt_call(
        pool.clone(), "mimir_extract_facts", "llama-3.3-70b-versatile", "mimir_extract_facts_v1",
        latency, true, None, None,
    );

    let parsed = parse_extracted_facts(&raw);
    let mut written = 0usize;
    for f in parsed.facts.into_iter().take(8) {
        let key = f.fact_key.trim().to_lowercase().replace(' ', "_");
        if key.is_empty() { continue; }
        if set_memory_fact(
            pool, "tree", Some(&tree_id), None, &key, None,
            f.fact_value, f.confidence.clamp(0.0, 1.0), "session_consolidation",
        )
        .await
        .is_ok()
        {
            written += 1;
        }
    }
    println!("🧠 [memory] extracted {} long-term facts for tree {}", written, tree_id);
    Ok(written)
}

#[tauri::command]
pub async fn consolidate_session_cmd(
    session_id: String,
    client: State<'_, reqwest::Client>,
    database: State<'_, Database>,
) -> Result<(), String> {
    consolidate_session(&database.pool, &*client, &session_id).await?;
    let _ = extract_longterm_facts(&database.pool, &*client, &session_id).await;
    Ok(())
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

    #[test]
    fn parse_consolidation_handles_fenced_json() {
        let raw = "```json\n{\"summary\": \"Covered RRF and pgvector.\", \"key_topics\": [\"rrf\", \"pgvector\"]}\n```";
        let out = parse_consolidation(raw).unwrap();
        assert_eq!(out.summary, "Covered RRF and pgvector.");
        assert_eq!(out.key_topics, vec!["rrf", "pgvector"]);
    }

    #[test]
    fn parse_extracted_facts_tolerates_garbage() {
        let good = "{\"facts\": [{\"fact_key\": \"career goal\", \"fact_value\": \"ML engineering\", \"confidence\": 0.8}]}";
        let out = parse_extracted_facts(good);
        assert_eq!(out.facts.len(), 1);
        assert_eq!(out.facts[0].fact_key, "career goal");

        let bad = "no json here at all";
        assert_eq!(parse_extracted_facts(bad).facts.len(), 0);
    }
}
