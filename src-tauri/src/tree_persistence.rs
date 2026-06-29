// src-tauri/src/tree_persistence.rs

use sqlx::Row;
use uuid::Uuid;
use crate::database::Database;
use crate::llm_client::SkillTree;
use crate::prompt_builders::Concept;

// Synthetic "Skills" project (migration 043, Option C) — all skill-seeded
// trees live under this id. skill_project_links should NOT include this row;
// the skill→tree linkage is recorded in skill_trees instead.
const SKILLS_PROJECT_ID: &str = "00000000-0000-0000-0000-000000000001";

// ─── Database helpers ────────────────────────────────────────────────────────

/// Save AI-generated skill tree to database.
/// Returns (tree_id, leaf_node_ids) so callers can trigger auto-matching.
///
/// `sorted_concepts` — the topologically-sorted concept graph produced during
/// Phase 1. When provided the function:
///   1. Serialises it to JSONB and stores it on the `trees` row.
///   2. After inserting all nodes, matches each branch/leaf node title against
///      the concept names and sets `concept_slug` to the matching concept id.
pub(crate) async fn save_tree_to_database(
    skill_tree: &SkillTree,
    database: &Database,
    sorted_concepts: Option<&[Concept]>,
) -> Result<(String, Vec<String>), String> {
    let tree_id = Uuid::new_v4().to_string();
    let tree_name = format!("AI Generated - {}", chrono::Utc::now().format("%Y-%m-%d %H:%M"));

    // Serialise concept graph for storage (best-effort — never fatal)
    let concept_graph_json: Option<serde_json::Value> = sorted_concepts
        .map(|cs| serde_json::to_value(cs).unwrap_or(serde_json::Value::Null));

    sqlx::query(
        "INSERT INTO trees (id, project_id, name, concept_graph) VALUES ($1, $2, $3, $4)"
    )
    .bind(&tree_id)
    .bind(&skill_tree.project_id)
    .bind(&tree_name)
    .bind(&concept_graph_json)
    .execute(&database.pool)
    .await
    .map_err(|e| format!("Failed to create tree: {}", e))?;

    println!("📦 Created tree: {}", tree_id);

    let mut node_ids: Vec<(String, Option<String>)> = Vec::new();
    let mut leaf_node_ids: Vec<String> = Vec::new();
    let mut order_counter: i32 = 0;

    for phase in &skill_tree.phases {
        let phase_node_id = Uuid::new_v4().to_string();
        let phase_tasks = serde_json::json!([]);

        sqlx::query(
            "INSERT INTO tree_nodes (id, tree_id, parent_id, type, title, description, progress, tasks, resources, x, y, order_index) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)"
        )
        .bind(&phase_node_id).bind(&tree_id).bind(None::<&str>).bind("trunk")
        .bind(&phase.name).bind(&phase.description).bind(0i32)
        .bind(&phase_tasks).bind(None::<serde_json::Value>)
        .bind(None::<f64>).bind(None::<f64>).bind(order_counter)
        .execute(&database.pool)
        .await
        .map_err(|e| format!("Failed to create phase node: {}", e))?;

        node_ids.push((phase_node_id.clone(), None));
        order_counter += 1;
        println!("  🌳 Phase: {}", phase.name);

        for (skill_index, skill) in phase.skills.iter().enumerate() {
            let skill_node_id = Uuid::new_v4().to_string();
            let skill_tasks = serde_json::json!([]);
            // First skill of each phase is unlocked; subsequent skills start locked
            let is_locked = skill_index > 0;

            sqlx::query(
                "INSERT INTO tree_nodes (id, tree_id, parent_id, type, title, description, progress, tasks, resources, x, y, order_index, is_locked) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)"
            )
            .bind(&skill_node_id).bind(&tree_id).bind(&phase_node_id).bind("branch")
            .bind(&skill.name).bind(&skill.description).bind(0i32)
            .bind(&skill_tasks).bind(None::<serde_json::Value>)
            .bind(None::<f64>).bind(None::<f64>).bind(order_counter)
            .bind(is_locked)
            .execute(&database.pool)
            .await
            .map_err(|e| format!("Failed to create skill node: {}", e))?;

            node_ids.push((skill_node_id.clone(), Some(phase_node_id.clone())));
            order_counter += 1;
            println!("    🌿 Skill: {}", skill.name);

            for checkpoint in &skill.checkpoints {
                let cp_node_id = Uuid::new_v4().to_string();
                let cp_tasks = serde_json::json!({
                    "mastery_criteria": checkpoint.mastery_criteria,
                    "exercises": checkpoint.exercises,
                    "notes": "",
                    "completed": false
                });

                sqlx::query(
                    "INSERT INTO tree_nodes (id, tree_id, parent_id, type, title, description, progress, tasks, resources, x, y, order_index) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)"
                )
                .bind(&cp_node_id).bind(&tree_id).bind(&skill_node_id).bind("leaf")
                .bind(&checkpoint.title).bind(&checkpoint.mastery_criteria).bind(0i32)
                .bind(&cp_tasks).bind(None::<serde_json::Value>)
                .bind(None::<f64>).bind(None::<f64>).bind(order_counter)
                .execute(&database.pool)
                .await
                .map_err(|e| format!("Failed to create checkpoint node: {}", e))?;

                leaf_node_ids.push(cp_node_id.clone());
                node_ids.push((cp_node_id.clone(), Some(skill_node_id.clone())));
                order_counter += 1;
                println!("      🍃 Checkpoint: {}", checkpoint.title);
            }
        }
    }

    // Create edges
    for (node_id, parent_id) in &node_ids {
        if let Some(parent) = parent_id {
            let edge_id = Uuid::new_v4().to_string();
            sqlx::query!(
                "INSERT INTO tree_edges (id, tree_id, source_node_id, target_node_id) VALUES ($1, $2, $3, $4)",
                edge_id,
                tree_id,
                parent,
                node_id
            )
            .execute(&database.pool)
            .await
            .map_err(|e| format!("Failed to create edge: {}", e))?;
        }
    }

    let edge_count = node_ids.iter().filter(|(_, p)| p.is_some()).count();
    println!(
        "✅ Saved tree: {} nodes, {} edges, {} leaf nodes",
        node_ids.len(),
        edge_count,
        leaf_node_ids.len()
    );

    // Populate concept_slug on nodes from the concept graph.
    // For each (node_id, node_title, node_description) try:
    //   1. Case-insensitive exact match on concept name
    //   2. Concept name is a substring of node title (or vice-versa)
    //   3. Concept id appears in the node title/description
    if let Some(concepts) = sorted_concepts {
        if !concepts.is_empty() {
            // Collect all nodes with their titles and descriptions
            let node_rows = sqlx::query(
                "SELECT id, title, COALESCE(description, '') AS description, type \
                 FROM tree_nodes WHERE tree_id = $1"
            )
            .bind(&tree_id)
            .fetch_all(&database.pool)
            .await
            .unwrap_or_default();

            let mut slug_count = 0usize;
            for row in &node_rows {
                let node_id: String = match row.try_get("id") { Ok(v) => v, Err(_) => continue };
                let title: String = row.try_get("title").unwrap_or_default();
                let desc: String = row.try_get("description").unwrap_or_default();
                let title_lower = title.to_lowercase();
                let desc_lower = desc.to_lowercase();

                // Find best matching concept
                let matched = concepts.iter().find(|c| {
                    let name_lower = c.name.to_lowercase();
                    // Exact match
                    title_lower == name_lower
                    // Title contains concept name
                    || title_lower.contains(&name_lower)
                    // Concept name contains title (for short titles)
                    || (title_lower.len() >= 4 && name_lower.contains(&title_lower))
                    // Concept id appears in title or description
                    || title_lower.contains(&c.id)
                    || desc_lower.contains(&c.id)
                });

                if let Some(concept) = matched {
                    let _ = sqlx::query(
                        "UPDATE tree_nodes SET concept_slug = $1 WHERE id = $2"
                    )
                    .bind(&concept.id)
                    .bind(&node_id)
                    .execute(&database.pool)
                    .await;
                    slug_count += 1;
                }
            }
            println!("🔖 concept_slug set on {}/{} nodes", slug_count, node_rows.len());
        }
    }

    // ── Skill tagging pass (fire-and-forget) ────────────────────────────────
    // Walk this tree's nodes, resolve concept_slug → universal_skills.id, and
    // populate skill_trees (always) + skill_project_links (skipping the
    // synthetic Skills project). Errors are logged, never propagated — tree
    // generation has already succeeded by this point.
    let _ = tag_skills_for_tree(
        &database.pool,
        &tree_id,
        &skill_tree.project_id,
    ).await;

    Ok((tree_id, leaf_node_ids))
}

