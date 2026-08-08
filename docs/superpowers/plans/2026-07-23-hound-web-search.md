# Hound / Web Search (Phase 3) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give Mimir two new agent tools — `smart_search` and `smart_fetch` — backed by an optional Hound sidecar (HTTP, port 8765), registered only when Hound is detected at startup, with graceful degradation to today's behavior when it isn't.

**Architecture:** A new `hound_client.rs` speaks HTTP to Hound with tolerant response parsing and a startup health probe that yields a `HoundStatus` managed by Tauri. `mimir_agent.rs` gains a `hound_base_url` field on `ToolCtx`, a `tool_schemas(hound_available: bool)` that appends the two web tools only when available, and two `execute_tool` arms. `mimir_chat` reads the managed `HoundStatus` into the `ToolCtx`. No tables, no migrations, no `dev.sh` changes.

**Tech Stack:** Rust (Tauri 2, `reqwest`, `serde_json`, `tokio`).

**Spec:** `docs/superpowers/specs/2026-07-23-hound-web-search-design.md` — its **Grounding corrections** section is binding and overrides any conflicting detail.

## Global Constraints

- Never create a new `reqwest::Client` — use the shared client (`ctx.client` in tools, `http_client` in setup).
- Never read `.env` in Rust; no env vars added for this phase.
- Tauri commands return `Result<T, String>`; `database: State<'_, Database>` stays the LAST parameter of `mimir_chat` (the new `hound` State param goes before it).
- Response structs use `#[serde(default)]` on **every** field — the Hound HTTP API shape is UNVERIFIED (external optional program). Parsing must never panic; a mismatch returns a friendly error observation.
- No new tables/migrations; no changes to `mimir_memory.rs`, `orchestrator.rs`, `concept_graph.rs`, `dev.sh`, or the `mimir_retrieval` chat pipeline beyond the `ToolCtx` wiring.
- Hound base URL is the constant `http://127.0.0.1:8765`. Health timeout 2s; search timeout 15s; fetch timeout 30s.
- All shell commands run from `C:/Users/dhruv/projects/Yggdrasil/src-tauri`; branch `fix-compilation-errors`.
- Existing tests to preserve: 18 unit tests pass today (mimir_memory 5, mimir_agent 6, concept_graph 7). This plan adds 3 hound_client tests and updates one mimir_agent test (net: 21 after this phase).
- `cargo build` can take minutes on first run — normal, do not kill it.

---

### Task 1: `hound_client.rs` — status, tolerant parsers, HTTP calls, health

**Files:**
- Create: `src-tauri/src/hound_client.rs`
- Modify: `src-tauri/src/main.rs` (add `mod hound_client;` after `mod concept_graph;` — near the other `mod` lines ~line 31-32)

**Interfaces:**
- Produces (used by Tasks 2–3):
  - `#[derive(Clone)] pub(crate) enum HoundStatus { Available { base_url: String }, Unavailable }`
  - `pub(crate) const HOUND_BASE_URL: &str = "http://127.0.0.1:8765";`
  - `#[derive(Debug, Serialize, Deserialize, Default, Clone)] pub(crate) struct SearchResult { title, url, snippet: String, relevance_score: f32, source: String }` (camelCase serde)
  - `#[derive(Debug, Serialize, Deserialize, Default, Clone)] pub(crate) struct FetchResult { title, content: String, content_ok: bool, page_type, url: String, metadata: serde_json::Value }` (camelCase serde)
  - `pub(crate) fn parse_search_results(body: &serde_json::Value) -> Vec<SearchResult>`
  - `pub(crate) fn parse_fetch_result(body: &serde_json::Value) -> FetchResult`
  - `pub(crate) async fn check_health(client: &reqwest::Client) -> HoundStatus`
  - `pub(crate) async fn smart_search(client: &reqwest::Client, base_url: &str, query: &str, count: Option<u32>) -> Result<Vec<SearchResult>, String>`
  - `pub(crate) async fn smart_fetch(client: &reqwest::Client, base_url: &str, url: &str, focus: Option<&str>) -> Result<FetchResult, String>`

- [ ] **Step 1: Create the module with types + failing parser tests**

Create `src-tauri/src/hound_client.rs`:

```rust
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
    #[serde(default)]
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
    #[serde(default)]
    pub content_ok: bool,
    #[serde(default)]
    pub page_type: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

/// Extract a search-result array from either a bare array or a common envelope
/// ({results:[...]} or {data:[...]}). Unknown/extra fields are ignored; missing
/// fields fall back to type defaults. snake_case and camelCase both decode
/// because serde matches the field name and we alias the snake_case variants.
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
```

