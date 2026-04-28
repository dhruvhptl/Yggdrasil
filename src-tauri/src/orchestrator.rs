// src-tauri/src/orchestrator.rs
//
// Event-driven cascade layer + background job queue.
//
// Cascade functions (on_*) are called by command handlers after writes succeed.
// They are best-effort: errors are logged but never propagated.
//
// The JobQueue runs a background worker that processes long-running tasks
// (rematch, reembed, autotag, rescrape, infer-deps) off the UI thread.
// Jobs are processed one at a time; the worker never crashes on error.

use sqlx::PgPool;
use sqlx::Row;
use tauri::{AppHandle, Emitter};
use tokio::sync::mpsc;

// ─── Job definitions ──────────────────────────────────────────────────────────

pub(crate) enum OrchestratorJob {
    RematchAllNodes { tree_id: String },
    ReembedResources { resource_ids: Vec<String> },
    InferSkillDeps,
    AutoTagResources { resource_ids: Vec<String> },
    MatchResourceToNodes { resource_id: String },
}

// ─── JobQueue (managed Tauri state) ──────────────────────────────────────────

pub struct JobQueue {
    sender: mpsc::Sender<OrchestratorJob>,
}

impl JobQueue {
    fn new(sender: mpsc::Sender<OrchestratorJob>) -> Self {
        Self { sender }
    }

    pub async fn send(&self, job: OrchestratorJob) -> Result<(), String> {
        self.sender.send(job).await.map_err(|e| e.to_string())
    }
}

// ─── Worker startup ───────────────────────────────────────────────────────────

pub fn start_worker(pool: PgPool, app: AppHandle, client: reqwest::Client) -> JobQueue {
    let (tx, mut rx) = mpsc::channel::<OrchestratorJob>(64);

    tokio::spawn(async move {
        while let Some(job) = rx.recv().await {
            match job {
                OrchestratorJob::RematchAllNodes { tree_id } => {
                    run_rematch_all_nodes(&pool, &app, &client, &tree_id).await;
                }
                OrchestratorJob::ReembedResources { resource_ids } => {
                    run_reembed_resources(&pool, &app, &client, resource_ids).await;
                }
                OrchestratorJob::InferSkillDeps => {
                    run_infer_skill_deps(&pool, &app, &client).await;
                }
                OrchestratorJob::AutoTagResources { resource_ids } => {
                    run_autotag_resources(&pool, &app, &client, resource_ids).await;
                }
                OrchestratorJob::MatchResourceToNodes { resource_id } => {
                    println!("🔍 [worker/debug] dispatching MatchResourceToNodes resource={}", resource_id);
                    run_match_resource_to_nodes(&pool, &app, &client, &resource_id).await;
                    println!("🔍 [worker/debug] MatchResourceToNodes done resource={}", resource_id);
                }
            }
        }
    });

    JobQueue::new(tx)
}

// ─── Tauri commands — manual job dispatch ────────────────────────────────────

#[tauri::command]
pub async fn enqueue_rematch(
    tree_id: String,
    queue: tauri::State<'_, JobQueue>,
) -> Result<(), String> {
    queue.send(OrchestratorJob::RematchAllNodes { tree_id }).await
}

#[tauri::command]
pub async fn enqueue_reembed(
    resource_ids: Vec<String>,
    queue: tauri::State<'_, JobQueue>,
) -> Result<(), String> {
    queue.send(OrchestratorJob::ReembedResources { resource_ids }).await
}

#[tauri::command]
pub async fn enqueue_autotag(
    resource_ids: Vec<String>,
    queue: tauri::State<'_, JobQueue>,
) -> Result<(), String> {
    queue.send(OrchestratorJob::AutoTagResources { resource_ids }).await
}

#[tauri::command]
pub async fn enqueue_infer_deps(
    queue: tauri::State<'_, JobQueue>,
) -> Result<(), String> {
    queue.send(OrchestratorJob::InferSkillDeps).await
}

// ─── Job implementations ──────────────────────────────────────────────────────

