// src-tauri/src/hound_client.rs
//
// Optional Hound sidecar — live web search + fetch. Hound is an MCP server
// (Streamable HTTP transport) at http://127.0.0.1:8765/mcp. Detected at startup
// via check_health; when absent, the agent simply never registers
// smart_search / smart_fetch.
//
// Protocol verified against Hound v12.4.1 (2026-07-28): a tool call is
// initialize (→ mcp-session-id response header) then tools/call; responses are
// SSE-framed JSON-RPC whose tool payload is a *stringified* JSON blob living in
// result.content[].text. Response structs use #[serde(default)] + aliases so a
// field-name drift degrades to typed defaults rather than a hard error.

use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::Duration;

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

// ─── MCP-over-HTTP transport ─────────────────────────────────────────────────
//
// Hound speaks MCP over Streamable HTTP at POST {base}/mcp. A tool call is two
// requests: `initialize` (response carries an `mcp-session-id` header) then
// `tools/call {name,args}` (with that session header). Both responses are
// SSE-framed JSON-RPC (`data: {...}` lines) and a tool's payload is a
// stringified JSON blob in `result.content[].text`.

const MCP_PROTOCOL_VERSION: &str = "2024-11-05";

fn initialize_request() -> serde_json::Value {
    json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": { "name": "yggdrasil", "version": "1.0" }
        }
    })
}

/// Pull the first JSON-RPC message carrying a `result`/`error` out of a response
/// body. Accepts SSE framing (`data: {...}` lines) or a bare JSON body — some MCP
/// servers content-negotiate to application/json instead of text/event-stream.
fn parse_sse_jsonrpc(body: &str) -> Result<serde_json::Value, String> {
    // application/json path: the whole body is the message.
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(body.trim()) {
        if v.get("result").is_some() || v.get("error").is_some() {
            return Ok(v);
        }
    }
    // text/event-stream path: scan `data:` lines.
    let mut fallback: Option<serde_json::Value> = None;
    for line in body.lines() {
        let line = line.trim_end_matches('\r');
        let payload = match line.strip_prefix("data:") {
            Some(rest) => rest.strip_prefix(' ').unwrap_or(rest),
            None => continue,
        };
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(payload) {
            if v.get("result").is_some() || v.get("error").is_some() {
                return Ok(v);
            }
            fallback = Some(v);
        }
    }
    fallback.ok_or_else(|| "no JSON-RPC data frame in Hound response".to_string())
}

/// Turn a JSON-RPC response into the tool's own JSON payload. MCP wraps tool
/// output in `result.content[]` text blocks (each a stringified JSON blob); we
/// concatenate the text blocks and parse them. JSON-RPC errors and
/// `result.isError` surface as `Err`.
fn extract_tool_json(rpc: &serde_json::Value) -> Result<serde_json::Value, String> {
    if let Some(err) = rpc.get("error") {
        let msg = err.get("message").and_then(|m| m.as_str()).unwrap_or("unknown error");
        return Err(format!("Hound MCP error: {}", msg));
    }
    let result = rpc.get("result").ok_or("Hound response had no result")?;
    // Prefer structuredContent when the server provides it.
    if let Some(sc) = result.get("structuredContent") {
        if !sc.is_null() {
            return Ok(sc.clone());
        }
    }
    let mut text = String::new();
    if let Some(items) = result.get("content").and_then(|c| c.as_array()) {
        for item in items {
            if item.get("type").and_then(|t| t.as_str()) == Some("text") {
                if let Some(t) = item.get("text").and_then(|t| t.as_str()) {
                    text.push_str(t);
                }
            }
        }
    }
    if result.get("isError").and_then(|b| b.as_bool()).unwrap_or(false) {
        return Err(format!("Hound tool error: {}", text.chars().take(300).collect::<String>()));
    }
    if text.trim().is_empty() {
        return Err("Hound tool result had no text content".to_string());
    }
    serde_json::from_str(text.trim()).map_err(|e| format!("Hound tool payload was not JSON: {}", e))
}

/// One MCP tool call: initialize (capture session) → tools/call → tool payload.
async fn mcp_tool_call(
    client: &reqwest::Client,
    base_url: &str,
    tool: &str,
    arguments: serde_json::Value,
    call_timeout: Duration,
) -> Result<serde_json::Value, String> {
    let endpoint = format!("{}/mcp", base_url);

    // 1. initialize — grab the session id from the response headers.
    let init = client
        .post(&endpoint)
        .header("Accept", "application/json, text/event-stream")
        .json(&initialize_request())
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("Hound initialize failed: {}", e))?;
    if !init.status().is_success() {
        return Err(format!("Hound initialize returned {}", init.status()));
    }
    let session = init
        .headers()
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let _ = init.text().await; // drain the initialize body

    // 2. tools/call — thread the session id through if we got one.
    let mut req = client
        .post(&endpoint)
        .header("Accept", "application/json, text/event-stream")
        .json(&json!({
            "jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": { "name": tool, "arguments": arguments }
        }))
        .timeout(call_timeout);
    if let Some(sid) = &session {
        req = req.header("mcp-session-id", sid.as_str());
    }
    let resp = req.send().await.map_err(|e| format!("Hound {} failed: {}", tool, e))?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| format!("Hound {} read failed: {}", tool, e))?;
    if !status.is_success() {
        return Err(format!(
            "Hound {} returned {}: {}",
            tool,
            status,
            text.chars().take(300).collect::<String>()
        ));
    }
    let rpc = parse_sse_jsonrpc(&text)?;
    extract_tool_json(&rpc)
}