Add `mod hound_client;` to `main.rs` after `mod concept_graph;`.

- [ ] **Step 2: Run tests to verify they pass and the module compiles**

Run: `cargo test hound_client 2>&1 | tail -8`
Expected: 3 passed (`parse_search_handles_bare_array_and_envelope`, `parse_search_tolerates_garbage`, `parse_fetch_handles_object_and_envelope_and_defaults`). Warnings about unused `check_health`/`smart_search`/etc. do not exist yet — they're added next; unused-warnings on the structs are fine.

- [ ] **Step 3: Add `check_health`, `smart_search`, `smart_fetch`**

Insert above the `#[cfg(test)]` module:

```rust
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
```

- [ ] **Step 4: Build + test**

Run: `cargo build 2>&1 | tail -5` → `Finished`, no errors (unused-function warnings on the new async fns are expected until Tasks 2–3 call them).
Run: `cargo test hound_client 2>&1 | tail -5` → 3 passed.

- [ ] **Step 5: Commit**

```bash
git add src/hound_client.rs src/main.rs
git commit -m "feat(hound): hound_client module — HoundStatus, tolerant parsers, health/search/fetch"
```

---

### Task 2: Agent tools — `ToolCtx` field, `tool_schemas(bool)`, execute arms

**Files:**
- Modify: `src-tauri/src/mimir_agent.rs`
- Modify: `src-tauri/src/mimir_retrieval.rs` (ONE line: add `hound_base_url: None` to the existing `ToolCtx` literal so the tree keeps building — Task 3 replaces `None` with the real value)

**Interfaces:**
- Consumes: `crate::hound_client::{smart_search, smart_fetch}` (Task 1).
- Produces (used by Task 3): `ToolCtx` now has `pub hound_base_url: Option<String>`; `tool_schemas(hound_available: bool) -> Vec<serde_json::Value>`.

- [ ] **Step 1: Add the `hound_base_url` field to `ToolCtx`**

In `mimir_agent.rs`, `ToolCtx` (line 234) currently ends with `pub message: String,`. Add a field:

```rust
pub(crate) struct ToolCtx<'a> {
    pub pool: &'a sqlx::PgPool,
    pub client: &'a reqwest::Client,
    pub groq_api_key: &'a str,
    pub tree_id: Option<String>,
    pub node_id: Option<String>,
    pub node_title: Option<String>,
    pub node_description: Option<String>,
    pub message: String,
    pub hound_base_url: Option<String>,
}
```

- [ ] **Step 2: Keep the tree compiling — set the field at the one construction site**

In `mimir_retrieval.rs`, the `ToolCtx { ... }` literal (~line 1173) currently ends with `message: message.clone(),`. Add directly after it:

```rust
                    hound_base_url: None,
```

(Task 3 replaces this `None` with the real managed value. This stopgap keeps `cargo build` green at the end of Task 2.)

- [ ] **Step 3: Change `tool_schemas` to take `hound_available` and update the test (write the failing test first)**

In `mimir_agent.rs`, update the existing test `tool_schemas_declares_all_seven_tools` (line 589) to the new signature + a second assertion. Replace the whole test fn with:

```rust
    #[test]
    fn tool_schemas_registers_web_tools_only_when_available() {
        let base: Vec<String> = tool_schemas(false)
            .iter()
            .filter_map(|s| s["function"]["name"].as_str().map(|x| x.to_string()))
            .collect();
        assert_eq!(
            base,
            vec!["search_mimir", "get_facts", "set_fact", "read_tree",
                 "query_graph", "path_between", "explain_node"]
        );

        let with_web: Vec<String> = tool_schemas(true)
            .iter()
            .filter_map(|s| s["function"]["name"].as_str().map(|x| x.to_string()))
            .collect();
        assert_eq!(with_web.len(), 9);
        assert_eq!(with_web[7], "smart_search");
        assert_eq!(with_web[8], "smart_fetch");
        for s in tool_schemas(true) {
            assert_eq!(s["type"], "function");
            assert!(s["function"]["parameters"]["type"] == "object");
        }
    }
```

Run: `cargo test mimir_agent 2>&1 | tail -6`
Expected: compile error — `tool_schemas` still takes no argument (the test calls `tool_schemas(false)`).

- [ ] **Step 4: Implement `tool_schemas(hound_available: bool)`**

Change the signature (line 37) from `pub(crate) fn tool_schemas() -> Vec<serde_json::Value> {` to:

```rust
pub(crate) fn tool_schemas(hound_available: bool) -> Vec<serde_json::Value> {
```

