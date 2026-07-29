// src-tauri/src/read_models.rs
// Consolidated read-model helpers — one Tauri command per common data shape.
// Replaces several round-trip fetches the frontend used to do separately.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::Row;
use std::collections::HashSet;
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

/// How well a checkpoint gap is understood by the knowledge graph.
///
/// - LibraryGap:   KG entry + prerequisite edges exist — library just doesn't cover it yet
/// - KnowledgeGap: no universal_skills row — genuinely unknown territory
/// - PartialKGGap: KG entry exists but no prerequisite edges yet
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "camelCase")]
pub enum GapType {
    LibraryGap,
    KnowledgeGap,
    PartialKGGap,
}

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
    pub has_weak_matches: bool,
    pub concept_slug: Option<String>,
    pub search_terms: Vec<String>,
    pub mastery_criteria: Option<String>,
    pub gap_type: GapType,
    pub prerequisite_concepts: Vec<String>, // human names of prerequisite skills
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeResourceGaps {
    pub tree_id: String,
    pub total_checkpoints: i32,
    pub green_matched_checkpoints: i32,
    pub unmatched_checkpoints: i32,
    pub coverage_percent: f32,
    pub library_gap_count: i32,
    pub knowledge_gap_count: i32,
    pub partial_kg_gap_count: i32,
    pub gaps: Vec<CheckpointGap>,
}

/// Deslugify a concept_slug into a human search term.
/// "linear_algebra_foundations" → "linear algebra foundations"
fn deslugify(slug: &str) -> String {
    slug.replace('_', " ")
}

