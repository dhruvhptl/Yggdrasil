# Human-in-the-Loop for Destructive Agent Actions (Phase 4 — HITL only) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give Mimir three destructive tools (`delete_fact`, `delete_resource`, `merge_skills`) that HALT for explicit user approval before executing — the agent proposes, a UI card confirms, and only then does a separate Tauri command perform the action.

**Architecture:** A new `hitl.rs` owns `ActionProposal`, the pure `requires_approval(name,args)` gate, and `execute_destructive_action_cmd` (the approved-execution path with name/title→id resolvers). `run_agent_turn` intercepts a destructive tool call and returns early with a `pending_approval` proposal instead of executing; `mimir_chat` threads that through to a new `MimirChatResponse.pending_approval` field the frontend renders as a confirmation card. Sub-agents are explicitly deferred.

**Tech Stack:** Rust (Tauri 2, sqlx non-macro, serde_json), React/TypeScript frontend.

**Spec:** `docs/superpowers/specs/2026-07-23-hitl-destructive-actions-design.md` — its **Grounding corrections** section is binding.

## Global Constraints

- SQL placeholders ALWAYS `$N`; non-macro `sqlx::query()` + `try_get`; no `query!` macros.
- Tauri commands return `Result<T, String>`; `database: State<'_, Database>` is the LAST parameter.
- Response/proposal structs use `#[serde(rename_all = "camelCase")]`.
- `requires_approval` is PURE: `(name: &str, args: &serde_json::Value) -> Option<ActionProposal>` — no DB, no ctx.
- A destructive tool call HALTS the turn (no execute, no further loop iterations); nothing executes until `execute_destructive_action_cmd` is called on approval.
- No migrations, no new tables, no changes to `mimir_memory.rs` schema, `orchestrator.rs`, `hound_client.rs`, `concept_graph.rs`, `dev.sh`. No sub-agent apparatus.
- `delete_fact` works by `fact_key`(+optional `entity_id`) scoped to tree (if `tree_id`) else user — NOT by fact id (`get_facts` never exposes ids).
- No Tauri event is emitted (mimir_chat has no `AppHandle`); the frontend reads `pendingApproval` from the `mimir_chat` invoke return value.
- All Rust shell commands run from `C:/Users/dhruv/projects/Yggdrasil/src-tauri`; frontend paths under `C:/Users/dhruv/projects/Yggdrasil/src`; branch `fix-compilation-errors`.
- Tests before this phase: 21 (memory 5, agent 6, concept_graph 7, hound_client 3). This phase adds 3 hitl tests and updates one agent test → 24.
- `cargo build` can take minutes; do not kill it.

---

### Task 1: `hitl.rs` — ActionProposal + `requires_approval`

**Files:**
- Create: `src-tauri/src/hitl.rs`
- Modify: `src-tauri/src/main.rs` (add `mod hitl;` after `mod hound_client;`)

**Interfaces:**
- Produces (used by Tasks 2–5): `#[derive(Debug, Clone, Serialize, Deserialize)] pub(crate) struct ActionProposal { action_type: String, summary: String, params: serde_json::Value }` (camelCase serde); `pub(crate) fn requires_approval(name: &str, args: &serde_json::Value) -> Option<ActionProposal>`.

- [ ] **Step 1: Create the module with the type + failing tests**

Create `src-tauri/src/hitl.rs`:

