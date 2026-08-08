# HITL-Gated Action Tools (Agent Step 4) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Give the Mimir agent three action tools — `complete_checkpoint` and `ingest_resource` (both HITL-gated) and `suggest_next` (read-only) — reusing the existing HITL proposal/approval mechanism and the already-generic confirmation card (no frontend change).

**Architecture:** State-mutating tools return an `ActionProposal` (via `requires_approval`) that halts the turn; on approval `execute_destructive_action_cmd` runs the action through newly-extracted inner functions. `suggest_next` executes read-only in `execute_tool`. Existing commands (`update_tree_node` checkpoint path, `get_growth_recommendations`, `ingest_mimir_url`) are refactored to delegate to reusable inner fns so both the command and the agent path share one implementation.

**Tech Stack:** Rust (Tauri 2, sqlx). No frontend changes (the HITL card is generic).

## Global Constraints

- Rust: `$N` placeholders; non-macro `sqlx::query()` for NEW queries; commands return `Result<T,String>` with `database`/State params. Structs `#[serde(rename_all="camelCase")]`.
- Reuse existing HITL: `hitl::requires_approval` (pure classifier) + `hitl::execute_destructive_action_cmd` (approved executor). The frontend `HitlConfirmation` card is generic — do NOT change it.
- Extractions must keep the existing command working (command delegates to the new inner fn); do not change existing command signatures except `execute_destructive_action_cmd` (which gains Tauri-injected `app`/`client`/`queue` — the frontend invoke passes none of those, so it is unaffected).
- No new crates or migrations.
- Work on branch `fix-compilation-errors`; commit locally, do NOT push.
- **Build/test in the isolated target dir.** Before any cargo command:
  `export CARGO_TARGET_DIR="C:/Users/dhruv/AppData/Local/Temp/claude/C--Users-dhruv-projects-Yggdrasil/497ed12f-520a-4774-9c54-620ab075f017/scratchpad/scan-target"`
- **Task 3 (ingest_resource) is the isolated-risk task** — it refactors the large `ingest_mimir_url`. Keep that extraction behavior-preserving (the command must behave identically after delegating).

## File Structure

| File | Change | Task |
|---|---|---|
| `src-tauri/src/tree_commands.rs` | `complete_checkpoint_inner`; `update_tree_node` delegates its 100% path to it | 1 |
| `src-tauri/src/read_models.rs` | `get_growth_recommendations_inner`; command delegates | 1 |
| `src-tauri/src/hitl.rs` | `requires_approval` + `execute_destructive_action_cmd` arms (checkpoint, suggest N/A); signature gains app/client/queue; tests | 2, 3 |
| `src-tauri/src/mimir_agent.rs` | `tool_schemas` 3 tool schemas; `execute_tool` arms (2 HITL-gated Err + `suggest_next`) | 2, 3 |
| `src-tauri/src/mimir_ingest.rs` | `ingest_url_inner`; `ingest_mimir_url` delegates | 3 |
| `src-tauri/src/main.rs` | (no new command; `execute_destructive_action_cmd` already registered) | — |

---

### Task 1: Extract reusable inner functions

**Files:**
- Modify: `src-tauri/src/tree_commands.rs` (add `complete_checkpoint_inner`; `update_tree_node`:200-220 delegates)
- Modify: `src-tauri/src/read_models.rs` (`get_growth_recommendations` :1152 → `_inner`)

**Interfaces:**
- Consumes: `crate::orchestrator::on_checkpoint_completed(pool, app, node_id, tree_id, queue)` (already pub; called at tree_commands.rs:215).
- Produces: `complete_checkpoint_inner(pool, app, queue, node_id, tree_id) -> Result<(),String>`; `get_growth_recommendations_inner(pool, season) -> Result<Vec<GrowthTarget>,String>`.

- [ ] **Step 1: `complete_checkpoint_inner`**

