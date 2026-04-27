// src-tauri/src/mimir_ingest.rs
// Ingestion, chunking, embedding, and rescraping logic for Mimir.

use serde_json::json;
use sha2::{Sha256, Digest};
use sqlx::Row;
use tauri::{Emitter, State};
use crate::constants::SCRAPER_URL;
use crate::database::Database;
use crate::mimir::{
    IngestResult, PdfIngestResult, PdfTextResult, RescrapeResult, ScrapedExternalLink,
    EMBED_DIM,
};

// ─── Embedding via Perplexity API ───────────────────────────────────────────

/// Call OpenRouter embeddings API (pplx-embed-v1-0.6b, 1024-dim native output).
/// Accepts a shared client to avoid per-call TCP connection overhead.
pub async fn get_embedding(client: &reqwest::Client, text: &str) -> Result<Vec<f32>, String> {
    let api_key = crate::mimir::openrouter_api_key()?;

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
pub fn vector_str(embedding: &[f32]) -> String {
    let parts: Vec<String> = embedding.iter().map(|v| format!("{}", v)).collect();
    format!("[{}]", parts.join(","))
}

// ─── Text chunking ──────────────────────────────────────────────────────────

pub fn chunk_text(text: &str, max_words: usize) -> Vec<String> {
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
pub fn sanitize_text(text: &str) -> String {
    text.chars()
        .filter(|&c| c == '\t' || c == '\n' || c == '\r' || (c as u32) > 0x1F)
        .collect()
}

// ─── Store chunks + embeddings ──────────────────────────────────────────────

pub async fn store_chunks_and_embeddings(
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
pub async fn store_sections_and_embeddings(
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
async fn fetch_url_content(client: &reqwest::Client, url: &str, force_dynamic: bool) -> Result<FetchResult, String> {
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
async fn fetch_page_title(client: &reqwest::Client, url: &str) -> String {
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
pub async fn ingest_mimir_url(
    url: String,
    title: Option<String>,
    force_dynamic: Option<bool>,
    parent_id: Option<String>,
    app: tauri::AppHandle,
    client: tauri::State<'_, reqwest::Client>,
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

    let fetch_result = fetch_url_content(&*client, &url, force_dynamic.unwrap_or(false)).await?;

    if fetch_result.text.len() < 50 {
        return Err("Could not extract meaningful text from URL".to_string());
    }

    // Derive title
    let page_title = match title.as_ref().filter(|t| !t.trim().is_empty()) {
        Some(t) => t.trim().to_string(),
        None => fetch_page_title(&*client, &url).await,
    };

    println!("  title: \"{}\"", page_title);

    let resource_id = uuid::Uuid::new_v4().to_string();

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

    store_chunks_and_embeddings(&mut tx, &*client, &resource_id, &fetch_result.text).await?;

    tx.commit().await.map_err(|e| e.to_string())?;

    println!("✅ Ingested URL: \"{}\" ({})", page_title, resource_id);

    // Fire-and-forget: auto-tag + match to nodes
    crate::orchestrator::on_resource_ingested(&database.pool, &app, &*client, &resource_id).await;

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
    app: tauri::AppHandle,
    client: tauri::State<'_, reqwest::Client>,
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

    store_chunks_and_embeddings(&mut tx, &*client, &resource_id, &text).await?;

    tx.commit().await.map_err(|e| e.to_string())?;

    println!("✅ Ingested text: \"{}\" ({})", resource_title, resource_id);

    crate::orchestrator::on_resource_ingested(&database.pool, &app, &*client, &resource_id).await;

    Ok(resource_id)
}

#[tauri::command]
pub async fn ingest_mimir_pdf(
    filename: String,
    pdf_base64: String,
    app: tauri::AppHandle,
    client: tauri::State<'_, reqwest::Client>,
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
        store_sections_and_embeddings(&mut tx, &*client, &resource_id, sections).await?;
    } else {
        store_chunks_and_embeddings(&mut tx, &*client, &resource_id, &text).await?;
    }

    tx.commit().await.map_err(|e| e.to_string())?;

    let text_preview = text.chars().take(500).collect::<String>();
    println!("✅ Ingested PDF: \"{}\" ({})", resource_title, resource_id);

    crate::orchestrator::on_resource_ingested(&database.pool, &app, &*client, &resource_id).await;

    Ok(PdfIngestResult {
        id: resource_id,
        title: resource_title,
        text_preview: Some(text_preview),
    })
}

#[tauri::command]
pub async fn extract_pdf_text(
    pdf_base64: String,
    client: tauri::State<'_, reqwest::Client>,
) -> Result<PdfTextResult, String> {
    // Use Python scraper (pymupdf) for better layout handling — same as ingest_mimir_pdf
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

// ─── Rescrape ───────────────────────────────────────────────────────────────

pub(crate) async fn rescrape_one(pool: &sqlx::PgPool, client: &reqwest::Client, resource_id: &str) -> RescrapeResult {
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
    let new_text = match fetch_url_content(client, &url, false).await {
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
    client: tauri::State<'_, reqwest::Client>,
    database: State<'_, Database>,
) -> Result<RescrapeResult, String> {
    let result = rescrape_one(&database.pool, &*client, &resource_id).await;

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
    client: tauri::State<'_, reqwest::Client>,
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

        let result = rescrape_one(&database.pool, &*client, &id).await;

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

// ─── Re-embed PDFs from stored chunk text ────────────────────────────────────

/// Re-embed a single resource by ID. Returns Ok(true) if succeeded, Ok(false) if skipped/failed.
pub(crate) async fn reembed_resource_inner(pool: &sqlx::PgPool, client: &reqwest::Client, resource_id: &str) -> Result<bool, String> {
    let row = sqlx::query(
        "SELECT id, title, raw_text, sections_json FROM mimir_resources WHERE id = $1 AND type = 'pdf'"
    )
    .bind(resource_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;

    let row = match row {
        Some(r) => r,
        None => return Ok(false), // not a PDF or not found
    };

    let title: String = row.try_get("title").unwrap_or_default();
    let raw_text: Option<String> = row.try_get("raw_text").ok().flatten();
    let sections_json: Option<serde_json::Value> = row.try_get("sections_json").ok().flatten();

    let chunks = sqlx::query(
        "SELECT id, content FROM mimir_chunks WHERE resource_id = $1 ORDER BY chunk_index ASC"
    )
    .bind(resource_id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    if chunks.is_empty() {
        let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
        let result = if let Some(ref secs) = sections_json {
            let count = secs.as_array().map(|a| a.len()).unwrap_or(0);
            if count > 1 {
                store_sections_and_embeddings(&mut tx, client, resource_id, secs).await
            } else if let Some(ref text) = raw_text {
                store_chunks_and_embeddings(&mut tx, client, resource_id, text).await
            } else {
                Err("no text available".to_string())
            }
        } else if let Some(ref text) = raw_text {
            store_chunks_and_embeddings(&mut tx, client, resource_id, text).await
        } else {
            return Ok(false);
        };
        match result {
            Ok(_) => { tx.commit().await.map_err(|e| e.to_string())?; Ok(true) }
            Err(e) => Err(format!("re-chunk '{}' failed: {}", title, e)),
        }
    } else {
        let mut chunk_errors = 0u32;
        for chunk_row in &chunks {
            let chunk_id: String = chunk_row.try_get("id").unwrap_or_default();
            let content: String = chunk_row.try_get("content").unwrap_or_default();
            let _ = sqlx::query("DELETE FROM mimir_embeddings WHERE chunk_id = $1")
                .bind(&chunk_id).execute(pool).await;
            let embedding = match get_embedding(client, &content).await {
                Ok(e) => e,
                Err(e) => { println!("    ⚠️  Embedding chunk {} failed: {}", chunk_id, e); chunk_errors += 1; continue; }
            };
            let emb_id = uuid::Uuid::new_v4().to_string();
            let vec_str = vector_str(&embedding);
            if let Err(e) = sqlx::query(
                "INSERT INTO mimir_embeddings (id, chunk_id, embedding) VALUES ($1, $2, $3::vector)"
            ).bind(&emb_id).bind(&chunk_id).bind(&vec_str).execute(pool).await {
                println!("    ⚠️  Insert embedding chunk {} failed: {}", chunk_id, e);
                chunk_errors += 1;
            }
        }
        Ok(chunk_errors == 0)
    }
}

#[tauri::command]
pub async fn reembed_pdfs(
    client: tauri::State<'_, reqwest::Client>,
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
                    store_sections_and_embeddings(&mut tx, &*client, &resource_id, secs).await
                } else if let Some(ref text) = raw_text {
                    println!("  📄 No chunks for '{}' — re-chunking from raw_text (flat)…", title);
                    store_chunks_and_embeddings(&mut tx, &*client, &resource_id, text).await
                } else {
                    Err("no text available".to_string())
                }
            } else if let Some(ref text) = raw_text {
                println!("  📄 No chunks for '{}' — re-chunking from raw_text (no section structure)…", title);
                store_chunks_and_embeddings(&mut tx, &*client, &resource_id, text).await
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
            let embedding = match get_embedding(&*client, &content).await {
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
