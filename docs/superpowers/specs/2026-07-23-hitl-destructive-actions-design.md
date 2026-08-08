# Human-in-the-Loop for Destructive Agent Actions (Phase 4 — moderate scope)

**Date:** 2026-07-23
**Status:** Design approved — ready for implementation plan
**Scope decision:** HITL only. Sub-agents (coordinator routing + isolated context + per-sub-agent
summarization) are DEFERRED — they require a migration (`mimir_memory_shortterm` is `UNIQUE(session_id)`)
and a `run_agent_turn` tool-filtering change, for a benefit (cleaner context) that isn't yet felt in a
single-user app. Revisit when context pollution is a real problem. This spec is the moderate version
the Phase 4 design author recommended.
**Depends on:** Phase 0 (agent loop, `execute_tool`, `run_agent_turn`, `ToolCtx`, `MimirChatResponse`).

---

## Goal

Give Mimir the ability to perform destructive actions — `delete_fact`, `delete_resource`,
`merge_skills` — safely, by requiring explicit user approval in the UI before anything executes.
The agent *proposes*; the user *confirms*; only then does it run.

Today these three operations are NOT agent tools (they were deliberately deferred in Phase 0 precisely
because they're destructive). This phase adds them AND the approval gate together — the gate is what
makes adding them safe.

---

## Decisions locked during brainstorming

| Fork | Decision | Why |
|---|---|---|
| HITL interface | **Structured UI card (proposal returned in the chat response + `ygg-requires-approval` event), not conversational.** | Faster and safer than "May I?"→"Yes" round-trips. |
| Destructive set (day one) | **`delete_fact`, `delete_resource`, `merge_skills`.** | The only truly destructive operations. `set_fact` accumulates (non-destructive); searches are read-only. |
| Pause semantics | **A destructive tool call HALTS the turn.** The agent does not execute it, does not continue the loop; it returns a confirmation question + the proposal. | Destructive actions are rare and deserve to be their own explicit step, not buried mid-answer. |
| Execution path | **Separate Tauri command `execute_destructive_action_cmd(action_type, params, tree_id)`.** The frontend calls it on Approve; it performs the real resolution + action. | Decouples proposal from execution; the proposal never carries the power to act. |
| No-frontend fallback | **The proposal is still returned in the response; a caller with no card can invoke `execute_destructive_action_cmd` directly.** | HITL never blocks; execution is always an explicit, separate call. |
| Argument shapes | **LLM-friendly (names/keys, not raw UUIDs).** `delete_fact{fact_key, entity_id?}`, `delete_resource{title}`, `merge_skills{source, target}`. Resolution to ids happens at execution. | `get_facts`/`search_mimir` expose keys/titles to the model, not ids (grounding fact). |

---

## Grounding corrections (verified against Phase 0/3 code 2026-07-23 — AUTHORITATIVE)

1. **`delete_fact` works by key, not id.** The `get_facts` tool returns `{factKey, value, confidence, source, entityId}` — NOT the fact id. `delete_memory_fact(pool, fact_id)` needs an id. So the destructive `delete_fact` tool takes `{fact_key, entity_id?}`, and `execute_destructive_action_cmd` resolves + deletes within the current scope (tree if `tree_id` present, else user) via SQL, not via the id-based helper.
2. **`requires_approval` is pure `(name, &args) -> Option<ActionProposal>`** — no DB, no `ctx`. The human `summary` is built from args alone (e.g. `"Delete fact 'career_goal'"`). Interception happens in `run_agent_turn`'s tool-dispatch loop, before `execute_tool` is called.
3. **`AgentTurnResult` and `MimirChatResponse` each gain `pending_approval: Option<ActionProposal>`** (current fields: AgentTurnResult `{answer, sources, stats, tool_calls_made}`; MimirChatResponse `{answer, sources, suggestions}`). serde `#[serde(rename_all="camelCase")]` ⇒ frontend sees `pendingApproval`.
4. **Backend fns to reuse in the command:** `delete_memory_fact` / direct SQL delete (mimir_memory.rs), `merge_skills_inner(canonical_id, alias_ids: &[String], pool)` (skill_commands.rs:1559), `delete_mimir_resource` (mimir_manage.rs:106) or its inner. Skill/resource resolution (name/title → id) is new SQL in the command.
5. **No migration, no new tables.** Proposals are ephemeral (returned in the response, not stored). `mimir_memory_shortterm` is untouched (that was the sub-agent concern, now deferred).
6. **`tool_schemas(hound_available: bool)`** (Phase 3) gets the 3 destructive schemas appended UNCONDITIONALLY (independent of Hound). They're always available; the gate — not registration — is what protects them.

---

## Architecture

```
Agent turn (run_agent_turn loop)
    │  model requests a tool call
    ├─ requires_approval(name, args)?
    │     ├─ None  → execute_tool as normal (unchanged)
    │     └─ Some(proposal) → HALT: return AgentTurnResult {
    │                            answer: "I'd like to <summary>. Approve?",
    │                            pending_approval: Some(proposal), ... }
    ▼
mimir_chat
    ├─ if pending_approval: emit `ygg-requires-approval` (payload=proposal)
    └─ MimirChatResponse { ..., pending_approval }
    ▼
Frontend HitlConfirmation card  →  Approve → invoke execute_destructive_action_cmd(action_type, params, tree_id)
                                   Reject  → discard (optional follow-up message)
```

---

## Component 1 — `hitl.rs` (new module)

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ActionProposal {
    pub action_type: String,        // "delete_fact" | "delete_resource" | "merge_skills"
    pub summary: String,            // human-readable, built from args
    pub params: serde_json::Value,  // the tool arguments, replayed on approval
}

/// Pure. Returns Some(proposal) iff the tool is destructive. No DB, no ctx.
pub(crate) fn requires_approval(name: &str, args: &serde_json::Value) -> Option<ActionProposal>;
// delete_fact  → summary "Delete fact '<fact_key>'"
// delete_resource → summary "Delete resource '<title>'"
// merge_skills → summary "Merge skill '<source>' into '<target>'"
// any other name → None
```

Unit tests: each destructive name yields a proposal with the right action_type + summary + params; a
non-destructive name (`search_mimir`, `set_fact`, `smart_search`) yields None; missing args → a safe
summary placeholder, still Some.

## Component 2 — Destructive tool schemas + defensive arms (`mimir_agent.rs`)

- `tool_schemas(hound_available)` appends 3 schemas unconditionally: `delete_fact{fact_key, entity_id?}`,
  `delete_resource{title}`, `merge_skills{source, target}`. Descriptions state the action requires user approval.
- `execute_tool` gets 3 defensive arms for these names returning
  `Err("<name> requires approval and cannot execute directly")` — normally UNREACHABLE (the loop
  intercepts first), present only so a stray direct call can't act.

## Component 3 — Interception + result plumbing (`mimir_agent.rs`)

- `AgentTurnResult` gains `pub pending_approval: Option<ActionProposal>` (default `None` on the normal answer path).
- In `run_agent_turn`, in the loop where tool calls are dispatched: for each requested call, first
  `if let Some(proposal) = crate::hitl::requires_approval(&call.name, &call.arguments)` → build a
  confirmation `answer` (`format!("I'd like to {}. Approve?", proposal.summary.to_lowercase-ish)`), and
  `return Ok(AgentTurnResult { answer, sources: dedup(state.sources), stats: state.stats, tool_calls_made: state.tool_calls_made, pending_approval: Some(proposal) })`. Halt — no execute, no further iterations.
- Log the proposal to `prompt_logs` (command `"mimir_hitl_proposal"`), fire-and-forget.

## Component 4 — Response plumbing (`mimir_retrieval.rs`, `mimir.rs`)

- `MimirChatResponse` gains `pub pending_approval: Option<ActionProposal>` (camelCase → `pendingApproval`).
- `mimir_chat`: thread `pending_approval` from the `AgentTurnResult` through the `(answer, sources, stats)`
  resolution (make it `(answer, sources, stats, pending_approval)`; classic fallback path sets `None`).
  If `Some`, `app.emit("ygg-requires-approval", &proposal)` (mimir_chat has the `AppHandle`? — if not,
  the frontend reads it from the response field; the event is a convenience, use whichever handle is in
  scope, else rely solely on the response field). Set it on the returned `MimirChatResponse`.

## Component 5 — `execute_destructive_action_cmd` (`hitl.rs`, registered in `main.rs`)

```rust
#[tauri::command]
pub async fn execute_destructive_action_cmd(
    action_type: String,
    params: serde_json::Value,
    tree_id: Option<String>,
    database: State<'_, Database>,
) -> Result<String, String>
```
- `delete_fact` → `DELETE FROM mimir_memory_facts WHERE fact_key=$1 AND ((scope='tree' AND tree_id=$2) OR (scope='user' AND $2 IS NULL)) [AND entity_id=$3 when provided]`; return count deleted.
- `delete_resource` → resolve `title` → id (`SELECT id FROM mimir_resources WHERE title ILIKE $1 LIMIT 1`), then delete via the existing resource-delete path; return the title deleted or "not found".
- `merge_skills` → resolve `source`/`target` names → `universal_skills.id` (name or alias, case-insensitive), then `merge_skills_inner(target_id, &[source_id], &database.pool)`; return a summary. If either name doesn't resolve → friendly error.
- Every branch returns a human-readable result string; unknown `action_type` → `Err`.

## Component 6 — Frontend (`src/components/HitlConfirmation.tsx` + chat integration)

- A confirmation card: shows `pendingApproval.summary`, an **Approve** and a **Reject** button.
- Rendered by the chat UI when a `MimirChatResponse` carries `pendingApproval` (and/or on the
  `ygg-requires-approval` event).
- **Approve** → `invoke('execute_destructive_action_cmd', { actionType, params, treeId })`, then show
  the returned result as a system/assistant line.
- **Reject** → dismiss the card; optionally post a short "Okay, I won't." line. No backend call.
- Follows existing component conventions (see `MimirChat.tsx`); dev-only nothing special.

---

## Files

**Create:** `src-tauri/src/hitl.rs` (ActionProposal, requires_approval + tests, execute_destructive_action_cmd);
`src/components/HitlConfirmation.tsx`.
**Modify:** `src-tauri/src/main.rs` (`mod hitl;` + register command); `src-tauri/src/mimir_agent.rs`
(3 schemas, 3 defensive arms, `AgentTurnResult.pending_approval`, interception in `run_agent_turn`);
`src-tauri/src/mimir_retrieval.rs` (thread `pending_approval`, set on response, emit event);
`src-tauri/src/mimir.rs` (`MimirChatResponse.pending_approval`); the chat UI (render the card).
**No change:** migrations/tables, `mimir_memory.rs` schema, `orchestrator.rs`, `dev.sh`, `hound_client.rs`,
`concept_graph.rs`, and the whole sub-agent apparatus (deferred).

---

## Error handling

- Non-destructive tools → `requires_approval` returns None → unchanged behavior.
- Destructive tool proposed → turn halts with the proposal; nothing executes until approval.
- Approve on a stale/invalid target (fact already gone, title/skill not found) → command returns a
  friendly "nothing matched" string, not a crash.
- Reject / no approval ever → proposal is ephemeral; nothing happens; the agent moves on next message.
- `merge_skills` name ambiguity → resolve first match; if none, friendly error (no partial merge).

---

## Verification checklist

- `cargo build` clean; `cargo test` passes (requires_approval: each destructive name → proposal, each
  read-only name → None; summaries correct).
- Ask Mimir to delete a fact → turn halts, `MimirChatResponse.pendingApproval` set, `ygg-requires-approval`
  fires, no deletion yet.
- Approve → `execute_destructive_action_cmd` deletes the fact; row gone from `mimir_memory_facts`.
- Reject → fact unchanged; no backend call.
- Ask to delete a resource / merge two skills → same propose→approve→execute flow; correct backend effect.
- A normal read-only question → no proposal, answer as usual; all 9 existing tools still work.
- Approve a stale target (already-deleted fact) → friendly "nothing matched", no crash.

## Build order (for the plan)

1. `hitl.rs` — ActionProposal + `requires_approval` (+ unit tests).
2. `mimir_agent.rs` — 3 schemas + 3 defensive arms + `AgentTurnResult.pending_approval` + interception.
3. `mimir.rs` + `mimir_retrieval.rs` — `MimirChatResponse.pending_approval` + thread through + event.
4. `execute_destructive_action_cmd` (+ resolvers) in `hitl.rs`; register in `main.rs`.
5. Frontend `HitlConfirmation.tsx` + chat integration.
6. `cargo build` + verification checklist.

## Explicitly out of scope (deferred / never)

Sub-agents (coordinator, isolated context, per-sub-agent summarization) — deferred until context
pollution is felt (needs a migration + `run_agent_turn` change). More destructive tools, undo/history of
approvals, batch approvals, confidence-scored proposals — later if needed.