```rust
// src-tauri/src/hitl.rs
//
// Human-in-the-loop gate for destructive agent tools. requires_approval() is a
// pure classifier: destructive tool call → a proposal the user must approve
// before execute_destructive_action_cmd() performs it. The agent never executes
// these directly; run_agent_turn halts and surfaces the proposal.

use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::Row;
use tauri::State;

use crate::database::Database;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ActionProposal {
    pub action_type: String,
    pub summary: String,
    pub params: serde_json::Value,
}

/// Pure. Some(proposal) iff `name` is a destructive tool. Summary is built from
/// args alone (no DB). Unknown/read-only tools → None.
pub(crate) fn requires_approval(name: &str, args: &serde_json::Value) -> Option<ActionProposal> {
    match name {
        "delete_fact" => {
            let key = args["fact_key"].as_str().unwrap_or("(unspecified)");
            Some(ActionProposal {
                action_type: "delete_fact".into(),
                summary: format!("delete the fact '{}'", key),
                params: args.clone(),
            })
        }
        "delete_resource" => {
            let title = args["title"].as_str().unwrap_or("(unspecified)");
            Some(ActionProposal {
                action_type: "delete_resource".into(),
                summary: format!("delete the resource '{}'", title),
                params: args.clone(),
            })
        }
        "merge_skills" => {
            let source = args["source"].as_str().unwrap_or("(unspecified)");
            let target = args["target"].as_str().unwrap_or("(unspecified)");
            Some(ActionProposal {
                action_type: "merge_skills".into(),
                summary: format!("merge skill '{}' into '{}'", source, target),
                params: args.clone(),
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destructive_tools_yield_proposals() {
        let p = requires_approval("delete_fact", &json!({ "fact_key": "career_goal" })).unwrap();
        assert_eq!(p.action_type, "delete_fact");
        assert_eq!(p.summary, "delete the fact 'career_goal'");

        let r = requires_approval("delete_resource", &json!({ "title": "RRF paper" })).unwrap();
        assert_eq!(r.action_type, "delete_resource");
        assert!(r.summary.contains("RRF paper"));

        let m = requires_approval("merge_skills", &json!({ "source": "SQL", "target": "Databases" })).unwrap();
        assert_eq!(m.action_type, "merge_skills");
        assert_eq!(m.summary, "merge skill 'SQL' into 'Databases'");
    }

    #[test]
    fn readonly_tools_yield_none() {
        for n in ["search_mimir", "get_facts", "set_fact", "read_tree", "query_graph",
                  "path_between", "explain_node", "smart_search", "smart_fetch"] {
            assert!(requires_approval(n, &json!({})).is_none(), "{} should not require approval", n);
        }
    }

    #[test]
    fn missing_args_still_proposes_safely() {
        let p = requires_approval("delete_fact", &json!({})).unwrap();
        assert!(p.summary.contains("(unspecified)"));
    }
}
```

Add `mod hitl;` to `main.rs` after `mod hound_client;`.

Note: `sqlx::Row`, `State`, `Database`, `json` (non-test) are used by `execute_destructive_action_cmd` added in Task 4. Until then they're unused → warnings only (acceptable). If a warning-free intermediate is preferred, add `#![allow(unused_imports)]`-free approach is fine; leave the imports — Task 4 consumes them.

- [ ] **Step 2: Run the tests**

Run: `cargo test hitl 2>&1 | tail -8`
Expected: 3 passed (`destructive_tools_yield_proposals`, `readonly_tools_yield_none`, `missing_args_still_proposes_safely`).

- [ ] **Step 3: Build**

Run: `cargo build 2>&1 | tail -5` → `Finished` (unused-import warnings for the Task-4 items are fine).

- [ ] **Step 4: Commit**

```bash
git add src/hitl.rs src/main.rs
git commit -m "feat(hitl): ActionProposal + pure requires_approval gate"
```

---

### Task 2: Destructive tool schemas + defensive arms + `AgentTurnResult.pending_approval`

**Files:**
- Modify: `src-tauri/src/mimir_agent.rs`

**Interfaces:**
- Consumes: `crate::hitl::{requires_approval, ActionProposal}` (Task 1).
- Produces (used by Task 3): `AgentTurnResult` gains `pub pending_approval: Option<crate::hitl::ActionProposal>`. `tool_schemas(bool)` now also returns the 3 destructive schemas (always).

- [ ] **Step 1: Update the Phase 3 tool-count test first (it will fail to compile/assert)**