async fn run_rematch_all_nodes(pool: &PgPool, app: &AppHandle, client: &reqwest::Client, tree_id: &str) {
    let rows = match sqlx::query(
        "SELECT id FROM tree_nodes WHERE tree_id = $1 AND type = 'leaf'"
    )
    .bind(tree_id)
    .fetch_all(pool)
    .await
    {
        Ok(r) => r,
        Err(e) => { println!("⚠️  [job/rematch] fetch nodes failed: {}", e); return; }
    };

    let node_ids: Vec<String> = rows.iter()
        .map(|r| r.try_get::<String, _>("id").unwrap_or_default())
        .collect();

    let total = node_ids.len();

    for (i, node_id) in node_ids.iter().enumerate() {
        if i > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        match crate::mimir_retrieval::match_node_impl(pool, client, node_id).await {
            Ok(_) => {
                // Cache the title embedding so MatchResourceToNodes can use the HNSW index
                if let Ok(Some(row)) = sqlx::query(
                    "SELECT title FROM tree_nodes WHERE id = $1 AND title_embedding IS NULL"
                )
                .bind(node_id)
                .fetch_optional(pool)
                .await
                {
                    if let Ok(title) = row.try_get::<String, _>("title") {
                        if let Ok(emb) = crate::mimir_ingest::get_embedding(client, &title).await {
                            let vec_str = crate::mimir_ingest::vector_str(&emb);
                            let _ = sqlx::query(
                                "UPDATE tree_nodes SET title_embedding = $1::vector WHERE id = $2"
                            )
                            .bind(&vec_str)
                            .bind(node_id)
                            .execute(pool)
                            .await;
                        }
                    }
                }
            }
            Err(e) => println!("⚠️  [job/rematch] node {}: {}", node_id, e),
        }
        let _ = app.emit("ygg-rematch-progress", serde_json::json!({
            "treeId": tree_id,
            "completed": i + 1,
            "total": total,
        }));
    }

    let _ = app.emit("ygg-rematch-complete", serde_json::json!({ "treeId": tree_id }));
    println!("📡 [job/rematch] ygg-rematch-complete (tree={})", tree_id);
}

async fn run_reembed_resources(pool: &PgPool, app: &AppHandle, client: &reqwest::Client, resource_ids: Vec<String>) {
    let total = resource_ids.len();

    for (i, resource_id) in resource_ids.iter().enumerate() {
        match crate::mimir_ingest::reembed_resource_inner(pool, client, resource_id).await {
            Ok(_) => {}
            Err(e) => println!("⚠️  [job/reembed] resource {}: {}", resource_id, e),
        }
        let _ = app.emit("ygg-reembed-progress", serde_json::json!({
            "resourceId": resource_id,
            "completed": i + 1,
            "total": total,
        }));
    }

    let _ = app.emit("ygg-reembed-complete", serde_json::json!({}));
    println!("📡 [job/reembed] ygg-reembed-complete ({} resources)", total);
}

async fn run_infer_skill_deps(pool: &PgPool, app: &AppHandle, client: &reqwest::Client) {
    match crate::skill_commands::infer_skill_deps_inner(client, pool).await {
        Ok(count) => println!("🔗 [job/infer-deps] {} deps written", count),
        Err(e) => println!("⚠️  [job/infer-deps] failed: {}", e),
    }
    let _ = app.emit("ygg-skills-updated", serde_json::json!({}));
    println!("📡 [job/infer-deps] ygg-skills-updated emitted");
}

async fn run_autotag_resources(pool: &PgPool, app: &AppHandle, client: &reqwest::Client, resource_ids: Vec<String>) {
    for resource_id in &resource_ids {
        if let Err(e) = auto_tag_single(pool, client, resource_id).await {
            println!("⚠️  [job/autotag] resource {}: {}", resource_id, e);
        } else {
            let _ = app.emit("ygg-resource-ingested", serde_json::json!({ "resourceId": resource_id }));
        }
    }
    println!("📡 [job/autotag] done ({} resources)", resource_ids.len());
}

// ─── on_tree_generated ────────────────────────────────────────────────────────