Change the body so it builds the base vec into a mutable local, conditionally appends the two web schemas, and returns it. The existing `vec![ ... ]` currently ends with the `explain_node` schema (added in Phase 1) followed by `]`. Replace the final `]` of that vec literal with `];` bound to `let mut schemas = vec![ ... ];`, i.e.:

- At the start of the function body, change `vec![` to `let mut schemas = vec![`.
- After the closing `]` of that vec, add `;` then the append block below, then `schemas`:

```rust
    if hound_available {
        schemas.push(json!({
            "type": "function",
            "function": {
                "name": "smart_search",
                "description": "Search the live web. Returns relevant results with snippets and source citations. Call this BEFORE smart_fetch to find URLs.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "The search query — be specific." },
                        "count": { "type": "integer", "description": "Number of results (max 10, default 6)." }
                    },
                    "required": ["query"]
                }
            }
        }));
        schemas.push(json!({
            "type": "function",
            "function": {
                "name": "smart_fetch",
                "description": "Fetch a URL and return its content as readable text. Handles articles, documentation, PDFs, and most websites. Use URLs returned by smart_search.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "url": { "type": "string", "description": "The full URL to fetch." },
                        "focus": { "type": "string", "description": "Optional: a short phrase describing what part of the page matters." }
                    },
                    "required": ["url"]
                }
            }
        }));
    }
    schemas
}
```

- [ ] **Step 5: Update the one caller of `tool_schemas` in `run_agent_turn`**

In `run_agent_turn` (line 514), change `let tools = tool_schemas();` to:

```rust
    let tools = tool_schemas(ctx.hound_base_url.is_some());
```

- [ ] **Step 6: Add the two `execute_tool` arms**

In `execute_tool` (line 275), add these arms directly before the `other => Err(format!("unknown tool '{}'", other)),` arm:

```rust
        "smart_search" => {
            let Some(base_url) = ctx.hound_base_url.as_deref() else {
                return Ok("Web search is not available (Hound is not running).".to_string());
            };
            let query = args["query"].as_str().unwrap_or(&ctx.message).to_string();
            let count = args["count"].as_u64().map(|n| n as u32);
            let results = crate::hound_client::smart_search(ctx.client, base_url, &query, count).await?;
            if results.is_empty() {
                return Ok(format!("No web results found for '{}'.", query));
            }
            let mut out = String::from("Web results:\n");
            for r in results.iter().take(10) {
                out.push_str(&format!("- {} — {}\n  {}\n  (source: {})\n", r.title, r.url, r.snippet, r.source));
            }
            Ok(out.chars().take(6000).collect())
        }
        "smart_fetch" => {
            let Some(base_url) = ctx.hound_base_url.as_deref() else {
                return Ok("Web fetch is not available (Hound is not running).".to_string());
            };
            let Some(url) = args["url"].as_str() else {
                return Err("smart_fetch requires 'url'".to_string());
            };
            let focus = args["focus"].as_str();
            let fetched = crate::hound_client::smart_fetch(ctx.client, base_url, url, focus).await?;
            if !fetched.content_ok {
                return Ok(format!("Couldn't access that page ({}). {}", url, fetched.content.chars().take(200).collect::<String>()));
            }
            let header = format!("{} ({})\n", fetched.title, fetched.url);
            let body: String = fetched.content.chars().take(8000).collect();
            Ok(format!("{}{}", header, body))
        }
```

- [ ] **Step 7: Build + test**

Run: `cargo build 2>&1 | tail -5` → `Finished`, no errors.
Run: `cargo test 2>&1 | tail -6` → 21 passed (memory 5, agent 6, concept_graph 7, hound_client 3; the renamed agent test replaces one-for-one).

- [ ] **Step 8: Commit**

```bash
git add src/mimir_agent.rs src/mimir_retrieval.rs
git commit -m "feat(agent): smart_search / smart_fetch tools; tool_schemas gated on Hound availability"
```

---

### Task 3: Startup health check + wire managed state into `mimir_chat`

**Files:**
- Modify: `src-tauri/src/main.rs` (health probe + `manage` in `setup`)
- Modify: `src-tauri/src/mimir_retrieval.rs` (`hound` State param on `mimir_chat`; set `hound_base_url` from it)

**Interfaces:**
- Consumes: `crate::hound_client::{check_health, HoundStatus}` (Task 1); `ToolCtx.hound_base_url` (Task 2).

- [ ] **Step 1: Health probe + managed state in `main.rs`**

In `main.rs` `setup`, the block currently has (around lines 70-80):

```rust
                let http_client = reqwest::Client::new();
                let queue = orchestrator::start_worker(pool.clone(), app_handle.clone(), http_client.clone());
                app_handle.manage(http_client.clone());
```

