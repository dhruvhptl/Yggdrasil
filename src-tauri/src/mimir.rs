// src-tauri/src/mimir.rs
// Direct Mimir implementation — no sidecar needed.
// Embeddings via Perplexity pplx-embed-v1-0.6b (1024-dim, native output) via OpenRouter.
// RAG chat via Groq. Scraping via Python service (port 3002).
// Note: do NOT use dotenvy or load_env here — env vars are loaded by dev.sh/build.sh
// before the process starts. Future edits must read env vars via std::env::var() only.

use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Sha256, Digest};
use sqlx::Row;
use tauri::{Emitter, State};
use crate::database::Database;

const SCRAPER_URL: &str = "http://127.0.0.1:3002";
const EMBED_DIM: usize = 1024;

// ─── Structs ────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all(serialize = "camelCase", deserialize = "snake_case"))]
pub struct MimirResource {
    pub id: String,
    pub title: String,
    pub url: Option<String>,
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
    #[serde(default)]
    pub matched_section_title: Option<String>,
    #[serde(default)]
    pub matched_page_start: Option<i32>,
    #[serde(default)]
    pub matched_page_end: Option<i32>,
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfIngestResult {
    pub id: String,
    pub title: String,
    pub text_preview: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PdfTextResult {
    pub text: String,
    pub pages: i32,
    pub chars: i32,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MimirChatSource {
    pub title: String,
    pub url: Option<String>,
    pub chunk: String,
    pub score: f32,
    pub section_title: Option<String>,
    pub page_start: Option<i32>,
    pub page_end: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MimirChatResponse {
    pub answer: String,
    pub sources: Vec<MimirChatSource>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RescrapeResult {
    pub success: bool,
    pub changed: bool,
    pub old_chunks: i32,
    pub new_chunks: i32,
    pub message: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredLink {
    pub url: String,
    pub text: String,
}

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

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LinkedNodeTitle {
    pub title: String,
}

// ─── Env helpers ────────────────────────────────────────────────────────────

fn openrouter_api_key() -> Result<String, String> {
    std::env::var("OPENROUTER_API_KEY")
        .map_err(|_| "OPENROUTER_API_KEY not configured".to_string())
}

fn groq_api_key() -> Result<String, String> {
    std::env::var("GROQ_API_KEY")
        .map_err(|_| "GROQ_API_KEY not configured".to_string())
}

// ─── Embedding via Perplexity API ───────────────────────────────────────────

/// Call OpenRouter embeddings API (pplx-embed-v1-0.6b, 1024-dim native output).
/// Accepts a shared client to avoid per-call TCP connection overhead.
async fn get_embedding(client: &reqwest::Client, text: &str) -> Result<Vec<f32>, String> {
    let api_key = openrouter_api_key()?;

    // Do NOT include "dimensions" — pplx-embed-v1-0.6b natively outputs 1024 dims.
    // The "dimensions" parameter is an OpenAI-specific MRL feature not supported here.
    let body = json!({
        "model": "perplexity/pplx-embed-v1-0.6b",
        "input": [text]
    });

    let response = client
        .post("https://openrouter.ai/api/v1/embeddings")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| format!("OpenRouter embedding request failed: {}", e))?;

    if !response.status().is_success() {
        let err_text = response.text().await.unwrap_or_default();
        return Err(format!("OpenRouter embedding error: {}", err_text));
    }

    let result: serde_json::Value = response.json().await
        .map_err(|e| format!("Failed to parse embedding response: {}", e))?;

    // OpenRouter returns standard OpenAI-format: data[0].embedding as float array
    let embedding_val = &result["data"][0]["embedding"];

    if let Some(arr) = embedding_val.as_array() {
        // Float array response
        let raw: Vec<f32> = arr
            .iter()
            .map(|v| v.as_f64().unwrap_or(0.0) as f32)
            .collect();

        if raw.len() != EMBED_DIM {
            return Err(format!(
                "Expected {} dimensions, got {}", EMBED_DIM, raw.len()
            ));
        }

        // L2-normalize
        let norm: f32 = raw.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm < 1e-9 {
            return Ok(raw);
        }
        Ok(raw.iter().map(|x| x / norm).collect())
    } else if let Some(b64) = embedding_val.as_str() {
        // Base64-encoded int8 fallback
        let bytes = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD, b64
        ).map_err(|e| format!("Base64 decode failed: {}", e))?;

        if bytes.len() != EMBED_DIM {
            return Err(format!(
                "Expected {} dimensions, got {} bytes", EMBED_DIM, bytes.len()
            ));
        }

        let raw: Vec<f32> = bytes.iter().map(|&b| b as i8 as f32).collect();
        let norm: f32 = raw.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm < 1e-9 {
            return Ok(raw);
        }
        Ok(raw.iter().map(|x| x / norm).collect())
    } else {
        Err("Unexpected embedding format in response".to_string())
    }
}

/// Format embedding as pgvector literal string: [0.1,0.2,...]
fn vector_str(embedding: &[f32]) -> String {
    let parts: Vec<String> = embedding.iter().map(|v| format!("{}", v)).collect();
    format!("[{}]", parts.join(","))
}

// ─── Text chunking ──────────────────────────────────────────────────────────

fn chunk_text(text: &str, max_words: usize) -> Vec<String> {
    let paragraphs: Vec<&str> = text
        .split("\n\n")
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .collect();

    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();

    for para in &paragraphs {
        let current_words = current.split_whitespace().count();
        let para_words = para.split_whitespace().count();

        if current_words + para_words > max_words {
            let trimmed = current.trim().to_string();
            if !trimmed.is_empty() {
                chunks.push(trimmed);
            }
            if para_words > max_words {
                let words: Vec<&str> = para.split_whitespace().collect();
                for chunk_start in (0..words.len()).step_by(max_words) {
                    let end = (chunk_start + max_words).min(words.len());
                    chunks.push(words[chunk_start..end].join(" "));
                }
                current = String::new();
            } else {
                current = para.to_string();
            }
        } else {
            if current.is_empty() {
                current = para.to_string();
            } else {
                current.push_str("\n\n");
                current.push_str(para);
            }
        }
    }

    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        chunks.push(trimmed);
    }

    chunks.into_iter().filter(|c| c.len() > 20).collect()
}

/// Strip null bytes and ASCII control chars that Postgres rejects (keep \t \n \r).
fn sanitize_text(text: &str) -> String {
    text.chars()
        .filter(|&c| c == '\t' || c == '\n' || c == '\r' || (c as u32) > 0x1F)
        .collect()
}

// ─── Store chunks + embeddings ──────────────────────────────────────────────

async fn store_chunks_and_embeddings(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    client: &reqwest::Client,
    resource_id: &str,
    text: &str,
) -> Result<usize, String> {
    let chunks = chunk_text(text, 400);
    println!("  → {} chunks to embed", chunks.len());

    for (i, raw_chunk) in chunks.iter().enumerate() {
        let chunk = sanitize_text(raw_chunk);
        let chunk_id = uuid::Uuid::new_v4().to_string();

        sqlx::query(
            "INSERT INTO mimir_chunks (id, resource_id, content, chunk_index) VALUES ($1, $2, $3, $4)"
        )
        .bind(&chunk_id)
        .bind(resource_id)
        .bind(&chunk)
        .bind(i as i32)
        .execute(&mut **tx)
        .await
        .map_err(|e| format!("Failed to insert chunk: {}", e))?;

        let embedding = get_embedding(client, &chunk).await?;
        let emb_id = uuid::Uuid::new_v4().to_string();
        let vec_str = vector_str(&embedding);

        sqlx::query(
            "INSERT INTO mimir_embeddings (id, chunk_id, embedding) VALUES ($1, $2, $3::vector)"
        )
        .bind(&emb_id)
        .bind(&chunk_id)
        .bind(&vec_str)
        .execute(&mut **tx)
        .await
        .map_err(|e| format!("Failed to insert embedding: {}", e))?;
    }

    Ok(chunks.len())
}

// ─── Section-aware chunking for PDFs ────────────────────────────────────────

/// Chunk a section's blocks into ≤400-word chunks with 50-word overlap,
/// preserving block-level page provenance per chunk.
async fn store_sections_and_embeddings(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    client: &reqwest::Client,
    resource_id: &str,
    sections: &serde_json::Value,
) -> Result<usize, String> {
    let sections_arr = sections.as_array()
        .ok_or_else(|| "sections_json is not an array".to_string())?;

    let mut chunk_index = 0i32;
    let mut total_chunks = 0usize;
    const MAX_WORDS: usize = 400;
    const OVERLAP_WORDS: usize = 50;

    for sec in sections_arr {
        let section_title = sec["title"].as_str().unwrap_or("").to_string();
        let blocks = match sec["blocks"].as_array() {
            Some(b) => b,
            None => continue,
        };

        // Accumulate words from blocks, tracking page provenance per word position
        let mut words: Vec<(String, i32)> = Vec::new(); // (word, page)
        for block in blocks {
            let text = block["text"].as_str().unwrap_or("");
            let page = block["page"].as_i64().unwrap_or(1) as i32;
            for word in text.split_whitespace() {
                words.push((word.to_string(), page));
            }
        }

        if words.is_empty() {
            continue;
        }

        // Slide window over words
        let mut start = 0usize;
        while start < words.len() {
            let end = (start + MAX_WORDS).min(words.len());
            let chunk_words = &words[start..end];
            let chunk_text = sanitize_text(&chunk_words.iter().map(|(w, _)| w.as_str()).collect::<Vec<_>>().join(" "));

            if chunk_text.len() > 20 {
                let page_start = chunk_words.first().map(|(_, p)| *p);
                let page_end = chunk_words.last().map(|(_, p)| *p);
                let chunk_id = uuid::Uuid::new_v4().to_string();

                sqlx::query(
                    "INSERT INTO mimir_chunks \
                     (id, resource_id, content, chunk_index, section_title, page_start, page_end) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7)"
                )
                .bind(&chunk_id)
                .bind(resource_id)
                .bind(&chunk_text)
                .bind(chunk_index)
                .bind(if section_title.is_empty() { None } else { Some(&section_title) })
                .bind(page_start)
                .bind(page_end)
                .execute(&mut **tx)
                .await
                .map_err(|e| format!("Failed to insert chunk: {}", e))?;

                let embedding = get_embedding(client, &chunk_text).await?;
                let emb_id = uuid::Uuid::new_v4().to_string();
                let vec_str = vector_str(&embedding);

                sqlx::query(
                    "INSERT INTO mimir_embeddings (id, chunk_id, embedding) VALUES ($1, $2, $3::vector)"
                )
                .bind(&emb_id)
                .bind(&chunk_id)
                .bind(&vec_str)
                .execute(&mut **tx)
                .await
                .map_err(|e| format!("Failed to insert embedding: {}", e))?;

                chunk_index += 1;
                total_chunks += 1;
            }

            if end >= words.len() { break; }
            start = end.saturating_sub(OVERLAP_WORDS);
        }
    }

    Ok(total_chunks)
}

// ─── URL fetching ───────────────────────────────────────────────────────────

#[derive(Debug)]
struct FetchResult {
    text: String,
    page_type: String,
    external_links: Vec<ScrapedExternalLink>,
}

/// Fetch URL via Python scraper (port 3002), fall back to reqwest + scraper crate.
async fn fetch_url_content(url: &str, force_dynamic: bool) -> Result<FetchResult, String> {
    let client = reqwest::Client::new();

    // Try Python scraper first
    match client
        .post(format!("{}/fetch", SCRAPER_URL))
        .json(&json!({ "url": url, "force_dynamic": force_dynamic }))
        .timeout(std::time::Duration::from_secs(90))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            let data: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
            let text = data["text"].as_str().unwrap_or("").to_string();
            let page_type = data["page_type"].as_str().unwrap_or("article").to_string();
            let external_links = data["external_links"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .map(|l| ScrapedExternalLink {
                            url: l["url"].as_str().unwrap_or("").to_string(),
                            text: l["text"].as_str().unwrap_or("").to_string(),
                            link_type: l["type"].as_str().unwrap_or("").to_string(),
                        })
                        .collect()
                })
                .unwrap_or_default();

            println!(
                "[scraper] {} → {} chars, type={}",
                data["fetcher_used"].as_str().unwrap_or("unknown"),
                text.len(),
                page_type
            );
            return Ok(FetchResult { text, page_type, external_links });
        }
        Ok(resp) => {
            let err: serde_json::Value = resp.json().await.unwrap_or_default();
            let detail = err["detail"].as_str().unwrap_or("Scraper error");
            return Err(detail.to_string());
        }
        Err(e) => {
            // Connection refused → scraper not running, fall back to reqwest + scraper crate
            let err_str = e.to_string();
            if err_str.contains("connect") || err_str.contains("refused") || err_str.contains("tcp") {
                println!("[scraper] Service unavailable — falling back to reqwest + scraper crate");
            } else {
                return Err(format!("Scraper request failed: {}", e));
            }
        }
    }

