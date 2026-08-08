# Agent Visibility (Step 1) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Surface the Mimir agent's process live in chat — tool-call blocks (running → success/error), a muted reasoning block, and clickable source citations — plus fix the "0 sources" web-search bug.

**Architecture:** The synchronous agent loop (`run_agent_turn`) emits `ygg-agent-tool` / `ygg-agent-think` Tauri events during the turn; the frontend generates a `turnId`, subscribes filtered by it, and renders blocks live. The final answer + sources arrive via the `mimir_chat` return value. Tool calls + reasoning are persisted on the assistant message (migration 053) so they re-render collapsed after reload.

**Tech Stack:** Rust (Tauri 2, sqlx, serde_json), React 19 + TypeScript + Tailwind, Postgres.

## Global Constraints

- Rust: `$N` placeholders (never `?N`); non-macro `sqlx::query()`; commands return `Result<T, String>` with `database: State<'_, Database>` last; structs `#[serde(rename_all = "camelCase")]`.
- JSONB not TEXT for structured columns. Migrations are append-only; highest is 052 → this adds **053**; never modify existing migrations.
- Truncate arbitrary text with `crate::text_util::truncate_chars` — never byte-slice.
- Tauri events use the `ygg-*` prefix; frontend `listen()` must `unlisten()` on turn completion (not in `finally`).
- No new crates or npm deps. Frontend `turnId` uses `crypto.randomUUID()`.
- Agent model is set via env (`MIMIR_AGENT_MODEL=deepseek/deepseek-v4-flash`) — do not hardcode model names in new code.
- Keep all work local on branch `fix-compilation-errors`; do not push.
- **Build/test in an isolated target dir** (the user's `bash dev.sh` may hold `target/`):
  `export CARGO_TARGET_DIR="C:/Users/dhruv/AppData/Local/Temp/claude/C--Users-dhruv-projects-Yggdrasil/497ed12f-520a-4774-9c54-620ab075f017/scratchpad/scan-target"`
- **Frontend has no unit-test harness** (no vitest/jest). Pure TS helpers are written to be trivially correct and verified in the runtime checklist (Task 5) rather than adding a test framework. This is a deliberate TDD exception for the frontend only; all Rust logic is still TDD'd.

## File Structure

| File | Responsibility | Task |
|---|---|---|
| `src-tauri/migrations/053_chat_message_tool_calls.sql` | Add `tool_calls JSONB`, `reasoning TEXT` to `mimir_chat_messages` | 1 |
| `src-tauri/src/mimir.rs` | `StoredChatMessage` gains `tool_calls`/`reasoning` | 1 |
| `src-tauri/src/mimir_retrieval.rs` | Persist + load new columns; `mimir_chat` signature; carry tool_calls/reasoning; build ToolCtx w/ emitter | 1, 2 |
| `src-tauri/src/mimir_agent.rs` | `ToolCtx`/`AgentTurnState`/`AgentTurnResult` fields; `ToolCallRecord`; emit helper + events; `extract_reasoning`; `web_results_to_sources`; smart_search sources fix | 2 |
| `src/lib/validators.ts` | `StoredChatMessageSchema` gains `toolCalls`/`reasoning` | 1 |
| `src/lib/citations.ts` | Pure `tokenizeCitations(text)` helper | 3 |
| `src/components/ui/StatusBadge.tsx` | running/success/error indicator | 3 |
| `src/components/ui/ToolCallBlock.tsx` | Collapsible tool-call block | 3 |
| `src/components/ui/SourceCard.tsx` | Clickable source citation card | 3 |
| `src/components/MimirChat.tsx` | turnId, event listeners, live render, thinking block, citations, reload render | 4 |

---

### Task 1: Migration 053 + persistence round-trip

**Files:**
- Create: `src-tauri/migrations/053_chat_message_tool_calls.sql`
- Modify: `src-tauri/src/mimir.rs:119-127` (`StoredChatMessage`)
- Modify: `src-tauri/src/mimir_retrieval.rs:1326-1335` (assistant INSERT), `:1523-1547` (`get_chat_session` SELECT + build)
- Modify: `src/lib/validators.ts:50-56` (`StoredChatMessageSchema`)

**Interfaces:**
- Consumes: nothing (first task).
- Produces: `mimir_chat_messages.tool_calls JSONB`, `mimir_chat_messages.reasoning TEXT`; `StoredChatMessage { …, tool_calls: Option<serde_json::Value>, reasoning: Option<String> }` (serialized `toolCalls`, `reasoning`); `get_chat_session` returns them. The assistant INSERT binds `$5=tool_calls (JSONB)`, `$6=reasoning (TEXT)` — Task 2 supplies real values; Task 1 binds `serde_json::Value::Null` and `Option::<String>::None`.

- [ ] **Step 1: Write the migration**

Create `src-tauri/migrations/053_chat_message_tool_calls.sql`:
```sql
ALTER TABLE mimir_chat_messages
    ADD COLUMN IF NOT EXISTS tool_calls JSONB,
    ADD COLUMN IF NOT EXISTS reasoning  TEXT;
```

- [ ] **Step 2: Extend `StoredChatMessage`**

In `src-tauri/src/mimir.rs`, add two fields to the struct (currently ends at `created_at`):
```rust
pub struct StoredChatMessage {
    pub id: String,
    pub role: String,
    pub content: String,
    pub sources: Option<Vec<MimirChatSource>>,
    pub tool_calls: Option<serde_json::Value>,
    pub reasoning: Option<String>,
    pub created_at: String,
}
```

- [ ] **Step 3: Persist the new columns (assistant INSERT)**

In `src-tauri/src/mimir_retrieval.rs`, replace the assistant INSERT (around :1326) so it writes the two columns. For Task 1 bind literal empties (Task 2 replaces these with real values):
```rust
let asst_msg_id = uuid::Uuid::new_v4().to_string();
let _ = sqlx::query(
    "INSERT INTO mimir_chat_messages (id, session_id, role, content, sources, tool_calls, reasoning) \
     VALUES ($1, $2, 'assistant', $3, $4, $5, $6)"
)
.bind(&asst_msg_id)
.bind(sid)
.bind(&answer)
.bind(&sources_json)
.bind(serde_json::Value::Null)          // tool_calls — Task 2 supplies real value
.bind(Option::<String>::None)           // reasoning  — Task 2 supplies real value
.execute(&database.pool)
.await;
```

- [ ] **Step 4: Return the new columns from `get_chat_session`**

In `src-tauri/src/mimir_retrieval.rs`, update the SELECT (around :1523) and the row → struct build (around :1541):
```rust
let rows = sqlx::query(
    "SELECT id, role, content, sources, tool_calls, reasoning, \
            created_at::TEXT AS created_at \
     FROM mimir_chat_messages \
     WHERE session_id = $1 \
     ORDER BY created_at ASC \
     LIMIT 20"
)
.bind(&session_id)
.fetch_all(&database.pool)
.await
.map_err(|e| e.to_string())?;

let messages = rows.iter().map(|row| {
    let sources_val: Option<serde_json::Value> = row.try_get("sources").ok();
    let sources: Option<Vec<MimirChatSource>> = sources_val.and_then(|v| serde_json::from_value(v).ok());
    StoredChatMessage {
        id: row.try_get("id").unwrap_or_default(),
        role: row.try_get("role").unwrap_or_default(),
        content: row.try_get("content").unwrap_or_default(),
        sources,
        tool_calls: row.try_get::<Option<serde_json::Value>, _>("tool_calls").unwrap_or(None),
        reasoning: row.try_get::<Option<String>, _>("reasoning").unwrap_or(None),
        created_at: row.try_get("created_at").unwrap_or_default(),
    }
```

- [ ] **Step 5: Extend the frontend schema**

In `src/lib/validators.ts`, update `StoredChatMessageSchema`:
```ts
export const StoredChatMessageSchema = z.object({
  id: z.string(),
  role: z.string(),
  content: z.string(),
  sources: z.array(SourceSchema).nullable(),
  toolCalls: z.array(z.unknown()).nullable().optional(),
  reasoning: z.string().nullable().optional(),
  createdAt: z.string(),
});
```

- [ ] **Step 6: Build (verify it compiles)**

Run: `cargo build --manifest-path src-tauri/Cargo.toml` (with `CARGO_TARGET_DIR` set per Global Constraints)
Expected: `Finished` with no errors (pre-existing warnings OK).

- [ ] **Step 7: Commit**

```bash
git add src-tauri/migrations/053_chat_message_tool_calls.sql src-tauri/src/mimir.rs src-tauri/src/mimir_retrieval.rs src/lib/validators.ts
git commit -m "feat(agent): migration 053 + persist/load tool_calls & reasoning"
```

---

### Task 2: Backend events, reasoning, and the sources fix

**Files:**
- Modify: `src-tauri/src/mimir_agent.rs` (ToolCtx :325-335, AgentTurnState :337-342, AgentTurnResult :638-644, execute_tool smart_search arm :575-590, run_agent_turn :651-747; add `ToolCallRecord`, `extract_reasoning`, `web_results_to_sources`, `emit_agent_event`)
- Modify: `src-tauri/src/mimir_retrieval.rs` (`mimir_chat` signature :962-974, ToolCtx build :1180-1190, agent-outcome tuple :1214-1215, assistant INSERT binds from Task 1)

**Interfaces:**
- Consumes: Task 1's INSERT (`$5` tool_calls, `$6` reasoning).
- Produces: `ToolCallRecord { call_index: u32, tool_name: String, status: String, input: Value, output: Option<String>, duration_ms: Option<u64> }` (serde camelCase); `AgentTurnResult` gains `tool_calls: Vec<ToolCallRecord>` + `reasoning: Option<String>`; `ToolCtx` gains `app: Option<tauri::AppHandle>` + `turn_id: String`; events `ygg-agent-tool` and `ygg-agent-think`; `mimir_chat` gains params `app: tauri::AppHandle` and `turn_id: String`.

- [ ] **Step 1: Write failing tests for the two pure helpers**

Add to the `#[cfg(test)] mod tests` in `src-tauri/src/mimir_agent.rs`:
```rust
#[test]
fn extract_reasoning_reads_reasoning_fields() {
    let with = json!({ "role": "assistant", "reasoning": "  I should search first.  " });
    assert_eq!(extract_reasoning(&with).as_deref(), Some("I should search first."));
    let alt = json!({ "role": "assistant", "reasoning_content": "alt" });
    assert_eq!(extract_reasoning(&alt).as_deref(), Some("alt"));
    let none = json!({ "role": "assistant", "content": "hi" });
    assert_eq!(extract_reasoning(&none), None);
    let empty = json!({ "role": "assistant", "reasoning": "   " });
    assert_eq!(extract_reasoning(&empty), None);
}

#[test]
fn web_results_map_to_sources_with_url_and_score() {
    let results = vec![crate::hound_client::SearchResult {
        title: "T".into(), url: "https://x".into(), snippet: "s".into(),
        relevance_score: 0.9, source: "brave".into(),
    }];
    let s = web_results_to_sources(&results);
    assert_eq!(s.len(), 1);
    assert_eq!(s[0].title, "T");
    assert_eq!(s[0].url.as_deref(), Some("https://x"));
    assert_eq!(s[0].chunk, "s");
    assert!((s[0].score - 0.9).abs() < 1e-6);

    let empty_url = vec![crate::hound_client::SearchResult { url: "".into(), ..Default::default() }];
    assert_eq!(web_results_to_sources(&empty_url)[0].url, None);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml mimir_agent 2>&1 | tail -20`
Expected: FAIL — `extract_reasoning` / `web_results_to_sources` not found.

- [ ] **Step 3: Implement the pure helpers + `ToolCallRecord`**

Add near the top of `src-tauri/src/mimir_agent.rs` (after the imports / `AgentStep`):
```rust
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ToolCallRecord {
    pub call_index: u32,
    pub tool_name: String,
    pub status: String,           // "success" | "error"
    pub input: serde_json::Value,
    pub output: Option<String>,
    pub duration_ms: Option<u64>,
}

/// Pull the model's chain-of-thought from an assistant message, if present.
pub(crate) fn extract_reasoning(assistant_msg: &serde_json::Value) -> Option<String> {
    assistant_msg
        .get("reasoning")
        .or_else(|| assistant_msg.get("reasoning_content"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Map Hound web-search results into chat sources so they become citations.
pub(crate) fn web_results_to_sources(
    results: &[crate::hound_client::SearchResult],
) -> Vec<crate::mimir::MimirChatSource> {
    results
        .iter()
        .map(|r| crate::mimir::MimirChatSource {
            title: r.title.clone(),
            url: if r.url.is_empty() { None } else { Some(r.url.clone()) },
            chunk: r.snippet.clone(),
            score: r.relevance_score,
            section_title: None,
            page_start: None,
            page_end: None,
        })
        .collect()
}
```
Ensure `use serde::Serialize;` is in scope (it is — other structs derive it).

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml mimir_agent 2>&1 | tail -20`
Expected: PASS (both new tests green, existing ones still green).

- [ ] **Step 5: Add fields to `ToolCtx`, `AgentTurnState`, `AgentTurnResult`**

`ToolCtx` (add two fields):
```rust
    pub hound_base_url: Option<String>,
    pub app: Option<tauri::AppHandle>,
    pub turn_id: String,
}
```
`AgentTurnState`:
```rust
#[derive(Default)]
pub(crate) struct AgentTurnState {
    pub sources: Vec<crate::mimir::MimirChatSource>,
    pub stats: crate::mimir_retrieval::RetrievalStats,
    pub tool_calls_made: u32,
    pub tool_calls: Vec<ToolCallRecord>,
    pub reasoning: Vec<String>,
}
```
`AgentTurnResult`:
```rust
pub(crate) struct AgentTurnResult {
    pub answer: String,
    pub sources: Vec<crate::mimir::MimirChatSource>,
    pub stats: crate::mimir_retrieval::RetrievalStats,
    pub tool_calls_made: u32,
    pub pending_approval: Option<crate::hitl::ActionProposal>,
    pub tool_calls: Vec<ToolCallRecord>,
    pub reasoning: Option<String>,
}
```

- [ ] **Step 6: Add the emit helper**

Add to `src-tauri/src/mimir_agent.rs`:
```rust
/// Emit a `ygg-*` event to the frontend if a Tauri handle is present (no-op in tests).
fn emit_agent_event<S: serde::Serialize>(ctx: &ToolCtx, event: &str, payload: &S) {
    if let Some(app) = &ctx.app {
        use tauri::Emitter;
        let _ = app.emit(event, payload);
    }
}
```

- [ ] **Step 7: Emit reasoning + tool events in the loop**

In `run_agent_turn`, right after `let (step, assistant_msg) = call_agent_llm(...).await?;` (:670-671), add reasoning capture:
```rust
if let Some(reason) = extract_reasoning(&assistant_msg) {
    emit_agent_event(ctx, "ygg-agent-think", &json!({ "turnId": ctx.turn_id, "text": reason }));
    state.reasoning.push(reason);
}
```
Then in the `AgentStep::ToolCalls(calls)` arm, replace the per-call block (currently :721-731, the `tool_calls_made += 1` / `println!` / `execute_tool` / push tool message) with:
```rust
state.tool_calls_made += 1;
let call_index = state.tool_calls_made;
println!("🛠  [agent] tool call {}/{}: {}", call_index, MAX_TOOL_CALLS, call.name);
emit_agent_event(ctx, "ygg-agent-tool", &json!({
    "turnId": ctx.turn_id, "callIndex": call_index, "toolName": call.name,
    "status": "running", "input": call.arguments,
}));
let started = std::time::Instant::now();
let observation = match execute_tool(&call.name, &call.arguments, ctx, &mut state).await {
    Ok(o) => o,
    Err(e) => format!("Tool error: {}", e),
};
let duration_ms = started.elapsed().as_millis() as u64;
let status = if observation.starts_with("Tool error:") { "error" } else { "success" };
let output_preview = crate::text_util::truncate_chars(&observation, 500);
emit_agent_event(ctx, "ygg-agent-tool", &json!({
    "turnId": ctx.turn_id, "callIndex": call_index, "toolName": call.name,
    "status": status, "output": output_preview, "durationMs": duration_ms,
}));
state.tool_calls.push(ToolCallRecord {
    call_index, tool_name: call.name.clone(), status: status.to_string(),
    input: call.arguments.clone(), output: Some(output_preview), duration_ms: Some(duration_ms),
});
messages.push(json!({ "role": "tool", "tool_call_id": call.id, "content": observation }));
```
(Keep the existing HITL `requires_approval` check and the `tool_calls_made >= MAX_TOOL_CALLS` budget branch above this, unchanged.)

- [ ] **Step 8: Populate `tool_calls`/`reasoning` in all `AgentTurnResult` returns**

There are three `return Ok(AgentTurnResult { … })` sites (Answer :688, HITL pending :705, and the max-iterations `Err` is unchanged). For the two `Ok` returns, add:
```rust
    tool_calls: state.tool_calls.clone(),
    reasoning: if state.reasoning.is_empty() { None } else { Some(state.reasoning.join("\n\n")) },