Directly after `app_handle.manage(http_client.clone());`, add:

```rust
                let hound_status = hound_client::check_health(&http_client).await;
                app_handle.manage(hound_status);
```

(`check_health` borrows the client; it's called before the client is moved elsewhere. If a later line moves `http_client`, this still precedes it. Confirm by reading — the manage above uses `.clone()`, so `http_client` is still available here.)

- [ ] **Step 2: Add the `hound` State param to `mimir_chat`**

In `mimir_retrieval.rs`, `mimir_chat`'s signature currently ends with (from Phase 0/Task 7 of the memory plan):

```rust
    queue: State<'_, crate::orchestrator::JobQueue>,
    database: State<'_, Database>,
```

Add the `hound` param before `database` (database stays last):

```rust
    queue: State<'_, crate::orchestrator::JobQueue>,
    hound: State<'_, crate::hound_client::HoundStatus>,
    database: State<'_, Database>,
```

- [ ] **Step 3: Set `hound_base_url` from the managed status**

In `mimir_chat`, just before the `let tool_ctx = crate::mimir_agent::ToolCtx { ... }` literal (~line 1173), add:

```rust
                let hound_base_url = match hound.inner() {
                    crate::hound_client::HoundStatus::Available { base_url } => Some(base_url.clone()),
                    crate::hound_client::HoundStatus::Unavailable => None,
                };
```

Then change the stopgap line added in Task 2 from `hound_base_url: None,` to:

```rust
                    hound_base_url: hound_base_url.clone(),
```

(Use `.clone()` because the `match` binding is captured into the struct; if the borrow checker is satisfied without it, plain `hound_base_url` is fine — but `.clone()` is safe and unambiguous.)

- [ ] **Step 4: Build + full test suite**

Run: `cargo build 2>&1 | tail -5` → `Finished`, no errors.
Run: `cargo test 2>&1 | tail -6` → 21 passed.

- [ ] **Step 5: Commit**

```bash
git add src/main.rs src/mimir_retrieval.rs
git commit -m "feat(hound): startup health probe + wire HoundStatus into mimir_chat ToolCtx"
```

---

### Task 4: Final verification (inline — controller runs this)

**Files:** none — verification only.

- [ ] **Step 1: Full build + tests**

Run: `cargo build 2>&1 | tail -5` → `Finished`, no errors.
Run: `cargo test 2>&1 | tail -8` → 21 passed, 0 failed.

- [ ] **Step 2: Diff review**

Run: `git log --oneline -3` → the three task commits.
Run: `git diff <base>..HEAD --stat` (base = commit before Task 1) → ONLY: `src/hound_client.rs`, `src/main.rs`, `src/mimir_agent.rs`, `src/mimir_retrieval.rs`. (No migrations, no dev.sh.)

- [ ] **Step 3: Runtime checklist (requires `bash dev.sh` — user-driven; do NOT fake). Split into no-Hound and with-Hound.**

**Without Hound running (default):**
- [ ] App starts; console logs `🌐 [hound] not detected — web tools disabled`.
- [ ] Chat works exactly as before; ask a normal question → no `smart_search`/`smart_fetch` calls (they aren't registered).

**With Hound running** (`pip install hound-mcp[all]` · `playwright install chromium` · `hound --http --host 127.0.0.1 --port 8765`, then restart Yggdrasil):
- [ ] Startup logs `🌐 [hound] available at http://127.0.0.1:8765`.
- [ ] "search the web for X" → console `🛠 [agent] tool call ... smart_search`; answer cites URLs.
- [ ] Fetch a specific URL → `smart_fetch`; answer includes page content.
- [ ] search→fetch chain: agent searches, picks a result, fetches, synthesizes.
- [ ] Kill Hound mid-session → next web tool call returns an error observation; agent recovers/answers from its own knowledge.
- [ ] **API-shape confirmation (the unverified premise):** if `smart_search`/`smart_fetch` return empty or errors against a real Hound, capture the actual JSON Hound returns (e.g. `curl -s -XPOST http://127.0.0.1:8765/api/search -d '{"query":"test"}'`) and adjust `hound_client.rs` field names / envelope handling to match. This is the one thing the unit tests could not verify.

- [ ] **Step 4: Hand off**

Use superpowers:finishing-a-development-branch to decide keep/merge/PR with the user.

---

## Deferred (not in this plan)

`smart_crawl` / `screenshot`, periodic health re-polling, Hound install automation, auto-ingest of
fetched pages, proxy config, and the optional `web_search_cmd` / `web_fetch_cmd` Tauri wrappers.
