# Mimir Memory + Minimal Agent (Phase 0 / Phase 3-lite)

**Date:** 2026-07-23
**Status:** Design approved — ready for implementation plan
**Builder:** Fable subagents (`claude-fable-5`), driven from the implementation plan
**Supersedes:** the original `2026-07-23-memory-system.md` draft (this design evolved from it)

---

## Goal

Turn Mimir's chat from a fixed "retrieve → answer" step into a **model-agnostic agent** that
(1) decides which tools to use before answering, and (2) has **persistent memory** it reads and
writes on its own — plus memory that fills in automatically from checkpoint/resource completion.

This is the foundation the later agent phases (sub-agents, concept graphs, web search) build on.
Everything valuable about today's chat — hybrid retrieval, GraphRAG, pre-matched chunks,
reranking, **source citations**, session history — is preserved by turning the retrieval
pipeline into a *tool* rather than deleting it.

---

## Decisions locked during brainstorming

| Fork | Decision | Why |
|---|---|---|
| Memory purpose | **Agent-ready** (memory as tools an agent reads/writes), not just passive chat recall | User prioritizes capabilities/tools over incremental UX polish |
| Scope | **Memory foundation + a minimal agent loop** (not foundation-only, not the full harness) | Gives the memory tools a live consumer so they aren't dead scaffolding |
| Agent surface | **Agent becomes the default Mimir chat** (with a safety fallback) | User wants one clean experience, not a toggle |
| Retrieval integration | **Approach A — retrieval becomes a `search_mimir` tool** the agent chooses to fire | Keeps all retrieval quality + citations; gives a *real* agent that controls its own info-gathering |
| Model | **Model-agnostic loop** via OpenAI-compatible tool-calling; default = **Groq LLaMA 3.3-70b**, swappable by config | Any tool-calling model works; matches the deepagents "any tool-calling LLM" principle |
| Consolidation cadence | **Every ~20 messages, incremental** (only new messages since last pass) | Sustainable cost; long sessions don't re-summarize old messages |
| Day-one tools | `search_mimir`, `get_facts`, `set_fact`, `read_tree` | Tight, useful set; more tools (web search, delete) deferred |

---

## Architecture overview

```
User message
   │
   ▼
mimir_chat (entry point, unchanged signature)
   │
   ├─ recall seed: compact "what we know about you" injected into system prompt
   │
   ▼
run_agent_turn  (mimir_agent.rs — model-agnostic loop)
   │   think → pick tool → execute → observe → repeat (cap ~6 tool calls)
   │        tools: search_mimir · get_facts · set_fact · read_tree
   │   on hard failure ─────────────────────────────┐
   ▼                                                 ▼
final answer (+ citations)                  fallback: classic one-shot synthesis
   │                                        (today's path, never worse than now)
   ▼
persist to mimir_chat_messages (+ sources)
   │
   └─ if message count crossed ~20 → enqueue OrchestratorJob::ConsolidateSession
                                          └─ then ExtractLongtermFacts
```

Separately, background events write facts with no agent involvement:
`on_checkpoint_completed` → `mastered_concept`; `on_resource_completed` → `resource_studied`.

---

## Component 1 — Memory substrate

New migration `src-tauri/migrations/049_mimir_memory.sql`.

### `mimir_memory_facts` (long-term memory, unified)