/// Resolve concept_slugs on a freshly inserted tree to skill_ids and write
/// link rows into skill_trees and skill_project_links. Best-effort — every
/// step swallows its own errors.
async fn tag_skills_for_tree(
    pool: &sqlx::PgPool,
    tree_id: &str,
    project_id: &str,
) -> () {
    // 1. Collect distinct non-null concept_slugs for this tree.
    let slug_rows = match sqlx::query(
        "SELECT DISTINCT concept_slug FROM tree_nodes \
         WHERE tree_id = $1 AND concept_slug IS NOT NULL"
    )
    .bind(tree_id)
    .fetch_all(pool)
    .await {
        Ok(rs) => rs,
        Err(e) => {
            println!("⚠️  tag_skills_for_tree: load slugs failed: {}", e);
            return;
        }
    };

    if slug_rows.is_empty() {
        return;
    }

    let slugs: Vec<String> = slug_rows.iter()
        .filter_map(|r| r.try_get::<String, _>("concept_slug").ok())
        .filter(|s| !s.is_empty())
        .collect();
    if slugs.is_empty() {
        return;
    }

    let is_skills_project = project_id == SKILLS_PROJECT_ID;
    let mut linked_trees = 0usize;
    let mut linked_projects = 0usize;

    // 2-4. Per-slug resolution + inserts.
    for slug in &slugs {
        let skill_row = sqlx::query(
            "SELECT id FROM universal_skills WHERE concept_slug = $1 LIMIT 1"
        )
        .bind(slug)
        .fetch_optional(pool)
        .await;

        let skill_id: String = match skill_row {
            Ok(Some(r)) => match r.try_get("id") {
                Ok(s) => s,
                Err(_) => continue,
            },
            Ok(None) => continue,
            Err(e) => {
                println!("⚠️  tag_skills_for_tree: resolve '{}' failed: {}", slug, e);
                continue;
            }
        };

        // skill_trees — always.
        match sqlx::query(
            "INSERT INTO skill_trees (skill_id, tree_id) VALUES ($1, $2) \
             ON CONFLICT DO NOTHING"
        )
        .bind(&skill_id)
        .bind(tree_id)
        .execute(pool)
        .await {
            Ok(r) if r.rows_affected() > 0 => linked_trees += 1,
            Ok(_) => {} // already linked
            Err(e) => println!("⚠️  tag_skills_for_tree: skill_trees insert failed for {}: {}", skill_id, e),
        }

        // skill_project_links — skip the synthetic Skills project.
        if !is_skills_project {
            match sqlx::query(
                "INSERT INTO skill_project_links (skill_id, project_id) VALUES ($1, $2) \
                 ON CONFLICT DO NOTHING"
            )
            .bind(&skill_id)
            .bind(project_id)
            .execute(pool)
            .await {
                Ok(r) if r.rows_affected() > 0 => linked_projects += 1,
                Ok(_) => {}
                Err(e) => println!("⚠️  tag_skills_for_tree: skill_project_links insert failed for {}: {}", skill_id, e),
            }
        }
    }

    if linked_trees > 0 || linked_projects > 0 {
        println!(
            "🏷️  Skill tagging: {} skill→tree, {} skill→project links written",
            linked_trees, linked_projects,
        );
    }
}

