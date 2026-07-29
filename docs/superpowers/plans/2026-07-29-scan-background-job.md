# scan_project → Background Job (Agent Step 2) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move `scan_project` off the synchronous agent turn onto the existing background job queue, so chat stays responsive during a 10–60s repo scan, with `ygg-scan-*` events driving an inline progress indicator.

**Architecture:** Rename the current `scan_project` → `scan_project_inner`; wrap it in `scan_project_with_events` that emits `ygg-scan-progress` (started) and `ygg-scan-complete` (with the `ScanResult`). Add an `OrchestratorJob::ScanProject` variant handled by `run_scan_project`, mirroring the existing `run_rematch_all_nodes` pattern. The agent tool arm and the extension bridge enqueue the job instead of running the scan inline.

**Tech Stack:** Rust (Tauri 2, sqlx, tokio mpsc), React 19 + TypeScript.

## Global Constraints

- Rust: `$N` placeholders; non-macro `sqlx::query()`; commands return `Result<T,String>` with `database: State<'_, Database>` last; `#[serde(rename_all="camelCase")]`.
- Reuse the existing `ygg-*` Tauri event pattern (orchestrator.rs); frontend `listen()` must `unlisten()` on cleanup (not in `finally`).
- No new crates or npm deps.
- Work stays on branch `fix-compilation-errors`; commit locally, do NOT push.
- **Build/test in the isolated target dir** (the user's dev server may hold `target/`). Before any cargo command:
  `export CARGO_TARGET_DIR="C:/Users/dhruv/AppData/Local/Temp/claude/C--Users-dhruv-projects-Yggdrasil/497ed12f-520a-4774-9c54-620ab075f017/scratchpad/scan-target"`
- **Scoping decisions (do not "improve" past these):** progress granularity is `started` + `complete` only — do NOT instrument the walk loop for per-file counts (deferred). Do NOT add a standalone `enqueue_scan_project` Tauri command — the agent tool and ext bridge enqueue the job directly; a command with no caller is dead code.

## File Structure

| File | Change | Task |
|---|---|---|
| `src-tauri/src/project_scanner.rs` | Rename `scan_project`→`scan_project_inner`; add `scan_project_with_events` | 1 |
| `src-tauri/src/orchestrator.rs` | Add `OrchestratorJob::ScanProject` variant + match arm + `run_scan_project` handler | 1 |
| `src-tauri/src/mimir_agent.rs` | `ToolCtx` gains `queue`; `scan_project` arm enqueues instead of scanning | 2 |
| `src-tauri/src/mimir_retrieval.rs` | Build `ToolCtx` with `queue` | 2 |
| `src-tauri/src/ext_server.rs` | Replace the fire-and-forget scan (`:555`) with a queue send | 2 |
| `src/components/MimirChat.tsx` | Subscribe to `ygg-scan-progress`/`ygg-scan-complete`; inline indicator | 3 |

---

### Task 1: Background scan job (scanner split + orchestrator variant)

**Files:**
- Modify: `src-tauri/src/project_scanner.rs` (rename `scan_project` at :390 → `scan_project_inner`; add `scan_project_with_events`)
- Modify: `src-tauri/src/orchestrator.rs` (enum :20-29; match :86-126; add `run_scan_project` near :209)

**Interfaces:**
- Consumes: `scan_project_inner(pool, path, tree_id) -> Result<ScanResult, String>` (the renamed current fn); `ScanResult { files_scanned, files_skipped, nodes_added, nodes_enriched, edges_added, errors }` (project_scanner.rs:20, derives Default).
- Produces: `scan_project_with_events(pool: &PgPool, app: &AppHandle, path: &str, tree_id: &str)`; `OrchestratorJob::ScanProject { path: String, tree_id: String }`; events `ygg-scan-progress` / `ygg-scan-complete`.

- [ ] **Step 1: Rename `scan_project` → `scan_project_inner`**

In `project_scanner.rs:390`, rename the function:
```rust
pub(crate) async fn scan_project_inner(pool: &PgPool, path: &str, tree_id: &str) -> Result<ScanResult, String> {
```
(Body unchanged.) There is exactly one existing caller — `ext_server.rs:556`; Task 2 rewrites that call, so at the end of Task 1 the old name may be temporarily unreferenced. To keep Task 1 building on its own, ALSO update `ext_server.rs:556` in this step to call `scan_project_inner` (Task 2 later replaces the whole spawn with a queue send). And update the agent tool arm caller in `mimir_agent.rs` (`scan_project` execute arm, ~:615) to `scan_project_inner` for now (Task 2 replaces it with an enqueue).

- [ ] **Step 2: Add `scan_project_with_events`**

Append to `project_scanner.rs`:
```rust
/// Background wrapper: emits a started event, runs the scan, emits a complete
/// event carrying the full ScanResult. Errors are emitted as a complete event
/// with an `error` field (never panics).
pub(crate) async fn scan_project_with_events(
    pool: &sqlx::PgPool,
    app: &tauri::AppHandle,
    path: &str,
    tree_id: &str,
) {
    use tauri::Emitter;
    let _ = app.emit("ygg-scan-progress", serde_json::json!({
        "treeId": tree_id, "status": "started", "path": path,
    }));
    match scan_project_inner(pool, path, tree_id).await {
        Ok(r) => {
            let _ = app.emit("ygg-scan-complete", serde_json::json!({
                "treeId": tree_id,
                "filesScanned": r.files_scanned,
                "filesSkipped": r.files_skipped,
                "nodesAdded": r.nodes_added,
                "nodesEnriched": r.nodes_enriched,
                "edgesAdded": r.edges_added,
                "errors": r.errors,
            }));
            println!("📡 [scan] complete: {} files, {} nodes, {} edges (tree={})",
                r.files_scanned, r.nodes_added, r.edges_added, tree_id);
        }
        Err(e) => {
            let _ = app.emit("ygg-scan-complete", serde_json::json!({
                "treeId": tree_id, "error": e,
            }));
            println!("⚠️  [scan] failed (tree={}): {}", tree_id, e);
        }
    }
}
```

- [ ] **Step 3: Add the `ScanProject` job variant + match arm**

In `orchestrator.rs`, add to the `OrchestratorJob` enum (after `ConsolidateSession`):
```rust
    ScanProject { path: String, tree_id: String },
```
And add a match arm in `start_worker` (after the `ConsolidateSession` arm, ~:125):
```rust
                OrchestratorJob::ScanProject { path, tree_id } => {
                    run_scan_project(&pool, &app, &path, &tree_id).await;
                }
```

- [ ] **Step 4: Add the `run_scan_project` handler**

In `orchestrator.rs` (near the other `run_*` handlers, ~:207):
```rust
async fn run_scan_project(pool: &PgPool, app: &AppHandle, path: &str, tree_id: &str) {
    crate::project_scanner::scan_project_with_events(pool, app, path, tree_id).await;
}
```

- [ ] **Step 5: Build**

Run: `cargo build --manifest-path src-tauri/Cargo.toml` (with `CARGO_TARGET_DIR` set).
Expected: `Finished`, no errors (pre-existing warnings OK).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/project_scanner.rs src-tauri/src/orchestrator.rs src-tauri/src/mimir_agent.rs src-tauri/src/ext_server.rs
git commit -m "feat(scanner): background ScanProject job + scan_project_with_events"
```

---

### Task 2: Wire the callers to enqueue

**Files:**
- Modify: `src-tauri/src/mimir_agent.rs` (`ToolCtx` :325-335; `scan_project` execute arm ~:610-624)
- Modify: `src-tauri/src/mimir_retrieval.rs` (ToolCtx build ~:1180-1195)
- Modify: `src-tauri/src/ext_server.rs` (the scan spawn ~:547-560)

**Interfaces:**
- Consumes: `OrchestratorJob::ScanProject` (Task 1); `crate::orchestrator::JobQueue` (has `async fn send(&self, job) -> Result<(),String>`).
- Produces: `ToolCtx.queue: Option<&'a crate::orchestrator::JobQueue>`.

- [ ] **Step 1: Add `queue` to `ToolCtx`**

In `mimir_agent.rs`, add a field to `ToolCtx` (after `turn_id`):
```rust
    pub queue: Option<&'a crate::orchestrator::JobQueue>,
```

- [ ] **Step 2: Enqueue in the `scan_project` arm**

Replace the body of the `"scan_project"` arm in `execute_tool` (~:610-624) with:
```rust
        "scan_project" => {
            let path = args["path"].as_str().ok_or("scan_project requires a 'path' argument")?;
            let Some(tree_id) = ctx.tree_id.as_deref() else {
                return Ok("No active tree to scan into — open a tree first.".to_string());
            };
            let Some(queue) = ctx.queue else {
                return Ok("Background scanning is unavailable right now.".to_string());
            };
            queue
                .send(crate::orchestrator::OrchestratorJob::ScanProject {
                    path: path.to_string(),
                    tree_id: tree_id.to_string(),
                })
                .await?;
            Ok(format!("Scan of '{}' started in the background — I'll surface the results when it finishes.", path))
        }
```

- [ ] **Step 3: Build `ToolCtx` with the queue**

In `mimir_retrieval.rs` where `ToolCtx { … }` is constructed (~:1180), add:
```rust
                    app: Some(app.clone()),
                    turn_id: turn_id.clone(),
                    queue: Some(&queue),
```
(`queue` is the existing `queue: State<'_, JobQueue>` param of `mimir_chat`; `State` derefs to `&JobQueue`, so `Some(&queue)` — if the borrow checker complains, use `Some(queue.inner())`.)

- [ ] **Step 4: Retro-fit the extension bridge**

In `ext_server.rs`, replace the fire-and-forget scan `tokio::spawn` block (~:547-560) with a direct enqueue using the handler's existing queue handle (the same one passed to `on_resource_ingested_async` at :544 — find its source on the Axum `state`, e.g. `state.queue`):
```rust
    // Enqueue a background project scan when both local_path and tree_id are provided
    if let (Some(lp), Some(tid)) = (
        body.local_path.as_ref().filter(|s| !s.is_empty()),
        body.tree_id.as_ref().filter(|s| !s.is_empty()),
    ) {
        let _ = state.queue.send(crate::orchestrator::OrchestratorJob::ScanProject {
            path: lp.clone(),
            tree_id: tid.clone(),
        }).await;
    }
```
(Confirm the queue field name on the Axum state struct — it is whatever `queue_bg` was cloned from. If the state holds no queue, clone it from the same place the ingest spawn got `queue_bg`.)

- [ ] **Step 5: Build + full test run**

Run: `cargo test --manifest-path src-tauri/Cargo.toml 2>&1 | tail -20`
Expected: build clean; all existing tests pass (this task changes wiring, not pure logic — no new unit tests). If the `mimir_agent` tool-count test asserts a schema count, `scan_project` is unchanged in the schema so it should still pass; if it references the arm behavior, update only if it breaks.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/mimir_agent.rs src-tauri/src/mimir_retrieval.rs src-tauri/src/ext_server.rs
git commit -m "feat(scanner): agent tool + ext bridge enqueue background scan"
```

---

### Task 3: Frontend scan indicator

**Files:**
- Modify: `src/components/MimirChat.tsx` (imports; a small scan-status state + listeners; render a compact indicator)

**Interfaces:**
- Consumes: `ygg-scan-progress { treeId, status, path }`, `ygg-scan-complete { treeId, filesScanned?, nodesAdded?, edgesAdded?, errors?, error? }`.

- [ ] **Step 1: Subscribe on mount, render a compact indicator**

In `MimirChat.tsx`, add a `useEffect` that listens for both events and stores a small status string in state (cleaning up on unmount):
```tsx
const [scanStatus, setScanStatus] = useState<string | null>(null);

useEffect(() => {
  let unsubP: (() => void) | undefined;
  let unsubC: (() => void) | undefined;
  (async () => {
    unsubP = await listen<{ status: string; path: string }>("ygg-scan-progress", ({ payload }) => {
      setScanStatus(`Scanning ${payload.path}…`);
    });
    unsubC = await listen<{ filesScanned?: number; nodesAdded?: number; edgesAdded?: number; error?: string }>(
      "ygg-scan-complete",
      ({ payload }) => {
        setScanStatus(
          payload.error
            ? `Scan failed: ${payload.error}`
            : `Scan complete — ${payload.filesScanned ?? 0} files, ${payload.nodesAdded ?? 0} concepts, ${payload.edgesAdded ?? 0} links.`
        );
        setTimeout(() => setScanStatus(null), 8000);
      }
    );
  })();
  return () => { unsubP?.(); unsubC?.(); };
}, []);
```
Render a compact indicator near the input (only when `scanStatus` is set):
```tsx
{scanStatus && (
  <div className="px-3 py-1 text-xs opacity-70 border-t border-white/10">🗂️ {scanStatus}</div>
)}
```
(`listen` is already imported by the Step-1 visibility work; `useState`/`useEffect` are already imported.)

- [ ] **Step 2: Verify the frontend compiles**

Run: `npx tsc --noEmit` (repo root). Expected: exit 0.

- [ ] **Step 3: Commit**

```bash
git add src/components/MimirChat.tsx
git commit -m "feat(scanner): inline background-scan status in Mimir chat"
```

---

### Task 4: Live verification (runtime checklist)

**Files:** none (manual verification in a running app).

- [ ] **Step 1:** `bash dev.sh`. Open a tree's Mimir chat.
- [ ] **Step 2:** Ask "scan my project at C:\Users\dhruv\projects\Yggdrasil" → chat returns "Scan started in the background…" **immediately** (no multi-second hang), and the 🗂️ indicator shows "Scanning …" then "Scan complete — N files, M concepts, K links."
- [ ] **Step 3:** Confirm the concept graph gained code-grounded nodes (ask "explain scan_project" or check via `query_graph`) — proving the background job actually merged, same as the old inline path.
- [ ] **Step 4:** Save a resource via the Chrome extension with a `local_path` + `tree_id` → the ext bridge returns immediately and the scan runs in the background (server log shows the job, no blocking).
- [ ] **Step 5:** Scan a non-existent path → `ygg-scan-complete` carries `error`, the indicator shows "Scan failed: …", chat still works.

---

## Self-Review

**Spec coverage (Step 2a–2e):**
- 2a split `scan_project_inner` + `scan_project_with_events` → Task 1 Steps 1-2. ✓
- 2b `OrchestratorJob::ScanProject` + handler → Task 1 Steps 3-4. ✓ (`enqueue_scan_project` command intentionally dropped — YAGNI, no caller; documented in Global Constraints.)
- 2c agent tool enqueues; `ToolCtx.queue` → Task 2 Steps 1-3. ✓
- 2d frontend subscription → Task 3. ✓
- 2e ext_server retro-fit → Task 2 Step 4. ✓
- Progress granularity: `started` + `complete` only (per-file counts deferred) — documented in Global Constraints. ✓

**Placeholder scan:** No TBD/TODO; every step has concrete code. Two spots flagged for the implementer to confirm from live code (the Axum state's queue field name in Task 2 Step 4; the `Some(&queue)` vs `Some(queue.inner())` borrow in Task 2 Step 3) — these are "confirm exact name", not placeholders.

**Type consistency:** `OrchestratorJob::ScanProject { path: String, tree_id: String }` identical across enum (Task 1 Step 3), match arm (Step 3), tool arm (Task 2 Step 2), ext bridge (Task 2 Step 4). Event names `ygg-scan-progress`/`ygg-scan-complete` identical in emitter (Task 1 Step 2) and listeners (Task 3). Payload keys (`filesScanned`, `nodesAdded`, `edgesAdded`, `error`) match between emit and frontend.

**Note on TDD:** this step is wiring (no new pure logic), so it's verified by `cargo build`/`cargo test` (existing suite) + the Task 4 runtime checklist — consistent with how the codebase verifies event/DB/queue code.

## Explicitly out of scope
Per-file granular progress; a manual "rescan" UI/command; scanning without a tree open; changing the scanner's extraction logic.
