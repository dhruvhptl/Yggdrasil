# Agentic Foundation — Modes, Tool Education, Repo Exploration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Mimir use its own toolkit and afford to use it — teach the model its ~20 tools + a cheap-verb-first doctrine, give the loop mode-aware budgets, replace the silent classic-synthesis fallback with an honest-partial "walk cut short" answer, add local repo-exploration tools (grep / recursive listing / paged reads / git log), and give the agent a persisted working plan.

**Architecture:** A `mode` param (`chat` | `dive`) on `mimir_chat` selects an `AgentLoopConfig` (tool/iteration/timeout budget) and a system-prompt framing block. The think→act→observe loop (`run_agent_turn`) is refactored to take that config, owns its own timeout (so partial state survives exhaustion), and returns an honest-partial answer built from accumulated `AgentTurnState` instead of erroring into classic synthesis. New repo tools reuse the existing `resolve_safe_path` allowlist gate and the scanner's `ignore::WalkBuilder`. A per-session `agent_plans` row is loaded into `ToolCtx.plan` and injected into the prompt each turn.

**Tech Stack:** Rust (Tauri 2, sqlx, Postgres), React 19 + TypeScript. New direct crate: `regex` (grep fallback). Reused: `ignore` 0.4 (already a dep), `std::process::Command` (git log).

## Global Constraints