One table for user/tree/project facts (the original spec's separate user-vs-tree tables are merged).
The collision bug in the original spec is fixed with an `entity_id` column + an expression unique index.

```sql
CREATE TABLE IF NOT EXISTS mimir_memory_facts (
    id          TEXT PRIMARY KEY,
    scope       TEXT NOT NULL CHECK(scope IN ('user','tree','project')),
    tree_id     TEXT REFERENCES trees(id) ON DELETE CASCADE,
    project_id  TEXT REFERENCES projects(id) ON DELETE CASCADE,
    fact_key    TEXT NOT NULL,
    entity_id   TEXT,                    -- node_id / resource_id / etc; NULL for singular facts
    fact_value  JSONB NOT NULL,
    confidence  FLOAT NOT NULL DEFAULT 0.7 CHECK(confidence >= 0.0 AND confidence <= 1.0),
    source      TEXT NOT NULL CHECK(source IN (
                    'agent_extraction','checkpoint_complete','resource_complete',
                    'user_said','manual','session_consolidation')),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Collision-safe uniqueness: same (scope+owner+key+entity) upserts in place;
-- different entities (e.g. two mastered concepts) coexist. COALESCE handles NULL owners/entities.
CREATE UNIQUE INDEX IF NOT EXISTS idx_memory_facts_unique ON mimir_memory_facts (
    scope, COALESCE(tree_id,''), COALESCE(project_id,''), fact_key, COALESCE(entity_id,'')
);
CREATE INDEX IF NOT EXISTS idx_memory_facts_tree  ON mimir_memory_facts(tree_id);
CREATE INDEX IF NOT EXISTS idx_memory_facts_scope ON mimir_memory_facts(scope);
```

**Why `entity_id` matters:** `set_fact('mastered_concept', entity_id=node_A)` and
`set_fact('mastered_concept', entity_id=node_B)` become two rows. Singular facts like
`prefers_format = "video"` use `entity_id = NULL` and update in place. Upserts target the
expression index via `ON CONFLICT (scope, COALESCE(tree_id,''), COALESCE(project_id,''), fact_key, COALESCE(entity_id,'')) DO UPDATE …`.

### `mimir_memory_shortterm` (short-term memory)

```sql
CREATE TABLE IF NOT EXISTS mimir_memory_shortterm (
    id              TEXT PRIMARY KEY,
    session_id      TEXT NOT NULL REFERENCES mimir_chat_sessions(id) ON DELETE CASCADE,
    summary         TEXT NOT NULL,
    key_topics      TEXT[] NOT NULL DEFAULT '{}',
    covered_through TIMESTAMPTZ,        -- created_at of the last message included (incremental marker)
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(session_id)                  -- one rolling summary row per session
);
CREATE INDEX IF NOT EXISTS idx_memory_shortterm_topics  ON mimir_memory_shortterm USING GIN(key_topics);
```

`covered_through` lets consolidation process only messages newer than the last pass (incremental).

### Cut from the original spec

- **`mimir_memory_working`** — dropped. It duplicated `mimir_chat_messages`, which already holds
  full per-session history. No function ever read it.

All PKs are app-generated `TEXT` (uuid v4 from Rust), matching existing convention — no DB-side
`gen_random_uuid()`.

---

## Component 2 — Model-agnostic LLM provider

New in `src-tauri/src/mimir_agent.rs`.

- **Config-selected model.** An `AgentModelConfig { base_url, api_key, model }` resolved from env
  (`MIMIR_AGENT_MODEL`, `MIMIR_AGENT_BASE_URL`, `MIMIR_AGENT_API_KEY`), defaulting to
  Groq LLaMA 3.3-70b via `GROQ_API_KEY`. Swapping to Gemini/Claude/GPT is a config change only.
- **One tool schema, many providers.** Tools are declared once in the **OpenAI function-calling
  schema**. Groq and OpenRouter (Gemini, Claude, GPT, Llama…) all accept the same
  `tools` array and return `tool_calls`, so the loop code is provider-independent.
- **Shared client.** Uses the existing `reqwest::Client` Tauri state — never a per-call client.
- **Caveat (documented, not solved here):** model-agnostic means *any tool-calling-capable* model.
  Models without function-calling would need a ReAct text-parsing fallback — explicitly out of scope
  for day one.

---

## Component 3 — The agent loop

New `run_agent_turn` in `mimir_agent.rs`.

- **Loop:** think → choose tool(s) → execute → feed result back as an observation → repeat.
  Hard cap **~6 tool calls per turn**, then the model must produce a final answer.
- **Errors are observations, not crashes.** A failing tool returns an error string the model sees,
  so it can retry or route around it.
- **Fallback safety net:** if the loop hard-fails (provider down, malformed responses, cap with no
  answer), fall back to today's one-shot synthesis path. Worst case ≥ current chat quality.
- **Recall seed:** before the loop, inject a compact "known facts about you / this tree" block
  (top N facts by confidence) into the system prompt, so Mimir remembers you even on turns where it
  doesn't call `get_facts`. This preserves the passive "it remembers me" feel *and* gives the agent
  active recall.
- **Citations + session unchanged:** `search_mimir` returns sources; the final answer carries them
  into `mimir_chat_messages.sources` exactly as today. Session create/update logic is untouched.

---

## Component 4 — The four tools

Registered in a tool registry (name → OpenAI schema + async executor).

| Tool | Behavior | Backed by |
|---|---|---|
| `search_mimir` | Run the full hybrid RAG + GraphRAG + rerank pipeline, return passages **with citations** | Refactor of existing `mimir_retrieval.rs` retrieval into a callable |
| `get_facts` | Return memory facts for the current user + tree (optionally filtered by `fact_key`) | `mimir_memory::get_memory_context` |
| `set_fact` | Write a durable fact (`fact_key`, `fact_value`, optional `entity_id`, `confidence`) with `source='agent_extraction'` | `mimir_memory::set_memory_fact` |
| `read_tree` | Return tree structure + progress (phases → skills → checkpoints, unlock/progress state) | New small tree-scoped read in `mimir_agent.rs` — **note:** existing read models key on `project_id`, but chat context carries `tree_id`; resolve via `trees.project_id` or query `tree_nodes` directly by `tree_id` |

**Deferred, not day one:** `delete_fact` (destructive — waits for the human-review gate),
`web_search` / `smart_fetch` (Hound sidecar, later phase).

---

## Component 5 — Distillation pipeline

New `OrchestratorJob` variants in `orchestrator.rs`; logic in `mimir_memory.rs`.

```rust
OrchestratorJob::ConsolidateSession { session_id: String }
// note: extraction is chained *inside* this job's handler (consolidate → extract, sequential),
// not a second enqueued variant — the worker loop holds no sender to re-enqueue with, and the
// chain is strictly sequential anyway.
```

- **Trigger:** after an agent turn is persisted, if the session's message count crossed a ~20
  multiple, enqueue `ConsolidateSession`. Its handler runs consolidation, then fact extraction.
  No cron — it rides the existing tokio mpsc job queue (cap 64).
- **`consolidate_session` (incremental):** each session has **one rolling summary row** in
  `mimir_memory_shortterm` (not one row per pass). Read only messages with
  `created_at > covered_through` (the newest ~20) plus the existing summary as context → one Groq
  call → update that row's `summary`, union new `key_topics` in (cap ~12), advance `covered_through`.
  First pass inserts the row.
  Feed user messages in full; feed the *answer text* of Mimir replies (not the raw retrieved chunks)
  to keep tokens low. Follows the `auto_tag_single` call pattern (Groq, small `max_tokens`).
- **`extract_longterm_facts`:** read recent short-term summaries → one LLM call →
  parse a JSON array of `{fact_key, fact_value, confidence}` → upsert each via `set_memory_fact`
  with `source='session_consolidation'`. Works off summaries, not raw messages — another layer cheaper.

---

## Component 6 — Auto-memory from events

In `orchestrator.rs`, with the collision fix applied (facts accumulate, not overwrite):

- **`on_checkpoint_completed`:** after existing logic, `set_memory_fact(scope='tree', tree_id,
  fact_key='mastered_concept', entity_id=node_id, value={title,node_id}, confidence=0.9,
  source='checkpoint_complete')`.
- **`on_resource_completed`:** for each linked tree, `set_memory_fact(scope='tree', tree_id,
  fact_key='resource_studied', entity_id=resource_id, value={title,resource_id}, confidence=0.8,
  source='resource_complete')`.

`entity_id` keying means every distinct checkpoint/resource produces its own durable fact.

---

## Tauri commands (register in `main.rs`)

```rust
get_memory_context_cmd(tree_id) -> MemoryContext
set_memory_fact_cmd(tree_id, fact_key, fact_value, entity_id?, confidence?, source?) -> String  // fact id
delete_memory_fact_cmd(fact_id) -> ()          // manual/user delete only (not exposed to the agent)
consolidate_session_cmd(session_id) -> ()      // manual trigger for testing
```

All response structs use `#[serde(rename_all = "camelCase")]`. Commands return `Result<T,String>`,
`database: State<'_, Database>` last, `$N` placeholders, non-macro `sqlx::query()`.

---

## Files

**Create**
- `src-tauri/migrations/049_mimir_memory.sql`
- `src-tauri/src/mimir_memory.rs` — facts CRUD, consolidation + extraction, Tauri commands
- `src-tauri/src/mimir_agent.rs` — model-agnostic loop, tool registry, tool schemas + executors

**Modify**
- `src-tauri/src/main.rs` — `mod mimir_memory; mod mimir_agent;` + register commands
- `src-tauri/src/mimir_retrieval.rs` — expose retrieval as a callable for `search_mimir`; route
  `mimir_chat` through `run_agent_turn` by default, with fallback to current synthesis.
  **⚠ Highest-risk step of the build:** `mimir_chat` is a ~1,300-line function; extracting the
  retrieval pipeline into a callable without changing its behavior is the refactor most likely to
  introduce regressions. Do it as a pure extraction (no logic changes), verify the classic path
  still works *before* wiring the agent, and keep citations/logging identical. Sources returned by
  multiple `search_mimir` calls in one turn are **accumulated and deduped** before persisting to
  `mimir_chat_messages.sources`.
- `src-tauri/src/orchestrator.rs` — new job variants + handlers; event fact writes
- Frontend: minimal. Chat UI stays; optional subtle "using tools…" indicator via a `ygg-*` event.

**Env (derived in `dev.sh`, no `.env`)**
- `MIMIR_AGENT_MODEL` (default `llama-3.3-70b-versatile`), `MIMIR_AGENT_BASE_URL`,
  `MIMIR_AGENT_API_KEY` (default to Groq).

---

## Error handling

- Any tool failure → error string returned as an observation; loop continues.
- Loop exhausts cap without answering → force a final synthesis from gathered context.
- Whole loop errors → fallback to classic one-shot synthesis (never below current behavior).
- LLM JSON parse failures in extraction → skip that fact, log, continue (fire-and-forget).
- Every agent turn still writes `prompt_logs` + `mimir_retrieval_logs` as today.

---

## Verification checklist

- [ ] `cargo build` passes, no new warnings of substance.
- [ ] Migration 049 applies cleanly on startup; tables + indexes exist.
- [ ] Ask Mimir a knowledge question → it calls `search_mimir`, answer includes citations.
- [ ] Complete a checkpoint → a `mastered_concept` fact row appears (keyed by node).
- [ ] Complete a second checkpoint → a *second* `mastered_concept` row (no overwrite — collision fix).
- [ ] Complete a resource → a `resource_studied` fact row appears.
- [ ] Tell Mimir something durable ("I'm switching to ML") → a `set_fact` write with `source='agent_extraction'`.
- [ ] New chat turn shows Mimir recalling a prior fact (recall seed / `get_facts`).
- [ ] Cross ~20 messages → `mimir_memory_shortterm` row created with summary + topics; `covered_through` set.
- [ ] `extract_longterm_facts` produces `session_consolidation` facts from summaries.
- [ ] Force a tool error → loop recovers; force a loop failure → classic synthesis fallback fires.
- [ ] Swap `MIMIR_AGENT_MODEL` to a Gemini/Claude model → loop still runs unchanged.

---

## Build order (for the implementation plan)

1. Migration 049.
2. `mimir_memory.rs` — schema structs, facts CRUD (with collision-safe upsert), Tauri commands.
3. `mimir_agent.rs` — model-agnostic provider + loop + tool registry; wire the four tools.
4. Refactor retrieval into a callable; route `mimir_chat` through the agent with fallback.
5. Distillation jobs (`ConsolidateSession`, `ExtractLongtermFacts`) + triggers.
6. Event fact writes in `on_checkpoint_completed` / `on_resource_completed`.
7. Register everything in `main.rs`; `cargo build`; run the verification checklist.

---

## Explicitly out of scope (later phases)

- Human-in-the-loop review gate for agent writes; `delete_fact` as an agent tool (Phase 4).
- Web search / anti-bot fetch / PDF OCR via Hound sidecar (Phase 3).
- Concept graphs as first-class tables + graph verbs (Phase 1, Graphify-inspired).
- Sub-agents with isolated context / routing (Phase 4).
- Project scanning via tree-sitter (Phase 5).
- External chat import (Perplexity/Claude) writing facts through the extension.

See `docs/superpowers/reference/2026-07-23-deepagents-hound-graphify.md` for the full roadmap.
