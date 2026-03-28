// src-tauri/src/mimir.rs
// Proxy commands that forward to the Mimir sidecar (localhost:3001).
// Also includes direct-DB commands that don't need the sidecar.

use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::Row;
use tauri::{Emitter, State};
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
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub node_count: i32,
    #[serde(default)]
    pub is_completed: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ScrapedExternalLink {
    pub url: String,
    pub text: String,
    #[serde(rename = "type")]
    pub link_type: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct IngestResult {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub page_type: String,
    #[serde(default)]
    pub external_links: Vec<ScrapedExternalLink>,
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
pub async fn ingest_mimir_url(url: String, title: Option<String>, force_dynamic: Option<bool>, parent_id: Option<String>) -> Result<IngestResult, String> {
    let client = reqwest::Client::new();
    let mut body = serde_json::json!({ "url": url });
    if let Some(t) = title {
        body["title"] = serde_json::json!(t);
    }
    if force_dynamic == Some(true) {
        body["force_dynamic"] = serde_json::json!(true);
    }
    if let Some(pid) = parent_id {
        body["parent_id"] = serde_json::json!(pid);
    }

    let response = client
        .post(format!("{}/ingest/url", MIMIR_URL))
        .json(&body)
        .timeout(std::time::Duration::from_secs(120))
        .send()
        .await
        .map_err(|_| "Mimir sidecar not running. Start it with: npm --prefix sidecar start".to_string())?;

    if response.status() == 409 {
        let body: serde_json::Value = response.json().await.unwrap_or_default();
        let existing_title = body["existing"]["title"].as_str().unwrap_or("existing resource");
        return Err(format!("DUPLICATE:{}", existing_title));
    }
    if !response.status().is_success() {
        let err: serde_json::Value = response.json().await.unwrap_or_default();
        return Err(err["error"].as_str().unwrap_or("Ingest failed").to_string());
    }

    response.json::<IngestResult>().await.map_err(|e| e.to_string())
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

    if response.status() == 409 {
        let body: serde_json::Value = response.json().await.unwrap_or_default();
        let existing_title = body["existing"]["title"].as_str().unwrap_or("existing resource");
        return Err(format!("DUPLICATE:{}", existing_title));
    }
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfIngestResult {
    pub id: String,
    pub title: String,
    pub text_preview: Option<String>,
}

#[tauri::command]
pub async fn ingest_mimir_pdf(filename: String, pdf_base64: String) -> Result<PdfIngestResult, String> {
    let client = reqwest::Client::new();
    let body = serde_json::json!({ "filename": filename, "pdf_base64": pdf_base64 });

    let response = client
        .post(format!("{}/ingest/pdf", MIMIR_URL))
        .json(&body)
        .timeout(std::time::Duration::from_secs(120))
        .send()
        .await
        .map_err(|_| "Mimir sidecar not running. Start it with: npm --prefix sidecar start".to_string())?;

    if response.status() == 409 {
        let body: serde_json::Value = response.json().await.unwrap_or_default();
        let existing_title = body["existing"]["title"].as_str().unwrap_or("existing resource");
        return Err(format!("DUPLICATE:{}", existing_title));
    }
    if !response.status().is_success() {
        let err: serde_json::Value = response.json().await.unwrap_or_default();
        return Err(err["error"].as_str().unwrap_or("Ingest failed").to_string());
    }

    let result: serde_json::Value = response.json().await.map_err(|e| e.to_string())?;
    Ok(PdfIngestResult {
        id: result["id"].as_str().unwrap_or("").to_string(),
        title: result["title"].as_str().unwrap_or("").to_string(),
        text_preview: result["textPreview"].as_str().map(|s| s.to_string()),
    })
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PdfTextResult {
    pub text: String,
    pub pages: i32,
    pub chars: i32,
}

#[tauri::command]
pub async fn extract_pdf_text(pdf_base64: String) -> Result<PdfTextResult, String> {
    let client = reqwest::Client::new();
    let body = serde_json::json!({ "pdf_base64": pdf_base64 });

    let response = client
        .post(format!("{}/ingest/extract-pdf-text", MIMIR_URL))
        .json(&body)
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await
        .map_err(|_| "Mimir sidecar not running. Start it with: npm --prefix sidecar start".to_string())?;

    if !response.status().is_success() {
        let err: serde_json::Value = response.json().await.unwrap_or_default();
        return Err(err["error"].as_str().unwrap_or("PDF text extraction failed").to_string());
    }

    response
        .json()
        .await
        .map_err(|e| format!("Failed to parse PDF text result: {}", e))
}

// ─── Chat (RAG) ─────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MimirChatSource {
    pub title: String,
    pub url: Option<String>,
    pub chunk: String,
    pub score: f32,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MimirChatResponse {
    pub answer: String,
    pub sources: Vec<MimirChatSource>,
}

#[tauri::command]
pub async fn mimir_chat(
    message: String,
    page: String,
    tree_id: Option<String>,
    node_title: Option<String>,
) -> Result<MimirChatResponse, String> {
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "message": message,
        "context": { "page": page, "treeId": tree_id, "nodeTitle": node_title }
    });

    let response = client
        .post(format!("{}/chat", MIMIR_URL))
        .json(&body)
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await
        .map_err(|_| "Mimir sidecar not running. Start it with: npm --prefix sidecar start".to_string())?;

    if !response.status().is_success() {
        let err: serde_json::Value = response.json().await.unwrap_or_default();
        return Err(err["error"].as_str().unwrap_or("Chat failed").to_string());
    }

    response
        .json()
        .await
        .map_err(|e| format!("Failed to parse chat response: {}", e))
}

// ─── Rescrape commands ───────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RescrapeResult {
    pub success: bool,
    pub changed: bool,
    pub old_chunks: i32,
    pub new_chunks: i32,
    pub message: String,
}

#[tauri::command]
pub async fn rescrape_resource(resource_id: String) -> Result<RescrapeResult, String> {
    let client = reqwest::Client::new();
    let response = client
        .post(format!("{}/rescrape/{}", MIMIR_URL, resource_id))
        .timeout(std::time::Duration::from_secs(120))
        .send()
        .await
        .map_err(|_| "Mimir sidecar not running".to_string())?;

    if response.status().as_u16() == 404 {
        return Err("Resource not found".to_string());
    }
    if response.status().as_u16() == 400 {
        let err: serde_json::Value = response.json().await.unwrap_or_default();
        return Err(err["error"].as_str().unwrap_or("Bad request").to_string());
    }
    if !response.status().is_success() {
        let err: serde_json::Value = response.json().await.unwrap_or_default();
        return Err(err["error"].as_str().unwrap_or("Rescrape failed").to_string());
    }

    response.json().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn rescrape_all(app: tauri::AppHandle) -> Result<Vec<RescrapeResult>, String> {
    let client = reqwest::Client::new();
    let mut response = client
        .post(format!("{}/rescrape", MIMIR_URL))
        .timeout(std::time::Duration::from_secs(600))
        .send()
        .await
        .map_err(|_| "Mimir sidecar not running".to_string())?;

    if !response.status().is_success() {
        let err: serde_json::Value = response.json().await.unwrap_or_default();
        return Err(err["error"].as_str().unwrap_or("Rescrape all failed").to_string());
    }

    let mut results: Vec<RescrapeResult> = Vec::new();
    let mut buffer = String::new();

    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        buffer.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(newline_pos) = buffer.find('\n') {
            let line = buffer[..newline_pos].trim().to_string();
            buffer = buffer[newline_pos + 1..].to_string();

            if line.is_empty() {
                continue;
            }

            let event: serde_json::Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(_) => continue,
            };

            match event["type"].as_str() {
                Some("progress") => {
                    let _ = app.emit("rescrape-progress", &event);
                    results.push(RescrapeResult {
                        success: event["status"].as_str().map(|s| s != "failed").unwrap_or(false),
                        changed: event["status"].as_str().map(|s| s == "improved").unwrap_or(false),
                        old_chunks: event["oldChunks"].as_i64().unwrap_or(0) as i32,
                        new_chunks: event["newChunks"].as_i64().unwrap_or(0) as i32,
                        message: event["status"].as_str().unwrap_or("").to_string(),
                    });
                }
                Some("complete") => {
                    let _ = app.emit("rescrape-complete", &event);
                }
                Some("error") => {
                    return Err(event["message"].as_str().unwrap_or("Rescrape all failed").to_string());
                }
                _ => {}
            }
        }
    }

    Ok(results)
}

