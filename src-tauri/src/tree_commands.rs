// src-tauri/src/tree_commands.rs - Tauri commands for creating and managing trees

use tauri::State;
use uuid::Uuid;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use crate::database::Database;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tree { pub id: String, pub project_id: String, pub name: String, pub created_at: String }

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
  pub x: Option<f32>,
  pub y: Option<f32>,
  pub order_index: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreeEdge { pub id: String, pub tree_id: String, pub source_node_id: String, pub target_node_id: String }

#[tauri::command]
pub async fn create_tree(project_id: String, name: String, database: State<'_, Database>) -> Result<Tree, String> {
  if name.trim().is_empty() { return Err("Tree name cannot be empty".into()); }
  let tree = Tree { id: Uuid::new_v4().to_string(), project_id, name: name.trim().into(), created_at: Utc::now().to_rfc3339() };
  sqlx::query!("INSERT INTO trees (id, project_id, name, created_at) VALUES (?1, ?2, ?3, ?4)", tree.id, tree.project_id, tree.name, tree.created_at)
    .execute(&database.pool).await.map_err(|e| e.to_string())?;
  Ok(tree)
}

#[tauri::command]
pub async fn get_trees(project_id: String, database: State<'_, Database>) -> Result<Vec<Tree>, String> {
  let rows = sqlx::query!("SELECT * FROM trees WHERE project_id = ?1 ORDER BY created_at DESC", project_id)
    .fetch_all(&database.pool).await.map_err(|e| e.to_string())?;
  Ok(rows.into_iter().map(|r| Tree { id: r.id, project_id: r.project_id, name: r.name, created_at: r.created_at }).collect())
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
  if !["trunk","branch","leaf"].contains(&r#type.as_str()) { return Err("Invalid node type".into()); }
  let node = TreeNode {
    id: Uuid::new_v4().to_string(), tree_id, parent_id, r#type, title: title.trim().into(), description: description.trim().into(), progress: 0,
    tasks: serde_json::json!([]), x: None, y: None, order_index: 0
  };
  sqlx::query!(
    "INSERT INTO tree_nodes (id, tree_id, parent_id, type, title, description, progress, tasks, x, y, order_index) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
    node.id, node.tree_id, node.parent_id, node.r#type, node.title, node.description, node.progress as i64, node.tasks.to_string(), node.x, node.y, node.order_index
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
  position: Option<(f32, f32)>,
  database: State<'_, Database>
) -> Result<(), String> {
  // Fetch existing
  let row = sqlx::query!("SELECT * FROM tree_nodes WHERE id = ?1", node_id).fetch_one(&database.pool).await.map_err(|e| e.to_string())?;
  let mut new_title = row.title;
  let mut new_description = row.description;
  let mut new_progress = row.progress as u8;
  let mut new_tasks = row.tasks;
  let mut x = row.x; let mut y = row.y;
  if let Some(t) = title { new_title = t; }
  if let Some(d) = description { new_description = d; }
  if let Some(p) = progress { new_progress = p; }
  if let Some(ts) = tasks { new_tasks = ts.to_string(); }
  if let Some((nx, ny)) = position { x = Some(nx); y = Some(ny); }
  sqlx::query!(
    "UPDATE tree_nodes SET title=?1, description=?2, progress=?3, tasks=?4, x=?5, y=?6 WHERE id=?7",
    new_title, new_description, new_progress as i64, new_tasks, x, y, node_id
  ).execute(&database.pool).await.map_err(|e| e.to_string())?;
  Ok(())
}

#[tauri::command]
pub async fn create_tree_edge(tree_id: String, source_node_id: String, target_node_id: String, database: State<'_, Database>) -> Result<TreeEdge, String> {
  let edge = TreeEdge { id: Uuid::new_v4().to_string(), tree_id, source_node_id, target_node_id };
  sqlx::query!("INSERT INTO tree_edges (id, tree_id, source_node_id, target_node_id) VALUES (?1, ?2, ?3, ?4)", edge.id, edge.tree_id, edge.source_node_id, edge.target_node_id)
    .execute(&database.pool).await.map_err(|e| e.to_string())?;
  Ok(edge)
}

#[tauri::command]
pub async fn get_tree_with_contents(tree_id: String, database: State<'_, Database>) -> Result<(Tree, Vec<TreeNode>, Vec<TreeEdge>), String> {
  let tree_row = sqlx::query!("SELECT * FROM trees WHERE id = ?1", tree_id).fetch_one(&database.pool).await.map_err(|e| e.to_string())?;
  let nodes = sqlx::query!("SELECT * FROM tree_nodes WHERE tree_id = ?1 ORDER BY order_index ASC", tree_id).fetch_all(&database.pool).await.map_err(|e| e.to_string())?;
  let edges = sqlx::query!("SELECT * FROM tree_edges WHERE tree_id = ?1", tree_id).fetch_all(&database.pool).await.map_err(|e| e.to_string())?;
  let tree = Tree { id: tree_row.id, project_id: tree_row.project_id, name: tree_row.name, created_at: tree_row.created_at };
  let node_models = nodes.into_iter().map(|n| TreeNode {
    id: n.id, tree_id: n.tree_id, parent_id: n.parent_id, r#type: n.r#type, title: n.title, description: n.description,
    progress: n.progress as u8, tasks: serde_json::from_str(&n.tasks).unwrap_or(serde_json::json!([])), x: n.x, y: n.y, order_index: n.order_index
  }).collect();
  let edge_models = edges.into_iter().map(|e| TreeEdge { id: e.id, tree_id: e.tree_id, source_node_id: e.source_node_id, target_node_id: e.target_node_id }).collect();
  Ok((tree, node_models, edge_models))
}