// ─── Public API ──────────────────────────────────────────────────────────────

/// Liveness probe: an MCP `initialize` with a 3s timeout. A 2xx → Available.
/// Never panics.
pub(crate) async fn check_health(client: &reqwest::Client) -> HoundStatus {
    let endpoint = format!("{}/mcp", HOUND_BASE_URL);
    let resp = client
        .post(&endpoint)
        .header("Accept", "application/json, text/event-stream")
        .json(&initialize_request())
        .timeout(Duration::from_secs(3))
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
    let args = json!({ "query": query, "count": count.unwrap_or(6).min(10) });
    let payload =
        mcp_tool_call(client, base_url, "mcp_smart_search", args, Duration::from_secs(20)).await?;
    Ok(parse_search_results(&payload))
}

pub(crate) async fn smart_fetch(
    client: &reqwest::Client,
    base_url: &str,
    url: &str,
    focus: Option<&str>,
) -> Result<FetchResult, String> {
    let mut args = json!({ "url": url });
    if let Some(f) = focus {
        args["focus"] = json!(f);
    }
    let payload =
        mcp_tool_call(client, base_url, "mcp_smart_fetch", args, Duration::from_secs(40)).await?;
    Ok(parse_fetch_result(&payload))
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

    #[test]
    fn sse_jsonrpc_extracts_result_frame() {
        let body = "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"ok\":true}}\n\n";
        let v = parse_sse_jsonrpc(body).unwrap();
        assert!(v["result"]["ok"].as_bool().unwrap());
    }

    #[test]
    fn sse_jsonrpc_accepts_bare_json_body() {
        // application/json content-negotiation path: whole body is the message.
        let body = "{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"ok\":true}}";
        let v = parse_sse_jsonrpc(body).unwrap();
        assert!(v["result"]["ok"].as_bool().unwrap());
    }

    #[test]
    fn sse_jsonrpc_errors_on_no_frame() {
        assert!(parse_sse_jsonrpc("event: ping\n\n").is_err());
    }

    #[test]
    fn extract_tool_json_unwraps_stringified_text_block() {
        // MCP wraps the tool's JSON output as a stringified text content block.
        let inner = json!({ "results": [ { "title": "A", "url": "https://a" } ] }).to_string();
        let rpc = json!({
            "jsonrpc": "2.0", "id": 2,
            "result": { "content": [ { "type": "text", "text": inner } ] }
        });
        let payload = extract_tool_json(&rpc).unwrap();
        let results = parse_search_results(&payload);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "A");
    }

    #[test]
    fn extract_tool_json_prefers_structured_content() {
        let rpc = json!({ "result": { "structuredContent": { "results": [] }, "content": [] } });
        let payload = extract_tool_json(&rpc).unwrap();
        assert!(payload.get("results").is_some());
    }

    #[test]
    fn extract_tool_json_surfaces_errors() {
        let rpc_err = json!({ "jsonrpc": "2.0", "id": 2, "error": { "code": -32600, "message": "bad" } });
        assert!(extract_tool_json(&rpc_err).is_err());

        let tool_err = json!({ "result": { "isError": true, "content": [ { "type": "text", "text": "boom" } ] } });
        assert!(extract_tool_json(&tool_err).is_err());
    }

    #[test]
    fn end_to_end_search_frame_parses_like_hound() {
        // Mirrors a real Hound mcp_smart_search response captured 2026-07-28:
        // an SSE frame whose result.content[0].text is a stringified JSON blob
        // using snake_case relevance_score.
        let inner = json!({
            "query": "pgvector",
            "results": [ {
                "title": "HNSW", "url": "https://neon.com", "snippet": "s",
                "source": "brave", "relevance_score": 1.0, "position": 1
            } ]
        })
        .to_string();
        let rpc = json!({
            "jsonrpc": "2.0", "id": 2,
            "result": { "content": [ { "type": "text", "text": inner } ] }
        });
        let body = format!("event: message\ndata: {}\n\n", rpc);
        let parsed = parse_sse_jsonrpc(&body).unwrap();
        let payload = extract_tool_json(&parsed).unwrap();
        let results = parse_search_results(&payload);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "HNSW");
        assert_eq!(results[0].source, "brave");
        assert!((results[0].relevance_score - 1.0).abs() < 1e-6);
    }

    // Live end-to-end check against a running Hound (ignored by default —
    // needs the sidecar up + network). Run with:
    //   cargo test --manifest-path src-tauri/Cargo.toml hound -- --ignored --nocapture
    #[test]
    #[ignore]
    fn live_smart_search_against_running_hound() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let client = reqwest::Client::new();
            match check_health(&client).await {
                HoundStatus::Unavailable => panic!("Hound not running on {}", HOUND_BASE_URL),
                HoundStatus::Available { base_url } => {
                    let results = smart_search(&client, &base_url, "pgvector hnsw index", Some(3))
                        .await
                        .expect("smart_search failed");
                    assert!(!results.is_empty(), "expected at least one search result");
                    assert!(!results[0].url.is_empty(), "first result had no url");
                    eprintln!("live result[0]: {} — {}", results[0].title, results[0].url);
                }
            }
        });
    }
}
