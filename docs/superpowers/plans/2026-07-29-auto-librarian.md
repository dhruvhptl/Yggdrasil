# Auto-Librarian (Agent Step 5) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** A background janitor that every ~6h finds resources which missed the tag/match pipeline and re-enqueues the *existing* jobs to fix them, emitting a `ygg-library-suggestion` event so the Library page shows a small badge.

**Design (approved 2026-07-29):** auto-FIX (not notify-only). The poller enqueues `OrchestratorJob::AutoTagResources` for untagged resources (which already chains into matching + skill extraction) and `OrchestratorJob::MatchResourceToNodes` for tagged-but-unmatched ones. It only re-runs the same idempotent pipeline the app runs on ingest — no destructive surface. Ephemeral events (no persistence, no migration) for v1.

**Tech Stack:** Rust (Tauri 2, tokio, sqlx), React 19 + TS.

## Global Constraints

- Rust: `$N` placeholders; non-macro `sqlx::query()`; reuse the `ygg-*` event pattern; `app.emit` needs `use tauri::Emitter;`.
- No new crates, no npm deps, **no migration**.
- Work on branch `fix-compilation-errors`; commit locally, do NOT push.
- **Build/test in the isolated target dir.** Before any cargo command:
  `export CARGO_TARGET_DIR="C:/Users/dhruv/AppData/Local/Temp/claude/C--Users-dhruv-projects-Yggdrasil/497ed12f-520a-4774-9c54-620ab075f017/scratchpad/scan-target"`
- Frontend has no unit-test harness; the badge is verified in the runtime checklist (Task 3).

## Grounding (verified 2026-07-29)

- `mimir_resources` has `tags TEXT[]` (migration 013) and `parent_id` (migration 010). Untagged = `tags IS NULL OR cardinality(tags) = 0`; top-level (skip playlist children) = `parent_id IS NULL`. Unmatched = no row in `mimir_node_links`.
- `OrchestratorJob::AutoTagResources { resource_ids: Vec<String> }` and `MatchResourceToNodes { resource_id: String }` exist (orchestrator.rs:24-25); `AutoTagResources` already chains into match + skill-extract on completion.
- `main.rs` setup: `let pool = database.pool.clone();` (:76), `let queue = orchestrator::start_worker(pool.clone(), app_handle.clone(), http_client.clone());` (:80), `app_handle.manage(queue);` (:91), `ext_server::start(pool, …)` moves `pool` (:89). → call `start_auto_librarian` after :80 and before :91 (queue still owned), using `pool.clone()`.
- `ResourcesPage.tsx` already imports `listen` (@tauri-apps/api/event) + `useEffect`/`useState`.

## File Structure

| File | Change | Task |
|---|---|---|
| `src-tauri/src/auto_librarian.rs` | Create — `start_auto_librarian` + `run_pass` | 1 |
| `src-tauri/src/main.rs` | `mod auto_librarian;` + start call in setup | 1 |
| `src/pages/ResourcesPage.tsx` | Listen `ygg-library-suggestion`; badge/toast | 2 |

---

### Task 1: The poller + wiring

**Files:**
- Create: `src-tauri/src/auto_librarian.rs`
- Modify: `src-tauri/src/main.rs` (`mod auto_librarian;` near the other mods ~:38; start call in the setup block after `start_worker` ~:80)

**Interfaces:**
- Consumes: `crate::orchestrator::{JobQueue, OrchestratorJob}`.
- Produces: `pub fn start_auto_librarian(pool: sqlx::PgPool, app: tauri::AppHandle, queue: crate::orchestrator::JobQueue)`; event `ygg-library-suggestion { untagged, unmatched, queued }`.

- [ ] **Step 1: Create `auto_librarian.rs`**

