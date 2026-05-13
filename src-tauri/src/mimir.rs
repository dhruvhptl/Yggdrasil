// src-tauri/src/mimir.rs
// Thin shell: shared constants, env helpers, and public structs used across all mimir_* modules.
// Note: do NOT use dotenvy or load_env here — env vars are loaded by dev.sh/build.sh
// before the process starts. Future edits must read env vars via std::env::var() only.

use serde::{Deserialize, Serialize};

pub const EMBED_DIM: usize = 1024;

// ─── Env helpers ────────────────────────────────────────────────────────────

pub(crate) fn openrouter_api_key() -> Result<String, String> {
    std::env::var("OPENROUTER_API_KEY")
        .map_err(|_| "OPENROUTER_API_KEY not configured".to_string())
}

pub(crate) fn groq_api_key() -> Result<String, String> {
    std::env::var("GROQ_API_KEY")
        .map_err(|_| "GROQ_API_KEY not configured".to_string())
}

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
    #[serde(default)]
    pub transcript_source: Option<String>,
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

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    pub action: String,
    pub label: String,
    pub payload: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MimirChatResponse {
    pub answer: String,
    pub sources: Vec<MimirChatSource>,
    pub suggestions: Vec<Suggestion>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredChatMessage {
    pub id: String,
    pub role: String,
    pub content: String,
    pub sources: Option<Vec<MimirChatSource>>,
    pub created_at: String,
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetrievalStats {
    pub total_queries: i64,
    pub avg_candidates_before_rerank: f64,
    pub avg_candidates_after_rerank: f64,
    pub rerank_fallback_rate: f64,
    pub avg_prematch_chunks_used: f64,
    pub top_queried_nodes: Vec<TopQueriedNode>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TopQueriedNode {
    pub node_id: String,
    pub query_count: i64,
}