- **Migrations are append-only, auto-run on startup. Highest on disk today is `059`.** This plan adds exactly one: `060` (agent plans).
- Rust: `$N` placeholders (never `?N`); non-macro `sqlx::query()`; `#[serde(rename_all="camelCase")]` on any struct crossing the Tauri boundary; commands return `Result<T, String>`; `database: State<'_, Database>` (and other `State`) last.
- Never hardcode `DATABASE_URL`; never commit `.env`; env vars come from `dev.sh`/Bitwarden; no `dotenvy` in Rust. JSONB not TEXT.
- **Security (load-bearing): every new repo tool — `search_in_project`, `project_git_log`, `read_file` ranges, `list_project_files` depth — MUST call `crate::project_roots::resolve_safe_path(path, &ctx.project_roots)` FIRST and operate only on the returned safe path.** This is the `054` allowlist + denylist boundary. No path reaches the filesystem ungated.
- **No shell string execution.** `project_git_log` uses `std::process::Command::new("git")` with fixed args and the path passed as a single `.arg()` — never a shell string, never string-interpolated.
- **Honest-partial is locked: on loop exhaustion (iteration cap or the loop's own timeout) NEVER fall through to the silent classic (tool-less) synthesis.** Classic synthesis runs ONLY for a genuine pre-work failure (config unavailable, LLM/parse error, empty answer) or the outer safety-timeout firing on a true hang.
- **Budgets (locked):** `chat` = 10 tool calls / 14 iterations / 120 s; `dive` = 20 / 24 / 240 s. Default when `mode` is absent/unknown = `chat`.
- **Modes (locked):** `chat` (default) and `dive` only — no "skills" dimension. Session-persisted, swappable between messages, no thread lock. Unknown string → `chat`.
- **Tool education is additive:** append the tool map/doctrine; KEEP the FS-roots allowlist line and the Project-Mode grounding line unchanged.
- **Do NOT import, call, or browse the coding agent's Graphify tooling** (`graphify-out/graph.json`, `.opencode/plugins/graphify.js`). It is a dev-time reference pattern only.
- **Build/test in the isolated target dir.** Before any cargo command:
  `export CARGO_TARGET_DIR="C:/Users/dhruv/AppData/Local/Temp/claude/C--Users-dhruv-projects-Yggdrasil/497ed12f-520a-4774-9c54-620ab075f017/scratchpad/scan-target"`
  The crate is a **binary** (`universal-skill-tree`) — run tests with `cargo test --bin universal-skill-tree <name> -- --nocapture` (no lib target). Frontend gate: `npx tsc --noEmit` (exit 0).
- Branch `fix-compilation-errors`; commit locally, do NOT push. No Fable review (one consolidated v3 audit is due after the whole v3 stream).

## Milestones

- **Milestone 1 — Smarter with the tools it already has (Phases 0–2, Tasks 0–2):** loop governance + honest-partial exhaustion + tool education + modes. Observable win with the EXISTING tool set: Mimir reaches for the graph, sustains longer walks, never silently degrades, and `dive` gives deeper budgets + framing. Shippable and testable on its own.
- **Milestone 2 — Deep repo walking (Phases 3–5, Tasks 3–6):** the new local repo tools (grep, git log, paged reads, recursive listing), the agent-owned working plan, and the exploration-trail strip. Depends on Milestone 1's loop governance + budgets.

## Authoritative grounding (re-verified 2026-08-04, post-Project-Mode)

All anchors below confirmed against the current tree (`HEAD` = `c509af0`). Line numbers are **approximate** — every implementer verifies the exact block before editing.

- `tool_schemas(hound_available: bool, fs_tools_available: bool) -> Vec<serde_json::Value>` at `mimir_agent.rs:41`; called once per turn at `:877` as `tool_schemas(ctx.hound_base_url.is_some(), !ctx.project_roots.is_empty())`. Gating unit test at `:993` (`tool_schemas_registers_web_and_fs_tools_only_when_available`) — MUST be updated when tool arity changes.
- Loop constants: `const MAX_TOOL_CALLS: u32 = 6;` (`:859`), `const MAX_ITERATIONS: u32 = 8;` (`:860`). Loop `for _iteration in 0..MAX_ITERATIONS` at `:881`; `force_answer` at `:882`; tool-budget-exhausted push at `:939-945`; per-call count at `:947`; the 500-char UI preview at `:961`; **full observation pushed into `messages` at `:970`**; iteration-exhaustion `Err("agent loop exceeded max iterations without an answer")` at `:985`.
- `run_agent_turn(cfg: &AgentModelConfig, ctx: &ToolCtx<'_>, system_prompt: &str, history: &[serde_json::Value], user_message: &str, pool_for_log: sqlx::PgPool) -> Result<AgentTurnResult, String>` at `:864`.
- `struct AgentTurnResult { answer: String, sources: Vec<MimirChatSource>, stats: RetrievalStats, tool_calls_made: u32, pending_approval: Option<ActionProposal>, tool_calls: Vec<ToolCallRecord>, reasoning: Option<String> }` at `:841`.
- `struct AgentTurnState { sources, stats, tool_calls_made, tool_calls: Vec<ToolCallRecord>, reasoning: Vec<String> }` (`#[derive(Default)]`) at `:465`. `ToolCallRecord` has `tool_name: String`, `input: serde_json::Value`, etc.
- `struct ToolCtx<'a>` at `:433`, current fields end at `project_roots: Vec<ProjectRoot>` (`:446`) and `project_root_id: Option<String>` (`:447`).
- FS tool arms: `read_file` at `mimir_agent.rs:787-793` (whole-file, 100 KB cap, `resolve_safe_path` first); `list_project_files` at `:795-821` (one-level `read_dir`, 200-entry cap, skips `node_modules`/`target`/`.venv`/`.git`/`dist`/`build`). Graph verbs `query_graph`/`path_between`/`explain_node`/`graph_semantic_search` resolve via `graph_for_ctx`, all 6000-char capped.
- Scanner walk to mirror for grep: `ignore::WalkBuilder::new(root).standard_filters(true).filter_entry(|e| !matches!(name, "node_modules"|"target"|"venv"|"__pycache__"|".git")).build()` at `project_scanner.rs:459-465`.
- `mimir_chat(message, page, tree_id, node_id, project_root_id, node_title, project_name, tree_name, turn_id, app, client, queue, hound, database)` at `mimir_retrieval.rs:988` — a `mode: Option<String>` param slots in alongside `project_root_id`.
- Agent system prompt assembled at `mimir_retrieval.rs:1155-1200`: the ~5-tool paragraph at `:1167-1176`, FS-roots line at `:1178-1190`, Project-Mode grounding line at `:1192-1200`.
- Agent run + fallback wrapper at `mimir_retrieval.rs:1212-1259`: `agent_outcome: Option<AgentTurnResult> = None` (`:1212`); `tokio::time::timeout(120s, run_agent_turn(...))` (`:1236-1248`); `Ok(Ok(r)) => agent_outcome = Some(r)` (`:1249`), `Ok(Err(e)) => …classic` (`:1250`), `Err(_) => …classic` (`:1251`), config-unavailable classic (`:1254`); the `match agent_outcome { … None => classic synthesis }` at `:1259+`.
- Session id computed at `mimir_retrieval.rs:~1035` (`resolve_session_id(...)` → `session_id_opt: Option<String>`), available before the `ToolCtx` build at `:~1215`.
- `regex` is **not** a direct dependency (Cargo.toml has `ignore = "0.4"`, tree-sitter crates); Task 3 adds `regex = "1"`.

## File Structure

| File | Change | Phase / Task |
|---|---|---|
| `src-tauri/src/mimir_agent.rs` | `AgentLoopConfig`, `AgentMode`, honest-partial return, internal timeout, tool-map arms, repo tools, plan tools, `ToolCtx.{mode,plan,session_id}`, gating test | 0,2,3,4 |
| `src-tauri/src/mimir_retrieval.rs` | `mode` param, per-mode config, prompt rewrite + plan injection, honest-partial branch, plan load, `ToolCtx` build | 0,1,2,4 |
| `src-tauri/Cargo.toml` | add `regex = "1"` | 3 |
| `src-tauri/migrations/060_agent_plans.sql` | create `agent_plans` | 4 |
| `src/components/MimirChat.tsx` | mode selector, plan block, trail strip | 2,4,5 |
| `src/contexts/MimirContext.tsx` | `mode` persistence | 2 |
| No change | `project_roots.rs` (reused), `orchestrator.rs`, `concept_graph.rs` verbs (reused), `hitl.rs`, `github.rs` | — |

---

## Task 0: Loop governance — mode-aware budgets + honest-partial exhaustion

**Files:**
- Modify: `src-tauri/src/mimir_agent.rs` (`AgentLoopConfig`, `honest_partial_answer`, `run_agent_turn` signature + internal timeout + return, observation cap)
- Modify: `src-tauri/src/mimir_retrieval.rs` (pass config, widen safety timeout, honest-partial branch)

**Interfaces:**
- Produces: `pub(crate) struct AgentLoopConfig { pub max_tool_calls: u32, pub max_iterations: u32, pub timeout_secs: u64 }` with `AgentLoopConfig::chat()` (10/14/120) and `AgentLoopConfig::dive()` (20/24/240); `run_agent_turn(cfg, loop_cfg: AgentLoopConfig, ctx, system_prompt, history, user_message, pool_for_log)`; `honest_partial_answer(state: &AgentTurnState, reason: &str) -> String`.
- Consumes: existing `AgentTurnState`, `AgentTurnResult`.

- [ ] **Step 1: Write the failing tests** (append to `mod tests` in `mimir_agent.rs`):

```rust
#[test]
fn loop_config_presets_have_expected_budgets() {
    let c = AgentLoopConfig::chat();
    assert_eq!((c.max_tool_calls, c.max_iterations, c.timeout_secs), (10, 14, 120));
    let d = AgentLoopConfig::dive();
    assert_eq!((d.max_tool_calls, d.max_iterations, d.timeout_secs), (20, 24, 240));
}

#[test]
fn honest_partial_answer_reports_files_and_findings() {
    let mut state = AgentTurnState::default();
    state.tool_calls_made = 3;
    state.tool_calls.push(ToolCallRecord {
        call_index: 1, tool_name: "read_file".into(), status: "success".into(),
        input: serde_json::json!({"path": "a.rs"}), output: None, duration_ms: None,
    });
    state.sources.push(crate::mimir::MimirChatSource {
        title: "auth.rs".into(), ..Default::default()
    });
    let msg = honest_partial_answer(&state, "timeout");
    assert!(msg.contains("1 file"));            // one file-touching tool call
    assert!(msg.contains("auth.rs"));           // finding surfaced
    assert!(msg.contains("timeout"));           // reason surfaced
    assert!(msg.to_lowercase().contains("cut short"));
}
```

> **Note on `MimirChatSource`:** if it does not `derive(Default)` or its fields differ, construct it with the real required fields (check `mimir.rs`); the assertions only need a `title`. Keep the test honest — a real `MimirChatSource`, not a stub type.

- [ ] **Step 2: Run the tests to verify they fail**

```
export CARGO_TARGET_DIR="C:/Users/dhruv/AppData/Local/Temp/claude/C--Users-dhruv-projects-Yggdrasil/497ed12f-520a-4774-9c54-620ab075f017/scratchpad/scan-target"
cargo test --bin universal-skill-tree loop_config_presets honest_partial_answer -- --nocapture
```
Expected: FAIL (`AgentLoopConfig`/`honest_partial_answer` undefined).

- [ ] **Step 3: Add `AgentLoopConfig` and `honest_partial_answer`** (near the loop, replacing the two `const`s at `:859-860`):

```rust
#[derive(Clone, Copy, Debug)]
pub(crate) struct AgentLoopConfig {
    pub max_tool_calls: u32,
    pub max_iterations: u32,
    pub timeout_secs: u64,
}
impl AgentLoopConfig {
    pub fn chat() -> Self { Self { max_tool_calls: 10, max_iterations: 14, timeout_secs: 120 } }
    pub fn dive() -> Self { Self { max_tool_calls: 20, max_iterations: 24, timeout_secs: 240 } }
}

/// Mechanical (no-LLM) summary returned when the loop is cut short, so the
/// richest path degrades to an honest partial — never a silent tool-less answer.
fn honest_partial_answer(state: &AgentTurnState, reason: &str) -> String {
    const FILE_TOOLS: [&str; 4] =
        ["read_file", "search_in_project", "list_project_files", "project_git_log"];
    let files_touched = state.tool_calls.iter()
        .filter(|c| FILE_TOOLS.contains(&c.tool_name.as_str()))
        .count();
    let titles: Vec<String> = state.sources.iter().take(5).map(|s| s.title.clone()).collect();
    let found = if titles.is_empty() {
        "I haven't assembled a full answer yet".to_string()
    } else {
        format!("So far I found: {}", titles.join("; "))
    };
    format!(
        "I explored {} file(s) across {} tool call(s). {}. The walk was cut short ({}) \
         — ask me to continue and I'll pick up where I left off.",
        files_touched, state.tool_calls_made, found, reason
    )
}
```

- [ ] **Step 4: Refactor `run_agent_turn`** — take the config, own the timeout, return honest-partial on exhaustion.

Change the signature (add `loop_cfg: AgentLoopConfig` after `cfg`):
```rust
pub(crate) async fn run_agent_turn(
    cfg: &AgentModelConfig,
    loop_cfg: AgentLoopConfig,
    ctx: &ToolCtx<'_>,
    system_prompt: &str,
    history: &[serde_json::Value],
    user_message: &str,
    pool_for_log: sqlx::PgPool,
) -> Result<AgentTurnResult, String> {
```
Inside: replace `0..MAX_ITERATIONS` with `0..loop_cfg.max_iterations`; replace every `MAX_TOOL_CALLS` with `loop_cfg.max_tool_calls` (`:882`, `:939`, `:949`). At the TOP of the loop body add an internal timeout check that breaks to honest-partial:
```rust
for _iteration in 0..loop_cfg.max_iterations {
    if t0.elapsed().as_secs() >= loop_cfg.timeout_secs {
        return Ok(exhausted_result(&mut state, "timeout"));
    }
    let force_answer = state.tool_calls_made >= loop_cfg.max_tool_calls;
    ...
```
Reword the tool-budget push (`:943`) copy to: `"Tool budget reached — synthesize an answer now from what you've gathered."` (no behavioral change).

Replace the post-loop `Err(...)` at `:975-985` with an honest-partial return, and factor a small helper so both exit points share it:
```rust
// (bottom of the loop — iterations exhausted)
Ok(exhausted_result(&mut state, "reached the step limit"))
}

fn exhausted_result(state: &mut AgentTurnState, reason: &str) -> AgentTurnResult {
    let answer = honest_partial_answer(state, reason);
    let reasoning = if state.reasoning.is_empty() { None } else { Some(state.reasoning.join("\n\n")) };
    AgentTurnResult {
        answer,
        sources: dedup_sources(std::mem::take(&mut state.sources)),
        stats: std::mem::take(&mut state.stats),
        tool_calls_made: state.tool_calls_made,
        pending_approval: None,
        tool_calls: std::mem::take(&mut state.tool_calls),
        reasoning,
    }
}
```
Keep the `log_prompt_call(... success=false, Some("cut short: {reason}") ...)` telemetry on this path (move it into `exhausted_result`'s callers or keep it inline before returning). **`run_agent_turn` now returns `Err` only for genuine failures** (LLM/parse/`call_agent_llm` `?`, empty answer at `:894`) — those still legitimately fall back to classic.

- [ ] **Step 5: Add the per-observation cap for repo tools** at the `messages.push` site (`:970`). Repo-tool output can be large; cap it so a 20-call walk stays context-sane. The graph verbs (6000) and other tools are already bounded, so cap ONLY the new repo tools by name:
```rust
const MAX_REPO_OBSERVATION_CHARS: usize = 6000;
// … at :970, replace the bare push with:
let obs_for_msg = if matches!(call.name.as_str(), "search_in_project" | "project_git_log") {
    crate::text_util::truncate_chars(&observation, MAX_REPO_OBSERVATION_CHARS)
} else {
    observation.clone()
};
messages.push(json!({ "role": "tool", "tool_call_id": call.id, "content": obs_for_msg }));
```
(The tool names won't exist until Task 3/4 — harmless string match until then. `truncate_chars` is `crate::text_util::truncate_chars(&str, usize) -> String`.)

- [ ] **Step 6: Run the unit tests to verify they pass**

```
cargo test --bin universal-skill-tree loop_config_presets honest_partial_answer -- --nocapture
```
Expected: PASS.

- [ ] **Step 7: Update the caller** in `mimir_retrieval.rs` (`:1236-1251`). Pass `AgentLoopConfig::chat()` for now (Task 2 makes it mode-driven), and widen the OUTER safety timeout to sit above the loop's own timeout so the loop's honest-partial fires first; the outer timeout stays a true-hang backstop → classic:
```rust
let loop_cfg = crate::mimir_agent::AgentLoopConfig::chat(); // Task 2: from mode
let safety = std::time::Duration::from_secs(loop_cfg.timeout_secs + 30);
match tokio::time::timeout(
    safety,
    crate::mimir_agent::run_agent_turn(&agent_cfg, loop_cfg, &tool_ctx, &agent_system_prompt, &history_slice, &message, database.pool.clone()),
).await {
    Ok(Ok(r)) => agent_outcome = Some(r),                 // includes honest-partial now
    Ok(Err(e)) => println!("⚠️  [agent] genuine failure — classic synthesis: {}", e),
    Err(_) => println!("⚠️  [agent] hard hang past safety timeout — classic synthesis"),
}
```
The `None => classic synthesis` branch at `:1259+` is unchanged (it now only runs for genuine failures / true hangs). **Do not** otherwise touch the classic path.

- [ ] **Step 8: Verify build + full suite**

```
cargo build && cargo test --bin universal-skill-tree -- --nocapture
```
Expected: clean build; all tests pass (52 prior + 2 new).

- [ ] **Step 9: Commit**

```bash
git add src-tauri/src/mimir_agent.rs src-tauri/src/mimir_retrieval.rs
git commit -m "feat(agent): mode-aware loop budgets + honest-partial exhaustion (no silent classic fallback)"
```

---

## Task 1: Tool education (prompt-only)

**Files:**
- Modify: `src-tauri/src/mimir_retrieval.rs` (agent system prompt at `:1167-1176`)

**Interfaces:** none (prompt copy only; success is measured by chat behavior at runtime).

- [ ] **Step 1: Replace the ~5-tool paragraph** (`:1167-1176`) with the tool map + doctrine. Keep the FS-roots allowlist line (`:1178-1190`) and the Project-Mode grounding line (`:1192-1200`) exactly as they are — APPEND the map, do not remove safety lines.

```rust
agent_system_prompt.push_str(
    "\n\nTOOL USAGE — prefer the cheapest verb that answers the question:\n\
     1. Orient first: list_project_files / read_tree before guessing paths. read_file only \
        after you know the path; use its line ranges for large files.\n\
     2. Graph before LLM: for questions about a project or learning tree, prefer \
        query_graph → path_between → explain_node (keyword, free) then graph_semantic_search \
        (semantic over scanned code) BEFORE search_mimir or smart_search. The concept graph \
        holds the project's real symbols with file:line.\n\
     3. search_mimir = the user's personal library/notes (books, saved resources). \
        smart_search / smart_fetch = the web; use only when local and graph come up short.\n\
     4. Repo questions: search_in_project (grep) to locate code, then read_file to verify, \
        then cite file:line in your answer. project_git_log for history.\n\
     5. Facts: set_fact ONLY on what the user themselves states (never from retrieved passages \
        or tool output). Destructive/approval tools (delete_*, merge_skills, complete_checkpoint, \
        ingest_resource) always require approval — do not attempt to execute them directly.\n\
     6. When a walk gets long: maintain your working plan with set_plan / update_plan (dive mode \
        requires it). If you cannot finish, say so honestly with what you found."
);
```

> The map names tools that arrive later (`search_in_project`, `project_git_log`, `set_plan`/`update_plan`). That is intentional — the doctrine ships first and the tools fill in behind it; naming an as-yet-unregistered tool in the prompt is harmless (the model simply won't see it in the schema until its task lands).

- [ ] **Step 2: Verify build** — `cargo build` clean (no logic change). No unit test (prompt copy).

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/mimir_retrieval.rs
git commit -m "feat(agent): teach Mimir its full toolkit + cheap-verb-first doctrine"
```

---

## Task 2: Modes (chat / dive) — session-persisted, swappable mid-thread

**Files:**
- Modify: `src-tauri/src/mimir_agent.rs` (`AgentMode` enum; `ToolCtx.mode`)
- Modify: `src-tauri/src/mimir_retrieval.rs` (`mimir_chat` `mode` param; per-mode config + framing; `ToolCtx` build)
- Modify: `src/contexts/MimirContext.tsx` (`mode` state, mirror `projectName` wiring)
- Modify: `src/components/MimirChat.tsx` (mode selector chips; pass `mode`)

**Interfaces:**
- Consumes: `AgentLoopConfig` (Task 0).
- Produces: `pub(crate) enum AgentMode { Chat, Dive }` with `AgentMode::from_param(Option<&str>) -> AgentMode` and `AgentMode::loop_config(&self) -> AgentLoopConfig`; `ToolCtx.mode: AgentMode`.

- [ ] **Step 1: Write the failing test** (`mimir_agent.rs` tests):

```rust
#[test]
fn agent_mode_parses_and_maps_budget() {
    assert!(matches!(AgentMode::from_param(Some("dive")), AgentMode::Dive));
    assert!(matches!(AgentMode::from_param(Some("DIVE")), AgentMode::Dive)); // case-insensitive
    assert!(matches!(AgentMode::from_param(Some("garbage")), AgentMode::Chat));
    assert!(matches!(AgentMode::from_param(None), AgentMode::Chat));
    assert_eq!(AgentMode::Dive.loop_config().max_tool_calls, 20);
    assert_eq!(AgentMode::Chat.loop_config().max_tool_calls, 10);
}
```

- [ ] **Step 2: Run to verify it fails**
```
cargo test --bin universal-skill-tree agent_mode_parses -- --nocapture
```
Expected: FAIL (`AgentMode` undefined).

- [ ] **Step 3: Add `AgentMode`** (in `mimir_agent.rs`, near `AgentLoopConfig`):
```rust
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum AgentMode { Chat, Dive }
impl AgentMode {
    pub fn from_param(s: Option<&str>) -> Self {
        match s.map(|v| v.trim().to_lowercase()).as_deref() {
            Some("dive") => AgentMode::Dive,
            _ => AgentMode::Chat,
        }
    }
    pub fn loop_config(&self) -> AgentLoopConfig {
        match self { AgentMode::Dive => AgentLoopConfig::dive(), AgentMode::Chat => AgentLoopConfig::chat() }
    }
}
```
Add `pub mode: AgentMode,` to `ToolCtx` (`:433` struct).

- [ ] **Step 4: Run the test to verify it passes** — `cargo test --bin universal-skill-tree agent_mode_parses`.

- [ ] **Step 5: Thread `mode` through `mimir_chat`** (`mimir_retrieval.rs:988`). Add `mode: Option<String>` to the params (alongside `project_root_id`; Tauri maps JS `mode`). Near the config/timeout setup:
```rust
let agent_mode = crate::mimir_agent::AgentMode::from_param(mode.as_deref());
let loop_cfg = agent_mode.loop_config();
```
Replace the `AgentLoopConfig::chat()` placeholder from Task 0 Step 7 with this `loop_cfg`. In the `ToolCtx { … }` build (`:~1215`) add `mode: agent_mode,`.

- [ ] **Step 6: Add the dive framing block** to `agent_system_prompt` (after the Task 1 tool map). `chat` appends nothing:
```rust
if agent_mode == crate::mimir_agent::AgentMode::Dive {
    agent_system_prompt.push_str(
        "\n\nYou are in DIVE mode — explore the project deeply before answering: \
         orient, use the graph, grep, read files, and cite file:line. Maintain a working plan \
         with set_plan. Prefer an honest partial answer over a shallow one."
    );
}
```

- [ ] **Step 7: Frontend — `MimirContext.tsx`.** Mirror the `projectName` wiring: add `mode: 'chat' | 'dive'` to the interface, default `'chat'`, a `useState`, include `mode` in `setMimirContext`'s param + assignment + the Provider `value`. (Session-persistence = the mode survives panel reopen because it lives in context, like `projectRootId`.)

- [ ] **Step 8: Frontend — `MimirChat.tsx` selector.** Read `mode` from `useMimirContext()`. Render two chips (Chat / Dive) in the chat header near the project picker; onClick → `setMimirContext({ mode: 'chat' | 'dive' })`. Pass `mode` in the `mimir_chat` invoke args object (`:~340`): add `mode,`. Swappable any time — no thread lock, no disabling mid-turn beyond the existing `loading` guard.

- [ ] **Step 9: Verify** — `npx tsc --noEmit` exit 0; `cargo build` clean; `cargo test --bin universal-skill-tree -- --nocapture` green.

- [ ] **Step 10: Commit**
```bash
git add src-tauri/src/mimir_agent.rs src-tauri/src/mimir_retrieval.rs src/contexts/MimirContext.tsx src/components/MimirChat.tsx
git commit -m "feat(agent): chat/dive modes — session-persisted budget + framing, swappable mid-thread"
```

---

## Task 3: Repo tool — `search_in_project` (local grep)

**Files:**
- Modify: `src-tauri/Cargo.toml` (add `regex = "1"`)
- Modify: `src-tauri/src/mimir_agent.rs` (schema + arm + matcher helpers + tests + gating test)

**Interfaces:**
- Consumes: `resolve_safe_path` (`project_roots.rs`), `ignore::WalkBuilder`, `ToolCtx.{project_roots, project_root_id}`.
- Produces: agent tool `search_in_project { pattern, path?, glob?, context_lines? }`; helpers `has_regex_meta(&str) -> bool`, `line_matches(line, pattern, &Option<regex::Regex>) -> bool`.

- [ ] **Step 1: Add the dependency** — in `src-tauri/Cargo.toml` under `[dependencies]`: `regex = "1"`.

- [ ] **Step 2: Write the failing tests** (`mimir_agent.rs` tests):
```rust
#[test]
fn has_regex_meta_detects_metacharacters() {
    assert!(!has_regex_meta("resolve_safe_path"));   // plain substring
    assert!(has_regex_meta("fn\\s+\\w+"));
    assert!(has_regex_meta("foo|bar"));
    assert!(has_regex_meta("read_.*"));
}

#[test]
fn line_matches_substring_then_regex() {
    // substring path (no regex compiled)
    assert!(line_matches("  let x = resolve_safe_path(p);", "resolve_safe_path", &None));
    assert!(!line_matches("nothing here", "resolve_safe_path", &None));
    // regex path
    let re = Some(regex::Regex::new(r"fn\s+\w+").unwrap());
    assert!(line_matches("pub fn run_agent_turn(", "fn\\s+\\w+", &re));
    assert!(!line_matches("let y = 2;", "fn\\s+\\w+", &re));
}
```

- [ ] **Step 3: Run to verify they fail** — `cargo test --bin universal-skill-tree has_regex_meta line_matches`. Expected FAIL (undefined).

- [ ] **Step 4: Implement the matcher helpers**:
```rust
fn has_regex_meta(p: &str) -> bool {
    p.chars().any(|c| matches!(c, '(' | ')' | '[' | ']' | '{' | '}' | '*' | '+' | '?' | '|' | '^' | '$' | '\\' | '.'))
}
fn line_matches(line: &str, pattern: &str, re: &Option<regex::Regex>) -> bool {
    match re { Some(r) => r.is_match(line), None => line.contains(pattern) }
}
```

- [ ] **Step 5: Run to verify they pass** — `cargo test --bin universal-skill-tree has_regex_meta line_matches`.

- [ ] **Step 6: Add the tool schema** in `tool_schemas` (gated inside the existing `if fs_tools_available { … }` block, like `read_file`):
```rust
schemas.push(json!({
    "type": "function",
    "function": {
        "name": "search_in_project",
        "description": "Grep the registered project for a pattern. Substring match by default; \
            regex when the pattern contains metacharacters. Returns file:line: matches. Use this to \
            locate code, then read_file to verify and cite.",
        "parameters": { "type": "object", "properties": {
            "pattern": { "type": "string", "description": "Text or regex to find." },
            "path": { "type": "string", "description": "Optional folder to search under (defaults to the active project root)." },
            "glob": { "type": "string", "description": "Optional file filter, e.g. '*.rs' or 'auth' (extension or name substring)." },
            "context_lines": { "type": ["number","string"], "description": "Optional lines of surrounding context (default 0)." }
        }, "required": ["pattern"] }
    }
}));
```

- [ ] **Step 7: Add the dispatch arm** in `execute_tool`'s match. Resolve the search root: explicit `path` → else the active project root (`ctx.project_root_id` → its registered path) → else the first registered root. ALWAYS through `resolve_safe_path`. Walk with the scanner's `WalkBuilder` skip list. Substring-first, regex fallback, glob path filter, `path:line: <trimmed ≤160>` with optional context, cap 200 hits:
```rust
"search_in_project" => {
    let pattern = args["pattern"].as_str().unwrap_or("").to_string();
    if pattern.trim().is_empty() { return Err("search_in_project requires 'pattern'".into()); }
    // Resolve a search root within the allowlist.
    let raw_root = match args["path"].as_str() {
        Some(p) if !p.is_empty() => p.to_string(),
        _ => {
            let by_id = ctx.project_root_id.as_deref()
                .and_then(|id| ctx.project_roots.iter().find(|r| r.id == id))
                .map(|r| r.path.clone());
            match by_id.or_else(|| ctx.project_roots.first().map(|r| r.path.clone())) {
                Some(p) => p,
                None => return Ok("No registered project folder to search.".into()),
            }
        }
    };
    let safe = crate::project_roots::resolve_safe_path(&raw_root, &ctx.project_roots)?;
    let glob = args["glob"].as_str().map(|s| s.to_lowercase());
    let ctx_lines: usize = args["context_lines"].as_u64().unwrap_or(0) as usize;
    let re = if has_regex_meta(&pattern) { regex::Regex::new(&pattern).ok() } else { None };

    let walker = ignore::WalkBuilder::new(&safe).standard_filters(true)
        .filter_entry(|e| {
            let n = e.file_name().to_string_lossy();
            !matches!(n.as_ref(), "node_modules" | "target" | "venv" | "__pycache__" | ".git")
        }).build();

    let mut hits: Vec<String> = Vec::new();
    let mut files_with_hits = 0usize;
    let mut total = 0usize;
    'outer: for entry in walker.flatten() {
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) { continue; }
        let p = entry.path();
        if let Some(g) = &glob {
            let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
            let matches_glob = g.strip_prefix("*.")
                .map(|ext| name.ends_with(&format!(".{}", ext)))
                .unwrap_or_else(|| name.contains(g.trim_start_matches('*')));
            if !matches_glob { continue; }
        }
        let text = match std::fs::read_to_string(p) { Ok(t) => t, Err(_) => continue }; // skips binaries
        let rel = p.strip_prefix(&safe).unwrap_or(p).to_string_lossy();
        let lines: Vec<&str> = text.lines().collect();
        let mut file_hit = false;
        for (i, line) in lines.iter().enumerate() {
            if line_matches(line, &pattern, &re) {
                file_hit = true; total += 1;
                let trimmed = crate::text_util::truncate_chars(line.trim(), 160);
                hits.push(format!("{}:{}: {}", rel, i + 1, trimmed));
                for c in 1..=ctx_lines {
                    if let Some(l) = lines.get(i + c) {
                        hits.push(format!("{}:{}| {}", rel, i + 1 + c, crate::text_util::truncate_chars(l.trim(), 160)));
                    }
                }
                if hits.len() >= 200 { if file_hit { files_with_hits += 1; } break 'outer; }
            }
        }
        if file_hit { files_with_hits += 1; }
    }
    if hits.is_empty() { return Ok(format!("No matches for '{}'.", pattern)); }
    let capped = total > 200 || hits.len() >= 200;
    let mut out = hits.join("\n");
    if capped { out.push_str(&format!("\n… capped at 200 matches across {} file(s).", files_with_hits)); }
    Ok(out)
}
```
(The `mode`/`AgentMode` import path is `crate::mimir_agent::…`; inside `mimir_agent.rs` refer directly.)

- [ ] **Step 8: Update the gating test** (`:993`) — assert `search_in_project` is present under the fs gate and absent in the base set:
```rust
assert!(with_fs.contains(&"search_in_project".to_string()));
assert!(!base.contains(&"search_in_project".to_string()));
```

- [ ] **Step 9: Verify** — `cargo build && cargo test --bin universal-skill-tree -- --nocapture` green (build compiles the new `regex` dep).

- [ ] **Step 10: Commit**
```bash
git add src-tauri/Cargo.toml src-tauri/src/mimir_agent.rs
git commit -m "feat(agent): search_in_project — allowlisted local grep (substring→regex, glob, capped)"
```

---

## Task 4: Repo tools — `project_git_log`, `list_project_files` depth, `read_file` ranges

**Files:**
- Modify: `src-tauri/src/mimir_agent.rs` (schemas + arms + tests + gating test)

**Interfaces:**
- Consumes: `resolve_safe_path`, existing `read_file`/`list_project_files` arms.
- Produces: agent tool `project_git_log { path?, max? }`; `read_file` gains `start_line?`/`end_line?`; `list_project_files` gains `max_depth?`/`glob?`; helper `slice_lines(&str, usize, usize) -> String`.

- [ ] **Step 1: Write the failing test** for the read-range slicer (`mimir_agent.rs` tests):
```rust
#[test]
fn slice_lines_is_1_based_inclusive() {
    let body = "a\nb\nc\nd\ne";
    assert_eq!(slice_lines(body, 2, 4), "b\nc\nd");
    assert_eq!(slice_lines(body, 1, 1), "a");
    assert_eq!(slice_lines(body, 4, 100), "d\ne");   // end clamps
    assert_eq!(slice_lines(body, 0, 2), "a\nb");      // start clamps to 1
}
```

- [ ] **Step 2: Run to verify it fails** — `cargo test --bin universal-skill-tree slice_lines`.

- [ ] **Step 3: Implement `slice_lines`**:
```rust
fn slice_lines(body: &str, start_line: usize, end_line: usize) -> String {
    let start = start_line.max(1);
    body.lines().enumerate()
        .filter(|(i, _)| { let n = i + 1; n >= start && n <= end_line })
        .map(|(_, l)| l).collect::<Vec<_>>().join("\n")
}
```

- [ ] **Step 4: Run to verify it passes** — `cargo test --bin universal-skill-tree slice_lines`.

- [ ] **Step 5: `read_file` line ranges.** In the existing `read_file` arm (`:787-793`), after `resolve_safe_path` + read, when `start_line`/`end_line` are present, return `slice_lines(...)` with a `:start-end` header instead of the whole file; keep the 100 KB whole-file cap on the no-range path:
```rust
let start = args["start_line"].as_u64().map(|n| n as usize);
let end = args["end_line"].as_u64().map(|n| n as usize);
if let (Some(s), Some(e)) = (start, end) {
    let body = std::fs::read_to_string(&safe).map_err(|err| err.to_string())?;
    return Ok(format!("{} [lines {}-{}]:\n{}", display, s, e, slice_lines(&body, s, e)));
}
// … existing whole-file (100 KB cap) path unchanged
```
Add `start_line`/`end_line` (`type: ["number","string"]`) to the `read_file` schema's properties (not required).

- [ ] **Step 6: `list_project_files` recursion.** Add optional `max_depth` (default 0 = current one level) and `glob` to the schema. In the arm, when `max_depth > 0`, walk with `ignore::WalkBuilder::new(safe).max_depth(Some(depth+1)).standard_filters(true)` + the same skip list, applying the same 200-entry cap and the glob filter from Task 3's pattern. `max_depth` absent/0 → today's exact behavior (backward compatible — verify no arg changes the current output).

- [ ] **Step 7: `project_git_log` schema** (gated on `fs_tools_available`):
```rust
schemas.push(json!({ "type": "function", "function": {
    "name": "project_git_log",
    "description": "Recent git history for a registered project folder (git log --oneline). Use for 'what changed recently' / project history.",
    "parameters": { "type": "object", "properties": {
        "path": { "type": "string", "description": "Project folder (defaults to the active project root)." },
        "max": { "type": ["number","string"], "description": "How many commits (default 15)." }
    }, "required": [] }
}}));
```

- [ ] **Step 8: `project_git_log` arm** — resolve the root exactly like `search_in_project` (explicit → active project root → first root), `resolve_safe_path`, then a fixed-arg `Command` (no shell):
```rust
"project_git_log" => {
    let raw_root = /* same resolution as search_in_project */;
    let safe = crate::project_roots::resolve_safe_path(&raw_root, &ctx.project_roots)?;
    let max = args["max"].as_u64().unwrap_or(15).clamp(1, 100);
    let out = std::process::Command::new("git")
        .arg("-C").arg(&safe).arg("log").arg("--oneline").arg("-n").arg(max.to_string())
        .output();
    match out {
        Err(_) => Ok("git is not available on this system.".into()),
        Ok(o) if !o.status.success() => {
            let err = String::from_utf8_lossy(&o.stderr);
            if err.contains("not a git repository") { Ok(format!("Not a git repository: {}", safe.display())) }
            else { Ok(format!("git log failed: {}", err.trim())) }
        }
        Ok(o) => {
            let text = String::from_utf8_lossy(&o.stdout);
            if text.trim().is_empty() { Ok("No commits found.".into()) }
            else { Ok(crate::text_util::truncate_chars(&text, 6000)) }
        }
    }
}
```
(Factor the root-resolution into a small `fn resolve_project_dir(args, ctx) -> Result<PathBuf, String>` shared by `search_in_project` and `project_git_log` to avoid duplication — reviewer will otherwise flag the copy.)

- [ ] **Step 9: Update the gating test** — add `project_git_log` present-under-fs / absent-in-base assertions.

- [ ] **Step 10: `project_git_log` runtime-shape test (git-optional).** Write a test that inits a temp git repo and asserts `git log --oneline` output shape; guard on `git` availability so it skips cleanly in envs without git:
```rust
#[test]
fn git_log_reads_a_temp_repo_or_skips() {
    use std::process::Command;
    if Command::new("git").arg("--version").output().is_err() { return; } // skip: no git
    let dir = std::env::temp_dir().join(format!("ygg-git-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let git = |args: &[&str]| Command::new("git").arg("-C").arg(&dir).args(args).output().unwrap();
    git(&["init"]); git(&["config","user.email","t@t"]); git(&["config","user.name","t"]);
    std::fs::write(dir.join("a.txt"), "x").unwrap();
    git(&["add","."]); git(&["commit","-m","seed","--no-gpg-sign"]);
    let out = git(&["log","--oneline","-n","5"]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("seed"));
    let _ = std::fs::remove_dir_all(&dir);
}
```

- [ ] **Step 11: Verify** — `cargo build && cargo test --bin universal-skill-tree -- --nocapture` green.

- [ ] **Step 12: Commit**
```bash
git add src-tauri/src/mimir_agent.rs
git commit -m "feat(agent): project_git_log + read_file ranges + list_project_files depth"
```

---

## Task 5: Working plan — agent-owned scratchpad (migration 060 + tools + injection + UI)

**Files:**
- Create: `src-tauri/migrations/060_agent_plans.sql`
- Modify: `src-tauri/src/mimir_agent.rs` (schemas + arms; `ToolCtx.{plan, session_id}`)
- Modify: `src-tauri/src/mimir_retrieval.rs` (load plan, inject into prompt, populate `ToolCtx`)
- Modify: `src/components/MimirChat.tsx` (render the plan block)

**Interfaces:**
- Consumes: `ToolCtx.session_id`, `mimir_chat_sessions` (059).
- Produces: agent tools `set_plan { content }`, `update_plan { content }`, `finalize_plan {}`; `ToolCtx.plan: Option<String>`, `ToolCtx.session_id: Option<String>`; helper `plan_prelude(Option<&str>) -> String`.

- [ ] **Step 1: Migration** — `src-tauri/migrations/060_agent_plans.sql`:
```sql
CREATE TABLE IF NOT EXISTS agent_plans (
    session_id TEXT PRIMARY KEY REFERENCES mimir_chat_sessions(id) ON DELETE CASCADE,
    content    TEXT NOT NULL,
    updated_at TIMESTAMPTZ DEFAULT NOW()
);
```

- [ ] **Step 2: Write the failing test** for the pure injection helper (`mimir_agent.rs` tests):
```rust
#[test]
fn plan_prelude_injects_when_present() {
    assert_eq!(plan_prelude(None), "");
    let p = plan_prelude(Some("1. read auth.rs\n2. grep for verify"));
    assert!(p.starts_with("Working plan:\n"));
    assert!(p.contains("read auth.rs"));
    assert!(p.ends_with("\n\n"));
}
```

- [ ] **Step 3: Run to verify it fails**, then **Step 4: implement**:
```rust
fn plan_prelude(plan: Option<&str>) -> String {
    match plan {
        Some(p) if !p.trim().is_empty() => format!("Working plan:\n{}\n\n", p),
        _ => String::new(),
    }
}
```
- [ ] **Step 5: Run to verify it passes** — `cargo test --bin universal-skill-tree plan_prelude`.

- [ ] **Step 6: Add `plan` + `session_id` to `ToolCtx`** (`:433`): `pub plan: Option<String>,` and `pub session_id: Option<String>,`.

- [ ] **Step 7: Plan tool schemas** (always registered — no fs gate; the doctrine already tells the model to use them):
```rust
for (name, desc, has_content) in [
    ("set_plan", "Write/replace your working plan for this multi-step task (a short numbered list of steps).", true),
    ("update_plan", "Replace your working plan with an updated version as you make progress.", true),
    ("finalize_plan", "Mark the plan complete and clear it once you've answered.", false),
] {
    let params = if has_content {
        json!({ "type": "object", "properties": { "content": { "type": "string", "description": "The plan text." } }, "required": ["content"] })
    } else { json!({ "type": "object", "properties": {}, "required": [] }) };
    schemas.push(json!({ "type": "function", "function": { "name": name, "description": desc, "parameters": params } }));
}
```

- [ ] **Step 8: Plan tool arms** in `execute_tool` — upsert/clear `agent_plans` keyed on `ctx.session_id`; no-op with a friendly message when there's no session:
```rust
"set_plan" | "update_plan" => {
    let content = args["content"].as_str().unwrap_or("").to_string();
    if content.trim().is_empty() { return Err("plan requires 'content'".into()); }
    let Some(sid) = ctx.session_id.as_deref() else { return Ok("No active session to attach a plan to.".into()); };
    sqlx::query("INSERT INTO agent_plans (session_id, content) VALUES ($1, $2) \
                 ON CONFLICT (session_id) DO UPDATE SET content = $2, updated_at = NOW()")
        .bind(sid).bind(&content).execute(ctx.pool).await.map_err(|e| e.to_string())?;
    Ok(format!("Plan saved:\n{}", content))
}
"finalize_plan" => {
    if let Some(sid) = ctx.session_id.as_deref() {
        sqlx::query("DELETE FROM agent_plans WHERE session_id = $1").bind(sid)
            .execute(ctx.pool).await.map_err(|e| e.to_string())?;
    }
    Ok("Plan finalized.".into())
}
```

- [ ] **Step 9: Load + inject in `mimir_chat`.** After `session_id_opt` is known (`:~1035`), before building the prompt:
```rust
let agent_plan: Option<String> = if let Some(sid) = session_id_opt.as_deref() {
    sqlx::query_scalar::<_, String>("SELECT content FROM agent_plans WHERE session_id = $1")
        .bind(sid).fetch_optional(&database.pool).await.ok().flatten()
} else { None };
```
Prepend the plan to the agent system prompt (before/after the tool map, either works): `agent_system_prompt = format!("{}{}", crate::mimir_agent::plan_prelude(agent_plan.as_deref()), agent_system_prompt);` (or `push_str` at the end — pick one, keep it deterministic). In the `ToolCtx` build add `plan: agent_plan.clone(), session_id: session_id_opt.clone(),`.

- [ ] **Step 10: Dive-requires-plan nudge.** In the dive framing (Task 2 Step 6), when `agent_mode == Dive && agent_plan.is_none()`, append: `" You have no plan yet — call set_plan with your first steps before exploring."`

- [ ] **Step 11: Update the gating test** — `set_plan`/`update_plan`/`finalize_plan` are present in BOTH `base` and `with_fs` sets (ungated).

- [ ] **Step 12: Frontend plan block.** In `MimirChat.tsx`, track the latest plan from `ygg-agent-tool` events whose `toolName` is `set_plan`/`update_plan` (read `input.content`); clear on `finalize_plan`. Render a collapsible block (reuse `ToolCallBlock` styling) above the message list labeled "📋 Plan". Keep it light.

- [ ] **Step 13: Verify** — `npx tsc --noEmit` exit 0; `cargo build && cargo test --bin universal-skill-tree -- --nocapture` green (migration 060 auto-runs on next app start; tests don't need it).

- [ ] **Step 14: Commit**
```bash
git add src-tauri/migrations/060_agent_plans.sql src-tauri/src/mimir_agent.rs src-tauri/src/mimir_retrieval.rs src/components/MimirChat.tsx
git commit -m "feat(agent): agent-owned working plan — set/update/finalize, persisted + injected"
```

---

## Task 6: Exploration trail UX (light v1)

**Files:**
- Modify: `src/components/MimirChat.tsx`

**Interfaces:** consumes existing `ygg-agent-tool` events (`toolName`, `input.path`).

- [ ] **Step 1: Accumulate touched files.** Add state `const [trail, setTrail] = useState<{ files: Set<string>; step: string } | null>(null)`. In the existing `ygg-agent-tool` listener (the one already handling live tool blocks), when `payload.toolName` ∈ `{read_file, search_in_project, list_project_files, project_git_log}`, add `payload.input?.path` (when present) to the file set and set `step` to a short label (e.g. `${payload.toolName} ${payload.input?.path ?? ''}`). Reset `trail` to `null` at the start of `handleSend` (like the other per-turn state).

- [ ] **Step 2: Render the strip.** One line under the latest assistant message while a turn is active: `📁 {files.size} file(s) · {step}`. Keep it crude (a muted single line) — restyle later per the spec.

- [ ] **Step 3: Verify** — `npx tsc --noEmit` exit 0.

- [ ] **Step 4: Commit**
```bash
git add src/components/MimirChat.tsx
git commit -m "feat(agent): light exploration-trail strip (files touched · current step)"
```

---

## Full verification (after Task 6)

- `export CARGO_TARGET_DIR=…scan-target` then `cargo build && cargo test --bin universal-skill-tree -- --nocapture` — clean, all green.
- `npx tsc --noEmit` — exit 0.
- **Runtime checklist (user, `bash dev.sh` — applies migration 060):**
  - Ask a repo question in **chat** → Mimir reaches for `list_project_files` / graph verbs before `search_mimir`/web (compare before/after).
  - Switch to **dive**, ask "how does X work in <project>" → observe orient → graph → grep → read → cite `file:line`; a 📋 plan block appears; the 📁 trail strip shows files.
  - Trigger a long walk → **honest-partial** block on timeout ("walk cut short, found X") — NEVER a silent classic answer.
  - `search_in_project` finds a symbol by substring and by regex; respects `.gitignore`; caps at 200.
  - `project_git_log` on the Yggdrasil root returns recent commits; on a non-git dir returns the friendly message.
  - Mode survives panel reopen; switching modes mid-thread works.
  - Project-chat (no node) still works with the picker; scan/coverage chips unaffected.

---

## Self-Review

**Spec coverage:** Phase 0 → Task 0 (loop config + honest-partial + observation cap). Phase 1 → Task 1 (tool education). Phase 2 → Task 2 (modes + selector + persistence). Phase 3 → Tasks 3 (grep) + 4 (git_log, read ranges, list depth). Phase 4 → Task 5 (migration 060 + plan tools + injection + UI). Phase 5 → Task 6 (trail). Testing/build-order/files all mapped. Locked decisions (honest-partial, budgets, modes, grep substring-first, local-first, trail-light, skills-out) each appear as Global Constraints and in the relevant task.

**Deferred/out-of-scope carried verbatim:** no file writes/bash/build execution, no sub-agents, no GitHub-remote tools, no skill packs, no skill-graph integration, no per-mode tool-schema gating, no git blame/show, no trail restyle, no streaming/parallel calls — none of these appear as tasks. `search_in_project` regex is a fallback (substring primary); `project_git_log` uses fixed-arg `Command` (no shell); every repo tool goes through `resolve_safe_path`.

**Type consistency:** `AgentLoopConfig` (Task 0) → `AgentMode::loop_config` (Task 2) → passed to `run_agent_turn(cfg, loop_cfg, …)` (Task 0 signature) → selected in `mimir_chat` from `AgentMode::from_param(mode.as_deref())` (Task 2). `honest_partial_answer(&AgentTurnState, &str)` (Task 0) built inside `run_agent_turn`, its file-tool list matches the tool names registered in Tasks 3–4 and the trail-strip list in Task 6. `ToolCtx` gains `mode` (Task 2), `plan` + `session_id` (Task 5) — each populated in the single `ToolCtx` build in `mimir_chat`. `resolve_project_dir(args, ctx)` shared by `search_in_project` (Task 3) and `project_git_log` (Task 4). `plan_prelude(Option<&str>)` (Task 5) consumes `agent_plan` loaded from `agent_plans` (migration 060). The gating test at `mimir_agent.rs:993` is updated by Tasks 3, 4, and 5 as tools are added.

**Migration number:** highest on disk is `059`; this plan adds only `060_agent_plans.sql`. ✓

**Placeholder scan:** no "TBD"/"handle errors"/"similar to Task N" — every code step carries real code or an exact edit target with the current line anchor.