Add to `tree_commands.rs` (near `update_tree_node`):
```rust
/// Mark a leaf checkpoint 100% and run the unlock/skill cascade. Shared by the
/// update_tree_node command path and the agent's complete_checkpoint action.
pub(crate) async fn complete_checkpoint_inner(
    pool: &sqlx::PgPool,
    app: &tauri::AppHandle,
    queue: &crate::orchestrator::JobQueue,
    node_id: &str,
    tree_id: &str,
) -> Result<(), String> {
    sqlx::query("UPDATE tree_nodes SET progress = 100 WHERE id = $1 AND tree_id = $2")
        .bind(node_id)
        .bind(tree_id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    crate::orchestrator::on_checkpoint_completed(pool, app, node_id, tree_id, queue).await;
    Ok(())
}
```
(Leave `update_tree_node` as-is — it already sets progress and calls `on_checkpoint_completed`. This inner fn is the shared entry the agent path will use; do not rip out the working command logic.)

- [ ] **Step 2: `get_growth_recommendations_inner`**

In `read_models.rs`, rename the current `get_growth_recommendations` body into an inner fn and make the command delegate. Change the command (`:1152`) to:
```rust
#[tauri::command]
pub async fn get_growth_recommendations(
    season: Option<String>,
    database: State<'_, Database>,
) -> Result<Vec<GrowthTarget>, String> {
    get_growth_recommendations_inner(&database.pool, season).await
}

pub(crate) async fn get_growth_recommendations_inner(
    pool: &sqlx::PgPool,
    season: Option<String>,
) -> Result<Vec<GrowthTarget>, String> {
    // ... the entire existing body, with every `&database.pool` / `pool = &database.pool`
    //     replaced by the `pool` parameter ...
}
```
Move the whole existing body verbatim into `_inner`, replacing the local `let pool = &database.pool;` with using the `pool` param directly (delete that line and keep `pool` referring to the arg). Verify no other `database.` reference remains in the moved body.

- [ ] **Step 3: Build + test**

Run: `cargo test --manifest-path src-tauri/Cargo.toml 2>&1 | tail -12`
Expected: build clean; existing tests pass.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/tree_commands.rs src-tauri/src/read_models.rs
git commit -m "refactor: complete_checkpoint_inner + get_growth_recommendations_inner"
```

---

### Task 2: complete_checkpoint (HITL) + suggest_next (read-only)

**Files:**
- Modify: `src-tauri/src/hitl.rs` (`requires_approval` add `complete_checkpoint`; `execute_destructive_action_cmd` signature gains `app`/`client`/`queue` + a `complete_checkpoint` arm; tests)
- Modify: `src-tauri/src/mimir_agent.rs` (`tool_schemas` add `complete_checkpoint` + `suggest_next`; `execute_tool` add `complete_checkpoint` to the HITL-gated Err arm + a `suggest_next` arm)

**Interfaces:**
- Consumes: `complete_checkpoint_inner`, `get_growth_recommendations_inner` (Task 1).
- Produces: `execute_destructive_action_cmd(action_type, params, tree_id, app, client, queue, database)` (new injected params).

- [ ] **Step 1: `requires_approval` — add `complete_checkpoint`**

In `hitl.rs::requires_approval`, add a match arm (before `_ => None`):
```rust
        "complete_checkpoint" => {
            let node = args["node_title"].as_str()
                .or_else(|| args["node_id"].as_str())
                .unwrap_or("(unspecified)");
            Some(ActionProposal {
                action_type: "complete_checkpoint".into(),
                summary: format!("mark the checkpoint '{}' complete", node),
                params: args.clone(),
            })
        }
