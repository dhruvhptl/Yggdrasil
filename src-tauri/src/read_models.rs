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
    let pool = &database.pool;

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
                + (if has_resources { 0.1 } else { 0.0 })
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