    // Fallback: reqwest + scraper crate (like cheerio)
    let resp = client
        .get(url)
        .header("User-Agent", "Yggdrasil/1.0 (+https://github.com/yggdrasil-app)")
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| format!("Failed to fetch URL: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }

    let html = resp.text().await.map_err(|e| e.to_string())?;
    let text = clean_html(&html);

    Ok(FetchResult {
        text,
        page_type: "article".to_string(),
        external_links: vec![],
    })
}

/// Clean HTML using scraper crate (equivalent to cheerio cleanup).
fn clean_html(html: &str) -> String {
    use scraper::{Html, Selector};

    let document = Html::parse_document(html);

    // Selectors for elements to remove
    let remove_selectors = [
        "script", "style", "nav", "header", "footer", "aside",
        ".nav", ".menu", ".sidebar",
        "[class*=\"cookie\"]", "[class*=\"banner\"]",
        "[class*=\"Cookie\"]", "[class*=\"Banner\"]",
    ];

    let body_sel = Selector::parse("body").unwrap();
    let remove_sels: Vec<Selector> = remove_selectors
        .iter()
        .filter_map(|s| Selector::parse(s).ok())
        .collect();

    let mut text = String::new();

    if let Some(body) = document.select(&body_sel).next() {
        // Collect ego_tree NodeIds of elements to skip
        let mut skip_ids: std::collections::HashSet<ego_tree::NodeId> =
            std::collections::HashSet::new();
        for sel in &remove_sels {
            for el in body.select(sel) {
                skip_ids.insert(el.id());
                for desc in el.descendants() {
                    skip_ids.insert(desc.id());
                }
            }
        }

        // Walk text nodes, skipping removed subtrees
        for desc in body.descendants() {
            if skip_ids.contains(&desc.id()) {
                continue;
            }
            if let Some(t) = desc.value().as_text() {
                text.push_str(t);
                text.push(' ');
            }
        }
    }

    // Normalize whitespace
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Fetch just the page title from a URL.
async fn fetch_page_title(url: &str) -> String {
    let client = reqwest::Client::new();
    match client
        .get(url)
        .header("User-Agent", "Yggdrasil/1.0")
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            if let Ok(html) = resp.text().await {
                use scraper::{Html, Selector};
                let doc = Html::parse_document(&html);
                if let Some(title_el) = Selector::parse("title").ok().and_then(|s| doc.select(&s).next()) {
                    let t = title_el.text().collect::<String>().trim().to_string();
                    if !t.is_empty() {
                        return t;
                    }
                }
                if let Some(h1_el) = Selector::parse("h1").ok().and_then(|s| doc.select(&s).next()) {
                    let t = h1_el.text().collect::<String>().trim().to_string();
                    if !t.is_empty() {
                        return t;
                    }
                }
            }
            url.to_string()
        }
        _ => url.to_string(),
    }
}