In `mimir_agent.rs`, replace the test `tool_schemas_registers_web_tools_only_when_available` body with the new expectations (7 base + 3 destructive always; +2 web when available):

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
                 "query_graph", "path_between", "explain_node",
                 "delete_fact", "delete_resource", "merge_skills"]
        );

        let with_web: Vec<String> = tool_schemas(true)
            .iter()
            .filter_map(|s| s["function"]["name"].as_str().map(|x| x.to_string()))
            .collect();
        assert_eq!(with_web.len(), 12);
        assert_eq!(with_web[7], "smart_search");
        assert_eq!(with_web[8], "smart_fetch");
        assert_eq!(with_web[9], "delete_fact");
        assert_eq!(with_web[10], "delete_resource");
        assert_eq!(with_web[11], "merge_skills");
    }
```

Run: `cargo test mimir_agent 2>&1 | tail -6`
Expected: FAIL — `tool_schemas(false)` currently returns only 7 names (the destructive schemas don't exist yet).

- [ ] **Step 2: Append the 3 destructive schemas in `tool_schemas`**

In `tool_schemas`, the function currently builds `let mut schemas = vec![ ...7 base... ];` then `if hound_available { schemas.push(smart_search); schemas.push(smart_fetch); }` then `schemas`. Insert the 3 destructive pushes AFTER the `if hound_available` block and BEFORE `schemas`:

```rust
    schemas.push(json!({
        "type": "function",
        "function": {
            "name": "delete_fact",
            "description": "Delete a stored memory fact. REQUIRES user approval before it executes — the user will be shown a confirmation.",
            "parameters": {
                "type": "object",
                "properties": {
                    "fact_key": { "type": "string", "description": "The fact key to delete, e.g. 'career_goal'." },
                    "entity_id": { "type": "string", "description": "Optional: the entity id if the fact is entity-scoped (e.g. a node/resource id)." }
                },
                "required": ["fact_key"]
            }
        }
    }));
    schemas.push(json!({
        "type": "function",
        "function": {
            "name": "delete_resource",
            "description": "Delete a resource from the user's library. REQUIRES user approval before it executes.",
            "parameters": {
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "The title of the resource to delete." }
                },
                "required": ["title"]
            }
        }
    }));
    schemas.push(json!({
        "type": "function",
        "function": {
            "name": "merge_skills",
            "description": "Merge one skill into another (the source is absorbed into the target). REQUIRES user approval before it executes.",
            "parameters": {
                "type": "object",
                "properties": {
                    "source": { "type": "string", "description": "The skill to absorb (by name)." },
                    "target": { "type": "string", "description": "The skill to keep (by name)." }
                },
                "required": ["source", "target"]
            }
        }
    }));
```

- [ ] **Step 3: Add defensive `execute_tool` arm (unreachable — the loop intercepts first)**

In `execute_tool`, directly before the `other => Err(format!("unknown tool '{}'", other)),` arm, add:

```rust
        "delete_fact" | "delete_resource" | "merge_skills" => {
            Err(format!("{} requires user approval and cannot execute directly", name))
        }