```
(The Answer return may move `state.sources`/`state.stats`; clone `tool_calls` before those moves or reorder so `tool_calls`/`reasoning` are read first.)

- [ ] **Step 9: Fix "0 sources" in the smart_search arm**

In `execute_tool`'s `"smart_search"` arm (:575-590), after `let results = …smart_search(...).await?;` and the empty check, add before returning:
```rust
    state.sources.extend(web_results_to_sources(&results));
```

- [ ] **Step 10: Thread AppHandle + turnId through `mimir_chat`**

In `src-tauri/src/mimir_retrieval.rs`, add params to the `mimir_chat` command signature:
```rust
pub async fn mimir_chat(
    message: String,
    page: String,
    tree_id: Option<String>,
    node_id: Option<String>,
    node_title: Option<String>,
    project_name: Option<String>,
    tree_name: Option<String>,
    turn_id: String,
    app: tauri::AppHandle,
    client: tauri::State<'_, reqwest::Client>,
    queue: State<'_, crate::orchestrator::JobQueue>,
    hound: State<'_, crate::hound_client::HoundStatus>,
    database: State<'_, Database>,
) -> Result<MimirChatResponse, String> {
```
In the ToolCtx build (:1180), add:
```rust
    hound_base_url: hound_base_url.clone(),
    app: Some(app.clone()),
    turn_id: turn_id.clone(),
};
```

- [ ] **Step 11: Carry tool_calls/reasoning to persistence**

Extend the agent-outcome tuple (:1214). Change the binding and both arms:
```rust
let (answer, sources, stats, pending_approval, tool_calls_json, reasoning) = match agent_outcome {
    Some(r) => {
        let tcj = serde_json::to_value(&r.tool_calls).unwrap_or(serde_json::Value::Null);
        (r.answer, r.sources, r.stats, r.pending_approval, tcj, r.reasoning)
    }
    None => {
        // ...existing classic-path block unchanged, but its final tuple gains two fields:
        // (answer, sources, stats, None)  ->  (answer, sources, stats, None, serde_json::Value::Null, None)
    }
};
```
Then update the assistant INSERT binds from Task 1 to use the real values:
```rust
.bind(&tool_calls_json)      // was serde_json::Value::Null
.bind(&reasoning)            // was Option::<String>::None
```

- [ ] **Step 12: Fix ToolCtx literals in existing tests (if any)**

Any `ToolCtx { … }` built in `#[cfg(test)]` code must add `app: None, turn_id: String::new(),`. Search: `grep -n "ToolCtx {" src-tauri/src/mimir_agent.rs`. (If none exist in tests, skip.)

