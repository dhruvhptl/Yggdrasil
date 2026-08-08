// src-tauri/src/export_commands.rs

use tauri::State;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use crate::database::Database;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResource {
    pub title: String,
    pub url: Option<String>,
    pub resource_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportQuest {
    pub title: String,
    pub mastery_criteria: String,
    pub exercises: Vec<String>,
    pub resources: Vec<ExportResource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportSkill {
    pub name: String,
    pub quests: Vec<ExportQuest>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportPhase {
    pub name: String,
    pub skills: Vec<ExportSkill>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportProject {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportTree {
    pub project: ExportProject,
    pub phases: Vec<ExportPhase>,
}

#[tauri::command]
pub async fn export_tree(
    project_id: String,
    database: State<'_, Database>,
) -> Result<ExportTree, String> {
    // 1. Fetch project
    let proj_row = sqlx::query(
        "SELECT name, description FROM projects WHERE id = $1"
    )
    .bind(&project_id)
    .fetch_one(&database.pool)
    .await
    .map_err(|e| format!("Project not found: {}", e))?;

    let project = ExportProject {
        name: proj_row.try_get("name").map_err(|e| e.to_string())?,
        description: proj_row.try_get("description").map_err(|e| e.to_string())?,
    };

    // 2. Fetch tree for this project
    let tree_row = sqlx::query(
        "SELECT id FROM trees WHERE project_id = $1 ORDER BY created_at DESC LIMIT 1"
    )
    .bind(&project_id)
    .fetch_optional(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    let tree_id = match tree_row {
        Some(row) => row.try_get::<String, _>("id").map_err(|e| e.to_string())?,
        None => return Ok(ExportTree { project, phases: vec![] }),
    };

    // 3. Fetch all nodes (include tasks for mastery_criteria/exercises)
    let nodes = sqlx::query(
        "SELECT id, parent_id, type, title, description, tasks, order_index \
         FROM tree_nodes WHERE tree_id = $1 ORDER BY order_index ASC"
    )
    .bind(&tree_id)
    .fetch_all(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    struct NodeData {
        id: String,
        parent_id: Option<String>,
        node_type: String,
        title: String,
        description: String,
        tasks: serde_json::Value,
        order_index: i32,
    }

    let mut all_nodes: Vec<NodeData> = Vec::new();
    for n in &nodes {
        all_nodes.push(NodeData {
            id: n.try_get("id").map_err(|e| e.to_string())?,
            parent_id: n.try_get("parent_id").map_err(|e| e.to_string())?,
            node_type: n.try_get("type").map_err(|e| e.to_string())?,
            title: n.try_get("title").map_err(|e| e.to_string())?,
            description: n.try_get::<Option<String>, _>("description").map_err(|e| e.to_string())?.unwrap_or_default(),
            tasks: n.try_get("tasks").map_err(|e| e.to_string())?,
            order_index: n.try_get::<Option<i32>, _>("order_index").map_err(|e| e.to_string())?.unwrap_or(0),
        });
    }

    // 4. Fetch all linked resources (mimir_node_links JOIN mimir_resources)
    //    for all leaf nodes in this tree at once
    let leaf_ids: Vec<String> = all_nodes.iter()
        .filter(|n| n.node_type == "leaf")
        .map(|n| n.id.clone())
        .collect();

    let mut node_resources: std::collections::HashMap<String, Vec<ExportResource>> =
        std::collections::HashMap::new();

    if !leaf_ids.is_empty() {
        // Build a parameterized IN clause: $1, $2, $3, ...
        let params: Vec<String> = (1..=leaf_ids.len()).map(|i| format!("${}", i)).collect();
        let in_clause = params.join(", ");
        let query_str = format!(
            "SELECT mnl.node_id, mr.title, mr.url, mr.type \
             FROM mimir_node_links mnl \
             JOIN mimir_resources mr ON mr.id = mnl.resource_id \
             WHERE mnl.node_id IN ({}) \
             ORDER BY mnl.relevance_score ASC NULLS LAST",
            in_clause
        );

        let mut query = sqlx::query(&query_str);
        for lid in &leaf_ids {
            query = query.bind(lid);
        }

        let resource_rows = query
            .fetch_all(&database.pool)
            .await
            .map_err(|e| e.to_string())?;

        for row in &resource_rows {
            let node_id: String = row.try_get("node_id").map_err(|e| e.to_string())?;
            let resource = ExportResource {
                title: row.try_get("title").map_err(|e| e.to_string())?,
                url: row.try_get("url").map_err(|e| e.to_string())?,
                resource_type: row.try_get("type").map_err(|e| e.to_string())?,
            };
            node_resources.entry(node_id).or_default().push(resource);
        }
    }

    // DB structure (from brain.rs):
    //   Phase nodes: type="trunk", parent_id=NULL
    //   Skill nodes: type="branch", parent_id=<phase_id>
    //   Quest nodes: type="leaf", parent_id=<skill_id>

    let mut phase_nodes: Vec<&NodeData> = all_nodes.iter()
        .filter(|n| n.node_type == "trunk")
        .collect();
    phase_nodes.sort_by_key(|n| n.order_index);

    let mut phases = Vec::new();
    for phase_node in &phase_nodes {
        let mut skill_nodes: Vec<&NodeData> = all_nodes.iter()
            .filter(|n| n.node_type == "branch" && n.parent_id.as_deref() == Some(&phase_node.id))
            .collect();
        skill_nodes.sort_by_key(|n| n.order_index);

        let mut skills = Vec::new();
        for skill_node in &skill_nodes {
            let mut quest_nodes: Vec<&NodeData> = all_nodes.iter()
                .filter(|n| n.node_type == "leaf" && n.parent_id.as_deref() == Some(&skill_node.id))
                .collect();
            quest_nodes.sort_by_key(|n| n.order_index);

            let quests: Vec<ExportQuest> = quest_nodes.iter().map(|q| {
                // Get linked resources from mimir_node_links
                let resources = node_resources
                    .get(&q.id)
                    .cloned()
                    .unwrap_or_default();

                // Extract mastery_criteria and exercises from tasks JSONB (checkpoint model)
                let mastery_criteria = q.tasks.get("mastery_criteria")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&q.description)
                    .to_string();

                let exercises: Vec<String> = q.tasks.get("exercises")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|e| e.as_str().map(String::from)).collect())
                    .unwrap_or_default();

                ExportQuest {
                    title: q.title.clone(),
                    mastery_criteria,
                    exercises,
                    resources,
                }
            }).collect();

            skills.push(ExportSkill {
                name: skill_node.title.clone(),
                quests,
            });
        }

        phases.push(ExportPhase {
            name: phase_node.title.clone(),
            skills,
        });
    }

    Ok(ExportTree { project, phases })
}