pub async fn on_tree_generated(
    pool: &PgPool,
    app: &AppHandle,
    tree_id: &str,
    project_id: &str,
) {
    if let Err(e) = sqlx::query(
        "UPDATE projects SET active_tree_id = $1 WHERE id = $2"
    )
    .bind(tree_id)
    .bind(project_id)
    .execute(pool)
    .await
    {
        println!("⚠️  [orch] active_tree_id update failed: {}", e);
    }

    if let Err(e) = crate::skill_commands::sync_trees_inner(pool).await {
        println!("⚠️  [orch] sync_trees_inner failed: {}", e);
    } else if let Err(e) = crate::skill_commands::recalculate_levels_inner(pool).await {
        println!("⚠️  [orch] recalculate_levels_inner failed: {}", e);
    }

    if let Err(e) = crate::skill_commands::sync_concept_slugs_inner(pool).await {
        println!("⚠️  [orch] sync_concept_slugs_inner failed: {}", e);
    }

    let _ = app.emit("ygg-tree-generated", serde_json::json!({
        "treeId": tree_id,
        "projectId": project_id,
    }));
    println!("📡 [orch] ygg-tree-generated emitted (tree={})", tree_id);
}

// ─── on_resource_ingested ─────────────────────────────────────────────────────

pub async fn on_resource_ingested_async(
    pool: &PgPool,
    app: &AppHandle,
    client: &reqwest::Client,
    resource_id: &str,
    queue: &JobQueue,
) {
    println!("🔍 [orch/debug] on_resource_ingested_async START resource={}", resource_id);

    println!("🔍 [orch/debug] calling auto_tag_single...");
    if let Err(e) = auto_tag_single(pool, client, resource_id).await {
        println!("⚠️  [orch] auto_tag_single failed for {}: {}", resource_id, e);
    }
    println!("🔍 [orch/debug] auto_tag_single done");

    // Enqueue node matching as a background job — non-blocking.
    println!("🔍 [orch/debug] sending MatchResourceToNodes to queue...");
    match queue.send(OrchestratorJob::MatchResourceToNodes {
        resource_id: resource_id.to_string(),
    }).await {
        Ok(()) => println!("🔍 [orch/debug] MatchResourceToNodes enqueued OK for {}", resource_id),
        Err(e) => println!("⚠️  [orch] enqueue MatchResourceToNodes failed: {}", e),
    }

    let _ = app.emit("ygg-resource-ingested", serde_json::json!({
        "resourceId": resource_id,
    }));
    println!("📡 [orch] ygg-resource-ingested emitted (resource={})", resource_id);
    println!("🔍 [orch/debug] on_resource_ingested_async END resource={}", resource_id);
}

// ─── on_resource_completed ────────────────────────────────────────────────────