// ─── Link discovery ──────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredLink {
    pub url: String,
    pub text: String,
}

#[tauri::command]
pub async fn discover_links(url: String) -> Result<Vec<DiscoveredLink>, String> {
    let client = reqwest::Client::new();
    let body = serde_json::json!({ "url": url });

    let response = client
        .post(format!("{}/discover", MIMIR_URL))
        .json(&body)
        .timeout(std::time::Duration::from_secs(90))
        .send()
        .await
        .map_err(|_| "Mimir sidecar not running".to_string())?;

    if !response.status().is_success() {
        let err: serde_json::Value = response.json().await.unwrap_or_default();
        return Err(err["error"].as_str().unwrap_or("Discovery failed").to_string());
    }

    let result: serde_json::Value = response.json().await.map_err(|e| e.to_string())?;
    let links = result["links"]
        .as_array()
        .ok_or_else(|| "Invalid response format".to_string())?;

    Ok(links.iter().map(|l| DiscoveredLink {
        url: l["url"].as_str().unwrap_or("").to_string(),
        text: l["text"].as_str().unwrap_or("").to_string(),
    }).collect())
}

// ─── Playlist fetch ──────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all(serialize = "camelCase", deserialize = "snake_case"))]
pub struct PlaylistVideo {
    pub video_id: String,
    pub title: String,
    pub url: String,
    pub position: i32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all(serialize = "camelCase", deserialize = "snake_case"))]