// ─── Tauri commands ─────────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_mimir_resources(
    database: State<'_, Database>,
) -> Result<Vec<MimirResource>, String> {
    let rows = sqlx::query(
        "SELECT mr.id, mr.title, mr.url, mr.type, mr.status, mr.user_notes, \
                mr.created_at::text, mr.parent_id, mr.tags, mr.is_completed, \
                COALESCE(nc.node_count, 0)::int AS node_count \
         FROM mimir_resources mr \
         LEFT JOIN ( \
           SELECT resource_id, COUNT(DISTINCT node_id)::int AS node_count \
           FROM mimir_node_links \
           GROUP BY resource_id \
         ) nc ON nc.resource_id = mr.id \
         ORDER BY mr.created_at DESC"
    )
    .fetch_all(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut resources = Vec::new();
    for row in &rows {
        resources.push(MimirResource {
            id: row.try_get("id").unwrap_or_default(),
            title: row.try_get("title").unwrap_or_default(),
            url: row.try_get("url").ok(),
            resource_type: row.try_get("type").unwrap_or_default(),
            status: row.try_get("status").unwrap_or_default(),
            user_notes: row.try_get("user_notes").ok(),
            created_at: row.try_get("created_at").unwrap_or_default(),
            parent_id: row.try_get("parent_id").ok(),
            tags: row.try_get::<Vec<String>, _>("tags").unwrap_or_default(),
            is_completed: row.try_get("is_completed").unwrap_or(false),
            node_count: row.try_get("node_count").unwrap_or(0),
            relevance_score: None,
            matched_section_title: None,
            matched_page_start: None,
            matched_page_end: None,
        });
    }

    Ok(resources)
}

#[tauri::command]
pub async fn get_node_resources(
    node_id: String,
    database: State<'_, Database>,
) -> Result<Vec<MimirResource>, String> {
    let rows = sqlx::query(
        "SELECT mr.id, mr.title, mr.url, mr.type, mr.status, mr.user_notes, \
                mr.created_at::text, mr.parent_id, mr.tags, mr.is_completed, \
                mnl.relevance_score, mnl.matched_section_title, \
                mnl.matched_page_start, mnl.matched_page_end \
         FROM mimir_resources mr \
         JOIN mimir_node_links mnl ON mnl.resource_id = mr.id \
         WHERE mnl.node_id = $1 \
         ORDER BY mnl.relevance_score ASC NULLS LAST"
    )
    .bind(&node_id)
    .fetch_all(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut resources = Vec::new();
    for row in &rows {
        resources.push(MimirResource {
            id: row.try_get("id").unwrap_or_default(),
            title: row.try_get("title").unwrap_or_default(),
            url: row.try_get("url").ok(),
            resource_type: row.try_get("type").unwrap_or_default(),
            status: row.try_get("status").unwrap_or_default(),
            user_notes: row.try_get("user_notes").ok(),
            created_at: row.try_get("created_at").unwrap_or_default(),
            parent_id: row.try_get("parent_id").ok(),
            tags: row.try_get::<Vec<String>, _>("tags").unwrap_or_default(),
            is_completed: row.try_get("is_completed").unwrap_or(false),
            node_count: 0,
            relevance_score: row.try_get("relevance_score").ok(),
            matched_section_title: row.try_get("matched_section_title").ok().flatten(),
            matched_page_start: row.try_get("matched_page_start").ok().flatten(),
            matched_page_end: row.try_get("matched_page_end").ok().flatten(),
        });
    }

    Ok(resources)
}

#[tauri::command]
pub async fn delete_mimir_resource(
    resource_id: String,
    database: State<'_, Database>,
) -> Result<(), String> {
    // CASCADE handles chunks + embeddings + node_links
    let result = sqlx::query("DELETE FROM mimir_resources WHERE id = $1")
        .bind(&resource_id)
        .execute(&database.pool)
        .await
        .map_err(|e| e.to_string())?;

    if result.rows_affected() == 0 {
        return Err("Resource not found".to_string());
    }
    Ok(())
}

// ─── Ingest commands ────────────────────────────────────────────────────────

#[tauri::command]
pub async fn ingest_mimir_url(
    url: String,
    title: Option<String>,
    force_dynamic: Option<bool>,
    parent_id: Option<String>,
    database: State<'_, Database>,
) -> Result<IngestResult, String> {
    // Duplicate check by URL
    let dup = sqlx::query("SELECT id, title FROM mimir_resources WHERE url = $1")
        .bind(&url)
        .fetch_optional(&database.pool)
        .await
        .map_err(|e| e.to_string())?;

    if let Some(row) = dup {
        let existing_title: String = row.try_get("title").unwrap_or_default();
        return Err(format!("DUPLICATE:{}", existing_title));
    }

    println!("📥 Ingesting URL: {}", url);

    let fetch_result = fetch_url_content(&url, force_dynamic.unwrap_or(false)).await?;

    if fetch_result.text.len() < 50 {
        return Err("Could not extract meaningful text from URL".to_string());
    }

    // Derive title
    let page_title = match title.as_ref().filter(|t| !t.trim().is_empty()) {
        Some(t) => t.trim().to_string(),
        None => fetch_page_title(&url).await,
    };

    println!("  title: \"{}\"", page_title);

    let resource_id = uuid::Uuid::new_v4().to_string();
    let client = reqwest::Client::new();

    let mut tx = database.pool.begin().await.map_err(|e| e.to_string())?;

    if let Some(ref pid) = parent_id {
        sqlx::query(
            "INSERT INTO mimir_resources (id, title, url, type, status, parent_id) \
             VALUES ($1, $2, $3, $4, $5, $6)"
        )
        .bind(&resource_id)
        .bind(&page_title)
        .bind(&url)
        .bind("webpage")
        .bind("read")
        .bind(pid)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    } else {
        sqlx::query(
            "INSERT INTO mimir_resources (id, title, url, type, status) \
             VALUES ($1, $2, $3, $4, $5)"
        )
        .bind(&resource_id)
        .bind(&page_title)
        .bind(&url)
        .bind("webpage")
        .bind("read")
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    }

    store_chunks_and_embeddings(&mut tx, &client, &resource_id, &fetch_result.text).await?;

    tx.commit().await.map_err(|e| e.to_string())?;

    println!("✅ Ingested URL: \"{}\" ({})", page_title, resource_id);
    Ok(IngestResult {
        id: resource_id,
        title: page_title,
        page_type: fetch_result.page_type,
        external_links: fetch_result.external_links,
    })
}

#[tauri::command]
pub async fn ingest_mimir_text(
    text: String,
    title: Option<String>,
    database: State<'_, Database>,
) -> Result<String, String> {
    let resource_title = title
        .as_ref()
        .filter(|t| !t.trim().is_empty())
        .map(|t| t.trim().to_string())
        .unwrap_or_else(|| "Pasted text".to_string());

    // SHA256 dedup
    let mut hasher = Sha256::new();
    hasher.update(text.trim().as_bytes());
    let content_hash = hex::encode(hasher.finalize());

    let dup = sqlx::query("SELECT id, title FROM mimir_resources WHERE content_hash = $1")
        .bind(&content_hash)
        .fetch_optional(&database.pool)
        .await
        .map_err(|e| e.to_string())?;

    if let Some(row) = dup {
        let existing_title: String = row.try_get("title").unwrap_or_default();
        return Err(format!("DUPLICATE:{}", existing_title));
    }

    println!("📥 Ingesting text: \"{}\" ({} chars)", resource_title, text.len());

    let resource_id = uuid::Uuid::new_v4().to_string();
    let client = reqwest::Client::new();
    let mut tx = database.pool.begin().await.map_err(|e| e.to_string())?;

    sqlx::query(
        "INSERT INTO mimir_resources (id, title, url, type, status, content_hash) \
         VALUES ($1, $2, NULL, $3, $4, $5)"
    )
    .bind(&resource_id)
    .bind(&resource_title)
    .bind("text")
    .bind("read")
    .bind(&content_hash)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    store_chunks_and_embeddings(&mut tx, &client, &resource_id, &text).await?;

    tx.commit().await.map_err(|e| e.to_string())?;

    println!("✅ Ingested text: \"{}\" ({})", resource_title, resource_id);
    Ok(resource_id)
}

#[tauri::command]
pub async fn ingest_mimir_pdf(
    filename: String,
    pdf_base64: String,
    database: State<'_, Database>,
) -> Result<PdfIngestResult, String> {
    let bytes = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD, &pdf_base64
    ).map_err(|e| format!("Invalid base64: {}", e))?;

    // SHA256 dedup on raw bytes
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let content_hash = hex::encode(hasher.finalize());

    let dup = sqlx::query("SELECT id, title FROM mimir_resources WHERE content_hash = $1")
        .bind(&content_hash)
        .fetch_optional(&database.pool)
        .await
        .map_err(|e| e.to_string())?;

    if let Some(row) = dup {
        let existing_title: String = row.try_get("title").unwrap_or_default();
        return Err(format!("DUPLICATE:{}", existing_title));
    }

    let resource_title = if filename.is_empty() {
        "Uploaded PDF".to_string()
    } else {
        filename.trim_end_matches(".pdf").trim_end_matches(".PDF").trim().to_string()
    };

    // Extract text via Python scraper (pymupdf) — better layout handling than pdf-extract
    let client = reqwest::Client::new();
    let scraper_resp = client
        .post(format!("{}/fetch-pdf", SCRAPER_URL))
        .json(&json!({ "pdf_base64": pdf_base64, "filename": filename }))
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await
        .map_err(|e| format!("Scraper request failed: {}", e))?;

    if !scraper_resp.status().is_success() {
        let err: serde_json::Value = scraper_resp.json().await.unwrap_or_default();
        return Err(err["detail"].as_str().unwrap_or("PDF extraction failed").to_string());
    }

    let pdf_result: serde_json::Value = scraper_resp.json().await
        .map_err(|e| format!("Failed to parse scraper response: {}", e))?;

    let text = pdf_result["text"].as_str().unwrap_or("").to_string();
    let pages = pdf_result["pages"].as_i64().unwrap_or(1) as i32;
    let sections = &pdf_result["sections"];
    let section_count = sections.as_array().map(|a| a.len()).unwrap_or(0);

    if text.len() < 50 {
        return Err("Could not extract meaningful text from PDF".to_string());
    }

    println!(
        "📥 Ingesting PDF: \"{}\" ({} chars, {} pages, {} sections)",
        resource_title, text.len(), pages, section_count
    );

    let resource_id = uuid::Uuid::new_v4().to_string();
    let sections_json = serde_json::to_value(sections).unwrap_or(serde_json::Value::Null);
    let mut tx = database.pool.begin().await.map_err(|e| e.to_string())?;

    sqlx::query(
        "INSERT INTO mimir_resources \
         (id, title, url, type, status, content_hash, raw_text, sections_json) \
         VALUES ($1, $2, NULL, $3, $4, $5, $6, $7)"
    )
    .bind(&resource_id)
    .bind(&resource_title)
    .bind("pdf")
    .bind("read")
    .bind(&content_hash)
    .bind(&text)
    .bind(&sections_json)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    // Use section-aware chunking if we have real structure, else flat chunking
    if section_count > 1 {
        store_sections_and_embeddings(&mut tx, &client, &resource_id, sections).await?;
    } else {
        store_chunks_and_embeddings(&mut tx, &client, &resource_id, &text).await?;
    }

    tx.commit().await.map_err(|e| e.to_string())?;

    let text_preview = text.chars().take(500).collect::<String>();
    println!("✅ Ingested PDF: \"{}\" ({})", resource_title, resource_id);
    Ok(PdfIngestResult {
        id: resource_id,
        title: resource_title,
        text_preview: Some(text_preview),
    })
}

#[tauri::command]
pub async fn extract_pdf_text(pdf_base64: String) -> Result<PdfTextResult, String> {
    // Use Python scraper (pymupdf) for better layout handling — same as ingest_mimir_pdf
    let client = reqwest::Client::new();
    let scraper_resp = client
        .post(format!("{}/fetch-pdf", SCRAPER_URL))
        .json(&json!({ "pdf_base64": pdf_base64, "filename": "" }))
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await
        .map_err(|e| format!("Scraper request failed: {}", e))?;

    if !scraper_resp.status().is_success() {
        let err: serde_json::Value = scraper_resp.json().await.unwrap_or_default();
        return Err(err["detail"].as_str().unwrap_or("PDF extraction failed").to_string());
    }

    let result: serde_json::Value = scraper_resp.json().await
        .map_err(|e| format!("Failed to parse scraper response: {}", e))?;

    let text = result["text"].as_str().unwrap_or("").to_string();
    let pages = result["pages"].as_i64().unwrap_or(1) as i32;

    if text.len() < 50 {
        return Err("Could not extract meaningful text from PDF".to_string());
    }

    println!("📄 Extracted PDF text: {} chars, {} pages", text.len(), pages);
    Ok(PdfTextResult {
        chars: text.len() as i32,
        text,
        pages,
    })
}

// ─── Match node to resources ────────────────────────────────────────────────

pub async fn match_node_impl(
    pool: &sqlx::PgPool,
    client: &reqwest::Client,
    node_id: &str,
) -> Result<Vec<MimirResource>, String> {
    // Fetch node title + description
    let node_row = sqlx::query("SELECT title, description FROM tree_nodes WHERE id = $1")
        .bind(node_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?;

    let node_row = match node_row {
        Some(r) => r,
        None => return Ok(vec![]),
    };

    let title: String = node_row.try_get("title").unwrap_or_default();
    let description: Option<String> = node_row.try_get("description").ok();
    let search_text = match description.filter(|d| !d.is_empty()) {
        Some(desc) => format!("{}: {}", title, desc),
        None => title.clone(),
    };

    // Check if any embeddings exist
    let count_row = sqlx::query("SELECT COUNT(*) AS n FROM mimir_embeddings")
        .fetch_one(pool)
        .await
        .map_err(|e| e.to_string())?;
    let count: i64 = count_row.try_get("n").unwrap_or(0);
    if count == 0 {
        return Ok(vec![]);
    }

    // Embed and search
    let embedding = get_embedding(client, &search_text).await?;
    let vec_str = vector_str(&embedding);

    // Per-resource: find the single best chunk (lowest distance) and carry its metadata
    let similar = sqlx::query(
        "SELECT DISTINCT ON (mc.resource_id) \
                mc.resource_id, mc.id AS chunk_id, \
                mc.section_title, mc.page_start, mc.page_end, \
                (me.embedding <=> $1::vector) AS distance \
         FROM mimir_embeddings me \
         JOIN mimir_chunks mc ON mc.id = me.chunk_id \
         ORDER BY mc.resource_id, distance ASC"
    )
    .bind(&vec_str)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    // Filter by threshold — same 0.55 cutoff, take top 10
    struct ChunkMatch {
        resource_id: String,
        chunk_id: String,
        section_title: Option<String>,
        page_start: Option<i32>,
        page_end: Option<i32>,
        distance: f64,
    }
    let mut matches: Vec<ChunkMatch> = similar
        .iter()
        .filter_map(|row| {
            let rid: String = row.try_get("resource_id").ok()?;
            let dist: f64 = row.try_get("distance").ok()?;
            if dist >= 0.55 { return None; }
            Some(ChunkMatch {
                resource_id: rid,
                chunk_id: row.try_get("chunk_id").unwrap_or_default(),
                section_title: row.try_get("section_title").ok().flatten(),
                page_start: row.try_get("page_start").ok().flatten(),
                page_end: row.try_get("page_end").ok().flatten(),
                distance: dist,
            })
        })
        .collect();
    matches.sort_by(|a, b| a.distance.partial_cmp(&b.distance).unwrap_or(std::cmp::Ordering::Equal));
    matches.truncate(10);

    // Upsert node links with chunk metadata
    for m in &matches {
        let link_id = uuid::Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO mimir_node_links \
               (id, resource_id, node_id, relevance_score, matched_chunk_id, matched_section_title, matched_page_start, matched_page_end) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
             ON CONFLICT (resource_id, node_id) DO UPDATE SET \
               relevance_score = EXCLUDED.relevance_score, \
               matched_chunk_id = EXCLUDED.matched_chunk_id, \
               matched_section_title = EXCLUDED.matched_section_title, \
               matched_page_start = EXCLUDED.matched_page_start, \
               matched_page_end = EXCLUDED.matched_page_end"
        )
        .bind(&link_id)
        .bind(&m.resource_id)
        .bind(node_id)
        .bind(m.distance as f32)
        .bind(&m.chunk_id)
        .bind(&m.section_title)
        .bind(m.page_start)
        .bind(m.page_end)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    }

    if matches.is_empty() {
        return Ok(vec![]);
    }

    // Return matched resources with chunk metadata from the link row
    let resource_ids: Vec<String> = matches.iter().map(|m| m.resource_id.clone()).collect();
    let rows = sqlx::query(
        "SELECT mr.id, mr.title, mr.url, mr.type, mr.status, mr.created_at::text, \
                mr.tags, mr.is_completed, \
                mnl.relevance_score, mnl.matched_section_title, mnl.matched_page_start, mnl.matched_page_end \
         FROM mimir_resources mr \
         JOIN mimir_node_links mnl ON mnl.resource_id = mr.id AND mnl.node_id = $2 \
         WHERE mr.id = ANY($1::text[]) \
         ORDER BY mnl.relevance_score ASC NULLS LAST"
    )
    .bind(&resource_ids)
    .bind(node_id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let resources: Vec<MimirResource> = rows
        .iter()
        .map(|row| MimirResource {
            id: row.try_get("id").unwrap_or_default(),
            title: row.try_get("title").unwrap_or_default(),
            url: row.try_get("url").ok(),
            resource_type: row.try_get("type").unwrap_or_default(),
            status: row.try_get("status").unwrap_or_default(),
            user_notes: None,
            created_at: row.try_get("created_at").unwrap_or_default(),
            parent_id: None,
            tags: row.try_get::<Vec<String>, _>("tags").unwrap_or_default(),
            is_completed: row.try_get("is_completed").unwrap_or(false),
            node_count: 0,
            relevance_score: row.try_get("relevance_score").ok(),
            matched_section_title: row.try_get("matched_section_title").ok().flatten(),
            matched_page_start: row.try_get("matched_page_start").ok().flatten(),
            matched_page_end: row.try_get("matched_page_end").ok().flatten(),
        })
        .collect();

    println!("🔗 Matched {} resources for node \"{}\"", resources.len(), title);
    Ok(resources)
}

#[tauri::command]
pub async fn match_node_to_resources(
    node_id: String,
    database: State<'_, Database>,
) -> Result<Vec<MimirResource>, String> {
    let client = reqwest::Client::new();
    match_node_impl(&database.pool, &client, &node_id).await
}

// ─── Chat (RAG) ─────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn mimir_chat(
    message: String,
    page: String,
    tree_id: Option<String>,
    node_title: Option<String>,
    database: State<'_, Database>,
) -> Result<MimirChatResponse, String> {
    let api_key = groq_api_key()?;
    let _ = page; // available for future per-page behavior
    let client = reqwest::Client::new();

    // 1. Embed the query (expand with node context)
    let query_text = match &node_title {
        Some(nt) if !nt.is_empty() => format!("{} [context: {}]", message, nt),
        _ => message.clone(),
    };
    let embedding = get_embedding(&client, &query_text).await?;
    let vec_str = vector_str(&embedding);

    // 2. Check for any embeddings
    let count_row = sqlx::query("SELECT COUNT(*) AS n FROM mimir_embeddings")
        .fetch_one(&database.pool)
        .await
        .map_err(|e| e.to_string())?;
    let emb_count: i64 = count_row.try_get("n").unwrap_or(0);

    let mut sources: Vec<MimirChatSource> = Vec::new();
    let mut context_blocks = String::new();

    if emb_count > 0 {
        // 3. pgvector cosine search — top 10, threshold < 0.85
        let rows = sqlx::query(
            "SELECT mc.content, mc.resource_id, mr.title, mr.url, \
                    mc.section_title, mc.page_start, mc.page_end, \
                    (me.embedding <=> $1::vector) AS distance \
             FROM mimir_embeddings me \
             JOIN mimir_chunks mc ON mc.id = me.chunk_id \
             JOIN mimir_resources mr ON mr.id = mc.resource_id \
             WHERE (me.embedding <=> $1::vector) < 0.85 \
             ORDER BY distance ASC \
             LIMIT 10"
        )
        .bind(&vec_str)
        .fetch_all(&database.pool)
        .await
        .map_err(|e| e.to_string())?;

        // Build candidate list
        struct Candidate {
            content: String,
            title: String,
            url: Option<String>,
            distance: f64,
            section_title: Option<String>,
            page_start: Option<i32>,
            page_end: Option<i32>,
        }
        let candidates: Vec<Candidate> = rows
            .iter()
            .map(|r| Candidate {
                content: r.try_get("content").unwrap_or_default(),
                title: r.try_get("title").unwrap_or_default(),
                url: r.try_get("url").ok(),
                distance: r.try_get("distance").unwrap_or(1.0),
                section_title: r.try_get("section_title").ok().flatten(),
                page_start: r.try_get("page_start").ok().flatten(),
                page_end: r.try_get("page_end").ok().flatten(),
            })
            .collect();

        // 4. LLM rerank to top 3
        let ranked = if candidates.len() > 3 {
            let rerank_chunks: String = candidates
                .iter()
                .enumerate()
                .map(|(i, c)| format!("[{}] {}", i, &c.content[..c.content.len().min(200)]))
                .collect::<Vec<_>>()
                .join("\n");

            let rerank_resp = client
                .post("https://api.groq.com/openai/v1/chat/completions")
                .header("Authorization", format!("Bearer {}", api_key))
                .header("Content-Type", "application/json")
                .json(&json!({
                    "model": "llama-3.1-8b-instant",
                    "messages": [
                        {
                            "role": "system",
                            "content": "You are a relevance filter. Given a question and a list of numbered text chunks, respond with ONLY a JSON array of the indices (0-based) of the 3 most relevant chunks. Example: [0, 3, 7]"
                        },
                        {
                            "role": "user",
                            "content": format!("Question: {}\n\nChunks:\n{}", message, rerank_chunks)
                        }
                    ],
                    "temperature": 0,
                    "max_tokens": 200
                }))
                .timeout(std::time::Duration::from_secs(15))
                .send()
                .await;

            match rerank_resp {
                Ok(resp) if resp.status().is_success() => {
                    let body: serde_json::Value = resp.json().await.unwrap_or_default();
                    let raw = body["choices"][0]["message"]["content"].as_str().unwrap_or("");
                    // Extract JSON array from response
                    if let Some(start) = raw.find('[') {
                        if let Some(end) = raw[start..].find(']') {
                            let arr_str = &raw[start..=start + end];
                            if let Ok(indices) = serde_json::from_str::<Vec<usize>>(arr_str) {
                                let picked: Vec<usize> = indices
                                    .into_iter()
                                    .filter(|&i| i < candidates.len())
                                    .collect();
                                if !picked.is_empty() {
                                    println!("  ↳ reranker selected indices: {:?}", picked);
                                    picked
                                } else {
                                    vec![0, 1, 2]
                                }
                            } else {
                                vec![0, 1, 2]
                            }
                        } else {
                            vec![0, 1, 2]
                        }
                    } else {
                        vec![0, 1, 2]
                    }
                }
                _ => {
                    println!("  ⚠️  Reranking failed, using top 3 by distance");
                    vec![0, 1, 2]
                }
            }
        } else {
            (0..candidates.len()).collect()
        };

        // Build sources and context
        for (rank, &idx) in ranked.iter().enumerate() {
            if idx >= candidates.len() { continue; }
            let c = &candidates[idx];
            sources.push(MimirChatSource {
                title: c.title.clone(),
                url: c.url.clone(),
                chunk: c.content.chars().take(300).collect(),
                score: ((1.0 - c.distance) * 1000.0).round() as f32 / 1000.0,
                section_title: c.section_title.clone(),
                page_start: c.page_start,
                page_end: c.page_end,
            });
            context_blocks.push_str(&format!(
                "[{}] {}\n— Source: {}\n\n",
                rank + 1,
                c.content,
                c.title
            ));
        }
    }

    // 5. Tree structure context
    let mut tree_block = String::new();
    if let Some(ref tid) = tree_id {
        let tree_rows = sqlx::query(
            "SELECT tn.id, tn.title, tn.parent_id, \
                    mr.title as resource_title, mr.url, mr.type \
             FROM tree_nodes tn \
             LEFT JOIN mimir_node_links mnl ON mnl.node_id = tn.id \
             LEFT JOIN mimir_resources mr ON mr.id = mnl.resource_id \
             WHERE tn.tree_id = $1 \
             ORDER BY tn.order_index ASC"
        )
        .bind(tid)
        .fetch_all(&database.pool)
        .await
        .unwrap_or_default();

        if !tree_rows.is_empty() {
            let mut node_map: std::collections::BTreeMap<
                String,
                (String, Option<String>, Vec<String>),
            > = std::collections::BTreeMap::new();

            for row in &tree_rows {
                let id: String = row.try_get("id").unwrap_or_default();
                let title: String = row.try_get("title").unwrap_or_default();
                let parent_id: Option<String> = row.try_get("parent_id").ok();
                let res_title: Option<String> = row.try_get("resource_title").ok();
                let res_url: Option<String> = row.try_get("url").ok();
                let res_type: Option<String> = row.try_get("type").ok();

                let entry = node_map
                    .entry(id.clone())
                    .or_insert_with(|| (title, parent_id, Vec::new()));

                if let Some(rt) = res_title {
                    let mut label = rt;
                    if let Some(t) = res_type {
                        label = format!("{} ({})", label, t);
                    }
                    if let Some(u) = res_url {
                        label = format!("{} — {}", label, u);
                    }
                    entry.2.push(label);
                }
            }

            let mut lines: Vec<String> = Vec::new();
            for (id, (title, parent_id, resources)) in &node_map {
                let indent = if parent_id.is_some() { "  " } else { "" };
                lines.push(format!("{}- {} [id={}]", indent, title, id));
                for r in resources {
                    lines.push(format!("{}    ↳ {}", indent, r));
                }
            }

            tree_block = format!("\n\nLearning Tree Structure:\n{}", lines.join("\n"));
            println!("  ↳ injected tree structure: {} nodes", node_map.len());
        }
    }

    // 6. Build system prompt
    let mut system_prompt = String::from(
        "You are Mimir, a learning assistant embedded in Yggdrasil skill tree app. \
         Answer based on the provided context chunks. If the context doesn't contain enough \
         information, say so clearly and suggest what kind of resource would help. \
         Always cite which chunk(s) you used."
    );

    if let Some(ref nt) = node_title {
        if !nt.is_empty() {
            system_prompt.push_str(&format!(
                " The user is currently on quest node: \"{}\". Bias your answer toward how it relates to that specific quest.",
                nt
            ));
        }
    }

    if !tree_block.is_empty() {
        system_prompt.push_str(
            " You have access to the user's learning tree and linked resources. \
             When asked about sequencing, recommend order based on: prerequisite depth \
             (shallower nodes first), resource type (short videos before dense docs), \
             topological order of the tree."
        );
        system_prompt.push_str(&tree_block);
    }

    system_prompt.push_str("\n\nContext:\n");
    if context_blocks.is_empty() {
        system_prompt.push_str("(No relevant resources found in your library.)");
    } else {
        system_prompt.push_str(&context_blocks);
    }

    // 7. Call Groq for synthesis
    let groq_resp = client
        .post("https://api.groq.com/openai/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&json!({
            "model": "llama-3.3-70b-versatile",
            "messages": [
                { "role": "system", "content": system_prompt },
                { "role": "user", "content": message }
            ],
            "temperature": 0.4,
            "max_tokens": 2048
        }))
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await
        .map_err(|e| format!("Groq API error: {}", e))?;

    if !groq_resp.status().is_success() {
        let err_text = groq_resp.text().await.unwrap_or_default();
        return Err(format!("Groq API error: {}", err_text));
    }

    let groq_data: serde_json::Value = groq_resp.json().await.map_err(|e| e.to_string())?;
    let answer = groq_data["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("No response from AI.")
        .to_string();

    println!("✅ Chat response: {} chars, {} sources", answer.len(), sources.len());
    Ok(MimirChatResponse { answer, sources })
}