// ─── Mastered concepts ───────────────────────────────────────────────────────

/// Returns concept slugs the user has already mastered — used by regenerate_tree
/// to tell the LLM what to build on rather than repeat.
///
/// Sources:
///   1. Leaf nodes in `tree_id` where progress = 100
///   2. universal_skills (any tree) where level >= 2 and concept_slug IS NOT NULL
pub(crate) async fn get_mastered_concepts(pool: &sqlx::PgPool, tree_id: &str) -> Vec<String> {
    let mut slugs: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Source 1: fully-completed leaf nodes in the old tree
    if let Ok(rows) = sqlx::query(
        "SELECT concept_slug FROM tree_nodes \
         WHERE tree_id = $1 AND type = 'leaf' AND progress = 100 \
           AND concept_slug IS NOT NULL"
    )
    .bind(tree_id)
    .fetch_all(pool)
    .await
    {
        for r in rows {
            if let Ok(s) = r.try_get::<String, _>("concept_slug") {
                slugs.insert(s);
            }
        }
    }

    // Source 2: universal_skills the user has evidenced across any tree
    if let Ok(rows) = sqlx::query(
        "SELECT concept_slug FROM universal_skills \
         WHERE concept_slug IS NOT NULL AND level >= 2"
    )
    .fetch_all(pool)
    .await
    {
        for r in rows {
            if let Ok(s) = r.try_get::<String, _>("concept_slug") {
                slugs.insert(s);
            }
        }
    }

    slugs.into_iter().collect()
}

// ─── Node embedding backfill ────────────────────────────────────────────────