#[tauri::command]
pub async fn get_tree_resource_gaps(
    tree_id: String,
    database: State<'_, Database>,
) -> Result<TreeResourceGaps, String> {
    // Fetch all leaf nodes with parent chain + green/weak match counts.
    // green = relevance_score < 0.3 (cosine distance); weak = any match but no green.
    let rows = sqlx::query(
        "SELECT
             lf.id                         AS node_id,
             lf.title                      AS title,
             COALESCE(lf.description, '')  AS description,
             COALESCE(lf.progress, 0)      AS progress,
             COALESCE(lf.is_locked, false) AS is_locked,
             lf.order_index                AS leaf_order,
             lf.concept_slug               AS concept_slug,
             lf.tasks                      AS tasks,
             br.title                      AS skill_name,
             br.order_index                AS skill_order,
             ph.title                      AS phase_name,
             ph.order_index                AS phase_order,
             COUNT(mnl.id) FILTER (WHERE mnl.relevance_score < 0.3)  AS green_count,
             COUNT(mnl.id) FILTER (WHERE mnl.relevance_score >= 0.3) AS weak_count
         FROM tree_nodes lf
         JOIN tree_nodes br ON br.id = lf.parent_id AND br.tree_id = lf.tree_id
         JOIN tree_nodes ph ON ph.id = br.parent_id AND ph.tree_id = lf.tree_id
         LEFT JOIN mimir_node_links mnl ON mnl.node_id = lf.id
         WHERE lf.tree_id = $1 AND lf.type = 'leaf'
         GROUP BY lf.id, lf.title, lf.description, lf.progress, lf.is_locked, lf.order_index,
                  lf.concept_slug, lf.tasks,
                  br.title, br.order_index,
                  ph.title, ph.order_index"
    )
    .bind(&tree_id)
    .fetch_all(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    let total_checkpoints = rows.len() as i32;
    let mut green_matched_checkpoints = 0i32;
    let mut gaps: Vec<CheckpointGap> = Vec::new();

    struct RawRow {
        node_id: String,
        title: String,
        description: String,
        progress: i32,
        is_locked: bool,
        leaf_order: i32,
        concept_slug: Option<String>,
        tasks: serde_json::Value,
        skill_name: String,
        skill_order: i32,
        phase_name: String,
        phase_order: i32,
        green_count: i64,
        weak_count: i64,
    }

    let parsed: Vec<RawRow> = rows.iter().map(|r| {
        Ok(RawRow {
            node_id: r.try_get("node_id").map_err(|e: sqlx::Error| e.to_string())?,
            title: r.try_get("title").map_err(|e: sqlx::Error| e.to_string())?,
            description: r.try_get("description").map_err(|e: sqlx::Error| e.to_string())?,
            progress: r.try_get::<i32, _>("progress").map_err(|e: sqlx::Error| e.to_string())?,
            is_locked: r.try_get::<bool, _>("is_locked").map_err(|e: sqlx::Error| e.to_string())?,
            leaf_order: r.try_get::<i32, _>("leaf_order").unwrap_or(0),
            concept_slug: r.try_get("concept_slug").unwrap_or(None),
            tasks: r.try_get("tasks").unwrap_or(serde_json::Value::Null),
            skill_name: r.try_get("skill_name").map_err(|e: sqlx::Error| e.to_string())?,
            skill_order: r.try_get::<i32, _>("skill_order").unwrap_or(0),
            phase_name: r.try_get("phase_name").map_err(|e: sqlx::Error| e.to_string())?,
            phase_order: r.try_get::<i32, _>("phase_order").unwrap_or(0),
            green_count: r.try_get("green_count").map_err(|e: sqlx::Error| e.to_string())?,
            weak_count: r.try_get("weak_count").map_err(|e: sqlx::Error| e.to_string())?,
        })
    }).collect::<Result<Vec<_>, String>>()?;

    // Gather concept_slugs for all gap nodes so we can do two batch queries:
    //   1. Which slugs have a universal_skills row?
    //   2. Which of those have skill_dependencies prerequisite edges?
    let gap_slugs: Vec<String> = parsed.iter()
        .filter(|r| r.green_count == 0)
        .filter_map(|r| r.concept_slug.clone())
        .collect();

    // slug → skill (id, name) — tells us if the concept is in the KG
    struct SkillEntry { id: String, name: String }
    let skill_by_slug: std::collections::HashMap<String, SkillEntry> = if !gap_slugs.is_empty() {
        let skill_rows = sqlx::query(
            "SELECT id, name, concept_slug
             FROM universal_skills
             WHERE concept_slug = ANY($1) AND concept_slug IS NOT NULL"
        )
        .bind(&gap_slugs)
        .fetch_all(&database.pool)
        .await
        .unwrap_or_default();

        skill_rows.iter().filter_map(|r| {
            let slug: Option<String> = r.try_get("concept_slug").ok()?;
            let slug = slug?;
            let id: String = r.try_get("id").ok()?;
            let name: String = r.try_get("name").ok()?;
            Some((slug, SkillEntry { id, name }))
        }).collect()
    } else {
        std::collections::HashMap::new()
    };

    // skill_id → Vec<(prereq_slug, prereq_name)> for skills that have prerequisite edges
    let kg_skill_ids: Vec<String> = skill_by_slug.values().map(|e| e.id.clone()).collect();
    struct PrereqEntry { slug: String, name: String }
    let prereq_map: std::collections::HashMap<String, Vec<PrereqEntry>> = if !kg_skill_ids.is_empty() {
        let prereq_rows = sqlx::query(
            "SELECT sd.source_skill_id,
                    COALESCE(up.concept_slug, '') AS prereq_slug,
                    up.name AS prereq_name
             FROM skill_dependencies sd
             JOIN universal_skills up ON up.id = sd.target_skill_id
             WHERE sd.source_skill_id = ANY($1)
               AND sd.relationship = 'prerequisite'"
        )
        .bind(&kg_skill_ids)
        .fetch_all(&database.pool)
        .await
        .unwrap_or_default();

        let mut map: std::collections::HashMap<String, Vec<PrereqEntry>> = std::collections::HashMap::new();
        for row in &prereq_rows {
            let src_id: String = row.try_get("source_skill_id").unwrap_or_default();
            let prereq_slug: String = row.try_get("prereq_slug").unwrap_or_default();
            let prereq_name: String = row.try_get("prereq_name").unwrap_or_default();
            if !src_id.is_empty() {
                map.entry(src_id).or_default().push(PrereqEntry { slug: prereq_slug, name: prereq_name });
            }
        }
        map
    } else {
        std::collections::HashMap::new()
    };

    for row in &parsed {
        if row.green_count > 0 {
            green_matched_checkpoints += 1;
            continue;
        }

        let has_weak_matches = row.weak_count > 0;

        let mastery_criteria = row.tasks.get("mastery_criteria")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from);

        // Classify gap and build search_terms / prerequisite_concepts
        let (gap_type, search_terms, prerequisite_concepts) = match &row.concept_slug {
            None => {
                // No concept slug at all — treat as KnowledgeGap
                let search_terms = vec![row.title.clone(), row.skill_name.clone()];
                (GapType::KnowledgeGap, search_terms, vec![])
            }
            Some(slug) => {
                match skill_by_slug.get(slug.as_str()) {
                    None => {
                        // Slug present but no KG entry → unknown territory
                        let search_terms = vec![deslugify(slug), row.skill_name.clone()];
                        (GapType::KnowledgeGap, search_terms, vec![])
                    }
                    Some(skill_entry) => {
                        let prereqs = prereq_map.get(&skill_entry.id);
                        let is_empty = prereqs.map_or(true, |p| p.is_empty());
                        if is_empty {
                            // KG entry exists but no prerequisite edges yet
                            let search_terms = vec![deslugify(slug)];
                            (GapType::PartialKGGap, search_terms, vec![])
                        } else {
                            let prereqs = prereqs.unwrap();
                            // Full KG context — search on prerequisites + own slug
                            let mut search_terms: Vec<String> = prereqs.iter()
                                .filter(|p| !p.slug.is_empty())
                                .map(|p| deslugify(&p.slug))
                                .collect();
                            search_terms.push(deslugify(slug));
                            let prerequisite_concepts: Vec<String> = prereqs.iter()
                                .map(|p| p.name.clone())
                                .collect();
                            (GapType::LibraryGap, search_terms, prerequisite_concepts)
                        }
                    }
                }
            }
        };

        gaps.push(CheckpointGap {
            node_id: row.node_id.clone(),
            title: row.title.clone(),
            description: row.description.clone(),
            phase_name: row.phase_name.clone(),
            skill_name: row.skill_name.clone(),
            progress: row.progress,
            is_locked: row.is_locked,
            has_weak_matches,
            concept_slug: row.concept_slug.clone(),
            search_terms,
            mastery_criteria,
            gap_type,
            prerequisite_concepts,
        });
    }

    // Sort: unlocked first, then LibraryGap before PartialKG before Knowledge,
    // then by phase→skill→leaf order
    let gap_type_order = |g: &GapType| match g {
        GapType::LibraryGap   => 0u8,
        GapType::PartialKGGap => 1,
        GapType::KnowledgeGap => 2,
    };
    gaps.sort_by(|a, b| {
        a.is_locked.cmp(&b.is_locked)
            .then_with(|| gap_type_order(&a.gap_type).cmp(&gap_type_order(&b.gap_type)))
            .then_with(|| {
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
        (green_matched_checkpoints as f32 / total_checkpoints as f32) * 100.0
    } else {
        100.0
    };
    let library_gap_count = gaps.iter().filter(|g| g.gap_type == GapType::LibraryGap).count() as i32;
    let knowledge_gap_count = gaps.iter().filter(|g| g.gap_type == GapType::KnowledgeGap).count() as i32;
    let partial_kg_gap_count = gaps.iter().filter(|g| g.gap_type == GapType::PartialKGGap).count() as i32;

    Ok(TreeResourceGaps {
        tree_id,
        total_checkpoints,
        green_matched_checkpoints,
        unmatched_checkpoints,
        coverage_percent,
        library_gap_count,
        knowledge_gap_count,
        partial_kg_gap_count,
        gaps,
    })
}

// ─── NodeNeighborhood ─────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NeighborNode {
    pub node_id: String,
    pub title: String,
    pub progress: i32,
    pub is_locked: bool,
    pub concept_slug: Option<String>,
    pub skill_level: Option<i32>,
    pub resources: Vec<MatchedResource>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeNeighborhood {
    pub node_id: String,
    pub prerequisites: Vec<NeighborNode>,
    pub dependents: Vec<NeighborNode>,
    pub siblings: Vec<NeighborNode>,
    pub prereq_path: Option<PrereqPath>,
}

async fn fetch_neighbor_resources(pool: &sqlx::PgPool, node_id: &str) -> Vec<MatchedResource> {
    let rows = sqlx::query(
        "SELECT mr.id AS resource_id, mr.title, mr.url, mr.type AS resource_type,
                mnl.matched_section_title, mnl.matched_page_start, mnl.matched_page_end,
                mnl.relevance_score
         FROM mimir_node_links mnl
         JOIN mimir_resources mr ON mr.id = mnl.resource_id
         WHERE mnl.node_id = $1
         ORDER BY mnl.relevance_score ASC NULLS LAST
         LIMIT 3"
    )
    .bind(node_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    rows.iter().filter_map(|r| {
        Some(MatchedResource {
            resource_id: r.try_get("resource_id").ok()?,
            title: r.try_get("title").ok()?,
            url: r.try_get("url").ok()?,
            resource_type: r.try_get("resource_type").ok()?,
            matched_section_title: r.try_get("matched_section_title").unwrap_or(None),
            matched_page_start: r.try_get("matched_page_start").unwrap_or(None),
            matched_page_end: r.try_get("matched_page_end").unwrap_or(None),
            relevance_score: r.try_get("relevance_score").unwrap_or(None),
        })
    }).collect()
}

async fn build_neighbor_nodes(
    pool: &sqlx::PgPool,
    rows: Vec<sqlx::postgres::PgRow>,
    skill_level_by_slug: &std::collections::HashMap<String, i32>,
) -> Vec<NeighborNode> {
    let mut nodes = Vec::new();
    for r in &rows {
        let node_id: String = match r.try_get("node_id") { Ok(v) => v, Err(_) => continue };
        let title: String = r.try_get("title").unwrap_or_default();
        let progress: i32 = r.try_get::<Option<i32>, _>("progress").unwrap_or(None).unwrap_or(0);
        let is_locked: bool = r.try_get("is_locked").unwrap_or(false);
        let concept_slug: Option<String> = r.try_get("concept_slug").unwrap_or(None);
        let skill_level = concept_slug.as_deref().and_then(|s| skill_level_by_slug.get(s).copied());
        let resources = fetch_neighbor_resources(pool, &node_id).await;
        nodes.push(NeighborNode { node_id, title, progress, is_locked, concept_slug, skill_level, resources });
    }
    nodes
}

#[tauri::command]
pub async fn get_node_neighborhood(
    node_id: String,
    database: State<'_, Database>,
) -> Result<NodeNeighborhood, String> {
    let pool = &database.pool;

    // Get this node's parent_id and concept_slug
    let node_row = sqlx::query(
        "SELECT parent_id, concept_slug FROM tree_nodes WHERE id = $1"
    )
    .bind(&node_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?
    .ok_or_else(|| format!("Node not found: {}", node_id))?;

    let parent_id: Option<String> = node_row.try_get("parent_id").unwrap_or(None);
    let concept_slug: Option<String> = node_row.try_get("concept_slug").unwrap_or(None);

    // Siblings: other leaf nodes under same parent
    let sibling_rows = if let Some(ref pid) = parent_id {
        sqlx::query(
            "SELECT id AS node_id, title, COALESCE(progress, 0) AS progress,
                    COALESCE(is_locked, false) AS is_locked, concept_slug
             FROM tree_nodes
             WHERE parent_id = $1 AND id != $2 AND type = 'leaf'
             ORDER BY order_index ASC"
        )
        .bind(pid)
        .bind(&node_id)
        .fetch_all(pool)
        .await
        .unwrap_or_default()
    } else {
        vec![]
    };

    // Resolve concept_slug → universal_skills.id for prerequisite/dependent lookup
    let skill_id: Option<String> = if let Some(ref slug) = concept_slug {
        sqlx::query(
            "SELECT id FROM universal_skills WHERE concept_slug = $1 LIMIT 1"
        )
        .bind(slug)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .and_then(|r| r.try_get::<String, _>("id").ok())
    } else {
        None
    };

    // Prerequisite neighbor nodes: skills whose concept_slug is a prerequisite of this node's skill
    // sd.source_skill_id = this_skill, sd.target_skill_id = prereq_skill
    let prereq_rows = if let Some(ref sid) = skill_id {
        sqlx::query(
            "SELECT tn.id AS node_id, tn.title, COALESCE(tn.progress, 0) AS progress,
                    COALESCE(tn.is_locked, false) AS is_locked, tn.concept_slug
             FROM skill_dependencies sd
             JOIN universal_skills us ON us.id = sd.target_skill_id
             JOIN tree_nodes tn ON tn.concept_slug = us.concept_slug
             WHERE sd.source_skill_id = $1
               AND sd.relationship IN ('prerequisite', 'part_of')
               AND tn.type = 'leaf'
             ORDER BY tn.order_index ASC
             LIMIT 8"
        )
        .bind(sid)
        .fetch_all(pool)
        .await
        .unwrap_or_default()
    } else {
        vec![]
    };

    // Dependent neighbor nodes: skills that depend on this node's skill
    // sd.source_skill_id = dependent_skill, sd.target_skill_id = this_skill
    let dependent_rows = if let Some(ref sid) = skill_id {
        sqlx::query(
            "SELECT tn.id AS node_id, tn.title, COALESCE(tn.progress, 0) AS progress,
                    COALESCE(tn.is_locked, false) AS is_locked, tn.concept_slug
             FROM skill_dependencies sd
             JOIN universal_skills us ON us.id = sd.source_skill_id
             JOIN tree_nodes tn ON tn.concept_slug = us.concept_slug
             WHERE sd.target_skill_id = $1
               AND sd.relationship IN ('prerequisite', 'part_of')
               AND tn.type = 'leaf'
             ORDER BY tn.order_index ASC
             LIMIT 8"
        )
        .bind(sid)
        .fetch_all(pool)
        .await
        .unwrap_or_default()
    } else {
        vec![]
    };

    // Gather all concept_slugs from all neighbor sets to batch-fetch skill levels
    let all_slugs: Vec<String> = sibling_rows.iter()
        .chain(prereq_rows.iter())
        .chain(dependent_rows.iter())
        .filter_map(|r| r.try_get::<Option<String>, _>("concept_slug").ok().flatten())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    let skill_level_by_slug: std::collections::HashMap<String, i32> = if !all_slugs.is_empty() {
        sqlx::query(
            "SELECT concept_slug, level FROM universal_skills
             WHERE concept_slug = ANY($1) AND concept_slug IS NOT NULL"
        )
        .bind(&all_slugs)
        .fetch_all(pool)
        .await
        .unwrap_or_default()
        .iter()
        .filter_map(|r| {
            let slug: Option<String> = r.try_get("concept_slug").ok()?;
            let level: Option<i32> = r.try_get("level").ok()?;
            Some((slug?, level?))
        })
        .collect()
    } else {
        std::collections::HashMap::new()
    };

    let siblings = build_neighbor_nodes(pool, sibling_rows, &skill_level_by_slug).await;
    let prerequisites = build_neighbor_nodes(pool, prereq_rows, &skill_level_by_slug).await;
    let dependents = build_neighbor_nodes(pool, dependent_rows, &skill_level_by_slug).await;

    // Compute prereq path for this node's skill (if it has one mapped to universal_skills)
    let prereq_path = if let Some(ref sid) = skill_id {
        let skill_meta_rows = sqlx::query(
            "SELECT id, name, COALESCE(state, 'adjacent') AS state,
                    COALESCE(origin, 'tree_quest') AS origin, COALESCE(level, 0) AS level
             FROM universal_skills"
        )
        .fetch_all(pool)
        .await
        .unwrap_or_default();

        let skill_meta: std::collections::HashMap<String, (String, String, String, i32)> = skill_meta_rows.iter()
            .filter_map(|r| {
                let id: String = r.try_get("id").ok()?;
                let name: String = r.try_get("name").ok()?;
                let state: String = r.try_get("state").ok()?;
                let origin: String = r.try_get("origin").ok()?;
                let level: i32 = r.try_get("level").unwrap_or(0);
                Some((id, (name, state, origin, level)))
            })
            .collect();

        let resource_titles_lower: Vec<String> = sqlx::query("SELECT title FROM mimir_resources")
            .fetch_all(pool)
            .await
            .unwrap_or_default()
            .iter()
            .filter_map(|r| r.try_get::<String, _>("title").ok())
            .map(|t| t.to_lowercase())
            .collect();

        let result = compute_prereq_path(pool, sid, &skill_meta, &resource_titles_lower, 4).await;
        Some(result)
    } else {
        None
    };

    Ok(NodeNeighborhood { node_id, prerequisites, dependents, siblings, prereq_path })
}

// ─── PrereqPath ───────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PathNode {
    pub skill_id: String,
    pub skill_name: String,
    pub state: String,
    pub origin: String,
    pub level: i32,
    pub has_resources: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PrereqPath {
    pub target_skill: String,
    pub target_skill_id: String,
    pub path: Vec<PathNode>,
    pub total_hops: i32,
    pub nearest_seed: Option<String>,
    pub is_reachable: bool,
}

/// BFS backward from `target_skill_id` through `skill_dependencies` looking for
/// a seed node (state = 'seed').  Returns the shortest path seed → … → target,
/// or `is_reachable: false` if no seed is found within `max_depth` hops.
///
/// Edge direction: `source_skill_id → target_skill_id` means "source depends on target"
/// (i.e. target is a prerequisite of source).  Walking *backwards* from the target
/// means following rows WHERE target_skill_id = current, giving us the skills that
/// list `current` as a prerequisite — which is the *forward* learning direction.
/// To find "what do I need to learn before target" we walk WHERE source_skill_id = current
/// and follow target_skill_id.
pub async fn compute_prereq_path(
    pool: &sqlx::PgPool,
    target_skill_id: &str,
    skill_meta: &std::collections::HashMap<String, (String, String, String, i32)>, // id → (name, state, origin, level)
    resource_titles_lower: &[String],
    max_depth: i32,
) -> PrereqPath {
    let target_name = skill_meta.get(target_skill_id)
        .map(|(n, _, _, _)| n.clone())
        .unwrap_or_else(|| target_skill_id.to_string());

    // BFS backward: from target, walk prerequisites (edges WHERE source_skill_id = node)
    // We want the path from seed→target, so we BFS from target backward and then reverse.
    struct BfsState {
        skill_id: String,
        depth: i32,
        came_from: Option<String>, // child in forward direction
    }

    let mut visited: std::collections::HashMap<String, (i32, Option<String>)> = std::collections::HashMap::new();
    visited.insert(target_skill_id.to_string(), (0, None));
    let mut queue: std::collections::VecDeque<BfsState> = std::collections::VecDeque::new();
    queue.push_back(BfsState { skill_id: target_skill_id.to_string(), depth: 0, came_from: None });

    let mut seed_found: Option<String> = None;

    'bfs: while let Some(state) = queue.pop_front() {
        if state.depth >= max_depth { continue; }

        // Check if current node is a seed
        if let Some((_, node_state, _, _)) = skill_meta.get(&state.skill_id) {
            if node_state == "seed" && state.skill_id != target_skill_id {
                seed_found = Some(state.skill_id.clone());
                break 'bfs;
            }
        }

        // Walk prerequisites of this node: rows WHERE source_skill_id = current
        // target_skill_id in that row is a prerequisite of current
        let prereq_rows = sqlx::query(
            "SELECT target_skill_id FROM skill_dependencies
             WHERE source_skill_id = $1
               AND relationship IN ('prerequisite', 'part_of')"
        )
        .bind(&state.skill_id)
        .fetch_all(pool)
        .await
        .unwrap_or_default();

        for r in &prereq_rows {
            let prereq_id: String = r.try_get("target_skill_id").unwrap_or_default();
            if prereq_id.is_empty() || visited.contains_key(&prereq_id) { continue; }
            visited.insert(prereq_id.clone(), (state.depth + 1, Some(state.skill_id.clone())));
            // Check if this is a seed — early exit next iteration
            queue.push_back(BfsState { skill_id: prereq_id, depth: state.depth + 1, came_from: Some(state.skill_id.clone()) });
        }
    }

    let Some(seed_id) = seed_found else {
        return PrereqPath {
            target_skill: target_name,
            target_skill_id: target_skill_id.to_string(),
            path: vec![],
            total_hops: -1,
            nearest_seed: None,
            is_reachable: false,
        };
    };

    // Reconstruct path: seed → ... → target by walking came_from backward from seed
    // came_from[node] = the node we arrived at `node` from (i.e. the child in forward direction)
    // so seed's came_from = Some(next_node), and target's came_from = None
    // Walk from seed following came_from until None
    let mut path_ids: Vec<String> = vec![seed_id.clone()];
    let mut cur = seed_id.clone();
    loop {
        let next = visited.get(&cur).and_then(|(_, parent)| parent.clone());
        match next {
            None => break,
            Some(n) => { path_ids.push(n.clone()); cur = n; }
        }
    }
    // path_ids is now seed → ... → target (forward learning direction)

    let path: Vec<PathNode> = path_ids.iter().map(|id| {
        let (name, state, origin, level) = skill_meta.get(id)
            .map(|(n, s, o, l)| (n.clone(), s.clone(), o.clone(), *l))
            .unwrap_or_else(|| (id.clone(), "adjacent".to_string(), "tree_quest".to_string(), 0));
        let name_lower = name.to_lowercase();
        let has_resources = resource_titles_lower.iter().any(|t| t.contains(&name_lower));
        PathNode { skill_id: id.clone(), skill_name: name, state, origin, level, has_resources }
    }).collect();

    let total_hops = (path.len() as i32).saturating_sub(1);
    let nearest_seed = path.first().map(|p| p.skill_name.clone());
    let seed_name = nearest_seed.clone();

    PrereqPath {
        target_skill: target_name,
        target_skill_id: target_skill_id.to_string(),
        path,
        total_hops,
        nearest_seed: seed_name,
        is_reachable: true,
    }
}

#[tauri::command]
pub async fn get_prereq_path(
    skill_id: String,
    database: State<'_, Database>,
) -> Result<PrereqPath, String> {
    let pool = &database.pool;

    let skill_rows = sqlx::query(
        "SELECT id, name, COALESCE(state, 'adjacent') AS state,
                COALESCE(origin, 'tree_quest') AS origin, COALESCE(level, 0) AS level
         FROM universal_skills"
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let skill_meta: std::collections::HashMap<String, (String, String, String, i32)> = skill_rows.iter()
        .filter_map(|r| {
            let id: String = r.try_get("id").ok()?;
            let name: String = r.try_get("name").ok()?;
            let state: String = r.try_get("state").ok()?;
            let origin: String = r.try_get("origin").ok()?;
            let level: i32 = r.try_get("level").unwrap_or(0);
            Some((id, (name, state, origin, level)))
        })
        .collect();

    let resource_titles_lower: Vec<String> = sqlx::query("SELECT title FROM mimir_resources")
        .fetch_all(pool)
        .await
        .unwrap_or_default()
        .iter()
        .filter_map(|r| r.try_get::<String, _>("title").ok())
        .map(|t| t.to_lowercase())
        .collect();

    Ok(compute_prereq_path(pool, &skill_id, &skill_meta, &resource_titles_lower, 4).await)
}

// ─── GrowthTarget ─────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrowthTarget {
    pub skill_id: String,
    pub skill_name: String,
    pub rationale: String,
    pub job_relevance_score: f32,
    pub prereq_distance: i32,
    pub nearest_seed: String,
    pub prereq_path: Vec<PathNode>,
    pub has_resources: bool,
    pub job_count: i64,
    pub final_score: f32,
    pub is_reachable: bool,
}

#[tauri::command]
pub async fn get_growth_recommendations(
    season: Option<String>,
    database: State<'_, Database>,
) -> Result<Vec<GrowthTarget>, String> {
    get_growth_recommendations_inner(&database.pool, season).await
}

pub(crate) async fn get_growth_recommendations_inner(
    pool: &sqlx::PgPool,
    season: Option<String>,
) -> Result<Vec<GrowthTarget>, String> {

    // 1. Load job skill demand (filtered by season)
    let job_rows = if let Some(ref s) = season {
        sqlx::query(
            "SELECT js.skill_name,
                    COUNT(DISTINCT js.job_id) AS job_count,
                    (SELECT COUNT(*) FROM job_applications WHERE season = $1) AS total_jobs,
                    SUM(CASE WHEN js.is_required THEN 1 ELSE 0 END)::float /
                        NULLIF(COUNT(DISTINCT js.job_id), 0) AS is_required_ratio
             FROM job_skills js
             JOIN job_applications ja ON js.job_id = ja.id
             WHERE ja.season = $1
             GROUP BY js.skill_name"
        )
        .bind(s)
        .fetch_all(pool)
        .await
    } else {
        sqlx::query(
            "SELECT js.skill_name,
                    COUNT(DISTINCT js.job_id) AS job_count,
                    (SELECT COUNT(*) FROM job_applications) AS total_jobs,
                    SUM(CASE WHEN js.is_required THEN 1 ELSE 0 END)::float /
                        NULLIF(COUNT(DISTINCT js.job_id), 0) AS is_required_ratio
             FROM job_skills js
             GROUP BY js.skill_name"
        )
        .fetch_all(pool)
        .await
    }
    .map_err(|e| e.to_string())?;

    if job_rows.is_empty() {
        return Ok(vec![]);
    }

    let total_jobs: i64 = job_rows.first()
        .and_then(|r| r.try_get("total_jobs").ok())
        .unwrap_or(1);
    if total_jobs == 0 {
        return Ok(vec![]);
    }

    // 2. Load all universal_skills with full metadata for compute_prereq_path
    let skill_rows = sqlx::query(
        "SELECT id, name, COALESCE(state, 'adjacent') AS state,
                COALESCE(origin, 'tree_quest') AS origin, COALESCE(level, 0) AS level
         FROM universal_skills"
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    // Build skill_meta map: id → (name, state, origin, level) — used by compute_prereq_path
    let skill_meta: std::collections::HashMap<String, (String, String, String, i32)> = skill_rows.iter()
        .filter_map(|r| {
            let id: String = r.try_get("id").ok()?;
            let name: String = r.try_get("name").ok()?;
            let state: String = r.try_get("state").ok()?;
            let origin: String = r.try_get("origin").ok()?;
            let level: i32 = r.try_get("level").unwrap_or(0);
            Some((id, (name, state, origin, level)))
        })
        .collect();

    // name (lowercase) → (id, state, level) — for job demand matching
    let skill_by_name: std::collections::HashMap<String, (String, String, i32)> = skill_rows.iter()
        .filter_map(|r| {
            let name: String = r.try_get("name").ok()?;
            let id: String = r.try_get("id").ok()?;
            let state: String = r.try_get("state").ok()?;
            let level: i32 = r.try_get("level").unwrap_or(0);
            Some((name.to_lowercase(), (id, state, level)))
        })
        .collect();

    // 3. Check resources: load all resource titles once for batch matching
    let resource_titles: Vec<String> = sqlx::query("SELECT title FROM mimir_resources")
        .fetch_all(pool)
        .await
        .unwrap_or_default()
        .iter()
        .filter_map(|r| r.try_get::<String, _>("title").ok())
        .map(|t| t.to_lowercase())
        .collect();

    // 4. Simple forward BFS from seeds (depth ≤ 3) to quickly check reachability
    //    before calling the full compute_prereq_path for reachable targets only.
    let dep_rows = sqlx::query(
        "SELECT source_skill_id, target_skill_id FROM skill_dependencies"
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut adj: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    for r in &dep_rows {
        let src: String = r.try_get("source_skill_id").unwrap_or_default();
        let tgt: String = r.try_get("target_skill_id").unwrap_or_default();
        if !src.is_empty() && !tgt.is_empty() {
            adj.entry(src).or_default().push(tgt);
        }
    }

    let seeds: Vec<(String, String)> = skill_rows.iter()
        .filter_map(|r| {
            let state: String = r.try_get("state").ok()?;
            if state != "seed" { return None; }
            let id: String = r.try_get("id").ok()?;
            let name: String = r.try_get("name").ok()?;
            Some((id, name))
        })
        .collect();

    // Forward BFS: seed → reachable skill_ids (distance ≤ 3)
    let mut reachability: std::collections::HashMap<String, (i32, String)> = std::collections::HashMap::new(); // id → (dist, seed_name)
    {
        struct FwdEntry { skill_id: String, dist: i32, seed_name: String }
        let mut queue: std::collections::VecDeque<FwdEntry> = std::collections::VecDeque::new();
        for (seed_id, seed_name) in &seeds {
            if !reachability.contains_key(seed_id) {
                reachability.insert(seed_id.clone(), (0, seed_name.clone()));
                queue.push_back(FwdEntry { skill_id: seed_id.clone(), dist: 0, seed_name: seed_name.clone() });
            }
        }
        while let Some(e) = queue.pop_front() {
            if e.dist >= 3 { continue; }
            if let Some(neighbors) = adj.get(&e.skill_id) {
                for nid in neighbors {
                    if reachability.contains_key(nid) { continue; }
                    reachability.insert(nid.clone(), (e.dist + 1, e.seed_name.clone()));
                    queue.push_back(FwdEntry { skill_id: nid.clone(), dist: e.dist + 1, seed_name: e.seed_name.clone() });
                }
            }
        }
    }

    // 5. Build growth targets from job demand
    let mut reachable_targets: Vec<GrowthTarget> = Vec::new();
    let mut disconnected: Vec<GrowthTarget> = Vec::new();

    for row in &job_rows {
        let skill_name: String = match row.try_get("skill_name") { Ok(v) => v, Err(_) => continue };
        let job_count: i64 = row.try_get("job_count").unwrap_or(0);
        let is_required_ratio: f64 = row.try_get::<Option<f64>, _>("is_required_ratio").unwrap_or(None).unwrap_or(0.0);

        // Only surface as a growth target if level <= 1 (gap or absent)
        let level = skill_by_name.get(&skill_name.to_lowercase())
            .map(|(_, _, lvl)| *lvl)
            .unwrap_or(0);
        if level > 1 { continue; }

        let job_frequency = job_count as f32 / total_jobs as f32;
        let name_lower = skill_name.to_lowercase();
        let has_resources = resource_titles.iter().any(|t| t.contains(&name_lower));

        let skill_entry = skill_by_name.get(&name_lower);
        let reach = skill_entry.and_then(|(id, _, _)| reachability.get(id));

        let (is_reachable, prereq_distance, nearest_seed, prereq_path) = match reach {
            Some((dist, seed_name)) if *dist > 0 => {
                // Compute full path for reachable targets
                let sid = skill_entry.map(|(id, _, _)| id.as_str()).unwrap_or("");
                let path_result = compute_prereq_path(pool, sid, &skill_meta, &resource_titles, 4).await;
                let path = path_result.path;
                (true, *dist, seed_name.clone(), path)
            }
            _ => (false, -1i32, String::new(), vec![]),
        };

        let final_score = if is_reachable {
            (job_frequency * 0.4)
                + (is_required_ratio as f32 * 0.2)
                + (1.0 / (prereq_distance + 1) as f32 * 0.3)
                + (if has_resources { 0.2 } else { 0.0 })
        } else {
            job_frequency
        };

        let rationale = if is_reachable {
            format!("Required by {} job{} · {} step{} from {}",
                job_count, if job_count == 1 { "" } else { "s" },
                prereq_distance, if prereq_distance == 1 { "" } else { "s" },
                nearest_seed)
        } else {
            format!("Required by {} job{} · no path from your seeds",
                job_count, if job_count == 1 { "" } else { "s" })
        };

        let skill_id = skill_entry.map(|(id, _, _)| id.clone()).unwrap_or_default();

        let target = GrowthTarget {
            skill_id,
            skill_name,
            rationale,
            job_relevance_score: job_frequency,
            prereq_distance,
            nearest_seed,
            prereq_path,
            has_resources,
            job_count,
            final_score,
            is_reachable,
        };

        if is_reachable {
            reachable_targets.push(target);
        } else {
            disconnected.push(target);
        }
    }

    reachable_targets.sort_by(|a, b| b.final_score.partial_cmp(&a.final_score).unwrap_or(std::cmp::Ordering::Equal));
    disconnected.sort_by(|a, b| b.job_count.cmp(&a.job_count));

    let mut result: Vec<GrowthTarget> = reachable_targets.into_iter().take(3).collect();
    result.extend(disconnected.into_iter().take(2));

    Ok(result)
}

// ─── LearningPath ─────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LearningStep {
    pub step: i32,
    pub skill_id: String,
    pub skill_name: String,
    pub skill_state: String,
    pub weighted_demand: f32,
    pub prereq_path: Vec<PathNode>,
    pub jobs_needing_this: Vec<String>,
    pub rationale: String,
    pub has_resources: bool,
    pub estimated_prereqs_complete: i32,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LearningPath {
    pub steps: Vec<LearningStep>,
    pub total_gap_skills: i32,
    pub seeded_skills_count: i32,
    pub target_jobs_count: i32,
    pub season: Option<String>,
}

fn normalize_skill_name(s: &str) -> String {
    s.trim().to_lowercase()
}

#[tauri::command]
pub async fn compute_learning_path(
    season: Option<String>,
    database: State<'_, Database>,
) -> Result<LearningPath, String> {
    let pool = &database.pool;

    // 1. Load jobs with computed weights
    // weight = sum of available ratings / max_possible; default 0.6 when all null
    struct JobWeight { id: String, company: String, position: String, weight: f32 }
    let job_rows = if let Some(ref s) = season {
        sqlx::query(
            "SELECT id, company, position,
                    COALESCE(rating_location, 0) + COALESCE(rating_alignment, 0) +
                    COALESCE(rating_salary, 0)   + COALESCE(rating_role, 0) AS rating_sum,
                    (CASE WHEN rating_location IS NULL AND rating_alignment IS NULL
                               AND rating_salary IS NULL AND rating_role IS NULL
                          THEN 1 ELSE 0 END) AS all_null
             FROM job_applications
             WHERE season = $1 AND status NOT IN ('rejected', 'withdrawn')"
        )
        .bind(s)
        .fetch_all(pool)
        .await
    } else {
        sqlx::query(
            "SELECT id, company, position,
                    COALESCE(rating_location, 0) + COALESCE(rating_alignment, 0) +
                    COALESCE(rating_salary, 0)   + COALESCE(rating_role, 0) AS rating_sum,
                    (CASE WHEN rating_location IS NULL AND rating_alignment IS NULL
                               AND rating_salary IS NULL AND rating_role IS NULL
                          THEN 1 ELSE 0 END) AS all_null
             FROM job_applications
             WHERE status NOT IN ('rejected', 'withdrawn')"
        )
        .fetch_all(pool)
        .await
    }
    .map_err(|e| e.to_string())?;

    if job_rows.is_empty() {
        return Ok(LearningPath { steps: vec![], total_gap_skills: 0, seeded_skills_count: 0, target_jobs_count: 0, season });
    }

    let jobs: Vec<JobWeight> = job_rows.iter().filter_map(|r| {
        let id: String = r.try_get("id").ok()?;
        let company: String = r.try_get("company").ok()?;
        let position: String = r.try_get("position").ok()?;
        let rating_sum: i32 = r.try_get::<Option<i32>, _>("rating_sum").ok().flatten().unwrap_or(0);
        let all_null: i32 = r.try_get::<Option<i32>, _>("all_null").ok().flatten().unwrap_or(1);
        let weight = if all_null == 1 { 0.6 } else { rating_sum as f32 / 20.0 };
        Some(JobWeight { id, company, position, weight })
    }).collect();

    let target_jobs_count = jobs.len() as i32;
    let job_weight_by_id: std::collections::HashMap<String, f32> = jobs.iter().map(|j| (j.id.clone(), j.weight)).collect();

    // 2. Load job skills and aggregate weighted demand
    let skill_rows = sqlx::query(
        "SELECT js.skill_name, js.job_id, js.is_required FROM job_skills js
         JOIN job_applications ja ON ja.id = js.job_id
         WHERE ja.status NOT IN ('rejected', 'withdrawn')"
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    // skill_name (normalized) → (weighted_demand, job_titles, job_count)
    let mut demand_map: std::collections::HashMap<String, (f32, Vec<String>, i32)> = std::collections::HashMap::new();
    for row in &skill_rows {
        let skill_name: String = match row.try_get("skill_name") { Ok(v) => v, Err(_) => continue };
        let job_id: String = row.try_get("job_id").unwrap_or_default();
        let is_required: bool = row.try_get("is_required").unwrap_or(false);
        let weight = job_weight_by_id.get(&job_id).copied().unwrap_or(0.6);
        let factor = if is_required { 1.2 } else { 0.8 };

        // Find company+position for this job
        let job_title = jobs.iter().find(|j| j.id == job_id)
            .map(|j| format!("{} — {}", j.company, j.position))
            .unwrap_or_default();

        let norm = normalize_skill_name(&skill_name);
        let entry = demand_map.entry(norm).or_insert((0.0, vec![], 0));
        entry.0 += weight * factor;
        if !entry.1.contains(&job_title) { entry.1.push(job_title); }
        entry.2 += 1;
    }

    // 3. Load all universal skills for matching
    let us_rows = sqlx::query(
        "SELECT id, name, COALESCE(state, 'adjacent') AS state,
                COALESCE(origin, 'tree_quest') AS origin, COALESCE(level, 0) AS level
         FROM universal_skills"
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let skill_meta: std::collections::HashMap<String, (String, String, String, i32)> = us_rows.iter()
        .filter_map(|r| {
            let id: String = r.try_get("id").ok()?;
            let name: String = r.try_get("name").ok()?;
            let state: String = r.try_get("state").ok()?;
            let origin: String = r.try_get("origin").ok()?;
            let level: i32 = r.try_get("level").unwrap_or(0);
            Some((id, (name, state, origin, level)))
        })
        .collect();

    // name → id, state, level (case-insensitive lookup)
    let skill_by_norm_name: std::collections::HashMap<String, (String, String, i32)> = us_rows.iter()
        .filter_map(|r| {
            let name: String = r.try_get("name").ok()?;
            let id: String = r.try_get("id").ok()?;
            let state: String = r.try_get("state").ok()?;
            let level: i32 = r.try_get("level").unwrap_or(0);
            Some((normalize_skill_name(&name), (id, state, level)))
        })
        .collect();

    let seeded_skills_count = us_rows.iter()
        .filter(|r| r.try_get::<String, _>("state").unwrap_or_default() == "seed")
        .count() as i32;

    // 4. Filter to gap skills (level <= 1 or not in universal_skills)
    let resource_titles: Vec<String> = sqlx::query("SELECT title FROM mimir_resources")
        .fetch_all(pool)
        .await
        .unwrap_or_default()
        .iter()
        .filter_map(|r| r.try_get::<String, _>("title").ok())
        .map(|t| t.to_lowercase())
        .collect();

    struct GapCandidate {
        skill_id: String,
        skill_name: String,
        skill_state: String,
        weighted_demand: f32,
        job_titles: Vec<String>,
        job_count: i32,
        has_resources: bool,
        prereq_path: PrereqPath,
    }

    let max_demand = demand_map.values().map(|(d, _, _)| *d).fold(0.0f32, f32::max).max(1.0);

    // Load dep graph for topological sort
    let dep_rows = sqlx::query(
        "SELECT source_skill_id, target_skill_id FROM skill_dependencies"
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut adj: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    let mut in_degree: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for r in &dep_rows {
        let src: String = r.try_get("source_skill_id").unwrap_or_default();
        let tgt: String = r.try_get("target_skill_id").unwrap_or_default();
        if !src.is_empty() && !tgt.is_empty() {
            adj.entry(src.clone()).or_default().push(tgt.clone());
            in_degree.entry(tgt).or_insert(0);
            *in_degree.entry(src).or_insert(0) += 0; // ensure src exists
        }
    }

    let mut candidates: Vec<GapCandidate> = Vec::new();

    for (norm_name, (weighted_demand, job_titles, job_count)) in &demand_map {
        let skill_entry = skill_by_norm_name.get(norm_name.as_str());
        let level = skill_entry.map(|(_, _, l)| *l).unwrap_or(0);
        if level > 1 { continue; } // already proficient — skip

        let skill_id = skill_entry.map(|(id, _, _)| id.clone()).unwrap_or_default();
        let skill_name = skill_entry.and_then(|(id, _, _)| skill_meta.get(id)).map(|(n, _, _, _)| n.clone())
            .unwrap_or_else(|| {
                // Fallback: capitalize first letter of each word
                norm_name.split_whitespace().map(|w| {
                    let mut c = w.chars();
                    c.next().map_or(String::new(), |f| f.to_uppercase().collect::<String>() + c.as_str())
                }).collect::<Vec<_>>().join(" ")
            });
        let skill_state = skill_entry.map(|(_, s, _)| s.clone()).unwrap_or_else(|| "gap".to_string());

        let name_lower = skill_name.to_lowercase();
        let has_resources = resource_titles.iter().any(|t| t.contains(&name_lower));

        let prereq_path = if skill_id.is_empty() {
            PrereqPath { target_skill: skill_name.clone(), target_skill_id: String::new(), path: vec![], total_hops: -1, nearest_seed: None, is_reachable: false }
        } else {
            compute_prereq_path(pool, &skill_id, &skill_meta, &resource_titles, 4).await
        };

        candidates.push(GapCandidate {
            skill_id,
            skill_name,
            skill_state,
            weighted_demand: *weighted_demand,
            job_titles: job_titles.clone(),
            job_count: *job_count,
            has_resources,
            prereq_path,
        });
    }

    let total_gap_skills = candidates.len() as i32;

    // 5. Score each gap skill
    struct ScoredCandidate {
        inner: GapCandidate,
        score: f32,
        prereqs_seeded: i32,
    }

    let scored: Vec<ScoredCandidate> = candidates.into_iter().map(|c| {
        let prereqs_seeded = c.prereq_path.path.iter().filter(|n| n.state == "seed").count() as i32;
        let path_cost = (c.prereq_path.path.len() as f32 + 1.0).max(1.0);
        let prereq_ratio = prereqs_seeded as f32 / path_cost;
        let score = (c.weighted_demand / max_demand * 0.5)
            + (1.0 / path_cost * 0.3)
            + (prereq_ratio * 0.1)
            + (if c.has_resources { 0.2 } else { 0.0 });
        ScoredCandidate { inner: c, score, prereqs_seeded }
    }).collect();

    // 6. Topological sort (Kahn's) — within same layer sort by score desc
    // Build DAG from gap skill IDs only
    let gap_ids: std::collections::HashSet<String> = scored.iter()
        .filter(|s| !s.inner.skill_id.is_empty())
        .map(|s| s.inner.skill_id.clone())
        .collect();

    // in_degree within gap candidates only
    let mut gap_indegree: std::collections::HashMap<String, usize> = gap_ids.iter()
        .map(|id| (id.clone(), 0))
        .collect();
    let mut gap_adj: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();

    for r in &dep_rows {
        let src: String = r.try_get("source_skill_id").unwrap_or_default();
        let tgt: String = r.try_get("target_skill_id").unwrap_or_default();
        // Only include edges where BOTH ends are gap candidates
        if gap_ids.contains(&src) && gap_ids.contains(&tgt) {
            gap_adj.entry(src.clone()).or_default().push(tgt.clone());
            *gap_indegree.entry(tgt).or_insert(0) += 1;
        }
    }

    // Kahn's BFS — use a Vec as a priority queue (re-sort each layer to avoid ordered_float dep)
    let score_by_id: std::collections::HashMap<String, f32> = scored.iter()
        .map(|s| (s.inner.skill_id.clone(), s.score))
        .collect();

    let mut ready: Vec<String> = gap_indegree.iter()
        .filter(|(_, &deg)| deg == 0)
        .map(|(id, _)| id.clone())
        .collect();
    // Sort ready queue: highest score first
    ready.sort_by(|a, b| {
        let sa = score_by_id.get(a).copied().unwrap_or(0.0);
        let sb = score_by_id.get(b).copied().unwrap_or(0.0);
        sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut ordered_ids: Vec<String> = Vec::new();
    while !ready.is_empty() {
        // Pop best (first element after sort)
        let id = ready.remove(0);
        ordered_ids.push(id.clone());
        if let Some(nexts) = gap_adj.get(&id) {
            let mut newly_ready: Vec<String> = Vec::new();
            for next in nexts {
                let deg = gap_indegree.entry(next.clone()).or_insert(1);
                *deg = deg.saturating_sub(1);
                if *deg == 0 {
                    newly_ready.push(next.clone());
                }
            }
            // Sort newly ready by score and append to ready
            newly_ready.sort_by(|a, b| {
                let sa = score_by_id.get(a).copied().unwrap_or(0.0);
                let sb = score_by_id.get(b).copied().unwrap_or(0.0);
                sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
            });
            ready.extend(newly_ready);
            // Re-sort to maintain priority within combined queue
            ready.sort_by(|a, b| {
                let sa = score_by_id.get(a).copied().unwrap_or(0.0);
                let sb = score_by_id.get(b).copied().unwrap_or(0.0);
                sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
            });
        }
    }

    // Build two indexes: by_id for known skills, unknown vec for gap skills with no universal_skills row
    let (unknown_vec, known_vec): (Vec<ScoredCandidate>, Vec<ScoredCandidate>) =
        scored.into_iter().partition(|s| s.inner.skill_id.is_empty());

    let mut by_id: std::collections::HashMap<String, ScoredCandidate> = known_vec.into_iter()
        .map(|s| (s.inner.skill_id.clone(), s))
        .collect();

    // Append any cycles (remaining non-zero indegree) sorted by score
    let mut remaining: Vec<String> = gap_indegree.iter()
        .filter(|(_, &deg)| deg > 0)
        .filter(|(id, _)| !id.is_empty())
        .map(|(id, _)| id.clone())
        .collect();
    remaining.sort_by(|a, b| {
        let sa = score_by_id.get(a).copied().unwrap_or(0.0);
        let sb = score_by_id.get(b).copied().unwrap_or(0.0);
        sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
    });
    for id in remaining { ordered_ids.push(id.clone()); }

    // Gap skills with no skill_id (not in universal_skills at all) — append at end sorted by score
    let mut unknown = unknown_vec;
    unknown.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

    // 7. Assemble ordered steps
    let mut steps: Vec<LearningStep> = Vec::new();

    for id in &ordered_ids {
        let Some(s) = by_id.remove(id) else { continue };
        let step_num = steps.len() as i32 + 1;
        let top_jobs: Vec<String> = s.inner.job_titles.iter().take(3).cloned().collect();
        let extra = if s.inner.job_titles.len() > 3 { format!(" +{} more", s.inner.job_titles.len() - 3) } else { String::new() };
        let nearest_seed = s.inner.prereq_path.nearest_seed.clone().unwrap_or_else(|| "baseline".to_string());
        let hops = s.inner.prereq_path.total_hops.max(0);
        let rationale = format!(
            "Required by {} job{} ({}{}) · {} step{} from {} · {}",
            s.inner.job_count,
            if s.inner.job_count == 1 { "" } else { "s" },
            top_jobs.join(", "),
            extra,
            hops,
            if hops == 1 { "" } else { "s" },
            nearest_seed,
            if s.inner.has_resources { "resources available" } else { "add resources" },
        );

        steps.push(LearningStep {
            step: step_num,
            skill_id: s.inner.skill_id,
            skill_name: s.inner.skill_name,
            skill_state: s.inner.skill_state,
            weighted_demand: s.inner.weighted_demand,
            prereq_path: s.inner.prereq_path.path,
            jobs_needing_this: s.inner.job_titles,
            rationale,
            has_resources: s.inner.has_resources,
            estimated_prereqs_complete: s.prereqs_seeded,
        });
    }

    // Append unknown-skill gap candidates
    for s in unknown {
        let step_num = steps.len() as i32 + 1;
        let top_jobs: Vec<String> = s.inner.job_titles.iter().take(3).cloned().collect();
        let extra = if s.inner.job_titles.len() > 3 { format!(" +{} more", s.inner.job_titles.len() - 3) } else { String::new() };
        let rationale = format!(
            "Required by {} job{} ({}{}) · not yet in your skill graph",
            s.inner.job_count,
            if s.inner.job_count == 1 { "" } else { "s" },
            top_jobs.join(", "),
            extra,
        );
        steps.push(LearningStep {
            step: step_num,
            skill_id: s.inner.skill_id,
            skill_name: s.inner.skill_name,
            skill_state: s.inner.skill_state,
            weighted_demand: s.inner.weighted_demand,
            prereq_path: s.inner.prereq_path.path,
            jobs_needing_this: s.inner.job_titles,
            rationale,
            has_resources: s.inner.has_resources,
            estimated_prereqs_complete: s.prereqs_seeded,
        });
    }

    Ok(LearningPath {
        steps,
        total_gap_skills,
        seeded_skills_count,
        target_jobs_count,
        season,
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
                    us.level, us.evidence, us.last_updated, us.review_needed, us.status,
                    us.origin, us.state
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
            origin: r.try_get("origin").unwrap_or_else(|_| "tree_quest".to_string()),
            state: r.try_get("state").unwrap_or_else(|_| "adjacent".to_string()),
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

// ─── ResourceStudyMap ─────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StudyMapNode {
    pub node_id: String,
    pub title: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StudyMapSection {
    pub section_title: String,
    pub page_start: Option<i32>,
    pub page_end: Option<i32>,
    pub node_count: i64,
    pub nodes: Vec<StudyMapNode>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StudyMapTreeBreakdown {
    pub project_id: String,
    pub project_name: String,
    pub node_count: i64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StudyMapEntry {
    pub resource_id: String,
    pub title: String,
    pub resource_type: String,
    pub url: Option<String>,
    pub coverage_count: i64,
    pub avg_relevance: f64,
    pub relevance_tier: String,
    pub tree_breakdown: Vec<StudyMapTreeBreakdown>,
    pub sections: Vec<StudyMapSection>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceStudyMap {
    pub entries: Vec<StudyMapEntry>,
    pub total_resources_with_links: i64,
    pub total_unlocked_nodes: i64,
    pub total_frontier_nodes: i64,
}

#[tauri::command]
pub async fn get_resource_study_map(
    project_ids: Option<Vec<String>>,
    include_frontier: Option<bool>,
    database: State<'_, Database>,
) -> Result<ResourceStudyMap, String> {
    let pool = &database.pool;
    let include_frontier = include_frontier.unwrap_or(false);

    // Count total unlocked leaf nodes: leaves whose parent branch is unlocked
    let total_unlocked: i64 = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)
         FROM tree_nodes leaf
         JOIN tree_nodes branch ON branch.id = leaf.parent_id
         WHERE leaf.type = 'leaf'
           AND branch.type = 'branch'
           AND branch.is_locked = false"
    )
    .fetch_one(pool)
    .await
    .unwrap_or(0);

    // Count frontier leaf nodes: leaves whose parent branch is locked but is the next-to-unlock
    // (the immediately preceding sibling branch under the same trunk has progress = 100).
    // Leaves themselves are never locked — only their parent branch node is.
    let frontier_count: i64 = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)
         FROM tree_nodes leaf
         JOIN tree_nodes branch ON branch.id = leaf.parent_id
         WHERE leaf.type = 'leaf'
           AND branch.type = 'branch'
           AND branch.is_locked = true
           AND EXISTS (
               SELECT 1 FROM tree_nodes prev_branch
               WHERE prev_branch.parent_id = branch.parent_id
                 AND prev_branch.type = 'branch'
                 AND prev_branch.order_index = (
                     SELECT MAX(order_index) FROM tree_nodes
                     WHERE parent_id = branch.parent_id
                       AND type = 'branch'
                       AND order_index < branch.order_index
                 )
                 AND prev_branch.progress = 100
           )"
    )
    .fetch_one(pool)
    .await
    .unwrap_or(0);

    println!("[study_map] include_frontier={}, unlocked_nodes={}, frontier_nodes={}", include_frontier, total_unlocked, frontier_count);

    // Count distinct resources that have at least one node link
    let total_resources_with_links: i64 = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(DISTINCT resource_id) FROM mimir_node_links WHERE relevance_score < 0.55"
    )
    .fetch_one(pool)
    .await
    .unwrap_or(0);

    let filter_projects = project_ids.as_ref().map(|v| !v.is_empty()).unwrap_or(false);

    // eligible_nodes CTE: unlocked leaves UNION frontier leaves (when $1=true).
    // Leaves are never locked — only their parent branch is. Frontier = leaves whose parent
    // branch is locked but the immediately-preceding sibling branch under the same trunk
    // has progress=100 (meaning that skill was just completed and this one is next).
    // The frontier UNION branch is gated on $1::bool = true so it produces zero rows when false.
    const ELIGIBLE_CTE_NO_PROJECT_FILTER: &str = "
        WITH eligible AS (
            SELECT leaf.id
            FROM tree_nodes leaf
            JOIN tree_nodes branch ON branch.id = leaf.parent_id
            WHERE leaf.type = 'leaf' AND branch.type = 'branch' AND branch.is_locked = false
            UNION
            SELECT leaf.id
            FROM tree_nodes leaf
            JOIN tree_nodes branch ON branch.id = leaf.parent_id
            WHERE leaf.type = 'leaf'
              AND branch.type = 'branch'
              AND branch.is_locked = true
              AND $1 = true
              AND EXISTS (
                  SELECT 1 FROM tree_nodes prev_branch
                  WHERE prev_branch.parent_id = branch.parent_id
                    AND prev_branch.type = 'branch'
                    AND prev_branch.order_index = (
                        SELECT MAX(order_index) FROM tree_nodes
                        WHERE parent_id = branch.parent_id
                          AND type = 'branch'
                          AND order_index < branch.order_index
                    )
                    AND prev_branch.progress = 100
              )
        )";
    const ELIGIBLE_CTE_WITH_PROJECT_FILTER: &str = "
        WITH eligible AS (
            SELECT leaf.id
            FROM tree_nodes leaf
            JOIN tree_nodes branch ON branch.id = leaf.parent_id
            JOIN trees t ON t.id = leaf.tree_id
            WHERE leaf.type = 'leaf' AND branch.type = 'branch' AND branch.is_locked = false
              AND t.project_id = ANY($2)
            UNION
            SELECT leaf.id
            FROM tree_nodes leaf
            JOIN tree_nodes branch ON branch.id = leaf.parent_id
            JOIN trees t ON t.id = leaf.tree_id
            WHERE leaf.type = 'leaf'
              AND branch.type = 'branch'
              AND branch.is_locked = true
              AND $1 = true
              AND t.project_id = ANY($2)
              AND EXISTS (
                  SELECT 1 FROM tree_nodes prev_branch
                  WHERE prev_branch.parent_id = branch.parent_id
                    AND prev_branch.type = 'branch'
                    AND prev_branch.order_index = (
                        SELECT MAX(order_index) FROM tree_nodes
                        WHERE parent_id = branch.parent_id
                          AND type = 'branch'
                          AND order_index < branch.order_index
                    )
                    AND prev_branch.progress = 100
              )
        )";

    struct RankedRow {
        resource_id: String,
        coverage_count: i64,
        avg_relevance: f64,
    }

    let ranked_rows: Vec<RankedRow> = if filter_projects {
        let ids = project_ids.as_ref().unwrap();
        let sql = format!(
            "{} SELECT mnl.resource_id,
                    COUNT(DISTINCT mnl.node_id) AS coverage_count,
                    CAST(AVG(mnl.relevance_score) AS FLOAT8) AS avg_relevance
             FROM mimir_node_links mnl
             JOIN eligible e ON e.id = mnl.node_id
             WHERE mnl.relevance_score < 0.55
             GROUP BY mnl.resource_id
             ORDER BY coverage_count DESC, avg_relevance ASC
             LIMIT 20",
            ELIGIBLE_CTE_WITH_PROJECT_FILTER
        );
        let rows = sqlx::query(&sql)
            .bind(include_frontier)
            .bind(ids)
            .fetch_all(pool)
            .await
            .map_err(|e| e.to_string())?;
        rows.iter().filter_map(|r| {
            Some(RankedRow {
                resource_id: r.try_get("resource_id").ok()?,
                coverage_count: r.try_get("coverage_count").ok()?,
                avg_relevance: r.try_get::<f64, _>("avg_relevance").unwrap_or(0.5),
            })
        }).collect()
    } else {
        let sql = format!(
            "{} SELECT mnl.resource_id,
                    COUNT(DISTINCT mnl.node_id) AS coverage_count,
                    CAST(AVG(mnl.relevance_score) AS FLOAT8) AS avg_relevance
             FROM mimir_node_links mnl
             JOIN eligible e ON e.id = mnl.node_id
             WHERE mnl.relevance_score < 0.55
             GROUP BY mnl.resource_id
             ORDER BY coverage_count DESC, avg_relevance ASC
             LIMIT 20",
            ELIGIBLE_CTE_NO_PROJECT_FILTER
        );
        let rows = sqlx::query(&sql)
            .bind(include_frontier)
            .fetch_all(pool)
            .await
            .map_err(|e| e.to_string())?;
        rows.iter().filter_map(|r| {
            Some(RankedRow {
                resource_id: r.try_get("resource_id").ok()?,
                coverage_count: r.try_get("coverage_count").ok()?,
                avg_relevance: r.try_get::<f64, _>("avg_relevance").unwrap_or(0.5),
            })
        }).collect()
    };

    if ranked_rows.is_empty() {
        return Ok(ResourceStudyMap {
            entries: vec![],
            total_resources_with_links,
            total_unlocked_nodes: total_unlocked,
            total_frontier_nodes: frontier_count,
        });
    }

    let top_resource_ids: Vec<String> = ranked_rows.iter().map(|r| r.resource_id.clone()).collect();

    // Fetch resource metadata
    let resource_rows = sqlx::query(
        "SELECT id, title, type AS resource_type, url
         FROM mimir_resources
         WHERE id = ANY($1)"
    )
    .bind(&top_resource_ids)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut resource_meta: std::collections::HashMap<String, (String, String, Option<String>)> = std::collections::HashMap::new();
    for r in &resource_rows {
        let id: String = r.try_get("id").unwrap_or_default();
        let title: String = r.try_get("title").unwrap_or_default();
        let rtype: String = r.try_get("resource_type").unwrap_or_default();
        let url: Option<String> = r.try_get("url").unwrap_or(None);
        resource_meta.insert(id, (title, rtype, url));
    }

    // Fetch supported nodes — reuse eligible CTE so frontier nodes appear when flag is on
    let node_rows = if filter_projects {
        let ids = project_ids.as_ref().unwrap();
        let sql = format!(
            "{} SELECT mnl.resource_id, mnl.node_id, tn.title,
                    mnl.matched_section_title, mnl.matched_page_start, mnl.matched_page_end
             FROM mimir_node_links mnl
             JOIN eligible e ON e.id = mnl.node_id
             JOIN tree_nodes tn ON tn.id = mnl.node_id
             WHERE mnl.resource_id = ANY($3)
               AND mnl.relevance_score < 0.55",
            ELIGIBLE_CTE_WITH_PROJECT_FILTER
        );
        sqlx::query(&sql)
            .bind(include_frontier)
            .bind(ids)
            .bind(&top_resource_ids)
            .fetch_all(pool)
            .await
            .map_err(|e| e.to_string())?
    } else {
        let sql = format!(
            "{} SELECT mnl.resource_id, mnl.node_id, tn.title,
                    mnl.matched_section_title, mnl.matched_page_start, mnl.matched_page_end
             FROM mimir_node_links mnl
             JOIN eligible e ON e.id = mnl.node_id
             JOIN tree_nodes tn ON tn.id = mnl.node_id
             WHERE mnl.resource_id = ANY($2)
               AND mnl.relevance_score < 0.55",
            ELIGIBLE_CTE_NO_PROJECT_FILTER
        );
        sqlx::query(&sql)
            .bind(include_frontier)
            .bind(&top_resource_ids)
            .fetch_all(pool)
            .await
            .map_err(|e| e.to_string())?
    };

    // Group raw node rows by (resource_id, section_key) for section assembly
    struct RawNode {
        node_id: String,
        title: String,
        section_title: Option<String>,
        page_start: Option<i32>,
        page_end: Option<i32>,
    }
    // resource_id → list of raw nodes
    let mut raw_nodes_by_resource: std::collections::HashMap<String, Vec<RawNode>> = std::collections::HashMap::new();
    for r in &node_rows {
        let resource_id: String = r.try_get("resource_id").unwrap_or_default();
        raw_nodes_by_resource.entry(resource_id).or_default().push(RawNode {
            node_id: r.try_get("node_id").unwrap_or_default(),
            title: r.try_get("title").unwrap_or_default(),
            section_title: r.try_get("matched_section_title").unwrap_or(None),
            page_start: r.try_get("matched_page_start").unwrap_or(None),
            page_end: r.try_get("matched_page_end").unwrap_or(None),
        });
    }

    // Build sections_by_resource: group nodes by section_title, sort by page_start
    let mut sections_by_resource: std::collections::HashMap<String, Vec<StudyMapSection>> = std::collections::HashMap::new();
    for (resource_id, raw_nodes) in raw_nodes_by_resource {
        // section_key → (page_start, page_end, nodes)
        let mut section_map: std::collections::HashMap<String, (Option<i32>, Option<i32>, Vec<StudyMapNode>)> = std::collections::HashMap::new();
        for n in raw_nodes {
            let key = n.section_title.clone().unwrap_or_else(|| "Other".to_string());
            let entry = section_map.entry(key).or_insert((n.page_start, n.page_end, vec![]));
            // track min page_start and max page_end across nodes in the section
            if let Some(ps) = n.page_start {
                entry.0 = Some(entry.0.map_or(ps, |existing| existing.min(ps)));
            }
            if let Some(pe) = n.page_end {
                entry.1 = Some(entry.1.map_or(pe, |existing| existing.max(pe)));
            }
            entry.2.push(StudyMapNode { node_id: n.node_id, title: n.title });
        }
        let mut sections: Vec<StudyMapSection> = section_map.into_iter().map(|(title, (page_start, page_end, nodes))| {
            let node_count = nodes.len() as i64;
            StudyMapSection { section_title: title, page_start, page_end, node_count, nodes }
        }).collect();
        // Sort: sections with page_start first (ascending), then "Other" (None) at end
        sections.sort_by(|a, b| match (a.page_start, b.page_start) {
            (Some(x), Some(y)) => x.cmp(&y),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.section_title.cmp(&b.section_title),
        });
        sections_by_resource.insert(resource_id, sections);
    }

    // Fetch project breakdown — group by project (not tree) so multiple tree versions don't produce duplicate pills
    let breakdown_rows = sqlx::query(
        "SELECT mnl.resource_id, p.id AS project_id, p.name AS project_name,
                COUNT(DISTINCT mnl.node_id) AS node_count
         FROM mimir_node_links mnl
         JOIN tree_nodes tn ON tn.id = mnl.node_id
         JOIN trees t ON t.id = tn.tree_id
         JOIN projects p ON p.id = t.project_id
         WHERE mnl.resource_id = ANY($1)
           AND mnl.relevance_score < 0.55
         GROUP BY mnl.resource_id, p.id, p.name"
    )
    .bind(&top_resource_ids)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut breakdown_by_resource: std::collections::HashMap<String, Vec<StudyMapTreeBreakdown>> = std::collections::HashMap::new();
    for r in &breakdown_rows {
        let resource_id: String = r.try_get("resource_id").unwrap_or_default();
        let project_id: String = r.try_get("project_id").unwrap_or_default();
        let project_name: String = r.try_get("project_name").unwrap_or_default();
        let node_count: i64 = r.try_get("node_count").unwrap_or(0);
        breakdown_by_resource.entry(resource_id).or_default().push(StudyMapTreeBreakdown {
            project_id,
            project_name,
            node_count,
        });
    }

    // Assemble entries in ranked order
    let entries: Vec<StudyMapEntry> = ranked_rows.into_iter().filter_map(|rr| {
        let (title, resource_type, url) = resource_meta.remove(&rr.resource_id)?;
        let relevance_tier = if rr.avg_relevance < 0.30 {
            "green".to_string()
        } else if rr.avg_relevance < 0.45 {
            "amber".to_string()
        } else {
            "grey".to_string()
        };
        Some(StudyMapEntry {
            resource_id: rr.resource_id.clone(),
            title,
            resource_type,
            url,
            coverage_count: rr.coverage_count,
            avg_relevance: rr.avg_relevance,
            relevance_tier,
            tree_breakdown: breakdown_by_resource.remove(&rr.resource_id).unwrap_or_default(),
            sections: sections_by_resource.remove(&rr.resource_id).unwrap_or_default(),
        })
    }).collect();

    Ok(ResourceStudyMap {
        entries,
        total_resources_with_links,
        total_unlocked_nodes: total_unlocked,
        total_frontier_nodes: frontier_count,
    })
}

// ─── TailoredProjects ─────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TailoredProject {
    pub project_name: String,
    pub project_description: Option<String>,
    pub yggdrasil_project_id: Option<String>,
    pub matched_skills: Vec<String>,
    pub missing_required_skills: Vec<String>,
    pub match_score: f32,
    pub talking_points: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TailoredProjects {
    pub job_id: String,
    pub company: String,
    pub position: String,
    pub required_skills_count: i32,
    pub top_projects: Vec<TailoredProject>,
}

fn normalize_skill(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[tauri::command]
pub async fn get_tailored_projects(
    job_id: String,
    database: State<'_, Database>,
) -> Result<TailoredProjects, String> {

    // Load job metadata
    let job_row = sqlx::query(
        "SELECT company, position FROM job_applications WHERE id = $1"
    )
    .bind(&job_id)
    .fetch_optional(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    let (company, position) = match job_row {
        None => return Err(format!("Job {} not found", job_id)),
        Some(r) => (
            r.try_get::<String, _>("company").map_err(|e| e.to_string())?,
            r.try_get::<String, _>("position").map_err(|e| e.to_string())?,
        ),
    };

    // Load required skills for this job
    let skills_rows = sqlx::query(
        "SELECT skill_name FROM job_skills WHERE job_id = $1 AND is_required = true"
    )
    .bind(&job_id)
    .fetch_all(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    let required_skills: Vec<String> = skills_rows
        .iter()
        .map(|r| {
            r.try_get::<String, _>("skill_name")
                .map(|s| normalize_skill(&s))
                .map_err(|e| e.to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;

    let required_skills_count = required_skills.len() as i32;

    // Load resume projects (joined with profile to get resume_id)
    let projects_rows = sqlx::query(
        "SELECT rp.name, rp.description, rp.tech_stack, rp.linked_project_id
         FROM resume_projects rp
         JOIN resume_profile rprofile ON rp.resume_id = rprofile.id
         ORDER BY rprofile.created_at DESC, rp.created_at
         LIMIT 100"
    )
    .fetch_all(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    let resume_projects: Vec<(String, Option<String>, Vec<String>, Option<String>)> = projects_rows
        .iter()
        .filter_map(|r| {
            let name: String = r.try_get("name").ok()?;
            let description: Option<String> = r.try_get("description").ok();
            let tech_stack_json: Option<Value> = r.try_get("tech_stack").ok();
            let linked_project_id: Option<String> = r.try_get("linked_project_id").ok();

            let tech_stack: Vec<String> = tech_stack_json
                .and_then(|tj| tj.as_array().map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(|s| normalize_skill(s)))
                        .collect()
                }))
                .unwrap_or_default();

            Some((name, description, tech_stack, linked_project_id))
        })
        .collect();

    // Match projects to job skills
    let mut tailored: Vec<TailoredProject> = resume_projects
        .iter()
        .map(|(name, desc, tech_stack, linked_id)| {
            // Convert to HashSet for O(1) lookups (tech_stack is already lowercased)
            let tech_set: HashSet<&str> = tech_stack.iter().map(|s| s.as_str()).collect();
            let matched_skills: Vec<String> = required_skills
                .iter()
                .filter(|rs| tech_set.contains(rs.as_str()))
                .cloned()
                .collect();
            let missing_required_skills: Vec<String> = required_skills
                .iter()
                .filter(|rs| !tech_set.contains(rs.as_str()))
                .cloned()
                .collect();
            let match_score = if required_skills.is_empty() {
                0.5
            } else {
                matched_skills.len() as f32 / required_skills.len() as f32
            };
            let talking_points: Vec<String> = matched_skills
                .iter()
                .map(|s| format!("Demonstrates {} through {}", s, name))
                .collect();

            TailoredProject {
                project_name: name.clone(),
                project_description: desc.clone(),
                yggdrasil_project_id: linked_id.clone(),
                matched_skills,
                missing_required_skills,
                match_score,
                talking_points,
            }
        })
        .collect();

    // Sort by match_score DESC, take top 3
    tailored.sort_by(|a, b| b.match_score.partial_cmp(&a.match_score).unwrap_or(std::cmp::Ordering::Equal));
    tailored.truncate(3);

    Ok(TailoredProjects {
        job_id,
        company,
        position,
        required_skills_count,
        top_projects: tailored,
    })
}

// ─── GapPath ─────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillStep {
    pub skill_id: String,
    pub skill_name: String,
    pub level: i32,
    pub state: String,
    pub origin: String,
    pub has_tree: bool,
    pub tree_node_id: Option<String>,
    pub tree_project_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GapPath {
    pub gap_skill_id: String,
    pub gap_skill_name: String,
    pub path: Vec<SkillStep>,
    pub estimated_depth: u32,
}

#[tauri::command]
pub async fn get_gap_path(
    skill_id: String,
    database: State<'_, Database>,
) -> Result<GapPath, String> {
    let pool = &database.pool;

    let target_row = sqlx::query(
        "SELECT id, name, COALESCE(level, 0) AS level
         FROM universal_skills
         WHERE id = $1"
    )
    .bind(&skill_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?
    .ok_or_else(|| format!("skill not found: {}", skill_id))?;

    let gap_skill_name: String = target_row.try_get("name").map_err(|e| e.to_string())?;

    let skill_rows = sqlx::query(
        "SELECT id, name, COALESCE(state, 'adjacent') AS state,
                COALESCE(origin, 'tree_quest') AS origin,
                COALESCE(level, 0) AS level,
                concept_slug
         FROM universal_skills"
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    struct SkillMeta {
        name: String,
        state: String,
        origin: String,
        level: i32,
        concept_slug: Option<String>,
    }
    let skill_meta: std::collections::HashMap<String, SkillMeta> = skill_rows.iter()
        .filter_map(|r| {
            let id: String = r.try_get("id").ok()?;
            Some((id, SkillMeta {
                name: r.try_get("name").ok()?,
                state: r.try_get("state").ok()?,
                origin: r.try_get("origin").ok()?,
                level: r.try_get("level").unwrap_or(0),
                concept_slug: r.try_get::<Option<String>, _>("concept_slug").ok().flatten(),
            }))
        })
        .collect();

    let dep_rows = sqlx::query(
        "SELECT source_skill_id, target_skill_id
         FROM skill_dependencies
         WHERE relationship = 'prerequisite'"
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut prereqs_of: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    for r in &dep_rows {
        let source: String = match r.try_get("source_skill_id") { Ok(v) => v, Err(_) => continue };
        let target: String = match r.try_get("target_skill_id") { Ok(v) => v, Err(_) => continue };
        prereqs_of.entry(target).or_default().push(source);
    }

    const MAX_DEPTH: u32 = 8;
    let mut depth_of: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    let mut queue: std::collections::VecDeque<(String, u32)> = std::collections::VecDeque::new();
    queue.push_back((skill_id.clone(), 0));
    depth_of.insert(skill_id.clone(), 0);

    while let Some((cur, d)) = queue.pop_front() {
        if d >= MAX_DEPTH { continue; }
        if cur != skill_id {
            if let Some(meta) = skill_meta.get(&cur) {
                if meta.level >= 2 { continue; }
            }
        }
        if let Some(parents) = prereqs_of.get(&cur) {
            for p in parents {
                if !depth_of.contains_key(p) {
                    depth_of.insert(p.clone(), d + 1);
                    queue.push_back((p.clone(), d + 1));
                }
            }
        }
    }

    let slugs: Vec<String> = depth_of.keys()
        .filter_map(|id| skill_meta.get(id).and_then(|m| m.concept_slug.clone()))
        .collect();
    let mut slug_to_node: std::collections::HashMap<String, (String, String)> = std::collections::HashMap::new();
    if !slugs.is_empty() {
        let node_rows = sqlx::query(
            "SELECT n.id, n.concept_slug, t.project_id
             FROM tree_nodes n
             JOIN trees t ON t.id = n.tree_id
             WHERE n.concept_slug = ANY($1)"
        )
        .bind(&slugs)
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;
        for r in node_rows {
            let id: String = match r.try_get("id") { Ok(v) => v, Err(_) => continue };
            let project_id: String = match r.try_get("project_id") { Ok(v) => v, Err(_) => continue };
            let slug: Option<String> = r.try_get("concept_slug").ok();
            if let Some(s) = slug {
                slug_to_node.entry(s).or_insert((id, project_id));
            }
        }
    }

    let mut entries: Vec<(String, u32)> = depth_of.iter()
        .filter(|(id, _)| **id != skill_id)
        .map(|(id, d)| (id.clone(), *d))
        .collect();
    entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    let mut path: Vec<SkillStep> = entries.into_iter()
        .filter_map(|(id, _)| {
            let meta = skill_meta.get(&id)?;
            let node_info = meta.concept_slug.as_ref().and_then(|s| slug_to_node.get(s).cloned());
            let (tree_node_id, tree_project_id) = match node_info {
                Some((n, p)) => (Some(n), Some(p)),
                None => (None, None),
            };
            Some(SkillStep {
                skill_id: id,
                skill_name: meta.name.clone(),
                level: meta.level,
                state: meta.state.clone(),
                origin: meta.origin.clone(),
                has_tree: tree_node_id.is_some(),
                tree_node_id,
                tree_project_id,
            })
        })
        .collect();

    let target_meta = skill_meta.get(&skill_id);
    let target_state = target_meta.map(|m| m.state.clone()).unwrap_or_else(|| "gap".to_string());
    let target_origin = target_meta.map(|m| m.origin.clone()).unwrap_or_else(|| "job_gap".to_string());
    let target_level = target_meta.map(|m| m.level).unwrap_or(0);
    let target_slug = target_meta.and_then(|m| m.concept_slug.clone());
    let target_node_info = target_slug.as_ref().and_then(|s| slug_to_node.get(s).cloned());
    let (target_node, target_project) = match target_node_info {
        Some((n, p)) => (Some(n), Some(p)),
        None => (None, None),
    };
    path.push(SkillStep {
        skill_id: skill_id.clone(),
        skill_name: gap_skill_name.clone(),
        level: target_level,
        state: target_state,
        origin: target_origin,
        has_tree: target_node.is_some(),
        tree_node_id: target_node,
        tree_project_id: target_project,
    });

    let estimated_depth = depth_of.values().copied().max().unwrap_or(0);

    Ok(GapPath {
        gap_skill_id: skill_id,
        gap_skill_name,
        path,
        estimated_depth,
    })
}

// ─── SkillInlineContext (Learning Path inline expander) ──────────────────────

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InlinePrereq {
    pub skill_id: String,
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InlineUnlock {
    pub skill_id: String,
    pub name: String,
    pub job_demand_count: i32,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InlineRelated {
    pub skill_id: String,
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InlineResource {
    pub title: String,
    pub url: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillInlineContext {
    pub prereqs_done: Vec<InlinePrereq>,
    pub unlocks: Vec<InlineUnlock>,
    pub related: Vec<InlineRelated>,
    pub resources: Vec<InlineResource>,
}

#[tauri::command]
pub async fn get_skill_inline_context(
    skill_id: String,
    database: State<'_, Database>,
) -> Result<SkillInlineContext, String> {
    let pool = &database.pool;

    // Look up this skill's name + domain so we can drive the "related" query.
    let row = sqlx::query(
        "SELECT name, domain_id FROM universal_skills WHERE id = $1"
    )
    .bind(&skill_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;

    let (skill_name, domain_id): (String, Option<String>) = match row {
        None => return Err(format!("skill not found: {}", skill_id)),
        Some(r) => (
            r.try_get("name").map_err(|e| e.to_string())?,
            r.try_get::<Option<String>, _>("domain_id").ok().flatten(),
        ),
    };

    // ── prereqs_done ────────────────────────────────────────────────────────
    // skills A such that A is a prerequisite of skill_id AND A is seed-state
    let prereq_rows = sqlx::query(
        "SELECT u.id, u.name
         FROM skill_dependencies d
         JOIN universal_skills u ON u.id = d.source_skill_id
         WHERE d.target_skill_id = $1
           AND d.relationship = 'prerequisite'
           AND u.state = 'seed'
         ORDER BY u.name ASC"
    )
    .bind(&skill_id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let prereqs_done: Vec<InlinePrereq> = prereq_rows.iter().filter_map(|r| {
        Some(InlinePrereq {
            skill_id: r.try_get("id").ok()?,
            name: r.try_get("name").ok()?,
        })
    }).collect();

    // ── unlocks ─────────────────────────────────────────────────────────────
    // skills B such that skill_id is a prerequisite of B, ranked by job demand
    let unlock_rows = sqlx::query(
        "SELECT u.id, u.name,
                COALESCE((
                    SELECT COUNT(DISTINCT j.job_id)::int
                    FROM job_skills j
                    WHERE LOWER(j.skill_name) = LOWER(u.name)
                ), 0) AS demand
         FROM skill_dependencies d
         JOIN universal_skills u ON u.id = d.target_skill_id
         WHERE d.source_skill_id = $1
         ORDER BY demand DESC, u.name ASC
         LIMIT 5"
    )
    .bind(&skill_id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let unlocks: Vec<InlineUnlock> = unlock_rows.iter().filter_map(|r| {
        Some(InlineUnlock {
            skill_id: r.try_get("id").ok()?,
            name: r.try_get("name").ok()?,
            job_demand_count: r.try_get::<i32, _>("demand").unwrap_or(0),
        })
    }).collect();

    // ── related (same domain, co-occur in jobs) ─────────────────────────────
    // Find jobs requiring this skill, then count other skills appearing in
    // those same jobs. Filter to same domain. Exclude this skill itself.
    let related: Vec<InlineRelated> = if let Some(dom) = domain_id {
        let related_rows = sqlx::query(
            "WITH this_jobs AS (
                 SELECT DISTINCT j.job_id
                 FROM job_skills j
                 WHERE LOWER(j.skill_name) = LOWER($1)
             )
             SELECT u.id, u.name, COUNT(DISTINCT j2.job_id) AS overlap
             FROM job_skills j2
             JOIN this_jobs tj ON tj.job_id = j2.job_id
             JOIN universal_skills u ON LOWER(u.name) = LOWER(j2.skill_name)
             WHERE u.id != $2
               AND u.domain_id = $3
             GROUP BY u.id, u.name
             ORDER BY overlap DESC, u.name ASC
             LIMIT 5"
        )
        .bind(&skill_name)
        .bind(&skill_id)
        .bind(&dom)
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;

        related_rows.iter().filter_map(|r| {
            Some(InlineRelated {
                skill_id: r.try_get("id").ok()?,
                name: r.try_get("name").ok()?,
            })
        }).collect()
    } else {
        Vec::new()
    };

    // ── resources ───────────────────────────────────────────────────────────
    let resource_rows = sqlx::query(
        "SELECT mr.title, mr.url
         FROM mimir_skill_links msl
         JOIN mimir_resources mr ON mr.id = msl.resource_id
         WHERE msl.skill_id = $1
         ORDER BY msl.relevance_score DESC NULLS LAST
         LIMIT 5"
    )
    .bind(&skill_id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let resources: Vec<InlineResource> = resource_rows.iter().filter_map(|r| {
        Some(InlineResource {
            title: r.try_get("title").ok()?,
            url: r.try_get("url").ok()?,
        })
    }).collect();

    Ok(SkillInlineContext { prereqs_done, unlocks, related, resources })
}

// ─── SkillGraphContext (GraphRAG-seeded tree generation) ────────────────────

#[tauri::command]
pub async fn get_skill_graph_context(
    skill_id: String,
    database: State<'_, Database>,
) -> Result<String, String> {
    let pool = &database.pool;

    let row = sqlx::query(
        "SELECT u.name,
                COALESCE(d.name, '') AS domain
         FROM universal_skills u
         LEFT JOIN skill_domains d ON d.id = u.domain_id
         WHERE u.id = $1"
    )
    .bind(&skill_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?
    .ok_or_else(|| format!("skill not found: {}", skill_id))?;

    let skill_name: String = row.try_get("name").map_err(|e| e.to_string())?;
    let domain: String = row.try_get("domain").unwrap_or_default();

    // Job demand for the focal skill
    let demand_row = sqlx::query(
        "SELECT COUNT(DISTINCT job_id)::int AS n
         FROM job_skills
         WHERE LOWER(skill_name) = LOWER($1)"
    )
    .bind(&skill_name)
    .fetch_one(pool)
    .await
    .map_err(|e| e.to_string())?;
    let job_demand_count: i32 = demand_row.try_get("n").unwrap_or(0);

    // Load all prereq edges once, then BFS both directions from skill_id, depth 3, cap 40 nodes
    let edges_rows = sqlx::query(
        "SELECT source_skill_id, target_skill_id
         FROM skill_dependencies
         WHERE relationship = 'prerequisite'"
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut prereqs_of: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    let mut unlocks_of: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    for r in &edges_rows {
        let s: String = match r.try_get("source_skill_id") { Ok(v) => v, Err(_) => continue };
        let t: String = match r.try_get("target_skill_id") { Ok(v) => v, Err(_) => continue };
        // source is prerequisite of target, target unlocks once source learned.
        prereqs_of.entry(t.clone()).or_default().push(s.clone());
        unlocks_of.entry(s).or_default().push(t);
    }

    fn bfs_collect(
        start: &str,
        adj: &std::collections::HashMap<String, Vec<String>>,
        max_depth: u32,
        cap: usize,
    ) -> Vec<(String, u32)> {
        let mut visited: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
        let mut queue: std::collections::VecDeque<(String, u32)> = std::collections::VecDeque::new();
        queue.push_back((start.to_string(), 0));
        visited.insert(start.to_string(), 0);
        while let Some((cur, d)) = queue.pop_front() {
            if visited.len() >= cap { break; }
            if d >= max_depth { continue; }
            if let Some(neighbors) = adj.get(&cur) {
                for n in neighbors {
                    if !visited.contains_key(n) {
                        visited.insert(n.clone(), d + 1);
                        queue.push_back((n.clone(), d + 1));
                    }
                }
            }
        }
        let mut out: Vec<(String, u32)> = visited.into_iter()
            .filter(|(id, _)| id != start)
            .collect();
        out.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
        out
    }

    let cap_per_side = 20;
    let prereq_ids = bfs_collect(&skill_id, &prereqs_of, 3, cap_per_side);
    let unlock_ids = bfs_collect(&skill_id, &unlocks_of, 3, cap_per_side);

    // Resolve all referenced ids to names in one shot.
    let mut all_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (id, _) in prereq_ids.iter().chain(unlock_ids.iter()) {
        all_ids.insert(id.clone());
    }
    let id_list: Vec<String> = all_ids.into_iter().collect();
    let name_rows = sqlx::query(
        "SELECT id, name FROM universal_skills WHERE id = ANY($1)"
    )
    .bind(&id_list)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let id_to_name: std::collections::HashMap<String, String> = name_rows.iter()
        .filter_map(|r| Some((r.try_get("id").ok()?, r.try_get("name").ok()?)))
        .collect();

    // Ordered prereqs root→leaf: deepest first (furthest from skill_id), then shallower
    let mut prereq_chain: Vec<&(String, u32)> = prereq_ids.iter().collect();
    prereq_chain.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let prereq_names: Vec<String> = prereq_chain.iter()
        .filter_map(|(id, _)| id_to_name.get(id).cloned())
        .collect();

    let unlock_names: Vec<String> = unlock_ids.iter()
        .filter_map(|(id, _)| id_to_name.get(id).cloned())
        .collect();

    // Top 10 market co-occurrences: skills sharing the most jobs via job_skills
    let cooc_rows = sqlx::query(
        "WITH this_jobs AS (
             SELECT DISTINCT job_id
             FROM job_skills
             WHERE LOWER(skill_name) = LOWER($1)
         )
         SELECT j.skill_name, COUNT(DISTINCT j.job_id) AS overlap
         FROM job_skills j
         JOIN this_jobs tj ON tj.job_id = j.job_id
         WHERE LOWER(j.skill_name) != LOWER($1)
         GROUP BY j.skill_name
         ORDER BY overlap DESC, j.skill_name ASC
         LIMIT 10"
    )
    .bind(&skill_name)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let cooc_names: Vec<String> = cooc_rows.iter()
        .filter_map(|r| r.try_get::<String, _>("skill_name").ok())
        .collect();

    // Build markdown
    let domain_line = if domain.is_empty() { "(unclassified)".to_string() } else { domain };
    let prereq_block = if prereq_names.is_empty() { "(none in graph)".to_string() } else { prereq_names.join(", ") };
    let unlock_block = if unlock_names.is_empty() { "(none in graph)".to_string() } else { unlock_names.join(", ") };
    let cooc_block = if cooc_names.is_empty() { "(no job data)".to_string() } else { cooc_names.join(", ") };

    let md = format!(
        "## Skill Graph Context: {skill_name}\n\
         Domain: {domain_line} | Required by {job_demand_count} jobs\n\n\
         ### Prerequisites (what to know first)\n\
         {prereq_block}\n\n\
         ### Unlocks (what this enables)\n\
         {unlock_block}\n\n\
         ### Market Co-occurrence (skills hired alongside this)\n\
         {cooc_block}\n"
    );

    Ok(md)
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillTreeRef {
    pub tree_id: String,
    pub tree_title: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeProjectRef {
    pub project_id: String,
}

#[tauri::command]
pub async fn get_project_id_for_tree(
    tree_id: String,
    database: State<'_, Database>,
) -> Result<Option<TreeProjectRef>, String> {
    let row = sqlx::query("SELECT project_id FROM trees WHERE id = $1")
        .bind(&tree_id)
        .fetch_optional(&database.pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(row.and_then(|r| r.try_get::<String, _>("project_id").ok())
        .map(|project_id| TreeProjectRef { project_id }))
}

#[tauri::command]
pub async fn get_tree_for_skill(
    skill_id: String,
    database: State<'_, Database>,
) -> Result<Option<SkillTreeRef>, String> {
    let pool = &database.pool;

    // Stage 1 — skill_trees (post-migration 043 canonical path).
    let direct = sqlx::query(
        "SELECT t.id AS tree_id, t.name AS tree_name
         FROM skill_trees st
         JOIN trees t ON t.id = st.tree_id
         WHERE st.skill_id = $1
           AND t.archived_at IS NULL
         ORDER BY t.created_at DESC
         LIMIT 1"
    )
    .bind(&skill_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;

    if let Some(r) = direct {
        let tree_id: String = r.try_get("tree_id").unwrap_or_default();
        let tree_title: String = r.try_get("tree_name").unwrap_or_default();
        return Ok(Some(SkillTreeRef { tree_id, tree_title }));
    }

    // Stage 2 — fallback via concept_slug for pre-migration trees.
    let fallback = sqlx::query(
        "SELECT t.id AS tree_id, t.name AS tree_name
         FROM universal_skills u
         JOIN tree_nodes n ON n.concept_slug = u.concept_slug
         JOIN trees t ON t.id = n.tree_id
         WHERE u.id = $1
           AND n.concept_slug IS NOT NULL
           AND t.archived_at IS NULL
         ORDER BY t.created_at DESC
         LIMIT 1"
    )
    .bind(&skill_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(fallback.map(|r| {
        let tree_id: String = r.try_get("tree_id").unwrap_or_default();
        let tree_title: String = r.try_get("tree_name").unwrap_or_default();
        SkillTreeRef { tree_id, tree_title }
    }))
}

// ─── SkillDetail (right-side drawer) ─────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetailPrereq {
    pub skill_id: String,
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetailUnlock {
    pub skill_id: String,
    pub name: String,
    pub job_demand_count: i32,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetailRelated {
    pub skill_id: String,
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetailTree {
    pub tree_id: String,
    pub tree_title: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetailProject {
    pub project_id: String,
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetailResource {
    pub resource_id: String,
    pub title: String,
    pub url: String,
    pub relevance_score: Option<f32>,
    pub section_title: Option<String>,
    pub page_start: Option<i32>,
    pub page_end: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillDetail {
    pub skill_id: String,
    pub name: String,
    pub display_name: Option<String>,
    pub domain_name: Option<String>,
    pub origin: String,
    pub state: String,
    pub status: String,
    pub job_demand_count: i32,
    pub prereqs: Vec<DetailPrereq>,
    pub unlocks: Vec<DetailUnlock>,
    pub related: Vec<DetailRelated>,
    pub trees: Vec<DetailTree>,
    pub projects: Vec<DetailProject>,
    pub resources: Vec<DetailResource>,
    pub notes: Option<String>,
}

const SKILLS_PROJECT_ID: &str = "00000000-0000-0000-0000-000000000001";

#[tauri::command]
pub async fn get_skill_detail(
    skill_id: String,
    database: State<'_, Database>,
) -> Result<SkillDetail, String> {
    let pool = &database.pool;

    // Core skill row + domain name + status + notes (left joins so missing
    // skill_profiles row produces defaults rather than failing).
    let core = sqlx::query(
        "SELECT u.name,
                u.display_name,
                u.domain_id,
                COALESCE(u.origin, 'tree_quest') AS origin,
                COALESCE(u.state, 'adjacent') AS state,
                u.concept_slug,
                COALESCE(d.name, '') AS domain_name,
                COALESCE(sp.status, 'untouched') AS status,
                sp.notes AS notes
         FROM universal_skills u
         LEFT JOIN skill_domains d ON d.id = u.domain_id
         LEFT JOIN skill_profiles sp ON sp.skill_id = u.id
         WHERE u.id = $1"
    )
    .bind(&skill_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?
    .ok_or_else(|| format!("skill not found: {}", skill_id))?;

    let name: String = core.try_get("name").map_err(|e| e.to_string())?;
    let display_name: Option<String> = core.try_get("display_name").ok();
    let origin: String = core.try_get("origin").map_err(|e| e.to_string())?;
    let state: String = core.try_get("state").map_err(|e| e.to_string())?;
    let status: String = core.try_get("status").unwrap_or_else(|_| "untouched".to_string());
    let notes: Option<String> = core.try_get("notes").ok();
    let domain_raw: String = core.try_get("domain_name").unwrap_or_default();
    let domain_name: Option<String> = if domain_raw.is_empty() { None } else { Some(domain_raw) };

    // Focal skill's own job demand count (case-insensitive match on job_skills.skill_name)
    let demand_row = sqlx::query(
        "SELECT COUNT(DISTINCT job_id)::int AS n
         FROM job_skills
         WHERE LOWER(skill_name) = LOWER($1)"
    )
    .bind(&name)
    .fetch_one(pool)
    .await
    .map_err(|e| e.to_string())?;
    let job_demand_count: i32 = demand_row.try_get("n").unwrap_or(0);

    // Direct prereqs (depth 1)
    let prereq_rows = sqlx::query(
        "SELECT u.id, u.name
         FROM skill_dependencies d
         JOIN universal_skills u ON u.id = d.source_skill_id
         WHERE d.target_skill_id = $1
           AND d.relationship = 'prerequisite'
         ORDER BY u.name ASC"
    )
    .bind(&skill_id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    let prereqs: Vec<DetailPrereq> = prereq_rows.iter().filter_map(|r| {
        Some(DetailPrereq {
            skill_id: r.try_get("id").ok()?,
            name: r.try_get("name").ok()?,
        })
    }).collect();

    // Direct unlocks (depth 1) with per-row job demand
    let unlock_rows = sqlx::query(
        "SELECT u.id, u.name,
                COALESCE((
                    SELECT COUNT(DISTINCT j.job_id)::int
                    FROM job_skills j
                    WHERE LOWER(j.skill_name) = LOWER(u.name)
                ), 0) AS demand
         FROM skill_dependencies d
         JOIN universal_skills u ON u.id = d.target_skill_id
         WHERE d.source_skill_id = $1
         ORDER BY demand DESC, u.name ASC"
    )
    .bind(&skill_id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    let unlocks: Vec<DetailUnlock> = unlock_rows.iter().filter_map(|r| {
        Some(DetailUnlock {
            skill_id: r.try_get("id").ok()?,
            name: r.try_get("name").ok()?,
            job_demand_count: r.try_get::<i32, _>("demand").unwrap_or(0),
        })
    }).collect();

    // Related — same domain, co-occur in same jobs, top 5
    let domain_id: Option<String> = core.try_get::<Option<String>, _>("domain_id").ok().flatten();
    let related: Vec<DetailRelated> = if let Some(dom) = domain_id {
        let rows = sqlx::query(
            "WITH this_jobs AS (
                 SELECT DISTINCT job_id
                 FROM job_skills
                 WHERE LOWER(skill_name) = LOWER($1)
             )
             SELECT u.id, u.name, COUNT(DISTINCT j2.job_id) AS overlap
             FROM job_skills j2
             JOIN this_jobs tj ON tj.job_id = j2.job_id
             JOIN universal_skills u ON LOWER(u.name) = LOWER(j2.skill_name)
             WHERE u.id != $2
               AND u.domain_id = $3
             GROUP BY u.id, u.name
             ORDER BY overlap DESC, u.name ASC
             LIMIT 5"
        )
        .bind(&name)
        .bind(&skill_id)
        .bind(&dom)
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;
        rows.iter().filter_map(|r| Some(DetailRelated {
            skill_id: r.try_get("id").ok()?,
            name: r.try_get("name").ok()?,
        })).collect()
    } else {
        Vec::new()
    };

    // Trees — from skill_trees, active only
    let tree_rows = sqlx::query(
        "SELECT t.id, t.name
         FROM skill_trees st
         JOIN trees t ON t.id = st.tree_id
         WHERE st.skill_id = $1
           AND t.archived_at IS NULL
         ORDER BY t.created_at DESC"
    )
    .bind(&skill_id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    let trees: Vec<DetailTree> = tree_rows.iter().filter_map(|r| Some(DetailTree {
        tree_id: r.try_get("id").ok()?,
        tree_title: r.try_get("name").ok()?,
    })).collect();

    // Projects — from skill_project_links, excluding the special Skills project
    let project_rows = sqlx::query(
        "SELECT p.id, p.name
         FROM skill_project_links spl
         JOIN projects p ON p.id = spl.project_id
         WHERE spl.skill_id = $1
           AND p.id != $2
         ORDER BY p.name ASC"
    )
    .bind(&skill_id)
    .bind(SKILLS_PROJECT_ID)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    let projects: Vec<DetailProject> = project_rows.iter().filter_map(|r| Some(DetailProject {
        project_id: r.try_get("id").ok()?,
        name: r.try_get("name").ok()?,
    })).collect();

    // Resources — top 8 by relevance_score ASC (lower = better).
    // Multiple rows per resource are possible when the skill matches
    // distinct chapters/sections (mig 045 widened the unique key).
    let resource_rows = sqlx::query(
        "SELECT mr.id, mr.title, mr.url, msl.relevance_score, \
                msl.matched_section_title, msl.matched_page_start, msl.matched_page_end \
         FROM mimir_skill_links msl \
         JOIN mimir_resources mr ON mr.id = msl.resource_id \
         WHERE msl.skill_id = $1 \
         ORDER BY msl.relevance_score ASC NULLS LAST \
         LIMIT 8"
    )
    .bind(&skill_id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    let resources: Vec<DetailResource> = resource_rows.iter().filter_map(|r| Some(DetailResource {
        resource_id: r.try_get("id").ok()?,
        title: r.try_get("title").ok()?,
        url: r.try_get("url").ok()?,
        relevance_score: r.try_get("relevance_score").ok(),
        section_title: r.try_get("matched_section_title").ok().flatten(),
        page_start: r.try_get("matched_page_start").ok().flatten(),
        page_end: r.try_get("matched_page_end").ok().flatten(),
    })).collect();

    Ok(SkillDetail {
        skill_id,
        name,
        display_name,
        domain_name,
        origin,
        state,
        status,
        job_demand_count,
        prereqs,
        unlocks,
        related,
        trees,
        projects,
        resources,
        notes,
    })
}

// ─── ANN skill similarity (migration 044) ───────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NearestSkill {
    pub skill_id: String,
    pub name: String,
    pub distance: f32,
}

/// Cosine-distance ANN over `universal_skills.embedding` (HNSW index, mig 044).
/// `exclude_ids` is filtered server-side via `id != ALL($2)`.
pub async fn find_nearest_skills(
    pool: &sqlx::PgPool,
    embedding: &[f32],
    limit: usize,
    exclude_ids: &[String],
) -> Result<Vec<NearestSkill>, String> {
    let vec_str = crate::mimir_ingest::vector_str(embedding);
    let limit_i64 = limit as i64;

    let rows = sqlx::query(
        "SELECT id, name, (embedding <=> $1::vector)::float4 AS distance
         FROM universal_skills
         WHERE embedding IS NOT NULL
           AND id != ALL($2)
         ORDER BY embedding <=> $1::vector
         LIMIT $3"
    )
    .bind(&vec_str)
    .bind(exclude_ids)
    .bind(limit_i64)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(rows.iter().filter_map(|r| {
        Some(NearestSkill {
            skill_id: r.try_get("id").ok()?,
            name: r.try_get("name").ok()?,
            distance: r.try_get::<f32, _>("distance").unwrap_or(1.0),
        })
    }).collect())
}