- [ ] **Step 13: Build + full test run**

Run: `cargo test --manifest-path src-tauri/Cargo.toml 2>&1 | tail -25`
Expected: build clean; all tests pass (the two new + existing).

- [ ] **Step 14: Commit**

```bash
git add src-tauri/src/mimir_agent.rs src-tauri/src/mimir_retrieval.rs
git commit -m "feat(agent): emit tool/think events, capture tool_calls+reasoning, fix web sources"
```

---

### Task 3: Frontend primitives + citation tokenizer

**Files:**
- Create: `src/lib/citations.ts`, `src/components/ui/StatusBadge.tsx`, `src/components/ui/ToolCallBlock.tsx`, `src/components/ui/SourceCard.tsx`

**Interfaces:**
- Consumes: nothing (self-contained presentational units).
- Produces: `tokenizeCitations(text): CitationToken[]`; `<StatusBadge status />`; `<ToolCallBlock toolName status input output durationMs defaultExpanded? />`; `<SourceCard index title url snippet />`. Task 4 imports all four.

- [ ] **Step 1: Citation tokenizer (`src/lib/citations.ts`)**

```ts
export type CitationToken =
  | { kind: 'text'; text: string }
  | { kind: 'cite'; index: number };

/** Split answer text into plain-text and [n] citation tokens. Pure + total. */
export function tokenizeCitations(text: string): CitationToken[] {
  const out: CitationToken[] = [];
  const re = /\[(\d+)\]/g;
  let last = 0;
  let m: RegExpExecArray | null;
  while ((m = re.exec(text)) !== null) {
    if (m.index > last) out.push({ kind: 'text', text: text.slice(last, m.index) });
    out.push({ kind: 'cite', index: parseInt(m[1], 10) });
    last = m.index + m[0].length;
  }
  if (last < text.length) out.push({ kind: 'text', text: text.slice(last) });
  return out;
}
```

