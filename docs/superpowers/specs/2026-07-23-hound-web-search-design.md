# Hound / Web Search (Phase 3) — Design

**Date:** 2026-07-23
**Status:** Design approved — ready for implementation plan
**Depends on:** Phase 0 (agent loop, tool registry, ToolCtx — all merged on `fix-compilation-errors`)

---

## Goal

Give Mimir live web access via two new agent tools — `smart_search` and `smart_fetch` — backed by
an **optional** Hound sidecar (HTTP mode, port 8765). If Hound isn't running, the tools are simply
not registered and chat behaves exactly as today. No new tables, no migrations, no `dev.sh` changes.

---

## Decisions locked during brainstorming

| Fork | Decision | Why |
|---|---|---|
| Sidecar interface | **HTTP, not MCP stdio** (`hound --http --port 8765`); Rust `reqwest` talks to it | Yggdrasil isn't an MCP client; HTTP mirrors the existing scraper (3002) + Axum bridge (1421) patterns |
| Tools exposed | **`smart_search` + `smart_fetch` only** (skip crawl/screenshot/cache/version) | Single-answer queries don't need crawl; Mimir is text-only |
| Degradation | **No Hound → tools not registered** (agent never sees them) | Clean; no silent divergent fallback |
| Discovery | **Startup health probe** of `127.0.0.1:8765/health` (2s timeout); no env var/config | Optional; install Hound, restart, recognized |
| URL saving | **`smart_fetch` may surface a save-candidate**; user always confirms; ingest pipeline unchanged | Keeps the library intentional |
| Health polling | **Startup-only** (no periodic re-check); restart to pick up a newly-installed Hound | Matches how the 3002 scraper is treated |

---

## Grounding corrections (verified against Phase 0 code 2026-07-23 — AUTHORITATIVE)

These override any conflicting detail below. The original design's "Component 3 — Managed state"
sketch (`execute_tool(..., state: &State<HoundStatus>)`) does NOT match the real agent architecture.

1. **`execute_tool` has no Tauri `State`.** Its real signature (`mimir_agent.rs:275`) is
   `execute_tool(name: &str, args: &Value, ctx: &ToolCtx<'_>, state: &mut AgentTurnState)`. Hound
   availability + base_url therefore flow through **`ToolCtx`**. Add a field:
   `pub hound_base_url: Option<String>` to `ToolCtx` (`mimir_agent.rs:234`). `Some(url)` ⇒ Hound
   available; `None` ⇒ unavailable.

2. **Conditional registration = signature change.** `run_agent_turn` (`mimir_agent.rs:514`) calls
   `let tools = tool_schemas();`. Change `tool_schemas()` → `tool_schemas(hound_available: bool)`,
   appending the two web-tool schemas only when `true`. In `run_agent_turn` call it as
   `tool_schemas(ctx.hound_base_url.is_some())`. The existing unit test
   `tool_schemas_declares_all_seven_tools` (`mimir_agent.rs:589`) must be updated: it should assert
   the 7 base tools with `tool_schemas(false)`, and a second assertion that `tool_schemas(true)`
   yields 9 (adds `smart_search`, `smart_fetch`).

3. **`mimir_chat` populates the field.** `mimir_chat` (`mimir_retrieval.rs`) builds the `ToolCtx`
   (~line 1173). Add a `hound: State<'_, HoundStatus>` parameter (placed before `database`, which
   stays the LAST parameter per convention) and set
   `hound_base_url: match hound.inner() { HoundStatus::Available { base_url } => Some(base_url.clone()), _ => None }`.
   Frontend `invoke` needs no change (state params are injected).

4. **Health check + managed state live in `main.rs` `setup`.** The shared client is `http_client`
   (created in `setup`). Before/near `app_handle.manage(http_client.clone())`, add
   `let hound_status = hound_client::check_health(&http_client).await;` then
   `app_handle.manage(hound_status);`. `check_health` sets its own 2s per-request timeout.

5. **The Hound HTTP API shape is UNVERIFIED (highest risk).** The endpoint paths (`/health`,
   `/api/search`, `/api/fetch`) and JSON response fields below are the spec author's assumptions
   about an external, optional program that cannot be exercised from this repo. Therefore:
   response parsing MUST be tolerant — deserialize into structs with `#[serde(default)]` on every
   field and never hard-fail on missing/extra keys; a parse miss returns a friendly error
   observation, not a panic. Unit tests validate only the *assumed* contract; the true check is the
   runtime checklist against a real `hound --http`. If the real API differs, only `hound_client.rs`
   changes — the agent/tool wiring is shape-independent.

---

## Architecture

```
Hound sidecar (optional):  hound --http --host 127.0.0.1 --port 8765
        ▲ reqwest (server-side; no CORS needed)
        │
  hound_client.rs   check_health / smart_search / smart_fetch  + HoundStatus + tolerant response types
        │
  mimir_agent.rs    tool_schemas(hound_available) + execute_tool arms; reads ctx.hound_base_url
        │
  mimir_chat        builds ToolCtx{ hound_base_url } from managed HoundStatus
```