// ─── Rescrape ───────────────────────────────────────────────────────────────

async fn rescrape_one(pool: &sqlx::PgPool, resource_id: &str) -> RescrapeResult {
    // Load resource
    let resource_row = match sqlx::query(
        "SELECT id, title, url, type FROM mimir_resources WHERE id = $1"
    )
    .bind(resource_id)
    .fetch_optional(pool)
    .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return RescrapeResult {
            success: false, changed: false, old_chunks: 0, new_chunks: 0,
            message: "Resource not found".to_string(),
        },
        Err(e) => return RescrapeResult {
            success: false, changed: false, old_chunks: 0, new_chunks: 0,
            message: format!("DB error: {}", e),
        },
    };

    let res_type: String = resource_row.try_get("type").unwrap_or_default();
    let res_url: Option<String> = resource_row.try_get("url").ok();
    let res_title: String = resource_row.try_get("title").unwrap_or_default();

    if res_type != "webpage" {
        return RescrapeResult {
            success: false, changed: false, old_chunks: 0, new_chunks: 0,
            message: "Not a URL resource".to_string(),
        };
    }

    let url = match res_url {
        Some(u) if !u.is_empty() => u,
        _ => return RescrapeResult {
            success: false, changed: false, old_chunks: 0, new_chunks: 0,
            message: "No URL to fetch".to_string(),
        },
    };

    // Skip YouTube videos — transcript was fetched at ingest time, not re-scrape-able
    if url.contains("youtube.com/watch") || url.contains("youtu.be/") {
        println!("⏭️  Skipping YouTube video: {}", res_title);
        return RescrapeResult {
            success: true, changed: false, old_chunks: 0, new_chunks: 0,
            message: "Skipped: YouTube video".to_string(),
        };
    }

    // Count existing chunks
    let chunk_row = match sqlx::query(
        "SELECT COUNT(*) AS count FROM mimir_chunks WHERE resource_id = $1"
    )
    .bind(resource_id)
    .fetch_one(pool)
    .await
    {
        Ok(r) => r,
        Err(e) => return RescrapeResult {
            success: false, changed: false, old_chunks: 0, new_chunks: 0,
            message: format!("DB error counting chunks: {}", e),
        },
    };

    let old_chunks: i64 = chunk_row.try_get("count").unwrap_or(0);

    // Fetch new content — don't delete before success
    let new_text = match fetch_url_content(&url, false).await {
        Ok(r) => r.text,
        Err(e) => return RescrapeResult {
            success: false, changed: false, old_chunks: old_chunks as i32, new_chunks: 0,
            message: format!("Fetch failed: {}", e),
        },
    };

    if new_text.len() < 50 {
        return RescrapeResult {
            success: false, changed: false, old_chunks: old_chunks as i32, new_chunks: 0,
            message: "Fetched text too short to use".to_string(),
        };
    }

    // Skip only if we already have chunks and fetched nothing useful
    if old_chunks > 0 && new_text.is_empty() {
        return RescrapeResult {
            success: true, changed: false, old_chunks: old_chunks as i32, new_chunks: old_chunks as i32,
            message: "No new content fetched".to_string(),
        };
    }

    // Delete old chunks + embeddings and re-embed atomically
    let client = reqwest::Client::new();
    let mut tx = match pool.begin().await {
        Ok(t) => t,
        Err(e) => return RescrapeResult {
            success: false, changed: false, old_chunks: old_chunks as i32, new_chunks: 0,
            message: format!("DB error starting rescrape transaction: {}", e),
        },
    };

    let _ = sqlx::query(
        "DELETE FROM mimir_embeddings WHERE chunk_id IN \
         (SELECT id FROM mimir_chunks WHERE resource_id = $1)"
    )
    .bind(resource_id)
    .execute(&mut *tx)
    .await;

    let _ = sqlx::query("DELETE FROM mimir_chunks WHERE resource_id = $1")
        .bind(resource_id)
        .execute(&mut *tx)
        .await;

    // Re-chunk and re-embed
    let new_chunks = match store_chunks_and_embeddings(&mut tx, &client, resource_id, &new_text).await {
        Ok(n) => n as i32,
        Err(e) => return RescrapeResult {
            success: false, changed: false, old_chunks: old_chunks as i32, new_chunks: 0,
            message: format!("Re-embed failed: {}", e),
        },
    };

    if let Err(e) = tx.commit().await {
        return RescrapeResult {
            success: false, changed: false, old_chunks: old_chunks as i32, new_chunks: 0,
            message: format!("DB error committing rescrape: {}", e),
        };
    }

    let _ = sqlx::query("UPDATE mimir_resources SET updated_at = NOW() WHERE id = $1")
        .bind(resource_id)
        .execute(pool)
        .await;

    println!("✅ Rescraped \"{}\": {} → {} chunks", res_title, old_chunks, new_chunks);
    RescrapeResult {
        success: true,
        changed: true,
        old_chunks: old_chunks as i32,
        new_chunks,
        message: format!("Updated: {} → {} chunks", old_chunks, new_chunks),
    }
}