- [ ] **Step 2: `StatusBadge` (`src/components/ui/StatusBadge.tsx`)**

```tsx
import { Loader2, Check, X } from "lucide-react";

export type ToolStatus = "running" | "success" | "error";

export function StatusBadge({ status }: { status: ToolStatus }) {
  if (status === "running") return <Loader2 className="w-3.5 h-3.5 animate-spin text-blue-400" />;
  if (status === "success") return <Check className="w-3.5 h-3.5 text-green-400" />;
  return <X className="w-3.5 h-3.5 text-red-400" />;
}
```

- [ ] **Step 3: `ToolCallBlock` (`src/components/ui/ToolCallBlock.tsx`)**

```tsx
import { useState } from "react";
import { ChevronRight, ChevronDown } from "lucide-react";
import { StatusBadge, type ToolStatus } from "./StatusBadge";

const TOOL_ICON: Record<string, string> = {
  search_mimir: "🔍", smart_search: "🌐", smart_fetch: "📄",
  read_tree: "📖", get_facts: "🧠", set_fact: "🧠",
  query_graph: "🕸️", path_between: "🧭", explain_node: "💡", scan_project: "🗂️",
};

export interface ToolCallBlockProps {
  toolName: string;
  status: ToolStatus;
  input?: unknown;
  output?: string;
  durationMs?: number;
  defaultExpanded?: boolean;
}

export function ToolCallBlock({ toolName, status, input, output, durationMs, defaultExpanded = false }: ToolCallBlockProps) {
  const [open, setOpen] = useState(defaultExpanded);
  const isError = status === "error";
  return (
    <div className={`rounded-md border text-sm ${isError ? "border-red-500/50 bg-red-500/5" : "border-white/10 bg-white/5"}`}>
      <button onClick={() => setOpen(o => !o)} className="w-full flex items-center gap-2 px-2.5 py-1.5 text-left">
        {open ? <ChevronDown className="w-3.5 h-3.5 opacity-60" /> : <ChevronRight className="w-3.5 h-3.5 opacity-60" />}
        <span>{TOOL_ICON[toolName] ?? "🛠️"}</span>
        <span className="font-mono text-xs">{toolName}</span>
        <StatusBadge status={status} />
        {durationMs != null && <span className="ml-auto text-xs opacity-50">{(durationMs / 1000).toFixed(1)}s</span>}
      </button>
      {open && (
        <div className="px-3 pb-2 space-y-1.5">
          {input != null && (
            <pre className="text-xs opacity-70 whitespace-pre-wrap break-words">{JSON.stringify(input, null, 2)}</pre>
          )}
          {output && (
            <pre className={`text-xs whitespace-pre-wrap break-words ${isError ? "text-red-300" : "opacity-80"}`}>{output}</pre>
          )}
        </div>
      )}
    </div>
  );
}
```

