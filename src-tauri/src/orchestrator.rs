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
            Ok(_) => {}
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

pub async fn on_resource_ingested(
    pool: &PgPool,
    app: &AppHandle,
    client: &reqwest::Client,
    resource_id: &str,
) {
    if let Err(e) = auto_tag_single(pool, client, resource_id).await {
        println!("⚠️  [orch] auto_tag_single failed for {}: {}", resource_id, e);
    }

    if let Err(e) = match_resource_to_nodes(pool, client, resource_id).await {
        println!("⚠️  [orch] match_resource_to_nodes failed for {}: {}", resource_id, e);
    }

    let _ = app.emit("ygg-resource-ingested", serde_json::json!({
        "resourceId": resource_id,
    }));
    println!("📡 [orch] ygg-resource-ingested emitted (resource={})", resource_id);
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

async fn match_resource_to_nodes(pool: &PgPool, client: &reqwest::Client, resource_id: &str) -> Result<(), String> {
    let chunk_row = sqlx::query(
        "SELECT mc.content FROM mimir_chunks mc WHERE mc.resource_id = $1 LIMIT 1"
    )
    .bind(resource_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;

    let first_content = match chunk_row {
        Some(r) => r.try_get::<String, _>("content").unwrap_or_default(),
        None => return Ok(()),
    };

    if first_content.is_empty() {
        return Ok(());
    }

    let leaf_rows = sqlx::query(
        "SELECT id FROM tree_nodes
         WHERE type = 'leaf'
           AND id NOT IN (
               SELECT node_id FROM mimir_node_links WHERE resource_id = $1
           )"
    )
    .bind(resource_id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    if leaf_rows.is_empty() {
        return Ok(());
    }

    let mut matched = 0usize;

    for row in &leaf_rows {
        let node_id: String = match row.try_get("id") {
            Ok(v) => v,
            Err(_) => continue,
        };
        match crate::mimir_retrieval::match_node_impl(pool, client, &node_id).await {
            Ok(resources) if resources.iter().any(|r| r.id == resource_id) => matched += 1,
            Ok(_) => {}
            Err(e) => println!("⚠️  [orch] match_resource_to_nodes node {}: {}", node_id, e),
        }
    }

    if matched > 0 {
        println!("🔗 [orch] new resource matched to {} nodes", matched);
    }

    Ok(())
}