#[tauri::command]
pub async fn rescrape_resource(
    resource_id: String,
    database: State<'_, Database>,
) -> Result<RescrapeResult, String> {
    let result = rescrape_one(&database.pool, &resource_id).await;

    if !result.success && result.message == "Resource not found" {
        return Err("Resource not found".to_string());
    }
    if !result.success && result.message == "Not a URL resource" {
        return Err("Only URL (webpage) resources can be re-scraped".to_string());
    }

    Ok(result)
}

#[tauri::command]
pub async fn rescrape_all(
    app: tauri::AppHandle,
    database: State<'_, Database>,
) -> Result<Vec<RescrapeResult>, String> {
    let rows = sqlx::query(
        "SELECT id, title, url FROM mimir_resources \
         WHERE type = 'webpage' AND url IS NOT NULL \
           AND url NOT LIKE '%youtube.com/watch%' \
           AND url NOT LIKE '%youtu.be/%' \
           AND (parent_id IS NULL \
                OR parent_id NOT IN (SELECT id FROM mimir_resources WHERE type = 'text')) \
         ORDER BY created_at ASC"
    )
    .fetch_all(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    let total = rows.len();
    println!("🔄 rescrape_all: found {} webpage resources to rescrape", total);
    let mut results: Vec<RescrapeResult> = Vec::new();
    let mut improved = 0u32;
    let mut unchanged = 0u32;
    let mut failed = 0u32;

    for (i, row) in rows.iter().enumerate() {
        if i > 0 {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }

        let id: String = row.try_get("id").unwrap_or_default();
        let title: String = row.try_get("title").unwrap_or_default();

        // Emit "starting" so the UI shows which URL is currently being fetched
        let _ = app.emit("rescrape-progress", json!({
            "status": "starting",
            "current": i + 1,
            "total": total,
            "resourceId": id,
            "title": title,
        }));

        let result = rescrape_one(&database.pool, &id).await;

        let status = if !result.success {
            failed += 1;
            "failed"
        } else if result.changed {
            improved += 1;
            "improved"
        } else {
            unchanged += 1;
            "unchanged"
        };

        println!("[rescrape] {}/{}: {} — {}", i + 1, total, title, status);

        let _ = app.emit("rescrape-progress", json!({
            "status": status,
            "current": i + 1,
            "total": total,
            "resourceId": id,
            "title": title,
            "oldChunks": result.old_chunks,
            "newChunks": result.new_chunks,
        }));

        results.push(result);
    }

    let _ = app.emit("rescrape-complete", json!({
        "improved": improved,
        "unchanged": unchanged,
        "failed": failed,
        "total": total,
    }));

    Ok(results)
}

// ─── Link discovery + Playlist (proxies to Python scraper) ──────────────────

#[tauri::command]
pub async fn discover_links(url: String) -> Result<Vec<DiscoveredLink>, String> {
    let client = reqwest::Client::new();
    let response = client
        .post(format!("{}/discover", SCRAPER_URL))
        .json(&json!({ "url": url }))
        .timeout(std::time::Duration::from_secs(90))
        .send()
        .await
        .map_err(|e| {
            if e.to_string().contains("connect") || e.to_string().contains("refused") {
                "Scraper service not running".to_string()
            } else {
                e.to_string()
            }
        })?;

    if !response.status().is_success() {
        let err: serde_json::Value = response.json().await.unwrap_or_default();
        return Err(err["detail"].as_str().unwrap_or("Discovery failed").to_string());
    }

    let result: serde_json::Value = response.json().await.map_err(|e| e.to_string())?;
    let links = result["links"]
        .as_array()
        .ok_or_else(|| "Invalid response format".to_string())?;

    Ok(links
        .iter()
        .map(|l| DiscoveredLink {
            url: l["url"].as_str().unwrap_or("").to_string(),
            text: l["text"].as_str().unwrap_or("").to_string(),
        })
        .collect())
}

#[tauri::command]
pub async fn fetch_playlist(url: String) -> Result<PlaylistInfo, String> {
    let client = reqwest::Client::new();
    let response = client
        .post(format!("{}/fetch-playlist", SCRAPER_URL))
        .json(&json!({ "url": url }))
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| {
            if e.to_string().contains("connect") || e.to_string().contains("refused") {
                "Scraper service not running".to_string()
            } else {
                e.to_string()
            }
        })?;

    if !response.status().is_success() {
        let err: serde_json::Value = response.json().await.unwrap_or_default();
        return Err(err["detail"].as_str().unwrap_or("Playlist fetch failed").to_string());
    }

    let raw = response.text().await.map_err(|e| e.to_string())?;
    serde_json::from_str::<PlaylistInfo>(&raw).map_err(|e| {
        eprintln!("[fetch_playlist] decode error: {e}\nraw body: {raw}");
        format!("Failed to parse playlist response: {e}")
    })
}