```rust
// src-tauri/src/auto_librarian.rs
//
// Background janitor: every ~6h, re-enqueue the existing tag/match jobs for
// resources that missed the pipeline, and emit ygg-library-suggestion so the
// Library page can show a badge. Only re-runs idempotent work — no destructive
// surface. First pass runs shortly after startup.

use sqlx::{PgPool, Row};
use tauri::{AppHandle, Emitter};

use crate::orchestrator::{JobQueue, OrchestratorJob};

const INTERVAL_SECS: u64 = 6 * 60 * 60; // 6 hours
const WARMUP_SECS: u64 = 120;           // let startup settle before the first pass
const BATCH_CAP: i64 = 20;              // never flood the 64-slot queue

pub fn start_auto_librarian(pool: PgPool, app: AppHandle, queue: JobQueue) {
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(WARMUP_SECS)).await;
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(INTERVAL_SECS));
        loop {
            interval.tick().await; // 1st tick fires immediately (≈ after warmup), then every 6h
            run_pass(&pool, &app, &queue).await;
        }
    });
}

async fn run_pass(pool: &PgPool, app: &AppHandle, queue: &JobQueue) {
    // 1. Untagged top-level resources → AutoTagResources (chains into match + skills).
    let untagged: Vec<String> = sqlx::query(
        "SELECT id FROM mimir_resources \
         WHERE parent_id IS NULL AND (tags IS NULL OR cardinality(tags) = 0) \
         ORDER BY created_at DESC LIMIT $1"
    )
    .bind(BATCH_CAP)
    .fetch_all(pool)
    .await
    .unwrap_or_default()
    .iter()
    .filter_map(|r| r.try_get::<String, _>("id").ok())
    .collect();
    let untagged_n = untagged.len();
    if !untagged.is_empty() {
        let _ = queue.send(OrchestratorJob::AutoTagResources { resource_ids: untagged }).await;
    }

    // 2. Tagged-but-unmatched top-level resources → MatchResourceToNodes (one job each).
    let unmatched: Vec<String> = sqlx::query(
        "SELECT r.id FROM mimir_resources r \
         WHERE r.parent_id IS NULL \
           AND r.tags IS NOT NULL AND cardinality(r.tags) > 0 \
           AND NOT EXISTS (SELECT 1 FROM mimir_node_links l WHERE l.resource_id = r.id) \
         ORDER BY r.created_at DESC LIMIT $1"
    )
    .bind(BATCH_CAP)
    .fetch_all(pool)
    .await
    .unwrap_or_default()
    .iter()
    .filter_map(|r| r.try_get::<String, _>("id").ok())
    .collect();
    let unmatched_n = unmatched.len();
    for id in unmatched {
        let _ = queue.send(OrchestratorJob::MatchResourceToNodes { resource_id: id }).await;
    }

    let queued = untagged_n + unmatched_n;
    if queued > 0 {
        let _ = app.emit("ygg-library-suggestion", serde_json::json!({
            "untagged": untagged_n,
            "unmatched": unmatched_n,
            "queued": queued,
        }));
        println!("🧹 [librarian] queued {} untagged + {} unmatched resource(s)", untagged_n, unmatched_n);
    }
}
```

- [ ] **Step 2: Register the module + start it**