/// Embed the title of every leaf node that doesn't yet have a cached embedding.
/// Returns the count of nodes embedded. One-time warmup for the HNSW cache.
#[tauri::command]
pub async fn backfill_node_embeddings(
    client: tauri::State<'_, reqwest::Client>,
    database: tauri::State<'_, Database>,
) -> Result<usize, String> {
    let rows = sqlx::query(
        "SELECT id, title FROM tree_nodes \
         WHERE type = 'leaf' AND title_embedding IS NULL"
    )
    .fetch_all(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    if rows.is_empty() {
        return Ok(0);
    }

    let total = rows.len();
    println!("🔖 backfill_node_embeddings: {} nodes to embed", total);
    let mut count = 0usize;
    let mut failed = 0usize;

    for (i, row) in rows.iter().enumerate() {
        // 100 ms between calls to stay under OpenRouter rate limits
        if i > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }

        let node_id: String = match row.try_get("id") { Ok(v) => v, Err(_) => continue };
        let title: String = row.try_get("title").unwrap_or_default();
        if title.is_empty() { continue; }

        match crate::mimir_ingest::get_embedding(&*client, &title).await {
            Ok(embedding) => {
                let vec_str = crate::mimir_ingest::vector_str(&embedding);
                let _ = sqlx::query(
                    "UPDATE tree_nodes SET title_embedding = $1::vector WHERE id = $2"
                )
                .bind(&vec_str)
                .bind(&node_id)
                .execute(&database.pool)
                .await;
                count += 1;
            }
            Err(e) => {
                failed += 1;
                println!("⚠️  backfill_node_embeddings: embed failed for {}: {}", node_id, e);
            }
        }
    }

    println!("✅ backfill_node_embeddings: {}/{} embedded, {} failed", count, total, failed);
    Ok(count)
}

// ─── Mimir auto-matching ─────────────────────────────────────────────────────

/// Semantically link library resources to each leaf node using native Mimir.
/// Best-effort: failures are logged but don't prevent tree generation from succeeding.
pub(crate) async fn auto_match_tree_nodes(pool: &sqlx::PgPool, client: &reqwest::Client, leaf_node_ids: &[String]) {
    if leaf_node_ids.is_empty() {
        return;
    }
    let mut matched = 0usize;

    for node_id in leaf_node_ids {
        match crate::mimir_retrieval::match_node_impl(pool, &client, node_id).await {
            Ok(resources) if !resources.is_empty() => { matched += 1; }
            Ok(_) => {} // no matching resources — fine
            Err(e) => println!("⚠️  Mimir auto-match error for node {}: {}", node_id, e),
        }
    }

    if matched > 0 {
        println!("🔗 Auto-matched {} leaf nodes to Mimir resources", matched);
    }
}

// ─── Diff-based tree regeneration ────────────────────────────────────────────