```

- [ ] **Step 4: Add `pending_approval` to `AgentTurnResult` + fill it at existing return sites**

Change the struct:

```rust
pub(crate) struct AgentTurnResult {
    pub answer: String,
    pub sources: Vec<crate::mimir::MimirChatSource>,
    pub stats: crate::mimir_retrieval::RetrievalStats,
    pub tool_calls_made: u32,
    pub pending_approval: Option<crate::hitl::ActionProposal>,
}
```

In `run_agent_turn`, the successful-answer return (the `AgentStep::Answer` arm that returns `Ok(AgentTurnResult { ... })`) must now set `pending_approval: None`. Add that field to that struct literal. (The max-iterations path returns `Err`, so it needs no change.)

- [ ] **Step 5: Build + test**

Run: `cargo build 2>&1 | tail -5` → `Finished`, no errors.
Run: `cargo test mimir_agent 2>&1 | tail -6` → the tool-count test passes; all mimir_agent tests green.

- [ ] **Step 6: Commit**

```bash
git add src/mimir_agent.rs
git commit -m "feat(hitl): destructive tool schemas + defensive arms + AgentTurnResult.pending_approval"
```

---

### Task 3: Interception in `run_agent_turn` + response plumbing

**Files:**
- Modify: `src-tauri/src/mimir_agent.rs` (interception in the loop)
- Modify: `src-tauri/src/mimir.rs` (`MimirChatResponse.pending_approval`)
- Modify: `src-tauri/src/mimir_retrieval.rs` (thread it through `mimir_chat`)

**Interfaces:**
- Consumes: `crate::hitl::requires_approval`, `AgentTurnResult.pending_approval` (Tasks 1–2).

- [ ] **Step 1: Intercept destructive calls in the loop**

In `run_agent_turn`, inside the `AgentStep::ToolCalls(calls) => { messages.push(assistant_msg); for call in calls { ... } }` block, make the FIRST statement of the `for call in calls` body an approval check (before the budget check and before `execute_tool`):

```rust
                for call in calls {
                    if let Some(proposal) = crate::hitl::requires_approval(&call.name, &call.arguments) {
                        crate::brain::log_prompt_call(
                            pool_for_log.clone(), "mimir_hitl_proposal", &cfg.model, "mimir_hitl_v1",
                            t0.elapsed().as_millis() as i64, true, None,
                            Some(json!({ "action_type": proposal.action_type })),
                        );
                        return Ok(AgentTurnResult {
                            answer: format!("I'd like to {}. Approve?", proposal.summary),
                            sources: dedup_sources(state.sources),
                            stats: state.stats,
                            tool_calls_made: state.tool_calls_made,
                            pending_approval: Some(proposal),
                        });
                    }
                    // ... existing budget check + execute_tool + observation push ...
                }
```

(`t0`, `cfg`, `pool_for_log`, `state` are all already in scope in `run_agent_turn`. `dedup_sources` and `json!` are already used in this file.)

- [ ] **Step 2: Add `pending_approval` to `MimirChatResponse`**

In `mimir.rs`, `MimirChatResponse` (currently `{ answer, sources, suggestions }`, with `#[serde(rename_all="camelCase")]`) gains:

```rust
    pub pending_approval: Option<crate::hitl::ActionProposal>,
```

- [ ] **Step 3: Thread `pending_approval` through `mimir_chat`**

In `mimir_retrieval.rs` `mimir_chat`, the resolution tuple is currently:

```rust
    let (answer, sources, stats) = match agent_outcome {
        Some(r) => (r.answer, r.sources, r.stats),
        None => { /* classic fallback */ (answer, retrieval.sources, retrieval.stats) }
    };
```

Change it to a 4-tuple carrying `pending_approval`:

```rust
    let (answer, sources, stats, pending_approval) = match agent_outcome {
        Some(r) => (r.answer, r.sources, r.stats, r.pending_approval),
        None => {
            // classic fallback (unchanged body) ...
            (answer, retrieval.sources, retrieval.stats, None)
        }
    };
```

