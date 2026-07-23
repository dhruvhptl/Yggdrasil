// src-tauri/src/hound_client.rs
//
// Optional Hound sidecar (HTTP mode, port 8765) — live web search + fetch.
// Detected at startup via check_health; when absent, the agent simply never
// registers smart_search / smart_fetch.
//
// NOTE: Hound's HTTP API shape is assumed, not verified against a running
// instance. All response types use #[serde(default)] and the parsers are
// tolerant (accept a top-level array or a {results:[...]} / {data:[...]}
// envelope). A mismatch degrades to empty/typed-default, never a panic.

use serde::{Deserialize, Serialize};
use serde_json::json;

pub(crate) const HOUND_BASE_URL: &str = "http://127.0.0.1:8765";

#[derive(Clone)]
pub(crate) enum HoundStatus {
    Available { base_url: String },
    Unavailable,
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SearchResult {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub snippet: String,
    #[serde(default, alias = "relevance_score")]
    pub relevance_score: f32,
    #[serde(default)]
    pub source: String,
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FetchResult {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub content: String,
    #[serde(default, alias = "content_ok")]
    pub content_ok: bool,
    #[serde(default, alias = "page_type")]
    pub page_type: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

/// Extract a search-result array from either a bare array or a common envelope
/// ({results:[...]} or {data:[...]}). Unknown/extra fields are ignored; missing
/// fields fall back to type defaults. Multi-word fields decode from either camelCase (default) or snake_case
/// (via #[serde(alias)]) so we tolerate either spelling from the unverified
/// Hound API. Unknown/extra fields are ignored; missing fields use type defaults.
pub(crate) fn parse_search_results(body: &serde_json::Value) -> Vec<SearchResult> {
    let arr = if body.is_array() {
        body.clone()
    } else if body.get("results").map(|v| v.is_array()).unwrap_or(false) {
        body["results"].clone()
    } else if body.get("data").map(|v| v.is_array()).unwrap_or(false) {
        body["data"].clone()
    } else {
        return Vec::new();
    };
    serde_json::from_value(arr).unwrap_or_default()
}

pub(crate) fn parse_fetch_result(body: &serde_json::Value) -> FetchResult {
    // Accept the object directly, or a {result:{...}} / {data:{...}} envelope.
    let obj = if body.get("result").map(|v| v.is_object()).unwrap_or(false) {
        body["result"].clone()
    } else if body.get("data").map(|v| v.is_object()).unwrap_or(false) {
        body["data"].clone()
    } else {
        body.clone()
    };
    serde_json::from_value(obj).unwrap_or_default()
}

// ─── HTTP calls ──────────────────────────────────────────────────────────────

/// Probe Hound's /health with a 2s timeout. Any 2xx → Available. Never panics.
pub(crate) async fn check_health(client: &reqwest::Client) -> HoundStatus {
    let url = format!("{}/health", HOUND_BASE_URL);
    let resp = client
        .get(&url)
        .timeout(std::time::Duration::from_secs(2))
        .send()
        .await;
    match resp {
        Ok(r) if r.status().is_success() => {
            println!("🌐 [hound] available at {}", HOUND_BASE_URL);
            HoundStatus::Available { base_url: HOUND_BASE_URL.to_string() }
        }
        _ => {
            println!("🌐 [hound] not detected — web tools disabled");
            HoundStatus::Unavailable
        }
    }
}

pub(crate) async fn smart_search(
    client: &reqwest::Client,
    base_url: &str,
    query: &str,
    count: Option<u32>,
) -> Result<Vec<SearchResult>, String> {
    let url = format!("{}/api/search", base_url);
    let body = json!({ "query": query, "count": count.unwrap_or(6).min(10) });
    let resp = client
        .post(&url)
        .json(&body)
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| format!("search request failed: {}", e))?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| format!("search read failed: {}", e))?;
    if !status.is_success() {
        return Err(format!("search returned {}: {}", status, text.chars().take(300).collect::<String>()));
    }
    let json_body: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("search parse failed: {}", e))?;
    Ok(parse_search_results(&json_body))
}

pub(crate) async fn smart_fetch(
    client: &reqwest::Client,
    base_url: &str,
    url: &str,
    focus: Option<&str>,
) -> Result<FetchResult, String> {
    let endpoint = format!("{}/api/fetch", base_url);
    let body = json!({ "url": url, "focus": focus });
    let resp = client
        .post(&endpoint)
        .json(&body)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| format!("fetch request failed: {}", e))?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| format!("fetch read failed: {}", e))?;
    if !status.is_success() {
        return Err(format!("fetch returned {}: {}", status, text.chars().take(300).collect::<String>()));
    }
    let json_body: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("fetch parse failed: {}", e))?;
    Ok(parse_fetch_result(&json_body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_search_handles_bare_array_and_envelope() {
        let bare = json!([
            { "title": "A", "url": "https://a", "snippet": "s", "relevanceScore": 0.9, "source": "brave" }
        ]);
        let r = parse_search_results(&bare);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].title, "A");
        assert_eq!(r[0].source, "brave");

        let env = json!({ "results": [ { "title": "B", "url": "https://b" } ] });
        let r2 = parse_search_results(&env);
        assert_eq!(r2.len(), 1);
        assert_eq!(r2[0].title, "B");
        assert_eq!(r2[0].snippet, ""); // missing → default
    }

    #[test]
    fn parse_search_tolerates_garbage() {
        assert!(parse_search_results(&json!({ "nope": 1 })).is_empty());
        assert!(parse_search_results(&json!("not json shaped")).is_empty());
    }

    #[test]
    fn parse_fetch_handles_object_and_envelope_and_defaults() {
        let direct = json!({ "title": "T", "content": "body", "contentOk": true, "pageType": "article", "url": "https://x" });
        let f = parse_fetch_result(&direct);
        assert_eq!(f.title, "T");
        assert!(f.content_ok);
        assert_eq!(f.page_type, "article");

        let env = json!({ "result": { "content": "hi" } });
        let f2 = parse_fetch_result(&env);
        assert_eq!(f2.content, "hi");
        assert!(!f2.content_ok); // missing bool → false
        assert!(f2.metadata.is_null());
    }
}