pub async fn on_resource_completed(
    pool: &PgPool,
    app: &AppHandle,
    resource_id: &str,
) {
    let linked = match sqlx::query(
        "SELECT DISTINCT mnl.node_id, tn.tree_id
         FROM mimir_node_links mnl
         JOIN tree_nodes tn ON tn.id = mnl.node_id
         WHERE mnl.resource_id = $1"
    )
    .bind(resource_id)
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            println!("⚠️  [orch] on_resource_completed fetch failed: {}", e);
            return;
        }
    };

    if linked.is_empty() {
        let _ = app.emit("ygg-resource-completed", serde_json::json!({ "resourceId": resource_id }));
        return;
    }

    let mut affected_trees: std::collections::HashSet<String> = std::collections::HashSet::new();

    for row in &linked {
        let node_id: String = match row.try_get("node_id") {
            Ok(v) => v, Err(e) => { println!("⚠️  [orch] node_id read: {}", e); continue; }
        };
        let tree_id: String = match row.try_get("tree_id") {
            Ok(v) => v, Err(e) => { println!("⚠️  [orch] tree_id read: {}", e); continue; }
        };

        if let Err(e) = sqlx::query(
            "UPDATE tree_nodes SET progress = LEAST(COALESCE(progress, 0) + 20, 100) WHERE id = $1"
        )
        .bind(&node_id)
        .execute(pool)
        .await
        {
            println!("⚠️  [orch] progress bump failed for node {}: {}", node_id, e);
        }

        affected_trees.insert(tree_id);
    }

    for tree_id in &affected_trees {
        if let Err(e) = recalculate_tree_progress_inner(pool, tree_id).await {
            println!("⚠️  [orch] recalculate_tree_progress failed for {}: {}", tree_id, e);
        }
        if let Err(e) = recalculate_unlocks_inner(pool, tree_id).await {
            println!("⚠️  [orch] recalculate_unlocks failed for {}: {}", tree_id, e);
        }
    }

    if let Err(e) = crate::skill_commands::sync_trees_inner(pool).await {
        println!("⚠️  [orch] sync_trees_inner failed: {}", e);
    } else if let Err(e) = crate::skill_commands::recalculate_levels_inner(pool).await {
        println!("⚠️  [orch] recalculate_levels_inner failed: {}", e);
    }

    // Write skill evidence for every high-confidence (green) node match.
    let skills_updated = write_mimir_resource_evidence(pool, resource_id).await;
    if skills_updated > 0 {
        if let Err(e) = crate::skill_commands::recalculate_levels_inner(pool).await {
            println!("⚠️  [orch] recalculate_levels_inner (mimir evidence) failed: {}", e);
        }
        let _ = app.emit("ygg-skills-updated", serde_json::json!({}));
        println!("📡 [orch] ygg-skills-updated emitted ({} skills from mimir evidence)", skills_updated);
    }

    let _ = app.emit("ygg-resource-completed", serde_json::json!({ "resourceId": resource_id }));
    println!("📡 [orch] ygg-resource-completed emitted (resource={})", resource_id);
}

// ─── on_checkpoint_completed ──────────────────────────────────────────────────