- [ ] **Step 4: `SourceCard` (`src/components/ui/SourceCard.tsx`)**

```tsx
import { ExternalLink } from "lucide-react";

export interface SourceCardProps {
  index: number;
  title: string;
  url?: string | null;
  snippet?: string;
}

export function SourceCard({ index, title, url, snippet }: SourceCardProps) {
  const body = (
    <div className="rounded-md border border-white/10 bg-white/5 px-2.5 py-1.5 hover:bg-white/10 transition-colors">
      <div className="flex items-center gap-1.5 text-xs">
        <span className="font-mono opacity-50">[{index}]</span>
        <span className="font-medium truncate">{title}</span>
        {url && <ExternalLink className="w-3 h-3 opacity-50 shrink-0" />}
      </div>
      {snippet && <p className="mt-0.5 text-xs opacity-60 line-clamp-2">{snippet}</p>}
    </div>
  );
  return url ? <a href={url} target="_blank" rel="noreferrer" className="block">{body}</a> : body;
}
```

- [ ] **Step 5: Verify the frontend compiles**

Run: `npm run build` (from repo root)
Expected: TypeScript build succeeds with no errors in the new files.

- [ ] **Step 6: Commit**

```bash
git add src/lib/citations.ts src/components/ui/StatusBadge.tsx src/components/ui/ToolCallBlock.tsx src/components/ui/SourceCard.tsx
git commit -m "feat(ui): ToolCallBlock, StatusBadge, SourceCard + citation tokenizer"
```