// ─── Direct DB commands (preserved from original) ───────────────────────────

#[tauri::command]
pub async fn get_chunk_counts(
    database: State<'_, Database>,
) -> Result<std::collections::HashMap<String, i64>, String> {
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
    let model = "llama-3.3-70b-versatile";
    let base_url = "https://api.groq.com/openai/v1/chat/completions";
    let api_key = groq_api_key()?;

    let rows = sqlx::query(
        "SELECT id, title FROM mimir_resources WHERE tags = '{}' OR tags IS NULL"
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
            .post(base_url)
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

        if let Err(e) = sqlx::query("UPDATE mimir_resources SET tags = $1 WHERE id = $2")
            .bind(&tags)
            .bind(&id)
            .execute(&database.pool)
            .await
        {
            println!("  ⚠️  Failed to save tags for '{}': {}", title, e);
            continue;
        }

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
    let mut matched = 0u32;
    for node_id in &node_ids {
        match match_node_impl(&database.pool, &client, node_id).await {
            Ok(resources) => {
                if !resources.is_empty() {
                    matched += 1;
                }
            }
            Err(e) => {
                println!("  ⚠️  match error for {}: {}", node_id, e);
                continue;
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

// ─── Re-embed PDFs from stored chunk text ────────────────────────────────────

#[tauri::command]
pub async fn reembed_pdfs(
    database: State<'_, Database>,
) -> Result<String, String> {
    // Fetch all PDF resources including section structure and raw_text for fallback
    let resources = sqlx::query(
        "SELECT id, title, raw_text, sections_json FROM mimir_resources WHERE type = 'pdf' ORDER BY created_at ASC"
    )
    .fetch_all(&database.pool)
    .await
    .map_err(|e| e.to_string())?;

    let total = resources.len();
    println!("📄 reembed_pdfs: found {} PDF resources", total);

    if total == 0 {
        return Ok("No PDF resources found".to_string());
    }

    let client = reqwest::Client::new();
    let mut succeeded = 0u32;
    let mut failed = 0u32;

    for row in &resources {
        let resource_id: String = row.try_get("id").unwrap_or_default();
        let title: String = row.try_get("title").unwrap_or_default();
        let raw_text: Option<String> = row.try_get("raw_text").ok().flatten();
        let sections_json: Option<serde_json::Value> = row.try_get("sections_json").ok().flatten();

        // Fetch all stored chunks for this resource
        let chunks = sqlx::query(
            "SELECT id, content FROM mimir_chunks WHERE resource_id = $1 ORDER BY chunk_index ASC"
        )
        .bind(&resource_id)
        .fetch_all(&database.pool)
        .await
        .map_err(|e| e.to_string())?;

        // If no chunks, attempt re-chunking from stored structure or raw_text
        if chunks.is_empty() {
            let mut tx = match database.pool.begin().await {
                Ok(t) => t,
                Err(e) => { println!("  ⚠️  DB error for '{}': {}", title, e); failed += 1; continue; }
            };

            let result = if let Some(ref secs) = sections_json {
                let count = secs.as_array().map(|a| a.len()).unwrap_or(0);
                if count > 1 {
                    println!("  📄 No chunks for '{}' — re-chunking from sections_json ({} sections)…", title, count);
                    store_sections_and_embeddings(&mut tx, &client, &resource_id, secs).await
                } else if let Some(ref text) = raw_text {
                    println!("  📄 No chunks for '{}' — re-chunking from raw_text (flat)…", title);
                    store_chunks_and_embeddings(&mut tx, &client, &resource_id, text).await
                } else {
                    Err("no text available".to_string())
                }
            } else if let Some(ref text) = raw_text {
                println!("  📄 No chunks for '{}' — re-chunking from raw_text (no section structure)…", title);
                store_chunks_and_embeddings(&mut tx, &client, &resource_id, text).await
            } else {
                println!("  ⚠️  No chunks, no sections_json, no raw_text for '{}' — skipping (re-upload required)", title);
                failed += 1;
                continue;
            };

            match result {
                Ok(n) => {
                    if let Err(e) = tx.commit().await {
                        println!("  ⚠️  Commit failed for '{}': {}", title, e);
                        failed += 1;
                    } else {
                        println!("  ✅ Re-embedded '{}' ({} chunks)", title, n);
                        succeeded += 1;
                    }
                }
                Err(e) => { println!("  ⚠️  Re-embed failed for '{}': {}", title, e); failed += 1; }
            }
            continue;
        }

        println!("  📄 Re-embedding '{}' ({} chunks)…", title, chunks.len());

        let mut chunk_errors = 0u32;
        for chunk_row in &chunks {
            let chunk_id: String = chunk_row.try_get("id").unwrap_or_default();
            let content: String = chunk_row.try_get("content").unwrap_or_default();

            // Delete old embedding for this chunk
            let _ = sqlx::query(
                "DELETE FROM mimir_embeddings WHERE chunk_id = $1"
            )
            .bind(&chunk_id)
            .execute(&database.pool)
            .await;

            // Re-embed
            let embedding = match get_embedding(&client, &content).await {
                Ok(e) => e,
                Err(e) => {
                    println!("    ⚠️  Embedding failed for chunk {}: {}", chunk_id, e);
                    chunk_errors += 1;
                    continue;
                }
            };

            let emb_id = uuid::Uuid::new_v4().to_string();
            let vec_str = vector_str(&embedding);

            if let Err(e) = sqlx::query(
                "INSERT INTO mimir_embeddings (id, chunk_id, embedding) VALUES ($1, $2, $3::vector)"
            )
            .bind(&emb_id)
            .bind(&chunk_id)
            .bind(&vec_str)
            .execute(&database.pool)
            .await
            {
                println!("    ⚠️  Insert embedding failed for chunk {}: {}", chunk_id, e);
                chunk_errors += 1;
            }
        }

        if chunk_errors == 0 {
            succeeded += 1;
            println!("  ✅ Re-embedded '{}'", title);
        } else {
            failed += 1;
            println!("  ⚠️  '{}' had {} chunk error(s)", title, chunk_errors);
        }
    }

    let summary = format!("Re-embedded {}/{} PDFs ({} failed)", succeeded, total, failed);
    println!("📄 {}", summary);
    Ok(summary)
}