pub async fn on_checkpoint_completed(
    pool: &PgPool,
    app: &AppHandle,
    node_id: &str,
    tree_id: &str,
    queue: &JobQueue,
) {
    if let Err(e) = recalculate_tree_progress_inner(pool, tree_id).await {
        println!("⚠️  [orch] recalculate_tree_progress failed: {}", e);
    }
    if let Err(e) = recalculate_unlocks_inner(pool, tree_id).await {
        println!("⚠️  [orch] recalculate_unlocks failed: {}", e);
    }

    if let Err(e) = crate::skill_commands::sync_trees_inner(pool).await {
        println!("⚠️  [orch] sync_trees_inner failed: {}", e);
    } else if let Err(e) = crate::skill_commands::recalculate_levels_inner(pool).await {
        println!("⚠️  [orch] recalculate_levels_inner failed: {}", e);
    }

    let parent_at_100 = sqlx::query(
        "SELECT parent.id
         FROM tree_nodes leaf
         JOIN tree_nodes parent ON parent.id = leaf.parent_id
         WHERE leaf.id = $1
           AND leaf.type = 'leaf'
           AND parent.type = 'branch'
           AND parent.progress = 100"
    )
    .bind(node_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();

    if parent_at_100.is_some() {
        // Enqueue debounced infer-deps via job queue (replaces the prior tokio::spawn debounce)
        if let Err(e) = queue.send(OrchestratorJob::InferSkillDeps).await {
            println!("⚠️  [orch] enqueue InferSkillDeps failed: {}", e);
        }
    }

    let _ = app.emit("ygg-checkpoint-completed", serde_json::json!({
        "nodeId": node_id,
        "treeId": tree_id,
    }));
    println!("📡 [orch] ygg-checkpoint-completed emitted (node={})", node_id);
}

// ─── on_resume_parsed ─────────────────────────────────────────────────────────

pub async fn on_resume_parsed(pool: &PgPool, app: &AppHandle) {
    if let Err(e) = crate::skill_commands::sync_resume_inner(pool).await {
        println!("⚠️  [orch] sync_resume_inner failed: {}", e);
    } else if let Err(e) = crate::skill_commands::recalculate_levels_inner(pool).await {
        println!("⚠️  [orch] recalculate_levels_inner failed: {}", e);
    }

    let _ = app.emit("ygg-skills-updated", serde_json::json!({}));
    println!("📡 [orch] ygg-skills-updated emitted (resume parsed)");
}

// ─── on_work_skills_extracted ─────────────────────────────────────────────────

pub async fn on_work_skills_extracted(pool: &PgPool, app: &AppHandle) {
    if let Err(e) = crate::skill_commands::sync_work_inner(pool).await {
        println!("⚠️  [orch] sync_work_inner failed: {}", e);
    } else if let Err(e) = crate::skill_commands::recalculate_levels_inner(pool).await {
        println!("⚠️  [orch] recalculate_levels_inner failed: {}", e);
    }

    let _ = app.emit("ygg-skills-updated", serde_json::json!({}));
    println!("📡 [orch] ygg-skills-updated emitted (work skills extracted)");
}

// ─── Private helpers ──────────────────────────────────────────────────────────

async fn recalculate_tree_progress_inner(pool: &PgPool, tree_id: &str) -> Result<(), String> {
    let rows = sqlx::query(
        "SELECT id, parent_id, progress FROM tree_nodes WHERE tree_id = $1"
    )
    .bind(tree_id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut children_map: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    let mut progress_map: std::collections::HashMap<String, i32> = std::collections::HashMap::new();
    let mut roots: Vec<String> = Vec::new();

    for row in &rows {
        let id: String = row.try_get("id").map_err(|e| e.to_string())?;
        let parent_id: Option<String> = row.try_get("parent_id").map_err(|e| e.to_string())?;
        let progress: Option<i32> = row.try_get("progress").map_err(|e| e.to_string())?;
        progress_map.insert(id.clone(), progress.unwrap_or(0));
        match parent_id {
            Some(pid) => children_map.entry(pid).or_default().push(id),
            None => roots.push(id),
        }
    }

    let mut order: Vec<String> = Vec::new();
    let mut stack = roots;
    while let Some(id) = stack.pop() {
        order.push(id.clone());
        if let Some(kids) = children_map.get(&id) {
            stack.extend(kids.iter().cloned());
        }
    }
    order.reverse();

    for id in &order {
        if let Some(kids) = children_map.get(id) {
            if !kids.is_empty() {
                let avg: i32 = kids.iter()
                    .map(|kid| *progress_map.get(kid).unwrap_or(&0))
                    .sum::<i32>() / kids.len() as i32;
                progress_map.insert(id.clone(), avg);
                sqlx::query("UPDATE tree_nodes SET progress = $1 WHERE id = $2")
                    .bind(avg)
                    .bind(id)
                    .execute(pool)
                    .await
                    .map_err(|e| e.to_string())?;
            }
        }
    }

    Ok(())
}

async fn recalculate_unlocks_inner(pool: &PgPool, tree_id: &str) -> Result<(), String> {
    sqlx::query(
        "UPDATE tree_nodes SET is_locked = false
         WHERE id IN (
             SELECT DISTINCT ON (parent_id) id
             FROM tree_nodes
             WHERE tree_id = $1 AND type = 'branch'
             ORDER BY parent_id, order_index ASC
         )"
    )
    .bind(tree_id)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    sqlx::query(
        "UPDATE tree_nodes SET is_locked = false
         WHERE id IN (
             SELECT next_skill.id
             FROM tree_nodes completed_skill
             JOIN tree_nodes next_skill
               ON next_skill.parent_id   = completed_skill.parent_id
              AND next_skill.tree_id     = completed_skill.tree_id
              AND next_skill.type        = 'branch'
              AND next_skill.order_index = (
                  SELECT MIN(order_index)
                  FROM tree_nodes
                  WHERE parent_id   = completed_skill.parent_id
                    AND type        = 'branch'
                    AND order_index > completed_skill.order_index
              )
             WHERE completed_skill.tree_id  = $1
               AND completed_skill.type     = 'branch'
               AND completed_skill.progress = 100
         )"
    )
    .bind(tree_id)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(())
}

async fn auto_tag_single(pool: &PgPool, client: &reqwest::Client, resource_id: &str) -> Result<(), String> {
    let row = sqlx::query(
        "SELECT title, tags FROM mimir_resources WHERE id = $1"
    )
    .bind(resource_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;

    let row = match row {
        Some(r) => r,
        None => return Ok(()),
    };

    let existing_tags: Vec<String> = row.try_get("tags").unwrap_or_default();
    if !existing_tags.is_empty() {
        return Ok(());
    }

    let title: String = row.try_get("title").unwrap_or_default();
    if title.is_empty() {
        return Ok(());
    }

    let api_key = std::env::var("GROQ_API_KEY")
        .map_err(|_| "GROQ_API_KEY not set".to_string())?;

    let response = client
        .post(crate::constants::GROQ_API_URL)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "model": "llama-3.3-70b-versatile",
            "messages": [
                {
                    "role": "system",
                    "content": "You are a tag classifier. Return ONLY a JSON array of tag strings."
                },
                {
                    "role": "user",
                    "content": format!(
                        "Given this resource title: '{}', suggest 2-4 tags from these categories:\n\
                         Technology: rust, typescript, python, react, sql, julia, node, postgres, tauri, d3, numpy\n\
                         Type: tutorial, docs, video, article, reference\n\
                         Level: beginner, intermediate, advanced\n\
                         Return ONLY a JSON array of strings, e.g. [\"rust\", \"docs\"]",
                        title
                    )
                }
            ],
            "temperature": 0.2,
            "max_tokens": 100
        }))
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| format!("auto_tag_single request failed: {}", e))?;

    let body: serde_json::Value = response.json().await
        .map_err(|e| format!("auto_tag_single parse failed: {}", e))?;

    let content = body["choices"][0]["message"]["content"].as_str().unwrap_or("[]");
    let clean = content
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    let tags: Vec<String> = serde_json::from_str(clean).unwrap_or_default();
    if tags.is_empty() {
        return Ok(());
    }

    sqlx::query("UPDATE mimir_resources SET tags = $1 WHERE id = $2")
        .bind(&tags)
        .bind(resource_id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;

    println!("🏷️  [orch] auto-tagged \"{}\" → {:?}", title, tags);
    Ok(())
}