---

### Task 4: MimirChat integration

**Files:**
- Modify: `src/components/MimirChat.tsx` (imports :2-10; `ChatMessage`/`StoredChatMessage` types ~:31; load map :143-160; send/invoke :205-262; message render :632-720)

**Interfaces:**
- Consumes: Task 3's components + `tokenizeCitations`; Task 1's `toolCalls`/`reasoning` on `StoredChatMessage`; Task 2's `ygg-agent-tool`/`ygg-agent-think` events and the `turn_id` param on `mimir_chat`.
- Produces: the live + persisted visibility UI (terminal deliverable).

- [ ] **Step 1: Imports + types**

In `src/components/MimirChat.tsx`:
```tsx
import { listen } from "@tauri-apps/api/event";
import { ToolCallBlock } from "./ui/ToolCallBlock";
import { SourceCard } from "./ui/SourceCard";
import { tokenizeCitations } from "../lib/citations";
import type { ToolStatus } from "./ui/StatusBadge";
```
Extend the local `ChatMessage` type (and the `StoredChatMessage` interface ~:31) with:
```tsx
interface LiveToolCall { callIndex: number; toolName: string; status: ToolStatus; input?: unknown; output?: string; durationMs?: number; }
// on ChatMessage:  toolCalls?: LiveToolCall[];  reasoning?: string;
```
When mapping `get_chat_session` results (:152) carry `toolCalls`/`reasoning` onto the loaded `ChatMessage` (they arrive as `m.toolCalls` / `m.reasoning`).

