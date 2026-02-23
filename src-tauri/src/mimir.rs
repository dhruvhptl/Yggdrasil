// src-tauri/src/mimir.rs
// Proxy commands that forward to the Mimir sidecar (localhost:3001).
// Also includes direct-DB commands that don't need the sidecar.

use serde::{Deserialize, Serialize};
use tauri::State;
use crate::database::Database;

const MIMIR_URL: &str = "http://localhost:3001";

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all(serialize = "camelCase", deserialize = "snake_case"))]
pub struct MimirResource {
    pub id: String,
    pub title: String,
    pub url: Option<String>,
    // sidecar column is "type"; frontend expects "resourceType"
    #[serde(rename(deserialize = "type", serialize = "resourceType"))]
    pub resource_type: String,
    pub status: String,
    pub user_notes: Option<String>,
    pub created_at: String,
    #[serde(default)]
    pub relevance_score: Option<f32>,
}

// ─── Sidecar proxy commands ──────────────────────────────────────────────────

#[tauri::command]
pub async fn get_mimir_resources() -> Result<Vec<MimirResource>, String> {
    let client = reqwest::Client::new();
    let response = client
        .get(format!("{}/resources", MIMIR_URL))
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
        .map_err(|_| "Mimir sidecar not running. Start it with: npm --prefix sidecar start".to_string())?;

    response
        .json()
        .await
        .map_err(|e| format!("Failed to parse resources: {}", e))
}

#[tauri::command]
pub async fn get_node_resources(node_id: String) -> Result<Vec<MimirResource>, String> {
    let client = reqwest::Client::new();
    let response = client
        .get(format!("{}/resources/by-node/{}", MIMIR_URL, node_id))
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
        .map_err(|_| "Mimir sidecar not running".to_string())?;

    response
        .json()
        .await
        .map_err(|e| format!("Failed to parse node resources: {}", e))
}

#[tauri::command]
pub async fn ingest_mimir_url(url: String, title: Option<String>) -> Result<String, String> {
    let client = reqwest::Client::new();
    let mut body = serde_json::json!({ "url": url });
    if let Some(t) = title {
        body["title"] = serde_json::json!(t);
    }

    let response = client
        .post(format!("{}/ingest/url", MIMIR_URL))
        .json(&body)
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await
        .map_err(|_| "Mimir sidecar not running. Start it with: npm --prefix sidecar start".to_string())?;

    if !response.status().is_success() {
        let err: serde_json::Value = response.json().await.unwrap_or_default();
        return Err(err["error"].as_str().unwrap_or("Ingest failed").to_string());
    }

    let result: serde_json::Value = response.json().await.map_err(|e| e.to_string())?;
    Ok(result["id"].as_str().unwrap_or("").to_string())
}

#[tauri::command]
pub async fn ingest_mimir_text(text: String, title: Option<String>) -> Result<String, String> {
    let client = reqwest::Client::new();
    let mut body = serde_json::json!({ "text": text });
    if let Some(t) = title {
        body["title"] = serde_json::json!(t);
    }

    let response = client
        .post(format!("{}/ingest/text", MIMIR_URL))
        .json(&body)
        .timeout(std::time::Duration::from_secs(120))
        .send()
        .await
        .map_err(|_| "Mimir sidecar not running. Start it with: npm --prefix sidecar start".to_string())?;

    if !response.status().is_success() {
        let err: serde_json::Value = response.json().await.unwrap_or_default();
        return Err(err["error"].as_str().unwrap_or("Ingest failed").to_string());
    }

    let result: serde_json::Value = response.json().await.map_err(|e| e.to_string())?;
    Ok(result["id"].as_str().unwrap_or("").to_string())
}

#[tauri::command]
pub async fn delete_mimir_resource(resource_id: String) -> Result<(), String> {
    let client = reqwest::Client::new();
    let response = client
        .delete(format!("{}/resources/{}", MIMIR_URL, resource_id))
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|_| "Mimir sidecar not running".to_string())?;

    if !response.status().is_success() {
        let err: serde_json::Value = response.json().await.unwrap_or_default();
        return Err(err["error"].as_str().unwrap_or("Delete failed").to_string());
    }

    Ok(())
}

#[tauri::command]
pub async fn match_node_to_resources(node_id: String) -> Result<Vec<MimirResource>, String> {
    let client = reqwest::Client::new();
    let response = client
        .post(format!("{}/match/{}", MIMIR_URL, node_id))
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await
        .map_err(|_| "Mimir sidecar not running".to_string())?;

    if !response.status().is_success() {
        return Ok(vec![]);
    }

    response
        .json()
        .await
        .map_err(|e| format!("Failed to parse match results: {}", e))
}

#[tauri::command]
pub async fn ingest_mimir_pdf(filename: String, pdf_base64: String) -> Result<String, String> {
    let client = reqwest::Client::new();
    let body = serde_json::json!({ "filename": filename, "pdf_base64": pdf_base64 });

    let response = client
        .post(format!("{}/ingest/pdf", MIMIR_URL))
        .json(&body)
        .timeout(std::time::Duration::from_secs(120))
        .send()
        .await
        .map_err(|_| "Mimir sidecar not running. Start it with: npm --prefix sidecar start".to_string())?;

    if !response.status().is_success() {
        let err: serde_json::Value = response.json().await.unwrap_or_default();
        return Err(err["error"].as_str().unwrap_or("Ingest failed").to_string());
    }

    let result: serde_json::Value = response.json().await.map_err(|e| e.to_string())?;
    Ok(result["id"].as_str().unwrap_or("").to_string())
}

// ─── Direct DB command (no sidecar needed) ───────────────────────────────────

#[tauri::command]
pub async fn link_resource_to_node(
    resource_id: String,
    node_id: String,
    database: State<'_, Database>,
) -> Result<(), String> {
    let link_id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO mimir_node_links (id, resource_id, node_id) VALUES ($1, $2, $3) \
         ON CONFLICT (resource_id, node_id) DO NOTHING",
    )
    .bind(&link_id)
    .bind(&resource_id)
    .bind(&node_id)
    .execute(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(())
}