### Startup flow
1. `setup` runs `check_health(&http_client)` → `HoundStatus::Available { base_url }` or `Unavailable`.
2. `app.manage(hound_status)`.
3. Per chat turn, `mimir_chat` reads it into `ToolCtx.hound_base_url`; `run_agent_turn` registers the
   2 web tools iff `Some`. If `None`, the agent never sees them — no error path in the loop.

---

## Component 1 — `hound_client.rs`

```rust
#[derive(Clone)]
pub(crate) enum HoundStatus {
    Available { base_url: String },
    Unavailable,
}

pub(crate) async fn check_health(client: &reqwest::Client) -> HoundStatus;
// GET http://127.0.0.1:8765/health with a 2s timeout. 2xx + parseable → Available{base_url:"http://127.0.0.1:8765"}, else Unavailable. Never panics.

pub(crate) async fn smart_search(client, base_url, query, count: Option<u32>) -> Result<Vec<SearchResult>, String>;
// POST {base_url}/api/search  { "query": query, "count": count.unwrap_or(6).min(10) }

pub(crate) async fn smart_fetch(client, base_url, url, focus: Option<&str>) -> Result<FetchResult, String>;
// POST {base_url}/api/fetch  { "url": url, "focus": focus }
```

Tolerant response types (all fields `#[serde(default)]`):
```rust
pub(crate) struct SearchResult { title, url, snippet, relevance_score: f32, source: String }
pub(crate) struct FetchResult  { title, content, content_ok: bool, page_type, url, metadata: serde_json::Value }
```
No CORS (server-side reqwest). Reuses the shared client; sets per-request timeouts (search ~15s, fetch ~30s).

## Component 2 — Agent tools (`mimir_agent.rs`)

- `smart_search { query, count? }` — "Search the live web… Call BEFORE smart_fetch to find URLs." Format results as titled bullets with URL + snippet + source; cap observation at 6000 chars.
- `smart_fetch { url, focus? }` — "Fetch a URL and return readable text…". On `content_ok:false` return the error text; cap content at ~8000 chars.
- Both arms read `ctx.hound_base_url`; if `None` (defensive — shouldn't happen since unregistered) return a friendly "web search unavailable" Ok-message. Tool errors become observations.

## Component 3 — Managed state (`main.rs`) — see Grounding correction 4.

## Component 4 — Tauri commands (OPTIONAL, deferred)
`web_search_cmd` / `web_fetch_cmd` thin wrappers — only if a frontend "Search the web" button is
built. Not part of this phase; the agent is the interface.

## Component 5 — No `dev.sh` / no migration changes.
User installs Hound independently: `pip install hound-mcp[all]` · `playwright install chromium` ·
`hound --http --host 127.0.0.1 --port 8765` (or Docker). Yggdrasil installs/configures nothing.

---

## Files

**Create:** `src-tauri/src/hound_client.rs` (client, HoundStatus, tolerant types, unit tests).
**Modify:** `src-tauri/src/main.rs` (`mod hound_client;` + health check + manage);
`src-tauri/src/mimir_agent.rs` (ToolCtx field; `tool_schemas(bool)`; 2 execute arms; update test);
`src-tauri/src/mimir_retrieval.rs` (`hound` State param on `mimir_chat`; set `hound_base_url`).
**No change:** memory/orchestrator/concept_graph/mimir_retrieval-pipeline/dev.sh; no tables/migrations.

---

## Error handling
- Hound absent → tools unregistered; no loop error path.
- Request fails / Hound crashes mid-session → tool returns error string as observation; agent recovers.
- No results → "No results found." `content_ok:false` → return Hound's error text.
- Parse mismatch (unverified API) → friendly error observation, never a panic.

---

## Verification checklist
- `cargo build` clean; `cargo test` passes (health-timeout, search/fetch response parsing on assumed shape, error formatting).
- Without Hound: `HoundStatus::Unavailable`; `tool_schemas(false)` = 7 tools; chat unchanged.
- With Hound: `Available` at startup; "search the web for X" → `🛠 [agent] tool call smart_search`, answer cites URLs; a specific-URL fetch → `smart_fetch`; search→fetch chain synthesizes an answer.
- Kill Hound mid-session → next tool call errors, agent recovers.
- All 7 existing tools still work.

## Build order (for the plan)
1. `hound_client.rs` (+ tolerant parsing + unit tests).
2. `mimir_agent.rs`: ToolCtx field, `tool_schemas(bool)`, 2 execute arms, update test.
3. `main.rs`: health check + managed state; `mimir_retrieval.rs`: `hound` param + set field.
4. `cargo build` + verification.

## Explicitly out of scope
`smart_crawl`, `screenshot`, periodic health polling, Hound install automation, auto-ingest of
fetched pages, proxy config, the optional Tauri command wrappers.