(The classic fallback branch's internal code is unchanged; only the returned tuple gains the trailing `None`.)

Then the final return:

```rust
    Ok(MimirChatResponse { answer, sources, suggestions, pending_approval })
```

- [ ] **Step 4: Build + full test suite**

Run: `cargo build 2>&1 | tail -5` → `Finished`, no errors.
Run: `cargo test 2>&1 | tail -6` → 24 passed (memory 5, agent 6, concept_graph 7, hound_client 3, hitl 3).

- [ ] **Step 5: Commit**

```bash
git add src/mimir_agent.rs src/mimir.rs src/mimir_retrieval.rs
git commit -m "feat(hitl): halt turn on destructive call; thread pending_approval to MimirChatResponse"
```

---

### Task 4: `execute_destructive_action_cmd` + resolvers

**Files:**
- Modify: `src-tauri/src/hitl.rs` (add the command)
- Modify: `src-tauri/src/main.rs` (register `hitl::execute_destructive_action_cmd`)

**Interfaces:**
- Consumes: `crate::skill_commands::merge_skills_inner(canonical_id, alias_ids: &[String], pool)` (skill_commands.rs) — confirm it is `pub(crate)`; if it is private, change its visibility to `pub(crate)` in the same commit.

- [ ] **Step 1: Add the command to `hitl.rs`**

Append to `hitl.rs` (above the `#[cfg(test)]` module):

```rust
// ─── Approved execution ──────────────────────────────────────────────────────

/// Execute a previously-approved destructive action. Resolves human-friendly
/// args (keys/titles/names) to rows and performs the action. Idempotent-ish:
/// a target that no longer exists returns a friendly "nothing matched".
#[tauri::command]
pub async fn execute_destructive_action_cmd(
    action_type: String,
    params: serde_json::Value,
    tree_id: Option<String>,
    database: State<'_, Database>,
) -> Result<String, String> {
    match action_type.as_str() {
        "delete_fact" => {
            let fact_key = params["fact_key"].as_str().unwrap_or("").trim().to_string();
            if fact_key.is_empty() {
                return Err("delete_fact requires fact_key".into());
            }
            let entity_id = params["entity_id"].as_str().filter(|s| !s.is_empty());
            // Scope: tree if tree_id present, else user. entity_id optional filter.
            let result = sqlx::query(
                "DELETE FROM mimir_memory_facts \
                 WHERE fact_key = $1 \
                   AND ( (scope = 'tree' AND tree_id = $2) OR (scope = 'user' AND $2 IS NULL) ) \
                   AND ($3::text IS NULL OR entity_id = $3)"
            )
            .bind(&fact_key)
            .bind(&tree_id)
            .bind(entity_id)
            .execute(&database.pool)
            .await
            .map_err(|e| e.to_string())?;
            let n = result.rows_affected();
            if n == 0 {
                Ok(format!("No fact matched '{}' — nothing deleted.", fact_key))
            } else {
                Ok(format!("Deleted {} fact(s) for '{}'.", n, fact_key))
            }
        }
        "delete_resource" => {
            let title = params["title"].as_str().unwrap_or("").trim().to_string();
            if title.is_empty() {
                return Err("delete_resource requires title".into());
            }
            let row = sqlx::query("SELECT id FROM mimir_resources WHERE title ILIKE $1 LIMIT 1")
                .bind(&title)
                .fetch_optional(&database.pool)
                .await
                .map_err(|e| e.to_string())?;
            let Some(row) = row else {
                return Ok(format!("No resource titled '{}' — nothing deleted.", title));
            };
            let id: String = row.try_get("id").map_err(|e| e.to_string())?;
            // CASCADE removes chunks/embeddings/node_links (see 003_mimir.sql).
            sqlx::query("DELETE FROM mimir_resources WHERE id = $1")
                .bind(&id)
                .execute(&database.pool)
                .await
                .map_err(|e| e.to_string())?;
            Ok(format!("Deleted resource '{}'.", title))
        }
        "merge_skills" => {
            let source = params["source"].as_str().unwrap_or("").trim().to_string();
            let target = params["target"].as_str().unwrap_or("").trim().to_string();
            if source.is_empty() || target.is_empty() {
                return Err("merge_skills requires source and target".into());
            }
            let source_id = resolve_skill_id(&database.pool, &source).await?;
            let target_id = resolve_skill_id(&database.pool, &target).await?;
            let (Some(source_id), Some(target_id)) = (source_id, target_id) else {
                return Ok(format!(
                    "Could not resolve both skills ('{}' → '{}') — nothing merged.",
                    source, target
                ));
            };
            if source_id == target_id {
                return Ok("Source and target are the same skill — nothing merged.".into());
            }
            crate::skill_commands::merge_skills_inner(&target_id, &[source_id], &database.pool)
                .await
                .map_err(|e| e.to_string())?;
            Ok(format!("Merged skill '{}' into '{}'.", source, target))
        }
        other => Err(format!("unknown destructive action '{}'", other)),
    }
}

/// Resolve a skill name (or alias) to a universal_skills id, case-insensitive.
async fn resolve_skill_id(pool: &sqlx::PgPool, name: &str) -> Result<Option<String>, String> {
    if let Some(row) = sqlx::query("SELECT id FROM universal_skills WHERE LOWER(name) = LOWER($1) LIMIT 1")
        .bind(name)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?
    {
        return Ok(row.try_get("id").ok());
    }
    let row = sqlx::query(
        "SELECT canonical_skill_id AS id FROM skill_aliases WHERE LOWER(alias) = LOWER($1) LIMIT 1"
    )
    .bind(name)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(row.and_then(|r| r.try_get::<String, _>("id").ok()))
}
```

- [ ] **Step 2: Ensure `merge_skills_inner` is callable**

Check `skill_commands.rs`: `merge_skills_inner` is declared `async fn merge_skills_inner(...)`. If it is not `pub(crate)`, change its signature to `pub(crate) async fn merge_skills_inner(...)`. Include that file in this commit if changed.

- [ ] **Step 3: Register the command in `main.rs`**

In `generate_handler![]`, add:

```rust
            hitl::execute_destructive_action_cmd,
```

- [ ] **Step 4: Build + test**

Run: `cargo build 2>&1 | tail -5` → `Finished`, no errors (the Task-1 unused-import warnings for `Row`/`State`/`Database`/`json` are now resolved — they're used here).
Run: `cargo test 2>&1 | tail -6` → 24 passed.

- [ ] **Step 5: Commit**

```bash
git add src/hitl.rs src/main.rs src/skill_commands.rs
git commit -m "feat(hitl): execute_destructive_action_cmd with key/title/name resolvers"
```

---

### Task 5: Frontend confirmation card

**Files:**
- Create: `src/components/HitlConfirmation.tsx`
- Modify: the chat UI that renders `mimir_chat` responses (find it: `src/components/MimirChat.tsx` and/or `src/contexts/MimirContext.tsx`)

**Interfaces:**
- Consumes: `MimirChatResponse.pendingApproval: { actionType, summary, params } | null` and the Tauri command `execute_destructive_action_cmd`.

- [ ] **Step 1: Read the chat component to find where responses are rendered**

Read `src/components/MimirChat.tsx` and `src/contexts/MimirContext.tsx`. Identify (a) the `invoke<MimirChatResponse>('mimir_chat', ...)` call site and the message/response state shape, and (b) where an assistant message is rendered. Note the exact names in your report — Steps 2–3 adapt to them.

- [ ] **Step 2: Create the confirmation card component**

Create `src/components/HitlConfirmation.tsx`:

```tsx
import { invoke } from '@tauri-apps/api/core';

export interface ActionProposal {
  actionType: string;
  summary: string;
  params: Record<string, unknown>;
}

interface Props {
  proposal: ActionProposal;
  treeId: string | null;
  onResolved: (resultMessage: string) => void;
}

export function HitlConfirmation({ proposal, treeId, onResolved }: Props) {
  const approve = async () => {
    try {
      const result = await invoke<string>('execute_destructive_action_cmd', {
        actionType: proposal.actionType,
        params: proposal.params,
        treeId: treeId,
      });
      onResolved(result);
    } catch (e) {
      onResolved(`Action failed: ${String(e)}`);
    }
  };
  const reject = () => onResolved("Okay, I won't do that.");

  return (
    <div className="hitl-card" style={{
      border: '1px solid var(--border, #444)', borderRadius: 8, padding: 12, margin: '8px 0',
    }}>
      <div style={{ marginBottom: 8 }}>
        <strong>Mimir wants to:</strong> {proposal.summary}
      </div>
      <div style={{ display: 'flex', gap: 8 }}>
        <button onClick={approve}>Approve</button>
        <button onClick={reject}>Reject</button>
      </div>
    </div>
  );
}
```

- [ ] **Step 3: Render the card when a response carries `pendingApproval`**

In the chat component, when a `mimir_chat` response has a non-null `pendingApproval`, render `<HitlConfirmation proposal={resp.pendingApproval} treeId={treeId ?? null} onResolved={...} />` beneath the assistant message. In `onResolved`, append the returned string as a new assistant/system line and clear the pending card. Use the component's existing message-append mechanism (identified in Step 1) and the `treeId` already available to the chat (the same value passed as `treeId` to `mimir_chat`).

(Exact wiring depends on the chat component's state shape from Step 1 — follow its existing patterns. Keep the `MimirChatResponse` TS type in sync: add `pendingApproval?: ActionProposal | null`.)

- [ ] **Step 4: Type-check / build the frontend**

Run: `cd C:/Users/dhruv/projects/Yggdrasil && npm run build 2>&1 | tail -15` (or the project's `tsc`/vite check). Expected: no TypeScript errors. If the project has no standalone typecheck script, ensure `npx tsc --noEmit` passes.

- [ ] **Step 5: Commit**

```bash
git add src/components/HitlConfirmation.tsx src/components/MimirChat.tsx src/contexts/MimirContext.tsx
git commit -m "feat(hitl): frontend confirmation card for destructive actions"
```

(Adjust the staged file list to whatever Step 3 actually touched.)

---

### Task 6: Final verification (inline — controller runs this)

**Files:** none — verification only.

- [ ] **Step 1: Rust build + tests**

Run: `cargo build 2>&1 | tail -5` → `Finished`, no errors.
Run: `cargo test 2>&1 | tail -8` → 24 passed, 0 failed.

- [ ] **Step 2: Diff review**

Run: `git log --oneline -6` → the task commits.
Run: `git diff <base>..HEAD --stat` (base = commit before Task 1) → expect: `src/hitl.rs`, `src/main.rs`, `src/mimir_agent.rs`, `src/mimir.rs`, `src/mimir_retrieval.rs`, `src/skill_commands.rs` (only if `merge_skills_inner` visibility changed), and the frontend files. No migrations.

- [ ] **Step 3: Runtime checklist (requires `bash dev.sh` — user-driven; do NOT fake)**

- [ ] Ask Mimir to delete a fact → response carries `pendingApproval`; card appears; NOTHING deleted yet. Check `SELECT count(*) FROM mimir_memory_facts WHERE fact_key='<key>';` unchanged.
- [ ] Approve → `execute_destructive_action_cmd` runs; the fact row is gone; the card resolves with the result message.
- [ ] Reject → fact unchanged; no backend call.
- [ ] Ask to delete a resource by title → propose → approve → resource (and its chunks/embeddings via CASCADE) gone.
- [ ] Ask to merge two skills by name → propose → approve → source merged into target (verify via skills page / `universal_skills`).
- [ ] Approve a stale target (delete an already-deleted fact) → friendly "nothing matched", no crash.
- [ ] A normal read-only question → no card, answer as usual; all 9 non-destructive tools still work.

- [ ] **Step 4: Hand off**

Use superpowers:finishing-a-development-branch to decide keep/merge/PR with the user. (No Fable review this phase — deferred to the consolidated v3 audit.)

---

## Deferred (not in this plan)

Sub-agents (coordinator routing, isolated per-mode context, per-sub-agent summarization) — need a
migration (`mimir_memory_shortterm` is `UNIQUE(session_id)`) + a `run_agent_turn` tool-filtering change;
revisit when context pollution is felt. Undo/approval history, batch approvals, more destructive tools.