- [ ] **Step 2: Generate turnId, subscribe to events, pass turnId to invoke**

In the send handler (around :205), before `invoke("mimir_chat", …)`:
```tsx
const turnId = crypto.randomUUID();
// index of the assistant placeholder message being built this turn:
const assistantIndex = /* the index you push the pending assistant ChatMessage at */;

const unlistenTool = await listen<LiveToolCall & { turnId: string }>("ygg-agent-tool", ({ payload }) => {
  if (payload.turnId !== turnId) return;
  setMessages(prev => {
    const next = [...prev];
    const msg = next[assistantIndex];
    if (!msg) return prev;
    const calls = [...(msg.toolCalls ?? [])];
    const at = calls.findIndex(c => c.callIndex === payload.callIndex);
    const rec: LiveToolCall = { callIndex: payload.callIndex, toolName: payload.toolName, status: payload.status, input: payload.input, output: payload.output, durationMs: payload.durationMs };
    if (at >= 0) calls[at] = { ...calls[at], ...rec }; else calls.push(rec);
    next[assistantIndex] = { ...msg, toolCalls: calls };
    return next;
  });
});
const unlistenThink = await listen<{ turnId: string; text: string }>("ygg-agent-think", ({ payload }) => {
  if (payload.turnId !== turnId) return;
  setMessages(prev => {
    const next = [...prev];
    const msg = next[assistantIndex];
    if (!msg) return prev;
    next[assistantIndex] = { ...msg, reasoning: [msg.reasoning, payload.text].filter(Boolean).join("\n\n") };
    return next;
  });
});
```
Add `turnId` to the invoke payload:
```tsx
const raw = await invoke("mimir_chat", { message, page, treeId, nodeId, nodeTitle, projectName, treeName, turnId });
```
After the turn resolves (success or error), call `unlistenTool(); unlistenThink();` — **not** in a `finally` that races the last event; call them right after you apply the final response to the assistant message.

- [ ] **Step 3: Render reasoning + tool blocks + citations in the message body**

In the message render (`messages.map`, :632), for assistant messages render — above the answer text — a collapsed reasoning block and the tool blocks, and render the answer through `tokenizeCitations`:
```tsx
{msg.reasoning && (
  <details className="mb-2 text-xs opacity-60">
    <summary className="cursor-pointer select-none">🤔 Thinking</summary>
    <pre className="mt-1 whitespace-pre-wrap break-words">{msg.reasoning}</pre>
  </details>
)}
{msg.toolCalls?.map(tc => (
  <div className="mb-1" key={tc.callIndex}>
    <ToolCallBlock toolName={tc.toolName} status={tc.status} input={tc.input} output={tc.output} durationMs={tc.durationMs} />
  </div>
))}
<div className="whitespace-pre-wrap">
  {tokenizeCitations(msg.content).map((t, k) =>
    t.kind === "text"
      ? <span key={k}>{t.text}</span>
      : <a key={k} href={msg.sources?.[t.index - 1]?.url ?? undefined} target="_blank" rel="noreferrer" className="text-blue-400">[{t.index}]</a>
  )}
</div>
```

