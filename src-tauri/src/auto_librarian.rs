// src-tauri/src/auto_librarian.rs
//
// Background janitor: every ~6h, re-run the full ingest enrichment chain
// (tag → match → skill-extract) for resources that missed the pipeline, and
// emit ygg-library-suggestion so the Library page can show a badge. Only
// re-runs idempotent work — no destructive surface. First pass runs shortly
// after startup.

use std::collections::HashSet;

use sqlx::{PgPool, Row};
use tauri::{AppHandle, Emitter};

use crate::orchestrator::JobQueue;

const INTERVAL_SECS: u64 = 6 * 60 * 60; // 6 hours
const WARMUP_SECS: u64 = 120;           // let startup settle before the first pass
const BATCH_CAP: i64 = 20;              // never flood the 64-slot queue

pub fn start_auto_librarian(pool: PgPool, app: AppHandle, client: reqwest::Client, queue: JobQueue) {
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(WARMUP_SECS)).await;
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(INTERVAL_SECS));
        loop {
            interval.tick().await; // 1st tick fires immediately (≈ after warmup), then every 6h
            run_pass(&pool, &app, &client, &queue).await;
        }
    });
}

async fn run_pass(pool: &PgPool, app: &AppHandle, client: &reqwest::Client, queue: &JobQueue) {
    // 1. Untagged top-level resources.
    let untagged: Vec<String> = sqlx::query(
        "SELECT id FROM mimir_resources \
         WHERE parent_id IS NULL AND (tags IS NULL OR cardinality(tags) = 0) \
         ORDER BY created_at DESC LIMIT $1"
    )
    .bind(BATCH_CAP)
    .fetch_all(pool)
    .await
    .unwrap_or_default()
    .iter()
    .filter_map(|r| r.try_get::<String, _>("id").ok())
    .collect();
    let untagged_n = untagged.len();

    // 2. Tagged-but-unmatched top-level resources.
    let unmatched: Vec<String> = sqlx::query(
        "SELECT r.id FROM mimir_resources r \
         WHERE r.parent_id IS NULL \
           AND r.tags IS NOT NULL AND cardinality(r.tags) > 0 \
           AND NOT EXISTS (SELECT 1 FROM mimir_node_links l WHERE l.resource_id = r.id) \
         ORDER BY r.created_at DESC LIMIT $1"
    )
    .bind(BATCH_CAP)
    .fetch_all(pool)
    .await
    .unwrap_or_default()
    .iter()
    .filter_map(|r| r.try_get::<String, _>("id").ok())
    .collect();
    let unmatched_n = unmatched.len();

    // Union + de-dup the stragglers, still capped so a pass can never flood the queue.
    let mut seen: HashSet<String> = HashSet::new();
    let mut stragglers: Vec<String> = Vec::new();
    for id in untagged.into_iter().chain(unmatched.into_iter()) {
        if seen.insert(id.clone()) {
            stragglers.push(id);
        }
    }
    stragglers.truncate(BATCH_CAP as usize);

    // Re-run the SAME recovery path a fresh ingest uses (tag → match → LLM skill
    // extraction) for each straggler. Idempotent: auto_tag_single re-tags safely,
    // and the enqueued match/extract jobs upsert their links.
    for id in &stragglers {
        crate::orchestrator::on_resource_ingested_async(pool, app, client, id, queue).await;
    }

    let queued = stragglers.len();
    if queued > 0 {
        let _ = app.emit("ygg-library-suggestion", serde_json::json!({
            "untagged": untagged_n,
            "unmatched": unmatched_n,
            "queued": queued,
        }));
        println!("🧹 [librarian] queued {} untagged + {} unmatched resource(s)", untagged_n, unmatched_n);
    }
}
