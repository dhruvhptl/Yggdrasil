# Agent Visibility (Step 1) — Design

**Date:** 2026-07-28
**Status:** Design approved — ready for implementation plan
**Depends on:** Mimir tool-calling agent (mimir_agent.rs), the existing `ygg-*` Tauri event
pattern (orchestrator.rs), and the `mimir_chat_messages` table.

---

## Goal

Make the Mimir agent *feel* agentic. Today the chat UI shows only "thinking…" then a block of
text; the user never sees the tool calls, sources, or reasoning that produced the answer. This step
surfaces the agent's process live in the chat: tool-call blocks appear as the loop runs, the model's
reasoning shows as a muted block, tool failures render inline, and answers carry clickable source
citations. It also fixes the "0 sources" bug (web-search results never became citations).

Out of this step: token streaming, parallel tool calls, sub-agents, FS/action tools, and the
DeepSeek-vs-Qwen per-turn model routing (that waits for the project-tool schema in a later step).

---

## Decisions locked during brainstorming

| Fork | Decision | Why |
|---|---|---|
| Tool-call history | **Collapse to a one-liner on turn end, click to re-expand, persisted** (survives reload). | Transparency fits a learning tool; the "expand later" ask requires persistence. |
| Reasoning / "thinking" | **Shown, muted, collapsed by default**, above the tool calls. Arrives as one chunk per step (loop is not token-streamed), not typed live. | Useful for *seeing why*; collapsed keeps it out of the way. |
| Sources delivery | **Via the `mimir_chat` return value** (final answer + sources), not a separate event. | The answer already arrives at turn end; only the live *tool* blocks need events. |
| Whole-turn agent failure | **Stays a silent fallback to classic synthesis** for v1 (per-tool errors DO render inline). | A whole-turn failure is rare; a scary banner is not worth it yet. |
| Agent model | **`deepseek/deepseek-v4-flash` via OpenRouter** (already set in dev.sh). | 1M ctx, tool_choice confirmed. Per-turn Qwen-coder routing deferred to the FS-tools step. |

---

## Grounding (verified against current code 2026-07-28 — AUTHORITATIVE)

1. **The agent loop is synchronous.** `run_agent_turn` (mimir_agent.rs:651) is a `for` loop over
   iterations returning one `AgentTurnResult`. Tauri events fire out-of-band from that return, so
   tool blocks stream live while the final answer arrives at the end. Insertion points already exist:
   the `println!` at **mimir_agent.rs:722** (tool start) and the `execute_tool` result match at
   **723–726** (success/error).
2. **`mimir_chat` has no `AppHandle` today** (mimir_retrieval.rs:962; params: message, page, tree_id,
   node_id, node_title, project_name, tree_name, client, queue, hound, database). It must gain
   `app: tauri::AppHandle` and a frontend-supplied `turn_id: String`. Tauri commands may add
   `app: AppHandle` freely.
3. **`ToolCtx`** (mimir_agent.rs) carries the per-turn context into `execute_tool`. It gains an owned
   `app: Option<tauri::AppHandle>` (cheap to clone) + `turn_id: String` so both the loop and long
   tools can emit. `Option` keeps the unit tests (which build a ToolCtx without a Tauri runtime)
   working.
4. **"0 sources" root cause:** the `smart_search` arm (mimir_agent.rs:585–589) formats results into a
   text observation but never does `state.sources.extend(...)`. Only `search_mimir` emits sources
   (mimir_agent.rs:387). `MimirChatSource` (mimir.rs:92 — `title`, `url: Option<String>`, `chunk`,
   `score`, `section_title`, `page_start`, `page_end`) can represent a web hit (title, url,
   snippet→chunk, relevance→score). Fix is a ~5-line map in the `smart_search` arm.