/// For a just-completed resource, find all green-matched nodes (relevance_score < 0.3),
/// upsert a universal_skill for each node title, and write a skill_evidence row.
/// Returns the number of evidence rows written.
async fn write_mimir_resource_evidence(pool: &PgPool, resource_id: &str) -> usize {
    // Fetch resource title for the evidence payload.
    let title: String = match sqlx::query(
        "SELECT title FROM mimir_resources WHERE id = $1"
    )
    .bind(resource_id)
    .fetch_optional(pool)
    .await
    {
        Ok(Some(r)) => r.try_get("title").unwrap_or_default(),
        _ => String::new(),
    };

    // Green matches: relevance_score is a cosine distance so lower = better match.
    let rows = match sqlx::query(
        "SELECT mnl.node_id, mnl.relevance_score, tn.title AS node_title
         FROM mimir_node_links mnl
         JOIN tree_nodes tn ON tn.id = mnl.node_id
         WHERE mnl.resource_id = $1
           AND mnl.relevance_score < 0.3"
    )
    .bind(resource_id)
    .fetch_all(pool)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            println!("⚠️  [orch] write_mimir_resource_evidence fetch failed: {}", e);
            return 0;
        }
    };

    if rows.is_empty() {
        return 0;
    }

    let mut written = 0usize;

    for row in &rows {
        let node_id: String = match row.try_get("node_id") {
            Ok(v) => v, Err(_) => continue,
        };
        let relevance_score: f64 = match row.try_get("relevance_score") {
            Ok(v) => v, Err(_) => continue,
        };
        let node_title: String = match row.try_get("node_title") {
            Ok(v) => v, Err(_) => continue,
        };
        if node_title.is_empty() {
            continue;
        }

        let evidence = serde_json::json!({
            "type": "mimir_resource",
            "resource_id": resource_id,
            "resource_title": title,
            "node_id": node_id,
            "relevance_score": relevance_score,
        });

        let skill_id = match crate::skill_commands::upsert_skill(pool, &node_title, None, evidence.clone()).await {
            Ok(id) => id,
            Err(e) => {
                println!("⚠️  [orch] upsert_skill '{}' failed: {}", node_title, e);
                continue;
            }
        };

        // Write the first-class skill_evidence row (idempotent via unique index).
        let evidence_id = format!("se_{}", uuid::Uuid::new_v4().to_string().replace('-', ""));
        match sqlx::query(
            "INSERT INTO skill_evidence (id, skill_id, source_type, source_id, payload, recorded_at)
             VALUES ($1, $2, 'mimir_resource', $3, $4::jsonb, NOW())
             ON CONFLICT DO NOTHING"
        )
        .bind(&evidence_id)
        .bind(&skill_id)
        .bind(resource_id)
        .bind(evidence.to_string())
        .execute(pool)
        .await
        {
            Ok(r) if r.rows_affected() > 0 => {
                written += 1;
                println!("📚 [orch] skill evidence: '{}' ← resource '{}' (score={:.3})", node_title, title, relevance_score);
            }
            Ok(_) => {} // already exists
            Err(e) => println!("⚠️  [orch] skill_evidence insert failed for '{}': {}", node_title, e),
        }
    }

    written
}