/// Copy user state (progress, notes, completed checkpoints) from nodes in the
/// old tree to corresponding nodes in the new tree.
///
/// Matching priority:
///   1. Same `concept_slug` (most stable — survives restructuring)
///   2. Case-insensitive exact title match
///   3. pg_trgm trigram similarity on title (threshold 0.4)
///
/// Returns `(nodes_carried, nodes_dropped)` where `nodes_dropped` is the count
/// of old nodes that had user state but could not be matched to the new tree.
pub(crate) async fn diff_and_carry_state(
    pool: &sqlx::PgPool,
    old_tree_id: &str,
    new_tree_id: &str,
) -> Result<(usize, usize, serde_json::Value), String> {
    use sqlx::Row;

    // Load old nodes with their user state
    let old_rows = sqlx::query(
        "SELECT id, title, COALESCE(description, '') AS description, \
                concept_slug, progress, tasks \
         FROM tree_nodes WHERE tree_id = $1"
    )
    .bind(old_tree_id)
    .fetch_all(pool)
    .await
    .map_err(|e| format!("diff: failed to load old nodes: {}", e))?;

    // Load new nodes
    let new_rows = sqlx::query(
        "SELECT id, title, COALESCE(description, '') AS description, concept_slug \
         FROM tree_nodes WHERE tree_id = $1"
    )
    .bind(new_tree_id)
    .fetch_all(pool)
    .await
    .map_err(|e| format!("diff: failed to load new nodes: {}", e))?;

    // Build lookup structures for new nodes
    struct NewNode {
        id: String,
        title_lower: String,
        concept_slug: Option<String>,
    }
    let new_nodes: Vec<NewNode> = new_rows.iter().map(|r| NewNode {
        id: r.try_get("id").unwrap_or_default(),
        title_lower: r.try_get::<String, _>("title").unwrap_or_default().to_lowercase(),
        concept_slug: r.try_get("concept_slug").unwrap_or(None),
    }).collect();

    let mut nodes_carried = 0usize;
    let mut state_losses: Vec<serde_json::Value> = Vec::new();

    for old_row in &old_rows {
        let old_id: String = old_row.try_get("id").unwrap_or_default();
        let old_title: String = old_row.try_get("title").unwrap_or_default();
        let old_slug: Option<String> = old_row.try_get("concept_slug").unwrap_or(None);
        let old_progress: i32 = old_row.try_get("progress").unwrap_or(0);
        let old_tasks: Option<serde_json::Value> = old_row.try_get("tasks").unwrap_or(None);

        // Determine if there's any user state worth carrying
        let has_progress = old_progress > 0;
        let has_notes = old_tasks.as_ref().and_then(|t| {
            t.get("notes").and_then(|n| n.as_str())
        }).map(|s| !s.is_empty()).unwrap_or(false);
        let has_completed = old_tasks.as_ref().and_then(|t| {
            t.get("completed").and_then(|c| c.as_bool())
        }).unwrap_or(false);

        // No state to carry — skip silently
        if !has_progress && !has_notes && !has_completed {
            continue;
        }

        let title_lower = old_title.to_lowercase();

        // Priority 1: concept_slug exact match
        let matched_id = if let Some(ref slug) = old_slug {
            if !slug.is_empty() {
                new_nodes.iter()
                    .find(|n| n.concept_slug.as_deref() == Some(slug.as_str()))
                    .map(|n| n.id.clone())
            } else {
                None
            }
        } else {
            None
        };

        // Priority 2: case-insensitive exact title match
        let matched_id = matched_id.or_else(|| {
            new_nodes.iter()
                .find(|n| n.title_lower == title_lower)
                .map(|n| n.id.clone())
        });

        // Priority 3: pg_trgm trigram similarity (threshold 0.4)
        let matched_id = if matched_id.is_none() {
            let trgm_row = sqlx::query(
                "SELECT id FROM tree_nodes \
                 WHERE tree_id = $1 \
                   AND similarity(LOWER(title), $2) > 0.4 \
                 ORDER BY similarity(LOWER(title), $2) DESC \
                 LIMIT 1"
            )
            .bind(new_tree_id)
            .bind(&title_lower)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();

            trgm_row.and_then(|r| r.try_get::<String, _>("id").ok())
        } else {
            matched_id
        };

        match matched_id {
            Some(new_id) => {
                // Merge tasks: keep new structure but copy notes + completed flags from old
                let merged_tasks = merge_tasks(&old_tasks, pool, &new_id).await;

                let _ = sqlx::query(
                    "UPDATE tree_nodes SET progress = $1, tasks = $2 WHERE id = $3"
                )
                .bind(old_progress)
                .bind(&merged_tasks)
                .bind(&new_id)
                .execute(pool)
                .await;

                nodes_carried += 1;
            }
            None => {
                // User state lost — record it
                state_losses.push(serde_json::json!({
                    "old_node_id": old_id,
                    "title": old_title,
                    "progress": old_progress,
                    "had_notes": has_notes,
                }));
            }
        }
    }

    let nodes_dropped = state_losses.len();
    if nodes_carried > 0 || nodes_dropped > 0 {
        println!(
            "🔄 diff_and_carry_state: carried={} dropped={}",
            nodes_carried, nodes_dropped
        );
    }

    Ok((nodes_carried, nodes_dropped, serde_json::Value::Array(state_losses)))
}

/// Merge old node's `notes` and `completed` into the new node's tasks JSON.
/// New node's structure (mastery_criteria, exercises) is preserved.
async fn merge_tasks(
    old_tasks: &Option<serde_json::Value>,
    pool: &sqlx::PgPool,
    new_node_id: &str,
) -> serde_json::Value {
    use sqlx::Row;

    let new_tasks_row = sqlx::query("SELECT tasks FROM tree_nodes WHERE id = $1")
        .bind(new_node_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();

    let mut new_tasks: serde_json::Value = new_tasks_row
        .and_then(|r| r.try_get::<Option<serde_json::Value>, _>("tasks").ok().flatten())
        .unwrap_or_else(|| serde_json::json!({}));

    if let Some(old) = old_tasks {
        if let Some(notes) = old.get("notes").and_then(|n| n.as_str()) {
            if !notes.is_empty() {
                new_tasks["notes"] = serde_json::json!(notes);
            }
        }
        if let Some(completed) = old.get("completed").and_then(|c| c.as_bool()) {
            if completed {
                new_tasks["completed"] = serde_json::json!(true);
            }
        }
    }

    new_tasks
}