5. **Message persistence:** assistant messages are inserted at **mimir_retrieval.rs:1327**
   (`id, session_id, role, content, sources`). The UI reads them via `get_chat_session`
   (**mimir_retrieval.rs:1501**, SELECT at 1526). The agent-history load (1027, `role, content` only)
   and the consolidation load (mimir_memory.rs:354) are **unaffected** — no new columns needed there.
6. **Highest migration is 052** → this step adds **053**.
7. **`ygg-*` events** are the established pattern (orchestrator emits, frontend `listen()`s and calls
   `unlisten()` on completion — not in `finally`).

---

## Architecture / data flow

```
Frontend: gen turnId → listen(ygg-agent-think/ygg-agent-tool filtered by turnId)
        → invoke('mimir_chat', { …, turnId })
                                   │
Backend: mimir_chat(app, turnId, …) builds ToolCtx{ app:Some, turn_id }
        → run_agent_turn:
             each LLM step:  reasoning present? → app.emit(ygg-agent-think {turnId, text})
             each tool call: emit ygg-agent-tool {turnId, callIndex, toolName, status:"running", input}
                             run execute_tool (elapsed timer)
                             emit ygg-agent-tool {…, status:"success"|"error", output, durationMs}
        → on Answer: persist assistant message (content, sources, tool_calls JSONB, reasoning)
        → return MimirChatResponse { answer, sources, … }  ← citations render from here
Frontend: blocks render live as events arrive; on return, answer renders with [n] links + SourceCards;
          on reload, get_chat_session returns tool_calls/reasoning → collapsed blocks re-render.
```

---

## Components

### 1. Migration 053 (`migrations/053_chat_message_tool_calls.sql`)
```sql
ALTER TABLE mimir_chat_messages
    ADD COLUMN IF NOT EXISTS tool_calls JSONB,
    ADD COLUMN IF NOT EXISTS reasoning  TEXT;
```
Nullable; existing rows stay `NULL` (they had no captured tool calls). `tool_calls` holds the JSON
array described under Event contracts (the persisted form of the per-turn blocks). `reasoning` is the
**concatenation** of any per-step reasoning for the turn — live rendering shows a think block per
step (from the events), but reload shows a single combined collapsed block from this column.

### 2. Backend (`mimir_agent.rs`, `mimir_retrieval.rs`)
- `mimir_chat`: add `app: tauri::AppHandle` and `turn_id: String`; put `app: Some(app.clone())` +
  `turn_id` on `ToolCtx`.
- `run_agent_turn`: accumulate a `Vec<ToolCallRecord>` and an `Option<String> reasoning` on
  `AgentTurnState`; emit `ygg-agent-think` when a step carries reasoning and `ygg-agent-tool` at
  start/finish of each `execute_tool`. Emission is a small helper `emit_agent_event(ctx, payload)`
  that no-ops when `ctx.app` is `None`.
- Reasoning extraction: read the provider's reasoning field from the assistant message
  (`reasoning` / `reasoning_content`) in `call_agent_llm`; return it alongside the step.
- `smart_search` arm: map each `SearchResult` → `MimirChatSource` and `state.sources.extend(...)`
  (the "0 sources" fix).
- Persistence: extend the assistant INSERT (mimir_retrieval.rs:1327) to write `tool_calls` +
  `reasoning`; extend `get_chat_session` (1501/1526) SELECT + returned struct to include them.

### 3. Frontend primitives (`src/components/ui/`)
- `StatusBadge({ status: 'running' | 'success' | 'error' })` — spinner / ✓ / red ✕.
- `ToolCallBlock({ toolName, status, input, output, durationMs, defaultExpanded? })` — collapsed
  header (icon · name · duration · `StatusBadge`); expands to input params + truncated output;
  `error` → red border + message.
- `SourceCard({ index, title, url, snippet })` — clickable; opens `url`.
- No Card/Modal/Button/Spinner extraction (scope creep; extract on second use).