async fn run_match_resource_to_nodes(pool: &PgPool, app: &AppHandle, client: &reqwest::Client, resource_id: &str) {
    println!("🔍 [job/match-resource/debug] START resource={}", resource_id);

    // Fetch resource title and the active tree for its project (for same-tree boost)
    let meta_row = sqlx::query(
        "SELECT mr.title, p.active_tree_id \
         FROM mimir_resources mr \
         LEFT JOIN mimir_node_links mnl ON mnl.resource_id = mr.id \
         LEFT JOIN tree_nodes tn ON tn.id = mnl.node_id \
         LEFT JOIN projects p ON p.active_tree_id = tn.tree_id \
         WHERE mr.id = $1 \
         LIMIT 1"
    )
    .bind(resource_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();

    // Fallback: just get the title without the tree join
    let (title, active_tree_id) = if let Some(row) = meta_row {
        let t = row.try_get::<String, _>("title").unwrap_or_default();
        let tree: Option<String> = row.try_get("active_tree_id").unwrap_or(None);
        println!("🔍 [job/match-resource/debug] meta ok title=\"{}\" active_tree={:?}", t, tree);
        (t, tree)
    } else {
        println!("🔍 [job/match-resource/debug] meta join returned no row, falling back to title-only query");
        let t = sqlx::query("SELECT title FROM mimir_resources WHERE id = $1")
            .bind(resource_id)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten()
            .and_then(|r| r.try_get::<String, _>("title").ok())
            .unwrap_or_default();
        println!("🔍 [job/match-resource/debug] fallback title=\"{}\"", t);
        (t, None)
    };

    // Get resource embedding from its first chunk
    let embed_result: Result<Vec<f32>, String> = async {
        let chunk_row = sqlx::query(
            "SELECT mc.content FROM mimir_chunks mc WHERE mc.resource_id = $1 LIMIT 1"
        )
        .bind(resource_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?;

        let content = match chunk_row {
            Some(r) => r.try_get::<String, _>("content").unwrap_or_default(),
            None => return Err("no chunks".to_string()),
        };
        if content.is_empty() {
            return Err("empty content".to_string());
        }
        crate::mimir_ingest::get_embedding(client, &content).await
    }.await;

    let embedding = match embed_result {
        Ok(e) => { println!("🔍 [job/match-resource/debug] embed OK ({} dims)", e.len()); e }
        Err(e) => {
            println!("⚠️  [job/match-resource] embed failed for {}: {}", resource_id, e);
            return;
        }
    };

    let vec_str = crate::mimir_ingest::vector_str(&embedding);

    // ANN search: top-10 leaf nodes by title_embedding cosine distance
    println!("🔍 [job/match-resource/debug] running HNSW ANN query...");
    let candidate_rows = match sqlx::query(
        "SELECT tn.id, tn.title, tn.tree_id, \
                tn.title_embedding <=> $1::vector AS distance \
         FROM tree_nodes tn \
         WHERE tn.type = 'leaf' \
           AND tn.title_embedding IS NOT NULL \
         ORDER BY tn.title_embedding <=> $1::vector \
         LIMIT 10"
    )
    .bind(&vec_str)
    .fetch_all(pool)
    .await
    {
        Ok(rows) => {
            println!("🔍 [job/match-resource/debug] ANN returned {} candidates", rows.len());
            rows
        }
        Err(e) => {
            println!("⚠️  [job/match-resource] ANN query failed: {}", e);
            vec![]
        }
    };

    // Tokenize the resource title for lexical overlap boost
    let stopwords: std::collections::HashSet<&str> = [
        "the", "a", "an", "of", "in", "for", "to", "and", "or", "with",
    ].iter().copied().collect();
    let resource_tokens: std::collections::HashSet<String> = title
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty() && !stopwords.contains(*t))
        .map(|t| t.to_string())
        .collect();

    // Rerank: apply boosts in-memory, collect (final_score, node_id, node_title, tree_id)
    struct Candidate {
        node_id: String,
        final_score: f64,
    }

    let mut candidates: Vec<Candidate> = Vec::with_capacity(candidate_rows.len());

    for row in &candidate_rows {
        let node_id: String = match row.try_get("id") { Ok(v) => v, Err(_) => continue };
        let node_title: String = row.try_get("title").unwrap_or_default();
        let node_tree_id: String = row.try_get("tree_id").unwrap_or_default();
        let ann_distance: f64 = row.try_get("distance").unwrap_or(1.0);

        let same_tree_boost = if active_tree_id.as_deref() == Some(node_tree_id.as_str()) {
            0.05_f64
        } else {
            0.0
        };

        let node_tokens: std::collections::HashSet<String> = node_title
            .to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|t| !t.is_empty() && !stopwords.contains(*t))
            .map(|t| t.to_string())
            .collect();
        let lexical_boost = if !resource_tokens.is_empty()
            && resource_tokens.iter().any(|tok| node_tokens.contains(tok))
        {
            0.03_f64
        } else {
            0.0
        };

        let final_score = ann_distance - same_tree_boost - lexical_boost;
        let _ = node_title; // used only for tokenization above
        candidates.push(Candidate { node_id, final_score });
    }

    // Sort ascending (lower = better), keep top 5, apply threshold
    candidates.sort_by(|a, b| a.final_score.partial_cmp(&b.final_score).unwrap_or(std::cmp::Ordering::Equal));
    candidates.truncate(5);

    println!("🔍 [job/match-resource/debug] reranked: {} candidates after truncate, scores: {:?}",
        candidates.len(),
        candidates.iter().map(|c| format!("{:.3}", c.final_score)).collect::<Vec<_>>()
    );

    let top_score = candidates.first().map(|c| c.final_score).unwrap_or(1.0);
    let mut matched = 0usize;
    let mut any_green = false;

    for candidate in &candidates {
        if candidate.final_score >= 0.55 {
            continue;
        }

        let link_id = uuid::Uuid::new_v4().to_string();
        let _ = sqlx::query(
            "INSERT INTO mimir_node_links (id, resource_id, node_id, relevance_score) \
             VALUES ($1, $2, $3, $4) \
             ON CONFLICT (resource_id, node_id) DO UPDATE SET \
               relevance_score = EXCLUDED.relevance_score"
        )
        .bind(&link_id)
        .bind(resource_id)
        .bind(&candidate.node_id)
        .bind(candidate.final_score as f32)
        .execute(pool)
        .await
        .ok();

        if candidate.final_score < 0.3 {
            any_green = true;
        }

        matched += 1;
    }

    if any_green {
        let _ = write_mimir_resource_evidence(pool, resource_id).await;
    }

    if matched > 0 {
        println!("🔗 [job/match-resource] matched {} nodes for resource '{}' (top score: {:.3})", matched, title, top_score);
    }

    let _ = app.emit("ygg-resource-ingested", serde_json::json!({ "resourceId": resource_id }));
}

