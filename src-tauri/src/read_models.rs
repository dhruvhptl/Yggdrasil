// src-tauri/src/read_models.rs
// Consolidated read-model helpers — one Tauri command per common data shape.
// Replaces several round-trip fetches the frontend used to do separately.

use serde::{Deserialize, Serialize};
use sqlx::Row;
use tauri::State;

use crate::database::Database;
use crate::skill_commands::{UniversalSkill, SkillDependency, SkillGap, SkillAlias};

// ─── TreeSummary ─────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeSummary {
    pub tree_id: String,
    pub tree_name: String,
    pub project_id: String,
    pub total_nodes: i64,
    pub completed_nodes: i64,
    pub overall_progress: f64,
    pub phase_count: i64,
    pub created_at: String,
}

#[tauri::command]
pub async fn get_active_tree_for_project(
    project_id: String,
    database: State<'_, Database>,
) -> Result<Option<TreeSummary>, String> {
    // Pick active_tree_id from projects, fall back to most recent tree
    let tree_row = sqlx::query(
        "SELECT t.id, t.name, t.created_at
         FROM trees t
         WHERE t.project_id = $1
         ORDER BY t.created_at DESC
         LIMIT 1"
    )
    .bind(&project_id)
    .fetch_optional(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    let (tree_id, tree_name, created_at) = match tree_row {
        None => return Ok(None),
        Some(r) => (
            r.try_get::<String, _>("id").map_err(|e| e.to_string())?,
            r.try_get::<String, _>("name").map_err(|e| e.to_string())?,
            r.try_get::<chrono::DateTime<chrono::Utc>, _>("created_at")
                .map(|dt| dt.to_rfc3339())
                .map_err(|e| e.to_string())?,
        ),
    };

    // Aggregate node counts in a single pass
    let stats_row = sqlx::query(
        "SELECT
             COUNT(*) FILTER (WHERE type = 'leaf')                         AS total_nodes,
             COUNT(*) FILTER (WHERE type = 'leaf' AND progress >= 100)     AS completed_nodes,
             COALESCE(AVG(progress) FILTER (WHERE type = 'leaf'), 0)       AS overall_progress,
             COUNT(*) FILTER (WHERE type = 'trunk')                        AS phase_count
         FROM tree_nodes
         WHERE tree_id = $1"
    )
    .bind(&tree_id)
    .fetch_one(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(Some(TreeSummary {
        tree_id,
        tree_name,
        project_id,
        total_nodes: stats_row.try_get("total_nodes").unwrap_or(0),
        completed_nodes: stats_row.try_get("completed_nodes").unwrap_or(0),
        overall_progress: stats_row.try_get::<f64, _>("overall_progress").unwrap_or(0.0),
        phase_count: stats_row.try_get("phase_count").unwrap_or(0),
        created_at,
    }))
}

// ─── NodeChatContext ──────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchedResource {
    pub resource_id: String,
    pub title: String,
    pub url: Option<String>,
    pub resource_type: String,
    pub matched_section_title: Option<String>,
    pub matched_page_start: Option<i32>,
    pub matched_page_end: Option<i32>,
    pub relevance_score: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeChatContext {
    pub node_id: String,
    pub title: String,
    pub description: String,
    pub mastery_criteria: String,
    pub exercises: Vec<String>,
    pub progress: i32,
    pub is_locked: bool,
    pub phase_name: Option<String>,
    pub skill_name: Option<String>,
    pub siblings: Vec<String>,
    pub matched_resources: Vec<MatchedResource>,
}

#[tauri::command]
pub async fn get_node_chat_context(
    node_id: String,
    database: State<'_, Database>,
) -> Result<NodeChatContext, String> {
    // Fetch the node itself
    let node_row = sqlx::query(
        "SELECT id, parent_id, title, description, tasks, progress, is_locked
         FROM tree_nodes WHERE id = $1"
    )
    .bind(&node_id)
    .fetch_optional(&database.pool)
    .await
    .map_err(|e| e.to_string())?
    .ok_or_else(|| format!("Node not found: {}", node_id))?;

    let title: String = node_row.try_get("title").map_err(|e| e.to_string())?;
    let description: String = node_row.try_get::<Option<String>, _>("description")
        .unwrap_or_default()
        .unwrap_or_default();
    let tasks: serde_json::Value = node_row.try_get("tasks").unwrap_or(serde_json::Value::Null);
    let progress: i32 = node_row.try_get::<Option<i32>, _>("progress").unwrap_or(None).unwrap_or(0);
    let is_locked: bool = node_row.try_get("is_locked").unwrap_or(false);
    let skill_parent_id: Option<String> = node_row.try_get("parent_id").unwrap_or(None);

    let mastery_criteria = tasks.get("mastery_criteria")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let exercises: Vec<String> = tasks.get("exercises")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|e| e.as_str().map(String::from)).collect())
        .unwrap_or_default();

    // Walk up: skill node (branch) → phase node (trunk)
    let mut skill_name: Option<String> = None;
    let mut phase_name: Option<String> = None;

    if let Some(ref skill_id) = skill_parent_id {
        let skill_row = sqlx::query(
            "SELECT title, parent_id FROM tree_nodes WHERE id = $1"
        )
        .bind(skill_id)
        .fetch_optional(&database.pool)
        .await
        .map_err(|e| e.to_string())?;

        if let Some(sr) = skill_row {
            skill_name = Some(sr.try_get::<String, _>("title").unwrap_or_default());
            let phase_id: Option<String> = sr.try_get("parent_id").unwrap_or(None);

            if let Some(ref pid) = phase_id {
                let phase_row = sqlx::query("SELECT title FROM tree_nodes WHERE id = $1")
                    .bind(pid)
                    .fetch_optional(&database.pool)
                    .await
                    .map_err(|e| e.to_string())?;
                phase_name = phase_row.and_then(|r| r.try_get::<String, _>("title").ok());
            }
        }
    }

    // Sibling quest nodes under same skill branch
    let siblings: Vec<String> = if let Some(ref skill_id) = skill_parent_id {
        let sib_rows = sqlx::query(
            "SELECT title FROM tree_nodes
             WHERE parent_id = $1 AND id != $2 AND type = 'leaf'
             ORDER BY order_index ASC"
        )
        .bind(skill_id)
        .bind(&node_id)
        .fetch_all(&database.pool)
        .await
        .unwrap_or_default();

        sib_rows.iter()
            .filter_map(|r| r.try_get::<String, _>("title").ok())
            .collect()
    } else {
        vec![]
    };

    // Pre-matched resources from mimir_node_links
    let resource_rows = sqlx::query(
        "SELECT mr.id AS resource_id, mr.title, mr.url, mr.type AS resource_type,
                mnl.matched_section_title, mnl.matched_page_start, mnl.matched_page_end,
                mnl.relevance_score
         FROM mimir_node_links mnl
         JOIN mimir_resources mr ON mr.id = mnl.resource_id
         WHERE mnl.node_id = $1
         ORDER BY mnl.relevance_score ASC NULLS LAST"
    )
    .bind(&node_id)
    .fetch_all(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    let matched_resources: Vec<MatchedResource> = resource_rows.iter().map(|r| {
        Ok(MatchedResource {
            resource_id: r.try_get("resource_id").map_err(|e: sqlx::Error| e.to_string())?,
            title: r.try_get("title").map_err(|e: sqlx::Error| e.to_string())?,
            url: r.try_get("url").map_err(|e: sqlx::Error| e.to_string())?,
            resource_type: r.try_get("resource_type").map_err(|e: sqlx::Error| e.to_string())?,
            matched_section_title: r.try_get("matched_section_title").map_err(|e: sqlx::Error| e.to_string())?,
            matched_page_start: r.try_get("matched_page_start").map_err(|e: sqlx::Error| e.to_string())?,
            matched_page_end: r.try_get("matched_page_end").map_err(|e: sqlx::Error| e.to_string())?,
            relevance_score: r.try_get("relevance_score").map_err(|e: sqlx::Error| e.to_string())?,
        })
    }).collect::<Result<Vec<_>, String>>()?;

    Ok(NodeChatContext {
        node_id,
        title,
        description,
        mastery_criteria,
        exercises,
        progress,
        is_locked,
        phase_name,
        skill_name,
        siblings,
        matched_resources,
    })
}

// ─── ProjectTreeSummary ───────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhaseBreakdown {
    pub phase_name: String,
    pub skill_count: i64,
    pub completed_skills: i64,
    pub checkpoints_total: i64,
    pub checkpoints_completed: i64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectTreeSummary {
    pub project_id: String,
    pub project_name: String,
    pub tree_id: Option<String>,
    pub tree_name: Option<String>,
    pub overall_progress: f64,
    pub phases: Vec<PhaseBreakdown>,
    pub total_matched_resources: i64,
    pub last_activity: Option<String>,
}

#[tauri::command]
pub async fn get_project_tree_summary(
    project_id: String,
    database: State<'_, Database>,
) -> Result<ProjectTreeSummary, String> {
    // Project name
    let project_row = sqlx::query("SELECT name FROM projects WHERE id = $1")
        .bind(&project_id)
        .fetch_optional(&database.pool)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Project not found: {}", project_id))?;
    let project_name: String = project_row.try_get("name").map_err(|e| e.to_string())?;

    // Active/latest tree
    let tree_row = sqlx::query(
        "SELECT id, name FROM trees WHERE project_id = $1 ORDER BY created_at DESC LIMIT 1"
    )
    .bind(&project_id)
    .fetch_optional(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    let (tree_id, tree_name) = match tree_row {
        None => {
            return Ok(ProjectTreeSummary {
                project_id,
                project_name,
                tree_id: None,
                tree_name: None,
                overall_progress: 0.0,
                phases: vec![],
                total_matched_resources: 0,
                last_activity: None,
            });
        }
        Some(r) => (
            r.try_get::<String, _>("id").map_err(|e| e.to_string())?,
            r.try_get::<String, _>("name").map_err(|e| e.to_string())?,
        ),
    };

    // Overall progress across leaf nodes
    let overall_row = sqlx::query(
        "SELECT COALESCE(AVG(progress), 0) AS overall_progress
         FROM tree_nodes WHERE tree_id = $1 AND type = 'leaf'"
    )
    .bind(&tree_id)
    .fetch_one(&database.pool)
    .await
    .map_err(|e| e.to_string())?;
    let overall_progress: f64 = overall_row.try_get("overall_progress").unwrap_or(0.0);

    // Phase breakdown in one query — aggregate per trunk node
    let phase_rows = sqlx::query(
        "SELECT
             ph.title AS phase_name,
             COUNT(DISTINCT br.id)                                           AS skill_count,
             COUNT(DISTINCT br.id) FILTER (
                 WHERE (
                     SELECT COUNT(*) FROM tree_nodes lf
                     WHERE lf.parent_id = br.id AND lf.type = 'leaf'
                 ) > 0
                 AND (
                     SELECT AVG(lf2.progress) FROM tree_nodes lf2
                     WHERE lf2.parent_id = br.id AND lf2.type = 'leaf'
                 ) >= 100
             )                                                               AS completed_skills,
             COUNT(lf.id)                                                    AS checkpoints_total,
             COUNT(lf.id) FILTER (WHERE lf.progress >= 100)                 AS checkpoints_completed
         FROM tree_nodes ph
         LEFT JOIN tree_nodes br ON br.parent_id = ph.id AND br.tree_id = ph.tree_id AND br.type = 'branch'
         LEFT JOIN tree_nodes lf ON lf.parent_id = br.id AND lf.tree_id = ph.tree_id AND lf.type = 'leaf'
         WHERE ph.tree_id = $1 AND ph.type = 'trunk'
         GROUP BY ph.id, ph.title, ph.order_index
         ORDER BY ph.order_index ASC"
    )
    .bind(&tree_id)
    .fetch_all(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    let phases: Vec<PhaseBreakdown> = phase_rows.iter().map(|r| {
        Ok(PhaseBreakdown {
            phase_name: r.try_get("phase_name").map_err(|e: sqlx::Error| e.to_string())?,
            skill_count: r.try_get("skill_count").map_err(|e: sqlx::Error| e.to_string())?,
            completed_skills: r.try_get("completed_skills").map_err(|e: sqlx::Error| e.to_string())?,
            checkpoints_total: r.try_get("checkpoints_total").map_err(|e: sqlx::Error| e.to_string())?,
            checkpoints_completed: r.try_get("checkpoints_completed").map_err(|e: sqlx::Error| e.to_string())?,
        })
    }).collect::<Result<Vec<_>, String>>()?;

    // Mimir-matched resource count across all nodes in the tree
    let res_row = sqlx::query(
        "SELECT COUNT(DISTINCT mnl.resource_id) AS total
         FROM mimir_node_links mnl
         JOIN tree_nodes tn ON tn.id = mnl.node_id
         WHERE tn.tree_id = $1"
    )
    .bind(&tree_id)
    .fetch_one(&database.pool)
    .await
    .map_err(|e| e.to_string())?;
    let total_matched_resources: i64 = res_row.try_get("total").unwrap_or(0);

    // Last activity — most recently updated leaf node
    let act_row = sqlx::query(
        "SELECT MAX(updated_at) AS last_activity
         FROM tree_nodes WHERE tree_id = $1 AND type = 'leaf'"
    )
    .bind(&tree_id)
    .fetch_one(&database.pool)
    .await
    .map_err(|e| e.to_string())?;
    let last_activity: Option<String> = act_row
        .try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("last_activity")
        .ok()
        .flatten()
        .map(|dt| dt.to_rfc3339());

    Ok(ProjectTreeSummary {
        project_id,
        project_name,
        tree_id: Some(tree_id),
        tree_name: Some(tree_name),
        overall_progress,
        phases,
        total_matched_resources,
        last_activity,
    })
}

// ─── TreeResourceGaps ────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckpointGap {
    pub node_id: String,
    pub title: String,
    pub description: String,
    pub phase_name: String,
    pub skill_name: String,
    pub progress: i32,
    pub is_locked: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeResourceGaps {
    pub tree_id: String,
    pub total_checkpoints: i32,
    pub matched_checkpoints: i32,
    pub unmatched_checkpoints: i32,
    pub coverage_percent: f32,
    pub gaps: Vec<CheckpointGap>,
}

#[tauri::command]
pub async fn get_tree_resource_gaps(
    tree_id: String,
    database: State<'_, Database>,
) -> Result<TreeResourceGaps, String> {
    // Fetch all leaf nodes with their parent chain (skill→phase) via two joins.
    // LEFT JOIN mimir_node_links to detect whether any resource is matched.
    let rows = sqlx::query(
        "SELECT
             lf.id          AS node_id,
             lf.title       AS title,
             COALESCE(lf.description, '') AS description,
             COALESCE(lf.progress, 0)     AS progress,
             COALESCE(lf.is_locked, false) AS is_locked,
             lf.order_index               AS leaf_order,
             br.title       AS skill_name,
             br.order_index AS skill_order,
             ph.title       AS phase_name,
             ph.order_index AS phase_order,
             COUNT(mnl.id)  AS link_count
         FROM tree_nodes lf
         JOIN tree_nodes br ON br.id = lf.parent_id AND br.tree_id = lf.tree_id
         JOIN tree_nodes ph ON ph.id = br.parent_id AND ph.tree_id = lf.tree_id
         LEFT JOIN mimir_node_links mnl ON mnl.node_id = lf.id
         WHERE lf.tree_id = $1 AND lf.type = 'leaf'
         GROUP BY lf.id, lf.title, lf.description, lf.progress, lf.is_locked, lf.order_index,
                  br.title, br.order_index,
                  ph.title, ph.order_index"
    )
    .bind(&tree_id)
    .fetch_all(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    let total_checkpoints = rows.len() as i32;
    let mut matched_checkpoints = 0i32;
    let mut gaps: Vec<CheckpointGap> = Vec::new();

    struct RawRow {
        node_id: String,
        title: String,
        description: String,
        progress: i32,
        is_locked: bool,
        leaf_order: i32,
        skill_name: String,
        skill_order: i32,
        phase_name: String,
        phase_order: i32,
        link_count: i64,
    }

    let parsed: Vec<RawRow> = rows.iter().map(|r| {
        Ok(RawRow {
            node_id: r.try_get("node_id").map_err(|e: sqlx::Error| e.to_string())?,
            title: r.try_get("title").map_err(|e: sqlx::Error| e.to_string())?,
            description: r.try_get("description").map_err(|e: sqlx::Error| e.to_string())?,
            progress: r.try_get::<i32, _>("progress").map_err(|e: sqlx::Error| e.to_string())?,
            is_locked: r.try_get::<bool, _>("is_locked").map_err(|e: sqlx::Error| e.to_string())?,
            leaf_order: r.try_get::<i32, _>("leaf_order").unwrap_or(0),
            skill_name: r.try_get("skill_name").map_err(|e: sqlx::Error| e.to_string())?,
            skill_order: r.try_get::<i32, _>("skill_order").unwrap_or(0),
            phase_name: r.try_get("phase_name").map_err(|e: sqlx::Error| e.to_string())?,
            phase_order: r.try_get::<i32, _>("phase_order").unwrap_or(0),
            link_count: r.try_get("link_count").map_err(|e: sqlx::Error| e.to_string())?,
        })
    }).collect::<Result<Vec<_>, String>>()?;

    for row in &parsed {
        if row.link_count > 0 {
            matched_checkpoints += 1;
        } else {
            gaps.push(CheckpointGap {
                node_id: row.node_id.clone(),
                title: row.title.clone(),
                description: row.description.clone(),
                phase_name: row.phase_name.clone(),
                skill_name: row.skill_name.clone(),
                progress: row.progress,
                is_locked: row.is_locked,
            });
        }
    }

    // Sort: unlocked first (can act on them now), then by phase→skill→leaf order
    gaps.sort_by(|a, b| {
        a.is_locked.cmp(&b.is_locked)
            .then_with(|| {
                // Recover phase/skill order from parsed rows for stable sort
                let pa = parsed.iter().find(|r| r.node_id == a.node_id);
                let pb = parsed.iter().find(|r| r.node_id == b.node_id);
                match (pa, pb) {
                    (Some(ra), Some(rb)) => ra.phase_order.cmp(&rb.phase_order)
                        .then(ra.skill_order.cmp(&rb.skill_order))
                        .then(ra.leaf_order.cmp(&rb.leaf_order)),
                    _ => std::cmp::Ordering::Equal,
                }
            })
    });

    let unmatched_checkpoints = gaps.len() as i32;
    let coverage_percent = if total_checkpoints > 0 {
        (matched_checkpoints as f32 / total_checkpoints as f32) * 100.0
    } else {
        100.0
    };

    Ok(TreeResourceGaps {
        tree_id,
        total_checkpoints,
        matched_checkpoints,
        unmatched_checkpoints,
        coverage_percent,
        gaps,
    })
}

// ─── SkillGraphSnapshot ───────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillGraphSnapshot {
    pub skills: Vec<UniversalSkill>,
    pub dependencies: Vec<SkillDependency>,
    pub aliases: Vec<SkillAlias>,
    pub gaps: Vec<SkillGap>,
    pub gap_count: i64,
    pub review_count: i64,
}

#[tauri::command]
pub async fn get_skill_graph_snapshot(
    database: State<'_, Database>,
) -> Result<SkillGraphSnapshot, String> {
    // All four queries run concurrently
    let (skills_rows, deps_rows, aliases_rows, gaps_row, counts_row) = tokio::try_join!(
        sqlx::query(
            "SELECT us.id, us.name,
                    COALESCE(sd.name, us.domain) AS domain,
                    us.level, us.evidence, us.last_updated, us.review_needed, us.status
             FROM universal_skills us
             LEFT JOIN skill_domains sd ON sd.id = us.domain_id
             ORDER BY us.review_needed DESC, us.level DESC, us.name ASC"
        ).fetch_all(&database.pool),
        sqlx::query(
            "SELECT id, source_skill_id, target_skill_id, relationship FROM skill_dependencies"
        ).fetch_all(&database.pool),
        sqlx::query(
            "SELECT sa.id, sa.canonical_skill_id, us.name AS canonical_skill_name, sa.alias
             FROM skill_aliases sa
             JOIN universal_skills us ON us.id = sa.canonical_skill_id
             ORDER BY us.name ASC, sa.alias ASC"
        ).fetch_all(&database.pool),
        sqlx::query(
            "SELECT js.skill_name,
                    COUNT(DISTINCT js.job_id) AS count,
                    (SELECT COUNT(*) FROM job_applications) AS total,
                    MAX(us.level) AS current_level
             FROM job_skills js
             LEFT JOIN universal_skills us ON LOWER(us.name) = LOWER(js.skill_name)
             GROUP BY js.skill_name"
        ).fetch_all(&database.pool),
        sqlx::query(
            "SELECT
                 COUNT(*) FILTER (WHERE review_needed = true) AS review_count,
                 (SELECT COUNT(DISTINCT js.skill_name)
                  FROM job_skills js
                  LEFT JOIN universal_skills us ON LOWER(us.name) = LOWER(js.skill_name)
                  WHERE us.level IS NULL OR us.level <= 1)   AS gap_count
             FROM universal_skills"
        ).fetch_one(&database.pool),
    ).map_err(|e| e.to_string())?;

    let skills: Vec<UniversalSkill> = skills_rows.iter().map(|r| {
        Ok(UniversalSkill {
            id: r.try_get("id").map_err(|e: sqlx::Error| e.to_string())?,
            name: r.try_get("name").map_err(|e: sqlx::Error| e.to_string())?,
            domain: r.try_get("domain").map_err(|e: sqlx::Error| e.to_string())?,
            level: r.try_get("level").map_err(|e: sqlx::Error| e.to_string())?,
            evidence: r.try_get("evidence").map_err(|e: sqlx::Error| e.to_string())?,
            last_updated: r.try_get::<chrono::DateTime<chrono::Utc>, _>("last_updated")
                .map(|dt| dt.to_rfc3339())
                .map_err(|e: sqlx::Error| e.to_string())?,
            review_needed: r.try_get("review_needed").unwrap_or(false),
            status: r.try_get("status").unwrap_or_else(|_| "active".to_string()),
        })
    }).collect::<Result<Vec<_>, String>>()?;

    let dependencies: Vec<SkillDependency> = deps_rows.iter().map(|r| {
        Ok(SkillDependency {
            id: r.try_get("id").map_err(|e: sqlx::Error| e.to_string())?,
            source_skill_id: r.try_get("source_skill_id").map_err(|e: sqlx::Error| e.to_string())?,
            target_skill_id: r.try_get("target_skill_id").map_err(|e: sqlx::Error| e.to_string())?,
            relationship: r.try_get("relationship").map_err(|e: sqlx::Error| e.to_string())?,
        })
    }).collect::<Result<Vec<_>, String>>()?;

    let aliases: Vec<SkillAlias> = aliases_rows.iter().map(|r| {
        Ok(SkillAlias {
            id: r.try_get("id").map_err(|e: sqlx::Error| e.to_string())?,
            canonical_skill_id: r.try_get("canonical_skill_id").map_err(|e: sqlx::Error| e.to_string())?,
            canonical_skill_name: r.try_get("canonical_skill_name").map_err(|e: sqlx::Error| e.to_string())?,
            alias: r.try_get("alias").map_err(|e: sqlx::Error| e.to_string())?,
        })
    }).collect::<Result<Vec<_>, String>>()?;

    // Build gaps list (same logic as get_skill_gaps)
    let mut gaps: Vec<SkillGap> = Vec::new();
    for row in &gaps_row {
        let skill_name: String = row.try_get("skill_name").map_err(|e: sqlx::Error| e.to_string())?;
        let count: i64 = row.try_get("count").map_err(|e: sqlx::Error| e.to_string())?;
        let total: i64 = row.try_get("total").map_err(|e: sqlx::Error| e.to_string())?;
        if total == 0 { continue; }
        let frequency = count as f64 / total as f64;
        let demand_score = frequency * 100.0;
        let current_level: i32 = row.try_get::<Option<i32>, _>("current_level")
            .unwrap_or(None)
            .unwrap_or(0);
        if current_level <= 1 {
            gaps.push(SkillGap {
                skill_name,
                demand_count: count,
                frequency,
                demand_score,
                current_level,
            });
        }
    }
    gaps.sort_by(|a, b| b.demand_score.partial_cmp(&a.demand_score).unwrap_or(std::cmp::Ordering::Equal));

    let gap_count: i64 = counts_row.try_get("gap_count").unwrap_or(0);
    let review_count: i64 = counts_row.try_get("review_count").unwrap_or(0);

    Ok(SkillGraphSnapshot {
        skills,
        dependencies,
        aliases,
        gaps,
        gap_count,
        review_count,
    })
}