In `main.rs`, add `mod auto_librarian;` alongside the other `mod` lines (~:38, after `mod project_roots;`).
In the setup block, right after `let queue = orchestrator::start_worker(...)` (~:80) and before `app_handle.manage(queue);` (~:91), add:
```rust
                crate::auto_librarian::start_auto_librarian(pool.clone(), app_handle.clone(), queue.clone());
```
(`pool` is still owned here — `start_worker` took `pool.clone()`, and `ext_server::start` doesn't move `pool` until later; `queue` is owned until `manage`. Use `.clone()` on all three.)

- [ ] **Step 3: Build**

Run: `cargo build --manifest-path src-tauri/Cargo.toml 2>&1 | tail -5`
Expected: `Finished`, no errors (pre-existing warnings OK).

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/auto_librarian.rs src-tauri/src/main.rs
git commit -m "feat(librarian): 6h auto-fix poller for untagged/unmatched resources"
```

---

### Task 2: Library badge

**Files:**
- Modify: `src/pages/ResourcesPage.tsx` (a `librarianNote` state + a mount `useEffect` listener + a small banner in the header area)

**Interfaces:**
- Consumes: `ygg-library-suggestion { untagged: number; unmatched: number; queued: number }`.

- [ ] **Step 1: Listen + render a banner**

Inside `ResourcesPage`, add state + a mount-time effect (cleaning up on unmount):
```tsx
const [librarianNote, setLibrarianNote] = useState<string | null>(null);

useEffect(() => {
  let unsub: (() => void) | undefined;
  (async () => {
    unsub = await listen<{ untagged: number; unmatched: number; queued: number }>(
      "ygg-library-suggestion",
      ({ payload }) => {
        setLibrarianNote(
          `🧹 Auto-librarian queued ${payload.queued} resource(s) — ${payload.untagged} to tag, ${payload.unmatched} to match.`
        );
        setTimeout(() => setLibrarianNote(null), 12000);
      }
    );
  })();
  return () => { unsub?.(); };
}, []);
```
Render it as a dismissible-by-timeout banner near the top of the page's returned JSX (only when set):
```tsx
{librarianNote && (
  <div className="mb-3 rounded-md border border-white/10 bg-white/5 px-3 py-2 text-sm opacity-80">
    {librarianNote}
  </div>
)}
```
(Place it just inside the page's outer container, above the existing header/controls. `listen`, `useEffect`, `useState` are already imported.)

- [ ] **Step 2: Verify the frontend compiles**

Run: `npx tsc --noEmit` (repo root). Expected: exit 0.

- [ ] **Step 3: Commit**

```bash
git add src/pages/ResourcesPage.tsx
git commit -m "feat(librarian): Library page badge for auto-librarian activity"
```

---

### Task 3: Live verification (runtime checklist)

**Files:** none.

- [ ] **Step 1:** Ensure there's at least one untagged and/or tagged-but-unmatched resource in the library (or temporarily lower `WARMUP_SECS` to ~10 and `INTERVAL_SECS` to test faster, then revert). `bash dev.sh`.
- [ ] **Step 2:** ~2 minutes after startup, confirm the console logs `🧹 [librarian] queued N …` and the Library page shows the banner. Confirm the enqueued resources actually get tagged/matched (their tags/node-links appear) — proving it re-ran the real pipeline.
- [ ] **Step 3:** With a fully-tidy library (nothing untagged/unmatched), confirm a pass emits nothing (no banner, no spurious jobs).

---

## Self-Review

**Design coverage:** 6h poller (Task 1); untagged→AutoTag + unmatched→Match via existing jobs (Task 1 run_pass); `ygg-library-suggestion` event (Task 1); Library badge (Task 2); no migration/persistence (v1). ✓

**Placeholder scan:** No TBD/TODO; full code for the new module + the frontend snippet. Task 1 Step 2 is a locate-and-insert into `main.rs` (mod line + one call), with the exact ownership reasoning given.

**Type consistency:** `start_auto_librarian(PgPool, AppHandle, JobQueue)` matches the `main.rs` call (all `.clone()`d). Event payload keys `untagged`/`unmatched`/`queued` identical between the Rust `emit` and the TS listener. `OrchestratorJob::AutoTagResources { resource_ids }` / `MatchResourceToNodes { resource_id }` match the enum.

**Correctness notes:** the `interval.tick()`-first-then-`run_pass` ordering means the first pass runs ≈ after `WARMUP_SECS` (the immediate first tick fires right after the warmup sleep), then every 6h. Batch cap + `parent_id IS NULL` keep it bounded and skip playlist children. `.unwrap_or_default()` on the queries means a DB hiccup skips a pass silently (acceptable for a background janitor; fails toward doing nothing).

**Note on TDD:** this is a background-timer + DB-query + event feature with no pure logic to unit-test in isolation; verified by `cargo build` + the Task 3 runtime checklist (consistent with how the codebase verifies orchestrator/event code).

## Explicitly out of scope
Persisting suggestions; a settings toggle for the interval; re-embedding stragglers; acting on failed-transcript rows; deduping against already-in-flight jobs (the 6h cadence + idempotent jobs make re-enqueue harmless).