### 4. MimirChat integration (`src/components/MimirChat.tsx`)
- Generate `turnId` (uuid) per send; `listen()` both events filtered by it; `unlisten()` on turn
  completion.
- Render live: the muted collapsed 🤔 thinking block (from `ygg-agent-think`) then `ToolCallBlock`s
  (from `ygg-agent-tool`, updating `running → success/error`).
- On return: render a `SourceCard` list below the message whenever `sources` is non-empty; and if
  the answer text contains `[n]` markers, make them clickable links to `sources[n-1]` (a pure
  `renderCitations(text, sources)` helper). Cards do not depend on `[n]` markers being present.
- On reload: `get_chat_session` now returns `toolCalls`/`reasoning`; render them as collapsed blocks.
- Types: extend the chat-message type + `MimirChatResponseSchema` (validators.ts) with
  `toolCalls?`, `reasoning?`.

---

## Event contracts

```ts
// ygg-agent-think
{ turnId: string; text: string }

// ygg-agent-tool  (also the persisted tool_calls[] element shape, minus turnId/status transitions)
{ turnId: string; callIndex: number; toolName: string;
  status: 'running' | 'success' | 'error';
  input: unknown;            // the tool arguments
  output?: string;           // truncated via truncate_chars on the Rust side
  durationMs?: number }
```
Persisted `tool_calls` is the array of the terminal (`success`/`error`) records for the turn.

---

## Error handling
- Per-tool error → `ygg-agent-tool { status: 'error', output: <message> }` → red inline block; the
  loop continues (error becomes an observation, as today).
- Whole-turn agent failure → existing silent fallback to classic synthesis (unchanged for v1).
- Missing/late events → the block just doesn't appear; the final answer still renders from the return
  value. No crash path.
- Output truncation in events uses `text_util::truncate_chars` (never byte-slice).

---

## Testing
- **Rust (TDD):** reasoning extraction from a sample LLM response; `smart_search`→`MimirChatSource`
  mapping; event-payload shaping helper. Event *emission* itself needs the Tauri runtime → runtime
  checklist.
- **Frontend:** pure `renderCitations(text, sources)` parser (unit test if a vitest harness exists;
  otherwise assert via a small standalone test). Visual blocks verified live.
- **Runtime checklist (`bash dev.sh`):** ask Mimir something that triggers search → blocks appear
  live (running→✓), thinking block present + collapsed, answer shows `[1][2]` + SourceCards, web
  search yields non-zero sources, reload re-renders collapsed blocks, a forced tool error shows a red
  block.

---

## Files
**Create:** `migrations/053_chat_message_tool_calls.sql`; `src/components/ui/StatusBadge.tsx`,
`ToolCallBlock.tsx`, `SourceCard.tsx`.
**Modify:** `mimir_agent.rs` (ToolCtx + events + reasoning + sources fix), `mimir_retrieval.rs`
(mimir_chat signature, persist, get_chat_session), `src/components/MimirChat.tsx`,
`src/lib/validators.ts`, chat-message TS type. `main.rs` — no new command (mimir_chat already
registered; only its signature changes).

---

## Build order (for the plan)
1. Migration 053 + persist/load wiring (save at 1327, `get_chat_session` at 1501) — no behavior yet.
2. Backend events: `mimir_chat` AppHandle+turnId, ToolCtx emitter, `ygg-agent-tool`/`ygg-agent-think`,
   reasoning extraction, `smart_search` sources fix. Unit-test the pure helpers.
3. `ui/` primitives (StatusBadge, ToolCallBlock, SourceCard) + `renderCitations` helper.
4. MimirChat: live event rendering, thinking block, citations, reload rendering; validators/type
   updates.
5. Live verification (runtime checklist).

---

## Explicitly out of scope
Token streaming; parallel/concurrent tool calls; DeepSeek-vs-Qwen per-turn routing; sub-agents;
FS/action tools; a cancel/stop button; surfacing whole-turn fallback as a banner.