pub struct PlaylistInfo {
    pub playlist_title: String,
    pub playlist_id: String,
    pub videos: Vec<PlaylistVideo>,
}

#[tauri::command]
pub async fn fetch_playlist(url: String) -> Result<PlaylistInfo, String> {
    let client = reqwest::Client::new();
    let body = serde_json::json!({ "url": url });

    let response = client
        .post(format!("{}/fetch-playlist", MIMIR_URL))
        .json(&body)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|_| "Mimir sidecar not running".to_string())?;

    if !response.status().is_success() {
        let err: serde_json::Value = response.json().await.unwrap_or_default();
        return Err(err["error"].as_str().unwrap_or("Playlist fetch failed").to_string());
    }

    let raw = response.text().await.map_err(|e| e.to_string())?;
    serde_json::from_str::<PlaylistInfo>(&raw)
        .map_err(|e| {
            eprintln!("[fetch_playlist] decode error: {e}\nraw body: {raw}");
            format!("Failed to parse playlist response: {e}")
        })
}

// ─── Direct DB commands (no sidecar needed) ──────────────────────────────────

#[tauri::command]
pub async fn get_chunk_counts(
    database: State<'_, Database>,
) -> Result<std::collections::HashMap<String, i64>, String> {
    use sqlx::Row;
    let rows = sqlx::query(
        "SELECT resource_id, COUNT(*) AS count FROM mimir_chunks GROUP BY resource_id",
    )
    .fetch_all(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut map = std::collections::HashMap::new();
    for row in rows {
        let resource_id: String = row.try_get("resource_id").map_err(|e| e.to_string())?;
        let count: i64 = row.try_get("count").map_err(|e| e.to_string())?;
        map.insert(resource_id, count);
    }
    Ok(map)
}

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

// ─── Tag commands ───────────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_distinct_tags(
    database: State<'_, Database>,
) -> Result<Vec<String>, String> {
    let rows = sqlx::query(
        "SELECT DISTINCT unnest(tags) AS tag FROM mimir_resources ORDER BY tag"
    )
    .fetch_all(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    let tags: Vec<String> = rows
        .iter()
        .map(|r| r.try_get::<String, _>("tag").unwrap_or_default())
        .collect();
    Ok(tags)
}

#[tauri::command]
pub async fn update_resource_tags(
    resource_id: String,
    tags: Vec<String>,
    database: State<'_, Database>,
) -> Result<(), String> {
    sqlx::query(
        "UPDATE mimir_resources SET tags = $1 WHERE id = $2"
    )
    .bind(&tags)
    .bind(&resource_id)
    .execute(&database.pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn auto_tag_existing_resources(
    database: State<'_, Database>,
) -> Result<String, String> {
    let env_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".env");
    dotenv::from_path(&env_path).ok();
    dotenv::dotenv().ok();

    let model = std::env::var("TREE_GEN_MODEL")
        .unwrap_or_else(|_| "llama-3.3-70b-versatile".to_string());
    let base_url = std::env::var("TREE_GEN_BASE_URL")
        .unwrap_or_else(|_| "https://api.groq.com/openai/v1/chat/completions".to_string());
    let api_key = std::env::var("TREE_GEN_API_KEY")
        .unwrap_or_else(|_| std::env::var("GROQ_API_KEY").unwrap_or_default());

    let rows = sqlx::query(
        "SELECT id, title FROM mimir_resources WHERE tags = '{}'"
    )
    .fetch_all(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    let total = rows.len();
    println!("🏷️  Auto-tagging {} untagged resources with model {}", total, model);

    let client = reqwest::Client::new();
    let mut tagged = 0u32;

    for row in &rows {
        let id: String = row.try_get("id").map_err(|e| e.to_string())?;
        let title: String = row.try_get("title").map_err(|e| e.to_string())?;

        let user_prompt = format!(
            "Given this resource title: '{}', suggest 2-4 tags from these categories:\n\
             Technology: rust, typescript, python, react, sql, julia, node, postgres, tauri, d3, numpy\n\
             Type: tutorial, docs, video, article, reference\n\
             Level: beginner, intermediate, advanced\n\
             Return ONLY a JSON array of strings, e.g. [\"rust\", \"docs\"]",
            title
        );

        let response = client
            .post(&base_url)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(&json!({
                "model": model,
                "messages": [
                    { "role": "system", "content": "You are a tag classifier. Return ONLY a JSON array of tag strings." },
                    { "role": "user",   "content": user_prompt }
                ],
                "temperature": 0.2,
                "max_tokens": 100
            }))
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await;

        let resp = match response {
            Ok(r) => r,
            Err(e) => {
                println!("  ⚠️  Failed to tag '{}': {}", title, e);
                continue;
            }
        };

        let body: serde_json::Value = match resp.json().await {
            Ok(v) => v,
            Err(e) => {
                println!("  ⚠️  Failed to parse response for '{}': {}", title, e);
                continue;
            }
        };

        let content = body["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("[]");

        // Strip markdown fences if present
        let clean = content
            .trim()
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim();

        let tags: Vec<String> = match serde_json::from_str(clean) {
            Ok(t) => t,
            Err(_) => {
                println!("  ⚠️  Bad JSON for '{}': {}", title, clean);
                continue;
            }
        };

        sqlx::query("UPDATE mimir_resources SET tags = $1 WHERE id = $2")
            .bind(&tags)
            .bind(&id)
            .execute(&database.pool)
            .await
            .map_err(|e| e.to_string())?;

        tagged += 1;
        println!("  ✅ [{}] {} → {:?}", tagged, title, tags);
    }

    let summary = format!("Tagged {} of {} resources", tagged, total);
    println!("🏷️  {}", summary);
    Ok(summary)
}

#[tauri::command]
pub async fn rematch_all_nodes(
    database: State<'_, Database>,
) -> Result<String, String> {
    let rows = sqlx::query(
        "SELECT id FROM tree_nodes WHERE type = 'leaf'"
    )
    .fetch_all(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    let node_ids: Vec<String> = rows
        .iter()
        .map(|r| r.try_get::<String, _>("id").unwrap_or_default())
        .collect();

    let total = node_ids.len();
    println!("🔗 Re-matching {} leaf nodes to resources", total);

    let client = reqwest::Client::new();

    let alive = client
        .get(format!("{}/health", MIMIR_URL))
        .timeout(std::time::Duration::from_secs(2))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false);

    if !alive {
        return Err("Mimir sidecar not running".to_string());
    }

    let mut matched = 0u32;
    for node_id in &node_ids {
        match client
            .post(format!("{}/match/{}", MIMIR_URL, node_id))
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => matched += 1,
            Ok(r) => println!("  ⚠️  match returned {} for node {}", r.status(), node_id),
            Err(e) => {
                println!("  ⚠️  match error for {}: {}", node_id, e);
                break;
            }
        }
    }

    let summary = format!("Matched {} of {} nodes", matched, total);
    println!("🔗 {}", summary);
    Ok(summary)
}

#[tauri::command]
pub async fn toggle_resource_completion(
    resource_id: String,
    database: State<'_, Database>,
) -> Result<bool, String> {
    let row = sqlx::query(
        "UPDATE mimir_resources SET is_completed = NOT is_completed WHERE id = $1 RETURNING is_completed"
    )
    .bind(&resource_id)
    .fetch_one(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    let new_val: bool = row.try_get("is_completed").map_err(|e| e.to_string())?;
    Ok(new_val)
}

#[tauri::command]
pub async fn on_resource_completed(
    resource_id: String,
    database: State<'_, Database>,
) -> Result<(), String> {
    // Find all tree nodes linked to this resource
    let linked = sqlx::query(
        "SELECT DISTINCT mnl.node_id, tn.tree_id
         FROM mimir_node_links mnl
         JOIN tree_nodes tn ON tn.id = mnl.node_id
         WHERE mnl.resource_id = $1"
    )
    .bind(&resource_id)
    .fetch_all(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    if linked.is_empty() {
        return Ok(());
    }

    // Increment progress by 20 (capped at 100) for each linked node
    let mut affected_trees: std::collections::HashSet<String> = std::collections::HashSet::new();
    for row in &linked {
        let node_id: String = row.try_get("node_id").map_err(|e| e.to_string())?;
        let tree_id: String = row.try_get("tree_id").map_err(|e| e.to_string())?;

        sqlx::query(
            "UPDATE tree_nodes SET progress = LEAST(COALESCE(progress, 0) + 20, 100) WHERE id = $1"
        )
        .bind(&node_id)
        .execute(&database.pool)
        .await
        .map_err(|e| e.to_string())?;

        affected_trees.insert(tree_id);
    }

    // Recalculate progress for each affected tree (bottom-up cascade)
    for tree_id in &affected_trees {
        let rows = sqlx::query(
            "SELECT id, parent_id, progress FROM tree_nodes WHERE tree_id = $1"
        )
        .bind(tree_id)
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

        // DFS → reverse for post-order (leaves first)
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
                        .execute(&database.pool)
                        .await
                        .map_err(|e| e.to_string())?;
                }
            }
        }

        println!("🔄 Recalculated progress for tree {}", tree_id);
    }

    Ok(())
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LinkedNodeTitle {
    pub title: String,
}

#[tauri::command]
pub async fn get_linked_node_titles(
    resource_id: String,
    database: State<'_, Database>,
) -> Result<Vec<LinkedNodeTitle>, String> {
    let rows = sqlx::query(
        "SELECT tn.title FROM mimir_node_links mnl JOIN tree_nodes tn ON tn.id = mnl.node_id WHERE mnl.resource_id = $1 ORDER BY tn.title"
    )
    .bind(&resource_id)
    .fetch_all(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    let titles: Vec<LinkedNodeTitle> = rows
        .iter()
        .map(|r| LinkedNodeTitle {
            title: r.try_get::<String, _>("title").unwrap_or_default(),
        })
        .collect();
    Ok(titles)
}
