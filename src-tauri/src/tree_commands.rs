// src-tauri/src/tree_commands.rs

use tauri::State;
use uuid::Uuid;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use crate::database::Database;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tree {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreeNode {
    pub id: String,
    pub tree_id: String,
    pub parent_id: Option<String>,
    pub r#type: String, // trunk|branch|leaf
    pub title: String,
    pub description: String,
    pub progress: u8,
    pub tasks: serde_json::Value,
    pub resources: Option<serde_json::Value>,
    pub x: Option<f32>,
    pub y: Option<f32>,
    pub order_index: i32,
    pub is_locked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreeEdge {
    pub id: String,
    pub tree_id: String,
    pub source_node_id: String,
    pub target_node_id: String,
}

#[tauri::command]
pub async fn create_tree(
    project_id: String,
    name: String,
    database: State<'_, Database>
) -> Result<Tree, String> {
    if name.trim().is_empty() {
        return Err("Tree name cannot be empty".into());
    }

    let tree = Tree {
        id: Uuid::new_v4().to_string(),
        project_id,
        name: name.trim().into(),
        created_at: String::new(), // set by DB DEFAULT NOW()
    };

    sqlx::query!(
        "INSERT INTO trees (id, project_id, name) VALUES ($1, $2, $3)",
        tree.id,
        tree.project_id,
        tree.name
    ).execute(&database.pool).await.map_err(|e| e.to_string())?;

    // Re-fetch to get DB-generated created_at
    let row = sqlx::query!(
        "SELECT id, project_id, name, created_at FROM trees WHERE id = $1",
        tree.id
    ).fetch_one(&database.pool).await.map_err(|e| e.to_string())?;

    Ok(Tree {
        id: row.id,
        project_id: row.project_id,
        name: row.name,
        created_at: row.created_at.to_rfc3339(),
    })
}

#[tauri::command]
pub async fn get_trees(
    project_id: String,
    database: State<'_, Database>
) -> Result<Vec<Tree>, String> {
    let rows = sqlx::query!(
        "SELECT id, project_id, name, created_at FROM trees WHERE project_id = $1 ORDER BY created_at DESC",
        project_id
    ).fetch_all(&database.pool).await.map_err(|e| e.to_string())?;

    Ok(rows.into_iter().map(|r| Tree {
        id: r.id,
        project_id: r.project_id,
        name: r.name,
        created_at: r.created_at.to_rfc3339(),
    }).collect())
}

#[tauri::command]
pub async fn create_tree_node(
    tree_id: String,
    parent_id: Option<String>,
    r#type: String,
    title: String,
    description: String,
    database: State<'_, Database>
) -> Result<TreeNode, String> {
    if !["trunk", "branch", "leaf"].contains(&r#type.as_str()) {
        return Err("Invalid node type".into());
    }

    let node = TreeNode {
        id: Uuid::new_v4().to_string(),
        tree_id,
        parent_id,
        r#type,
        title: title.trim().into(),
        description: description.trim().into(),
        progress: 0,
        tasks: serde_json::json!([]),
        resources: None,
        x: None,
        y: None,
        order_index: 0,
        is_locked: false,
    };

    let progress_i32 = node.progress as i32;

    sqlx::query!(
        r#"
        INSERT INTO tree_nodes (id, tree_id, parent_id, type, title, description, progress, tasks, resources, x, y, order_index)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
        "#,
        node.id,
        node.tree_id,
        node.parent_id,
        node.r#type,
        node.title,
        node.description,
        progress_i32,
        node.tasks,
        node.resources,
        node.x.map(|v| v as f64),
        node.y.map(|v| v as f64),
        node.order_index
    ).execute(&database.pool).await.map_err(|e| e.to_string())?;

    Ok(node)
}

#[tauri::command]
pub async fn update_tree_node(
    node_id: String,
    title: Option<String>,
    description: Option<String>,
    progress: Option<u8>,
    tasks: Option<serde_json::Value>,
    resources: Option<serde_json::Value>,
    position: Option<(f32, f32)>,
    database: State<'_, Database>
) -> Result<(), String> {
    let row = sqlx::query!(
        "SELECT title, description, progress, tasks, resources, x, y FROM tree_nodes WHERE id = $1",
        node_id
    ).fetch_one(&database.pool).await.map_err(|e| e.to_string())?;

    let new_title = title.unwrap_or(row.title);
    let new_description = description.or(row.description);
    let new_progress = progress.unwrap_or(row.progress.unwrap_or(0) as u8) as i32;
    let new_tasks = tasks.unwrap_or(row.tasks);
    let new_resources = resources.or(row.resources);
    let (new_x, new_y) = position
        .map(|(x, y)| (Some(x as f64), Some(y as f64)))
        .unwrap_or((row.x, row.y));

    sqlx::query!(
        "UPDATE tree_nodes SET title=$1, description=$2, progress=$3, tasks=$4, resources=$5, x=$6, y=$7 WHERE id=$8",
        new_title,
        new_description,
        new_progress,
        new_tasks,
        new_resources,
        new_x,
        new_y,
        node_id
    ).execute(&database.pool).await.map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
pub async fn create_tree_edge(
    tree_id: String,
    source_node_id: String,
    target_node_id: String,
    database: State<'_, Database>
) -> Result<TreeEdge, String> {
    let edge = TreeEdge {
        id: Uuid::new_v4().to_string(),
        tree_id,
        source_node_id,
        target_node_id,
    };

    sqlx::query!(
        "INSERT INTO tree_edges (id, tree_id, source_node_id, target_node_id) VALUES ($1, $2, $3, $4)",
        edge.id,
        edge.tree_id,
        edge.source_node_id,
        edge.target_node_id
    ).execute(&database.pool).await.map_err(|e| e.to_string())?;

    Ok(edge)
}

#[tauri::command]
pub async fn get_tree_with_contents(
    tree_id: String,
    database: State<'_, Database>
) -> Result<(Tree, Vec<TreeNode>, Vec<TreeEdge>), String> {
    let tree_row = sqlx::query!(
        "SELECT id, project_id, name, created_at FROM trees WHERE id = $1",
        tree_id
    ).fetch_one(&database.pool).await.map_err(|e| e.to_string())?;

    // Non-macro query: avoids sqlx offline cache, picks up new columns (e.g. is_locked) at runtime
    let node_rows = sqlx::query(
        "SELECT id, tree_id, parent_id, type, title, description, progress, tasks, resources, x, y, order_index, is_locked \
         FROM tree_nodes WHERE tree_id = $1 ORDER BY order_index ASC"
    )
    .bind(&tree_id)
    .fetch_all(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    let edge_rows = sqlx::query(
        "SELECT id, tree_id, source_node_id, target_node_id FROM tree_edges WHERE tree_id = $1"
    )
    .bind(&tree_id)
    .fetch_all(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    let tree = Tree {
        id: tree_row.id,
        project_id: tree_row.project_id,
        name: tree_row.name,
        created_at: tree_row.created_at.to_rfc3339(),
    };

    let node_models = node_rows.into_iter().map(|n| -> Result<TreeNode, sqlx::Error> {
        Ok(TreeNode {
            id: n.try_get("id")?,
            tree_id: n.try_get("tree_id")?,
            parent_id: n.try_get("parent_id")?,
            r#type: n.try_get("type")?,
            title: n.try_get("title")?,
            description: n.try_get::<Option<String>, _>("description")?.unwrap_or_default(),
            progress: n.try_get::<Option<i32>, _>("progress")?.unwrap_or(0) as u8,
            tasks: n.try_get("tasks")?,
            resources: n.try_get("resources")?,
            x: n.try_get::<Option<f64>, _>("x")?.map(|v| v as f32),
            y: n.try_get::<Option<f64>, _>("y")?.map(|v| v as f32),
            order_index: n.try_get::<Option<i32>, _>("order_index")?.unwrap_or(0),
            is_locked: n.try_get::<Option<bool>, _>("is_locked")?.unwrap_or(false),
        })
    }).collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;

    let edge_models = edge_rows.into_iter().map(|e| -> Result<TreeEdge, sqlx::Error> {
        Ok(TreeEdge {
            id: e.try_get("id")?,
            tree_id: e.try_get("tree_id")?,
            source_node_id: e.try_get("source_node_id")?,
            target_node_id: e.try_get("target_node_id")?,
        })
    }).collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;

    Ok((tree, node_models, edge_models))
}

#[tauri::command]
pub async fn delete_tree_node(
    node_id: String,
    database: State<'_, Database>
) -> Result<(), String> {
    sqlx::query!(
        "DELETE FROM tree_nodes WHERE id = $1",
        node_id
    ).execute(&database.pool).await.map_err(|e| e.to_string())?;

    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestRow {
    pub id: String,
    pub tree_id: String,
    pub project_id: String,
    pub project_name: String,
    pub tree_name: String,
    pub title: String,
    pub description: String,
    pub progress: u8,
    pub tasks: serde_json::Value, // JSONB — frontend receives actual array
    pub order_index: i32,
}

#[tauri::command]
pub async fn get_all_quests(
    database: State<'_, Database>
) -> Result<Vec<QuestRow>, String> {
    let rows = sqlx::query!(
        r#"
        SELECT n.id, n.tree_id, n.title, n.description, n.progress, n.tasks, n.order_index,
               t.project_id, t.name AS tree_name,
               p.name AS project_name
        FROM tree_nodes n
        JOIN trees t ON n.tree_id = t.id
        JOIN projects p ON t.project_id = p.id
        WHERE n.type = 'leaf'
        ORDER BY t.project_id, n.order_index ASC
        "#
    ).fetch_all(&database.pool).await.map_err(|e| e.to_string())?;

    Ok(rows.into_iter().map(|r| QuestRow {
        id: r.id,
        tree_id: r.tree_id,
        project_id: r.project_id,
        project_name: r.project_name,
        tree_name: r.tree_name,
        title: r.title,
        description: r.description.unwrap_or_default(),
        progress: r.progress.unwrap_or(0) as u8,
        tasks: r.tasks,
        order_index: r.order_index.unwrap_or(0) as i32,
    }).collect())
}

/// Recalculates progress for all branch/trunk nodes in a tree as the
/// average of their children's progress. Uses a bottom-up post-order
/// traversal so parent progress reflects updated child values.
/// Uses non-macro sqlx::query to avoid requiring an offline cache refresh.
#[tauri::command]
pub async fn recalculate_tree_progress(
    tree_id: String,
    database: State<'_, Database>,
) -> Result<(), String> {
    let rows = sqlx::query(
        "SELECT id, parent_id, progress FROM tree_nodes WHERE tree_id = $1"
    )
    .bind(&tree_id)
    .fetch_all(&database.pool)
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

    // Iterative DFS to get visit order, then reverse → post-order (leaves first)
    let mut order: Vec<String> = Vec::new();
    let mut stack = roots;
    while let Some(id) = stack.pop() {
        order.push(id.clone());
        if let Some(kids) = children_map.get(&id) {
            stack.extend(kids.iter().cloned());
        }
    }
    order.reverse();

    // Walk bottom-up: for any node that has children, set its progress to
    // the average of its children's (already-updated) progress.
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
                    .execute(&database.pool)
                    .await
                    .map_err(|e| e.to_string())?;
            }
        }
    }

    Ok(())
}

/// Idempotent unlock propagation: runs after any quest completion.
/// 1. Ensures the first skill of every phase is always unlocked.
/// 2. Unlocks the next skill on each branch when the previous skill hits 100%.
#[tauri::command]
pub async fn recalculate_unlocks(
    tree_id: String,
    database: State<'_, Database>,
) -> Result<(), String> {
    // Step 1: always unlock the first (lowest order_index) skill of each phase
    sqlx::query(
        r#"
        UPDATE tree_nodes SET is_locked = false
        WHERE id IN (
            SELECT DISTINCT ON (parent_id) id
            FROM tree_nodes
            WHERE tree_id = $1 AND type = 'branch'
            ORDER BY parent_id, order_index ASC
        )
        "#
    )
    .bind(&tree_id)
    .execute(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    // Step 2: for every skill at 100%, unlock the next sibling skill
    sqlx::query(
        r#"
        UPDATE tree_nodes SET is_locked = false
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
        )
        "#
    )
    .bind(&tree_id)
    .execute(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(())
}