- [ ] **Step 4: Render SourceCards below the message when sources exist**

Replace/augment the existing `msg.sources.map(...)` block (:704) so each source renders as a `SourceCard`:
```tsx
{msg.sources && msg.sources.length > 0 && (
  <div className="mt-2 space-y-1">
    {msg.sources.map((src, j) => (
      <SourceCard key={j} index={j + 1} title={src.title} url={src.url} snippet={src.chunk} />
    ))}
  </div>
)}
```

- [ ] **Step 5: Verify the frontend compiles**

Run: `npm run build`
Expected: succeeds; no type errors.

- [ ] **Step 6: Commit**

```bash
git add src/components/MimirChat.tsx
git commit -m "feat(agent): live tool-call blocks, thinking, and inline citations in Mimir chat"
```

---

### Task 5: Live verification (runtime checklist)

**Files:** none (manual verification in a running app).

**Interfaces:** Consumes the whole feature. No code output.

- [ ] **Step 1: Start the app**

Run: `bash dev.sh` (applies migration 053 on startup; starts Hound; agent = DeepSeek V4 Flash).

- [ ] **Step 2: Trigger a multi-tool turn**

Open a tree's Mimir chat and ask something that needs search + web, e.g. *"Search my library and the web for how HNSW indexes work, and relate it to my tree."* Confirm:
- Tool blocks appear **live** (running spinner → ✓ / red ✕), each with a duration.
- A muted, collapsed **🤔 Thinking** block appears above the tool calls.
- The answer renders with `[1][2]` links and **SourceCards** below.
- **Web search yields non-zero sources** (the "0 sources" bug is fixed).

- [ ] **Step 3: Verify persistence across reload**

Close/reopen the tree (or restart the app) and reopen the same node's chat. Confirm the assistant message re-renders its **collapsed** tool-call blocks and thinking block from persisted data.

- [ ] **Step 4: Verify an error block**

Temporarily make a tool fail (e.g., ask it to `scan_project` a non-existent path) and confirm a **red error block** renders inline and the agent still produces an answer.

- [ ] **Step 5: Confirm the model swap works end-to-end**

Check the dev console shows the agent completing turns against OpenRouter/DeepSeek (tool calls fire, no tool_choice/400 errors). If DeepSeek misbehaves, set `MIMIR_AGENT_MODEL=google/gemini-2.5-flash` and retry — no code change.

---

## Self-Review

**Spec coverage:**
- Live tool blocks → Task 2 (events) + Task 4 (render). ✓
- Collapsed/expandable + persisted history → Task 1 (columns/load) + Task 2 (capture) + Task 4 (reload render). ✓
- Reasoning block → Task 2 (`extract_reasoning` + `ygg-agent-think`) + Task 4 (details block). ✓
- Inline `[n]` citations + SourceCards → Task 3 (`tokenizeCitations`, `SourceCard`) + Task 4. ✓
- "0 sources" fix → Task 2 Step 9. ✓
- Error blocks → Task 2 (status="error") + Task 3 (red styling) + Task 4. ✓
- Migration 053 → Task 1. ✓
- AppHandle/turnId plumbing → Task 2 Step 10. ✓
- Whole-turn fallback stays silent → unchanged (no task touches the fallback branch). ✓ (per spec)

**Placeholder scan:** No TBD/TODO; every code step has real code. The one intentional two-phase item (Task 1 binds `Null`/`None`, Task 2 supplies real values) is called out explicitly in both tasks.

**Type consistency:** `ToolCallRecord` (Rust, camelCase) ↔ `LiveToolCall` (TS) fields match: `callIndex`, `toolName`, `status`, `input`, `output`, `durationMs`. Event names `ygg-agent-tool` / `ygg-agent-think` identical in Task 2 (emit) and Task 4 (listen). `turn_id` (Rust param) serialized/received as `turnId` (Tauri camelCases command args) — invoke passes `turnId`. `StoredChatMessage.tool_calls`/`reasoning` ↔ `toolCalls`/`reasoning` (serde camelCase) ↔ validators `toolCalls`/`reasoning`. Consistent.

**Note on TDD scope:** Rust pure helpers are TDD'd (Task 2 Steps 1-4). DB/Tauri-runtime code and all frontend code are verified in Task 5's runtime checklist (no frontend test harness exists; adding one is out of scope per Global Constraints).