```

- [ ] **Step 2: Extend the executor signature + add the arm**

Change `execute_destructive_action_cmd` to gain Tauri-injected params (put `database` last):
```rust
pub async fn execute_destructive_action_cmd(
    action_type: String,
    params: serde_json::Value,
    tree_id: Option<String>,
    app: tauri::AppHandle,
    client: State<'_, reqwest::Client>,
    queue: State<'_, crate::orchestrator::JobQueue>,
    database: State<'_, Database>,
) -> Result<String, String> {
```
Add a `complete_checkpoint` arm (before `other =>`):
```rust
        "complete_checkpoint" => {
            let Some(tree_id) = tree_id.as_deref().filter(|s| !s.is_empty()) else {
                return Ok("No active tree — open a tree first.".to_string());
            };
            // Resolve node id: explicit node_id, else a leaf whose title matches within this tree.
            let node_id = if let Some(nid) = params["node_id"].as_str().filter(|s| !s.is_empty()) {
                nid.to_string()
            } else {
                let title = params["node_title"].as_str().unwrap_or("").trim().to_string();
                if title.is_empty() { return Err("complete_checkpoint requires node_title or node_id".into()); }
                let row = sqlx::query(
                    "SELECT id FROM tree_nodes WHERE tree_id = $1 AND type = 'leaf' AND title ILIKE $2 LIMIT 1"
                )
                .bind(tree_id).bind(&title)
                .fetch_optional(&database.pool).await.map_err(|e| e.to_string())?;
                let Some(row) = row else {
                    return Ok(format!("No checkpoint titled '{}' in this tree — nothing done.", title));
                };
                row.try_get::<String, _>("id").map_err(|e| e.to_string())?
            };
            crate::tree_commands::complete_checkpoint_inner(&database.pool, &app, &queue, &node_id, tree_id).await?;
            Ok("Checkpoint marked complete — progress cascaded.".to_string())
        }
```
(The `client` param is unused here; Task 3's `ingest_resource` arm uses it. The `#[allow(unused_variables)]`-style concern: `client` will be used by Task 3; if the compiler warns about unused `client` at the end of Task 2, that's acceptable — a pre-existing-style warning — or bind `let _ = &client;` at the top of the match if you prefer. Do NOT remove the param.)

- [ ] **Step 3: `tool_schemas` — add the two schemas (always available)**

In `tool_schemas`, before the final return (unconditional — not behind a flag), push:
```rust
    tools.push(json!({
        "type": "function",
        "function": {
            "name": "complete_checkpoint",
            "description": "Mark a checkpoint (leaf node) in the current tree as complete. Requires user approval.",
            "parameters": { "type": "object", "properties": {
                "node_title": { "type": "string", "description": "Title of the checkpoint to complete." }
            }, "required": ["node_title"] }
        }
    }));
    tools.push(json!({
        "type": "function",
        "function": {
            "name": "suggest_next",
            "description": "Recommend what the user should learn next, weighted by their saved job requirements. Read-only.",
            "parameters": { "type": "object", "properties": {} }
        }
    }));
```

- [ ] **Step 4: `execute_tool` arms**

Add `complete_checkpoint` to the existing HITL-gated Err arm:
```rust
        "delete_fact" | "delete_resource" | "merge_skills" | "complete_checkpoint" => {
            Err(format!("{} requires user approval and cannot execute directly", name))
        }
```
Add a `suggest_next` arm (executes read-only):
```rust
        "suggest_next" => {
            let targets = crate::read_models::get_growth_recommendations_inner(ctx.pool, None)
                .await
                .unwrap_or_default();
            if targets.is_empty() {
                Ok("No growth recommendations yet — add some job applications so I can weight suggestions by demand.".to_string())
            } else {
                let lines: Vec<String> = targets.iter().take(5)
                    .map(|t| format!("- {}", t.skill_name))
                    .collect();
                Ok(format!("Suggested next skills to focus on:\n{}", lines.join("\n")))
            }
        }
```
(Confirm the field on `GrowthTarget` — use its skill-name field; read `read_models.rs` for the exact field name, e.g. `skill_name`. Adjust the `format!` accordingly.)

- [ ] **Step 5: Update the HITL test**

In `hitl.rs` tests, extend `destructive_tools_yield_proposals` to assert `complete_checkpoint` proposes, and `readonly_tools_yield_none` to include `suggest_next` (it must NOT require approval):
```rust
        let c = requires_approval("complete_checkpoint", &json!({ "node_title": "Learn RRF" })).unwrap();
        assert_eq!(c.action_type, "complete_checkpoint");
        assert!(c.summary.contains("Learn RRF"));
```
and add `"suggest_next"` to the readonly loop list.

- [ ] **Step 6: Build + test**

Run: `cargo test --manifest-path src-tauri/Cargo.toml 2>&1 | tail -14`
Expected: build clean; all tests pass (updated HITL tests + existing).

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/hitl.rs src-tauri/src/mimir_agent.rs
git commit -m "feat(agent): complete_checkpoint (HITL) + suggest_next tools"
```

---

### Task 3: ingest_resource (isolated-risk refactor)

**Files:**
- Modify: `src-tauri/src/mimir_ingest.rs` (`ingest_mimir_url`:541 → extract `ingest_url_inner`, command delegates)
- Modify: `src-tauri/src/hitl.rs` (`requires_approval` add `ingest_resource`; executor arm; test)
- Modify: `src-tauri/src/mimir_agent.rs` (`tool_schemas` add `ingest_resource`; `execute_tool` add it to the HITL-gated Err arm)

**Interfaces:**
- Consumes: the existing `ingest_mimir_url` body.
- Produces: `ingest_url_inner(pool, client, queue, app, url, title, force_dynamic, parent_id) -> Result<IngestResult,String>`.

- [ ] **Step 1: Extract `ingest_url_inner`**

Move the entire body of `ingest_mimir_url` (mimir_ingest.rs:541-…) into:
```rust
pub(crate) async fn ingest_url_inner(
    pool: &sqlx::PgPool,
    client: &reqwest::Client,
    queue: &crate::orchestrator::JobQueue,
    app: &tauri::AppHandle,
    url: &str,
    title: Option<&str>,
    force_dynamic: bool,
    parent_id: Option<&str>,
) -> Result<IngestResult, String> {
    // ... existing body, with substitutions:
    //   &database.pool  -> pool
    //   &*client / client.inner()  -> client
    //   &queue / queue (State)  -> queue
    //   &app / app  -> app
    //   url (String)  -> url (&str) — adjust .bind(&url) etc. as needed
    //   title: Option<String>  -> title: Option<&str>
    //   force_dynamic.unwrap_or(false)  -> force_dynamic
    //   parent_id: Option<String>  -> parent_id: Option<&str>
}
```
Then make the command a thin delegator (behavior identical):
```rust
#[tauri::command]
pub async fn ingest_mimir_url(
    url: String,
    title: Option<String>,
    force_dynamic: Option<bool>,
    parent_id: Option<String>,
    app: tauri::AppHandle,
    client: tauri::State<'_, reqwest::Client>,
    database: State<'_, Database>,
    queue: tauri::State<'_, crate::orchestrator::JobQueue>,
) -> Result<IngestResult, String> {
    ingest_url_inner(
        &database.pool, &client, &queue, &app,
        &url, title.as_deref(), force_dynamic.unwrap_or(false), parent_id.as_deref(),
    ).await
}
```
**This must be behavior-preserving** — the command still fetches, dedups, chunks, embeds, and calls `on_resource_ingested_async` exactly as before. Verify by reading the moved body for any missed `database.`/State usage.

- [ ] **Step 2: `requires_approval` — add `ingest_resource`**

In `hitl.rs::requires_approval` (before `_ => None`):
```rust
        "ingest_resource" => {
            let url = args["url"].as_str().unwrap_or("(unspecified)");
            Some(ActionProposal {
                action_type: "ingest_resource".into(),
                summary: format!("add '{}' to your library", url),
                params: args.clone(),
            })
        }
```

- [ ] **Step 3: Executor arm for `ingest_resource`**

In `execute_destructive_action_cmd` (the signature already gained `client`/`queue`/`app` in Task 2), add before `other =>`:
```rust
        "ingest_resource" => {
            let url = params["url"].as_str().unwrap_or("").trim().to_string();
            if url.is_empty() { return Err("ingest_resource requires url".into()); }
            let title = params["title"].as_str().filter(|s| !s.is_empty());
            crate::mimir_ingest::ingest_url_inner(
                &database.pool, &client, &queue, &app, &url, title, false, None,
            ).await?;
            Ok(format!("Added '{}' to your library — tagging + matching will run in the background.", url))
        }
```

- [ ] **Step 4: Agent tool schema + gated arm**

In `tool_schemas`, push (unconditional):
```rust
    tools.push(json!({
        "type": "function",
        "function": {
            "name": "ingest_resource",
            "description": "Add a URL to the user's resource library (fetches + indexes it). Requires user approval.",
            "parameters": { "type": "object", "properties": {
                "url": { "type": "string", "description": "The URL to add." },
                "title": { "type": "string", "description": "Optional title." }
            }, "required": ["url"] }
        }
    }));
```
Add `ingest_resource` to the HITL-gated Err arm:
```rust
        "delete_fact" | "delete_resource" | "merge_skills" | "complete_checkpoint" | "ingest_resource" => {
            Err(format!("{} requires user approval and cannot execute directly", name))
        }
```

- [ ] **Step 5: Test + build**

Add to the HITL `destructive_tools_yield_proposals` test:
```rust
        let ir = requires_approval("ingest_resource", &json!({ "url": "https://example.com/post" })).unwrap();
        assert_eq!(ir.action_type, "ingest_resource");
        assert!(ir.summary.contains("example.com"));
```
Run: `cargo test --manifest-path src-tauri/Cargo.toml 2>&1 | tail -14`
Expected: build clean; all tests pass.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/mimir_ingest.rs src-tauri/src/hitl.rs src-tauri/src/mimir_agent.rs
git commit -m "feat(agent): ingest_resource tool (HITL) + ingest_url_inner extraction"
```

---

### Task 4: Live verification (runtime checklist)

**Files:** none.

- [ ] **Step 1:** `bash dev.sh`. In a tree's Mimir chat: "mark the checkpoint 'X' complete" → an Approve/Reject card appears (summary "mark the checkpoint 'X' complete"); Approve → the node hits 100% and unlocks cascade (check the tree); Reject → nothing changes.
- [ ] **Step 2:** "what should I learn next?" → `suggest_next` returns a list immediately (no approval card), weighted by saved jobs (or the friendly empty message if no jobs).
- [ ] **Step 3:** "add https://… to my library" → Approve/Reject card; Approve → the resource appears in the Library and the tag/match pipeline runs in the background; the existing `ingest_mimir_url` UI path still works unchanged (ingest a URL from the Resources page).
- [ ] **Step 4:** Reject each action → confirm nothing is mutated.

---

## Self-Review

**Spec coverage (Step 4a-4e):**
- 4a inner fns: `complete_checkpoint_inner` (Task 1), `ingest_url_inner` (Task 3). `get_growth_recommendations_inner` added for suggest_next (Task 1). ✓
- 4b schemas: complete_checkpoint + ingest_resource (HITL), suggest_next (read-only) → Tasks 2-3. ✓
- 4c `requires_approval`: complete_checkpoint + ingest_resource arms (Tasks 2-3); suggest_next NOT added (read-only). ✓
- 4d execute_tool arms: 2 HITL-gated Err + suggest_next executes → Tasks 2-3. ✓
- 4e approved execution in `execute_destructive_action_cmd`: resolve title→node_id / url, call inner fns → Tasks 2-3. ✓

**Placeholder scan:** Task 1 Step 2 and Task 3 Step 1 say "move the existing body with these substitutions" rather than reproducing large existing bodies verbatim — that is a behavior-preserving extraction of code already in the repo, with the exact substitution list given, not a vague placeholder. Task 2 Step 4 flags confirming `GrowthTarget`'s field name from source — a locate-and-confirm, not a TBD.

**Type consistency:** `execute_destructive_action_cmd` signature (with app/client/queue) defined in Task 2, extended-by-use in Task 3 — the `client` param is added in Task 2 (unused until Task 3) so the signature changes exactly once. Action-type strings (`complete_checkpoint`/`ingest_resource`) identical across `requires_approval`, the executor arm, the schema, and the gated Err arm. Inner-fn signatures match between definition (Tasks 1/3) and call sites (executor arms / suggest_next).

**HITL correctness:** complete_checkpoint + ingest_resource are intercepted by `requires_approval` in `run_agent_turn` BEFORE `execute_tool`, so their Err arms are defensive only; suggest_next is read-only and not in `requires_approval`. The generic `HitlConfirmation` card renders any `ActionProposal` and invokes `execute_destructive_action_cmd` with `{actionType, params, treeId}` — the new injected params (app/client/queue) are supplied by Tauri, so no frontend change.

**Note on TDD:** `requires_approval` (pure) gets test coverage (Tasks 2-3); the inner-fn extractions + executor arms are DB/Tauri-runtime code, verified by build/test + the Task 4 runtime checklist (behavior-preserving extractions are compile-checked; the commands' existing behavior is unchanged).

## Explicitly out of scope
Gating tools by "tree open" via a new `tool_schemas` param (arms guard instead); editing/removing checkpoints; ingest of non-URL resources via the agent; a dedicated confirmation UI (the generic card suffices).
