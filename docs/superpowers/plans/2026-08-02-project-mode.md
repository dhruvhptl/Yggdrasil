# Project Mode — Mimir Grounds in Real Code Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let the user talk to Mimir about their real code projects grounded in scanned source — one de-duplicated concept graph per project, embedded symbols for semantic search, an LLM coverage pass that answers "did my finished repo cover the tree I planned," and project-anchored chat that no longer requires a selected tree node.

**Architecture:** `concept_graphs` gains a nullable `project_root_id`; a graph binds to a project **at scan time** (go-forward, no fuzzy backfill — existing graphs adopt project scope on re-scan). The scanner embeds each extracted symbol so the dormant HNSW index goes live. A coverage pass compares the project graph against the checkpoints of the tree it was scanned from and posts a chip. Chat sessions gain an optional `project_root_id` scope (with a partial unique index) so you can talk to Mimir with only a project selected.

**Tech Stack:** Rust (Tauri 2, sqlx, Postgres/pgvector), React 19 + TypeScript. Embeddings via OpenRouter `perplexity/pplx-embed-v1-0.6b` (1024-dim, L2-normalized).

## Global Constraints

- **Migrations are append-only, auto-run on startup. Highest on disk today is `056`.** This plan adds `057` (project graphs), `058` (project coverage), `059` (project sessions) — in that order.
- Rust: `$N` placeholders (never `?N`); non-macro `sqlx::query()`; `#[serde(rename_all="camelCase")]` on structs crossing the boundary; `database: State<'_, Database>` last; commands return `Result<T, String>`.
- Never hardcode `DATABASE_URL`; never commit `.env`; env vars come from `dev.sh`/Bitwarden; no `dotenvy` in Rust. JSONB not TEXT.
- Embeddings: `get_embeddings_batch(client, texts)` (mimir_ingest.rs:101), 50 texts/call, returns `Vec<Vec<f32>>` in order; format vectors for SQL with `vector_str(&[f32])` (mimir_ingest.rs:93) → bind as `$N::vector`. Do NOT add a `"dimensions"` field. Reuse the shared `reqwest::Client` (never construct a new one).
- **Decision (locked): graph↔project binding is go-forward-on-scan. NO backfill UPDATE.** Existing tree-scoped graphs adopt `project_root_id` the next time their folder is scanned.
- Branch `fix-compilation-errors`; commit locally, do NOT push. No Fable review (one consolidated v3 audit is due after the whole v3 stream).
- **Build/test in the isolated target dir.** Before any cargo command:
  `export CARGO_TARGET_DIR="C:/Users/dhruv/AppData/Local/Temp/claude/C--Users-dhruv-projects-Yggdrasil/497ed12f-520a-4774-9c54-620ab075f017/scratchpad/scan-target"`
  The crate is a **binary** (`universal-skill-tree`) — run tests with `cargo test --bin universal-skill-tree <name> -- --nocapture` (there is no lib target). Frontend gate: `npx tsc --noEmit` (exit 0).

## Milestones

- **Milestone 1 — Grounded coverage (Phases 0–3):** the payoff the user actually asked for ("compare my tree to my finished repo"). Shippable and testable on its own.
- **Milestone 2 — Project chat (Phases 4–5):** the "talk to Mimir about all my projects" UX. Depends on Milestone 1's project graphs + embeddings.

## File Structure

| File | Change | Phase |
|---|---|---|
| `src/components/MimirChat.tsx` | Fix StrictMode listener leak; "refreshed N" wording; later: project picker, coverage chip | 0, 3, 5 |
| `src-tauri/src/project_scanner.rs` | "new/refreshed" summary; project-graph resolution; embed symbols; coverage enqueue | 0, 1, 2, 3 |
| `src-tauri/migrations/057_project_graphs.sql` | Create — `project_root_id` on `concept_graphs`, drop `tree_id NOT NULL` | 1 |
| `src-tauri/src/project_roots.rs` | Add `resolve_project_root_id(pool, path)` | 1 |
| `src-tauri/src/concept_graph.rs` | `graph_id_for_project`; `graph_semantic_search`; project-target verbs | 1, 4 |
| `src-tauri/src/orchestrator.rs` | Thread `client` into scan; `ProjectCoverage` job | 2, 3 |
| `src-tauri/migrations/058_project_coverage.sql` | Create — `project_coverage` table | 3 |
| `src-tauri/src/project_coverage.rs` | Create — coverage pass + command | 3 |
| `src-tauri/src/mimir_agent.rs` | `ToolCtx.project_root_id`; semantic-search tool; project-aware verbs; system prompt | 4 |
| `src-tauri/migrations/059_project_sessions.sql` | Create — `project_root_id` on sessions + partial unique index | 5 |
| `src-tauri/src/mimir_retrieval.rs` | Shared `resolve_session_id`; `mimir_chat`/`get_chat_session`/`clear_chat_session` project scope | 5 |
| `src-tauri/src/main.rs` | Register new commands / module | 2, 3, 5 |
| `src/contexts/MimirContext.tsx` | `projectRootId` + `projectLabel` | 5 |

---

## Phase 0 — Quick UX truth fixes (no schema)

### Task 0: Honest scan message + StrictMode dedupe

**Files:**
- Modify: `src-tauri/src/project_scanner.rs` (`scan_project_with_events` summary text, ~:494-514)
- Modify: `src/components/MimirChat.tsx` (scan-complete summary text ~:156-166; listener effect ~:149-169)

**Interfaces:**
- Produces: no new signatures. `ygg-scan-complete` payload already carries `nodesEnriched` (project_scanner.rs:509) and `nodesAdded`.

- [ ] **Step 1: Server summary reports new AND refreshed.** In `scan_project_with_events`, change the success `summary`:

```rust
let summary = format!(
    "🗂️ Scan complete — {}: {} files scanned, {} new concepts, {} refreshed, {} links.",
    name, r.files_scanned, r.nodes_added, r.nodes_enriched, r.edges_added
);
```

- [ ] **Step 2: Client echo matches + reads `nodesEnriched`.** In `MimirChat.tsx`, widen the listener payload type to include `nodesEnriched?: number` and update the success string to `... ${payload.nodesAdded ?? 0} new concepts, ${payload.nodesEnriched ?? 0} refreshed, ${payload.edgesAdded ?? 0} links.` (keep the `treeId`/`nodeId` same-context guard from `1d3381b`).

- [ ] **Step 3: Fix the StrictMode double-listener leak.** Replace the scan-listener effect body so cleanup cancels a pending subscription:

```tsx
useEffect(() => {
  let unsubP: (() => void) | undefined;
  let unsubC: (() => void) | undefined;
  let cancelled = false;
  (async () => {
    const p = await listen<{ status: string; path: string }>("ygg-scan-progress", ({ payload }) => {
      setScanStatus(`Scanning ${payload.path}…`);
    });
    const c = await listen<{ treeId?: string; nodeId?: string | null; project?: string; filesScanned?: number; nodesAdded?: number; nodesEnriched?: number; edgesAdded?: number; error?: string }>(
      "ygg-scan-complete",
      ({ payload }) => {
        setScanStatus(null);
        const proj = payload.project ? `${payload.project}: ` : "";
        const summary = payload.error
          ? `🗂️ Scan of ${payload.project ?? "project"} failed: ${payload.error}`
          : `🗂️ Scan complete — ${proj}${payload.filesScanned ?? 0} files scanned, ${payload.nodesAdded ?? 0} new concepts, ${payload.nodesEnriched ?? 0} refreshed, ${payload.edgesAdded ?? 0} links.`;
        const sameCtx = payload.treeId === treeIdRef.current && (payload.nodeId ?? null) === (nodeIdRef.current ?? null);
        if (sameCtx) {
          setMessages((prev) => [...prev, { role: "mimir", content: summary, createdAt: new Date().toISOString() }]);
        }
      }
    );
    if (cancelled) { p(); c(); return; }
    unsubP = p; unsubC = c;
  })();
  return () => { cancelled = true; unsubP?.(); unsubC?.(); };
}, []);
```

- [ ] **Step 4: Verify.** `npx tsc --noEmit` (exit 0). Backend: `cargo build` clean.

- [ ] **Step 5: Commit.**

```bash
git add src-tauri/src/project_scanner.rs src/components/MimirChat.tsx
git commit -m "fix(scan): report refreshed concepts + stop StrictMode double-post"
```

---

## Phase 1 — Project-scoped concept graphs (go-forward)

### Task 1: Migration 057 + project-root path resolver

**Files:**
- Create: `src-tauri/migrations/057_project_graphs.sql`
- Modify: `src-tauri/src/project_roots.rs` (add `resolve_project_root_id`)
- Test: `project_roots.rs` `#[cfg(test)]` module

**Interfaces:**
- Produces: `resolve_project_root_id(pool: &PgPool, path: &str) -> Option<String>` — the registered root **id** whose canonical path contains `path`, else `None`.

- [ ] **Step 1: Migration 057** — `src-tauri/migrations/057_project_graphs.sql`:

```sql
-- Bind a concept graph to a registered project (go-forward, set at scan time).
ALTER TABLE concept_graphs
    ADD COLUMN IF NOT EXISTS project_root_id TEXT REFERENCES project_roots(id) ON DELETE SET NULL;
ALTER TABLE concept_graphs ALTER COLUMN tree_id DROP NOT NULL;
CREATE INDEX IF NOT EXISTS idx_cg_project ON concept_graphs(project_root_id);
```

- [ ] **Step 2: Write the failing test** for the resolver (in `project_roots.rs` tests, next to `strip_verbatim_normalizes_windows_prefixes`). Because `resolve_project_root_id` needs a pool, test the pure containment predicate instead — extract it:

```rust
/// True when `child` is inside `root` (both already canonicalized).
pub(crate) fn path_contains(root: &Path, child: &Path) -> bool {
    child.starts_with(root)
}
```

```rust
#[test]
fn path_contains_matches_nested_and_rejects_siblings() {
    let root = Path::new(r"C:\Users\dhruv\projects\kelvin");
    assert!(path_contains(root, Path::new(r"C:\Users\dhruv\projects\kelvin\src\main.py")));
    assert!(path_contains(root, root));
    assert!(!path_contains(root, Path::new(r"C:\Users\dhruv\projects\other")));
}
```

- [ ] **Step 3: Run test to verify it fails.** `cargo test --bin universal-skill-tree path_contains` → FAIL (`path_contains` not found).

- [ ] **Step 4: Implement** `path_contains` (above) and `resolve_project_root_id`:

```rust
pub(crate) async fn resolve_project_root_id(pool: &PgPool, path: &str) -> Option<String> {
    let canon = std::fs::canonicalize(path).ok()?;
    for r in get_project_roots(pool).await {
        if let Ok(rc) = std::fs::canonicalize(&r.path) {
            if path_contains(&rc, &canon) { return Some(r.id); }
        }
    }
    None
}
```

- [ ] **Step 5: Run test to verify it passes.** `cargo test --bin universal-skill-tree path_contains` → PASS. Then `cargo build` clean.

- [ ] **Step 6: Commit.**

```bash
git add src-tauri/migrations/057_project_graphs.sql src-tauri/src/project_roots.rs
git commit -m "feat(graph): project_root_id on concept_graphs + path->root resolver"
```

### Task 2: Scanner binds the graph to the project (adopt-on-rescan)

**Files:**
- Modify: `src-tauri/src/project_scanner.rs` (`resolve_or_create_graph` → project-aware; `scan_project_inner`)
- Modify: `src-tauri/src/concept_graph.rs` (add `graph_id_for_project`)

**Interfaces:**
- Consumes: `resolve_project_root_id` (Task 1), `graph_id_for_tree` (concept_graph.rs:223).
- Produces: `graph_id_for_project(pool, project_root_id) -> Result<Option<String>, String>`; `resolve_or_create_project_graph(pool, project_root_id: Option<&str>, tree_id: &str) -> Result<String, String>`.

- [ ] **Step 1: `graph_id_for_project`** in `concept_graph.rs` (mirror `graph_id_for_tree`):

```rust
pub(crate) async fn graph_id_for_project(pool: &PgPool, project_root_id: &str) -> Result<Option<String>, String> {
    let row = sqlx::query("SELECT id FROM concept_graphs WHERE project_root_id = $1 ORDER BY created_at DESC LIMIT 1")
        .bind(project_root_id)
        .fetch_optional(pool).await.map_err(|e| e.to_string())?;
    Ok(row.and_then(|r| r.try_get::<String, _>("id").ok()))
}
```

- [ ] **Step 2: `resolve_or_create_project_graph`** in `project_scanner.rs` (replaces the call to `resolve_or_create_graph` inside `scan_project_inner`). Adoption order: existing project graph → adopt this tree's existing graph → create new.

```rust
async fn resolve_or_create_project_graph(pool: &PgPool, project_root_id: Option<&str>, tree_id: &str) -> Result<String, String> {
    let Some(pr) = project_root_id else {
        return resolve_or_create_graph(pool, tree_id).await; // legacy: tree-only
    };
    if let Some(id) = crate::concept_graph::graph_id_for_project(pool, pr).await? {
        return Ok(id);
    }
    // Adopt this tree's pre-existing (tree-only) graph so re-scan migrates in place.
    if let Some(id) = crate::concept_graph::graph_id_for_tree(pool, tree_id).await? {
        sqlx::query("UPDATE concept_graphs SET project_root_id = $1 WHERE id = $2")
            .bind(pr).bind(&id).execute(pool).await.map_err(|e| e.to_string())?;
        return Ok(id);
    }
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO concept_graphs (id, tree_id, project_root_id) VALUES ($1, $2, $3)")
        .bind(&id).bind(tree_id).bind(pr).execute(pool).await.map_err(|e| e.to_string())?;
    Ok(id)
}
```

- [ ] **Step 3: Wire it into `scan_project_inner`.** At the top (after the `root.exists()` check), resolve the root id and use the new resolver:

```rust
let project_root_id = crate::project_roots::resolve_project_root_id(pool, path).await;
let graph_id = resolve_or_create_project_graph(pool, project_root_id.as_deref(), tree_id).await?;
```
(Replace the existing `let graph_id = resolve_or_create_graph(pool, tree_id).await?;`.)

- [ ] **Step 4: Verify.** `cargo build` clean. Runtime (deferred to the Phase-1 checklist): scanning `kelvin` twice from the same tree yields **one** graph now carrying `project_root_id`, and a second scan is `enriched`, not a new graph.

- [ ] **Step 5: Commit.**

```bash
git add src-tauri/src/project_scanner.rs src-tauri/src/concept_graph.rs
git commit -m "feat(scan): bind graph to project_root at scan time, adopt on re-scan"
```

---

## Phase 2 — Embed scanned symbols (light up HNSW)

### Task 3: Thread the HTTP client into the scan pipeline + embed nodes

**Files:**
- Modify: `src-tauri/src/orchestrator.rs` (`run_scan_project` gains `client`)
- Modify: `src-tauri/src/project_scanner.rs` (`scan_project_with_events`/`scan_project_inner` gain `client`; add `embed_graph_nodes`)
- Modify: `src-tauri/src/main.rs` (register the backfill command)

**Interfaces:**
- Consumes: `get_embeddings_batch` (mimir_ingest.rs:101), `vector_str` (mimir_ingest.rs:93).
- Produces: `embed_graph_nodes(pool, client, graph_id) -> Result<usize, String>` (count embedded); `backfill_scan_embeddings_cmd()` Tauri command.

- [ ] **Step 1: Thread `client` through the job.** In `orchestrator.rs`, the worker already holds `http_client` (from `start_worker`). Change `run_scan_project` and its call site (orchestrator.rs:127-128, :213) to pass `&http_client`, and forward to `scan_project_with_events(pool, app, client, path, tree_id, node_id)`. Then thread `client` into `scan_project_inner(pool, client, path, tree_id)`.

- [ ] **Step 2: Write the failing test** for the embed-text builder (pure). Add to `project_scanner.rs` tests:

```rust
#[test]
fn embed_text_joins_title_desc_path() {
    assert_eq!(embed_text("UNet", "", "kelvin/models/unet.py"), "UNet ::  :: kelvin/models/unet.py");
    assert_eq!(embed_text("rrf", "merge", "s.py"), "rrf :: merge :: s.py");
}
```

- [ ] **Step 3: Run to verify fail.** `cargo test --bin universal-skill-tree embed_text` → FAIL.

- [ ] **Step 4: Implement `embed_text` + `embed_graph_nodes`.**

```rust
fn embed_text(title: &str, description: &str, file_path: &str) -> String {
    format!("{} :: {} :: {}", title, description, file_path)
}

/// Embed every extracted node in this graph that still has NULL embedding.
/// Batch failures degrade gracefully (Err surfaces to the caller, which pushes
/// it to ScanResult.errors — the scan still completes without vectors).
async fn embed_graph_nodes(pool: &PgPool, client: &reqwest::Client, graph_id: &str) -> Result<usize, String> {
    use sqlx::Row;
    let rows = sqlx::query(
        "SELECT id, title, description, file_path FROM concept_graph_nodes \
         WHERE graph_id = $1 AND embedding IS NULL"
    ).bind(graph_id).fetch_all(pool).await.map_err(|e| e.to_string())?;
    let mut done = 0usize;
    for chunk in rows.chunks(50) {
        let texts: Vec<String> = chunk.iter().map(|r| embed_text(
            &r.try_get::<String,_>("title").unwrap_or_default(),
            &r.try_get::<String,_>("description").unwrap_or_default(),
            &r.try_get::<Option<String>,_>("file_path").ok().flatten().unwrap_or_default(),
        )).collect();
        let vecs = crate::mimir_ingest::get_embeddings_batch(client, &texts).await?;
        for (r, v) in chunk.iter().zip(vecs.iter()) {
            let id: String = r.try_get("id").unwrap_or_default();
            sqlx::query("UPDATE concept_graph_nodes SET embedding = $1::vector WHERE id = $2")
                .bind(crate::mimir_ingest::vector_str(v)).bind(&id)
                .execute(pool).await.map_err(|e| e.to_string())?;
            done += 1;
        }
    }
    Ok(done)
}
```

- [ ] **Step 5: Call it at the end of `scan_project_inner`** (after `merge_into_graph`, before returning `result`); on error push to `result.errors` instead of failing:

```rust
if let Err(e) = embed_graph_nodes(pool, client, &graph_id).await {
    result.errors.push(format!("embedding: {}", e));
}
```

- [ ] **Step 6: Backfill command** in `project_scanner.rs` + register in `main.rs`:

```rust
#[tauri::command]
pub async fn backfill_scan_embeddings_cmd(
    client: tauri::State<'_, reqwest::Client>,
    database: tauri::State<'_, crate::database::Database>,
) -> Result<usize, String> {
    use sqlx::Row;
    let graph_ids: Vec<String> = sqlx::query(
        "SELECT DISTINCT graph_id FROM concept_graph_nodes WHERE embedding IS NULL"
    ).fetch_all(&database.pool).await.map_err(|e| e.to_string())?
     .iter().filter_map(|r| r.try_get::<String,_>("graph_id").ok()).collect();
    let mut total = 0usize;
    for gid in graph_ids { total += embed_graph_nodes(&database.pool, &client, &gid).await?; }
    Ok(total)
}
```

- [ ] **Step 7: Verify.** `cargo test --bin universal-skill-tree embed_text` → PASS. `cargo build` clean.

- [ ] **Step 8: Commit.**

```bash
git add src-tauri/src/orchestrator.rs src-tauri/src/project_scanner.rs src-tauri/src/main.rs
git commit -m "feat(scan): embed extracted symbols (HNSW) + backfill command"
```

---

## Phase 3 — Coverage pass (the payoff)

### Task 4: `project_coverage` table + coverage pass + auto-enqueue + chip

**Files:**
- Create: `src-tauri/migrations/058_project_coverage.sql`
- Create: `src-tauri/src/project_coverage.rs`
- Modify: `src-tauri/src/orchestrator.rs` (add `ProjectCoverage` job; enqueue after scan)
- Modify: `src-tauri/src/main.rs` (`mod project_coverage;` + register command)
- Modify: `src/components/MimirChat.tsx` (listen `ygg-project-coverage`, render a chip)

**Interfaces:**
- Consumes: project graph nodes (Phase 1), embeddings (Phase 2), tree checkpoints (`tree_nodes`), `call_llm` (llm_client.rs).
- Produces: `coverage_for_project(pool, client, project_root_id, tree_id) -> Result<CoverageSummary, String>`; command `run_project_coverage_cmd(project_root_id, tree_id)`; event `ygg-project-coverage`.

- [ ] **Step 1: Migration 058** — `src-tauri/migrations/058_project_coverage.sql`:

```sql
CREATE TABLE IF NOT EXISTS project_coverage (
    id                TEXT PRIMARY KEY,
    project_root_id   TEXT NOT NULL REFERENCES project_roots(id) ON DELETE CASCADE,
    tree_id           TEXT REFERENCES trees(id) ON DELETE CASCADE,
    checkpoint_node_id TEXT NOT NULL,
    checkpoint_title  TEXT NOT NULL,
    status            TEXT NOT NULL CHECK(status IN ('covered','partial','gap')),
    evidence_node_id  TEXT,
    reason            TEXT NOT NULL DEFAULT '',
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(project_root_id, checkpoint_node_id)
);
CREATE INDEX IF NOT EXISTS idx_pcov_project ON project_coverage(project_root_id);
```

- [ ] **Step 2: `project_coverage.rs` — types + the pass.** The pass: gather the tree's skill checkpoints (branch nodes), for each embed its title and ANN over the project graph for top-3 evidence symbols, then one batched LLM call classifies each `covered|partial|gap` with an evidence node + one-line reason; upsert rows; return counts.

```rust
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CoverageSummary { pub covered: u32, pub partial: u32, pub gap: u32, pub total: u32 }

#[derive(Deserialize)]
struct LlmVerdict { checkpoint_id: String, status: String, evidence_node_id: Option<String>, reason: String }

pub(crate) async fn coverage_for_project(
    pool: &PgPool, client: &reqwest::Client, project_root_id: &str, tree_id: &str,
) -> Result<CoverageSummary, String> {
    // 1. Checkpoints = skill branch nodes (branch whose parent is a branch/phase).
    let checkpoints = sqlx::query(
        "SELECT n.id, n.title FROM tree_nodes n \
         JOIN tree_nodes parent ON n.parent_id = parent.id \
         WHERE n.tree_id = $1 AND n.type = 'branch' AND parent.type = 'branch'"
    ).bind(tree_id).fetch_all(pool).await.map_err(|e| e.to_string())?;

    let Some(graph_id) = crate::concept_graph::graph_id_for_project(pool, project_root_id).await? else {
        return Ok(CoverageSummary { covered: 0, partial: 0, gap: checkpoints.len() as u32, total: checkpoints.len() as u32 });
    };

    // 2. Per checkpoint, ANN top-3 evidence symbols (needs Phase 2 embeddings).
    let mut prompt_items = String::new();
    for c in &checkpoints {
        let cid: String = c.try_get("id").unwrap_or_default();
        let title: String = c.try_get("title").unwrap_or_default();
        let qvec = crate::mimir_ingest::get_embeddings_batch(client, &[title.clone()]).await?
            .into_iter().next().ok_or("no query embedding")?;
        let ev = sqlx::query(
            "SELECT id, title, file_path FROM concept_graph_nodes \
             WHERE graph_id = $1 AND embedding IS NOT NULL \
             ORDER BY embedding <=> $2::vector LIMIT 3"
        ).bind(&graph_id).bind(crate::mimir_ingest::vector_str(&qvec))
         .fetch_all(pool).await.map_err(|e| e.to_string())?;
        let ev_str = ev.iter().map(|r| format!("[{}] {} ({})",
            r.try_get::<String,_>("id").unwrap_or_default(),
            r.try_get::<String,_>("title").unwrap_or_default(),
            r.try_get::<Option<String>,_>("file_path").ok().flatten().unwrap_or_default(),
        )).collect::<Vec<_>>().join("; ");
        prompt_items.push_str(&format!("- checkpoint_id={} title=\"{}\" candidates: {}\n", cid, title, ev_str));
    }

    // 3. One batched LLM classification (JSON array out). Reuse the tree-gen model path.
    let prompt = format!(
        "For each learning checkpoint, decide if the codebase implements it based on the candidate code symbols.\n\
         Return ONLY a JSON array: [{{\"checkpoint_id\":\"…\",\"status\":\"covered|partial|gap\",\"evidence_node_id\":\"… or null\",\"reason\":\"≤12 words\"}}].\n\
         covered = clearly implemented; partial = related code but incomplete; gap = no evidence.\n\n{}",
        prompt_items
    );
    let raw = crate::llm_client::call_llm(client, &prompt, "coverage_v1").await?;
    let json = raw.trim().trim_start_matches("```json").trim_start_matches("```").trim_end_matches("```").trim();
    let verdicts: Vec<LlmVerdict> = serde_json::from_str(json).map_err(|e| format!("coverage parse: {} raw: {}", e, crate::text_util::truncate_chars(json, 200)))?;

    // 4. Upsert + tally.
    let mut sum = CoverageSummary { covered: 0, partial: 0, gap: 0, total: verdicts.len() as u32 };
    let title_by_id: std::collections::HashMap<String,String> = checkpoints.iter()
        .map(|c| (c.try_get::<String,_>("id").unwrap_or_default(), c.try_get::<String,_>("title").unwrap_or_default())).collect();
    for v in &verdicts {
        match v.status.as_str() { "covered" => sum.covered += 1, "partial" => sum.partial += 1, _ => sum.gap += 1 }
        sqlx::query(
            "INSERT INTO project_coverage (id, project_root_id, tree_id, checkpoint_node_id, checkpoint_title, status, evidence_node_id, reason) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8) \
             ON CONFLICT (project_root_id, checkpoint_node_id) DO UPDATE SET \
               status = EXCLUDED.status, evidence_node_id = EXCLUDED.evidence_node_id, reason = EXCLUDED.reason, created_at = now()"
        )
        .bind(uuid::Uuid::new_v4().to_string()).bind(project_root_id).bind(tree_id)
        .bind(&v.checkpoint_id).bind(title_by_id.get(&v.checkpoint_id).cloned().unwrap_or_default())
        .bind(if v.status=="covered"||v.status=="partial"||v.status=="gap" { &v.status } else { "gap" })
        .bind(&v.evidence_node_id).bind(&v.reason)
        .execute(pool).await.map_err(|e| e.to_string())?;
    }
    Ok(sum)
}

#[tauri::command]
pub async fn run_project_coverage_cmd(
    project_root_id: String, tree_id: String,
    client: tauri::State<'_, reqwest::Client>,
    database: tauri::State<'_, crate::database::Database>,
) -> Result<CoverageSummary, String> {
    coverage_for_project(&database.pool, &client, &project_root_id, &tree_id).await
}
```
(Confirm `call_llm`'s exact signature in `llm_client.rs` during implementation and match it; the second arg here is the prompt-version label.)

- [ ] **Step 3: `ProjectCoverage` job + auto-enqueue.** In `orchestrator.rs` add `ProjectCoverage { project_root_id: String, tree_id: String }`; the handler calls `coverage_for_project` then `app.emit("ygg-project-coverage", { projectRootId, treeId, covered, partial, gap, total })`. Enqueue it at the end of `scan_project_with_events` **only when** a `project_root_id` was resolved (thread it out of `scan_project_inner` via `ScanResult`, or re-resolve from `path`). Guard: skip if there are zero checkpoints.

- [ ] **Step 4: Register** `mod project_coverage;` + `run_project_coverage_cmd` in `main.rs`.

- [ ] **Step 5: Frontend chip.** In `MimirChat.tsx`, add a `listen("ygg-project-coverage", …)` that sets a small state and renders a chip near the scan footer: `"{covered}/{total} covered · {partial} partial · {gap} gaps"`. Use the same cancel-guarded async-listener pattern as Task 0.

- [ ] **Step 6: Verify.** `cargo build` clean; `npx tsc --noEmit` exit 0. Runtime (checklist): scan `kelvin` from its tree → coverage chip shows counts; row count in `project_coverage` = checkpoint count; re-run is idempotent (upsert).

- [ ] **Step 7: Commit.**

```bash
git add src-tauri/migrations/058_project_coverage.sql src-tauri/src/project_coverage.rs src-tauri/src/orchestrator.rs src-tauri/src/main.rs src/components/MimirChat.tsx
git commit -m "feat(coverage): tree-vs-repo coverage pass + chip (auto after scan)"
```

> **Milestone 1 complete.** The user's original goal — "did my finished repo cover the tree I planned" — now works end-to-end. Phases 4–5 add the project-chat UX.

---

## Phase 4 — Semantic + project-aware graph verbs

### Task 5: `graph_semantic_search` + project-targeted verbs + ToolCtx

**Files:**
- Modify: `src-tauri/src/concept_graph.rs` (add `graph_semantic_search`; make verbs accept a resolved `graph_id`)
- Modify: `src-tauri/src/mimir_agent.rs` (`ToolCtx.project_root_id`; new tool schema + dispatch; system-prompt roster line)
- Modify: `src-tauri/src/mimir_retrieval.rs` (build `ToolCtx` with `project_root_id`)

**Interfaces:**
- Consumes: embeddings (Phase 2), `graph_id_for_project` (Phase 1).
- Produces: `graph_semantic_search(pool, client, graph_id, query, top_k) -> Result<Vec<SemanticHit>, String>` where `SemanticHit { node_id, title, file_path, score }`.

- [ ] **Step 1: `graph_semantic_search`** in `concept_graph.rs`:

```rust
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SemanticHit { pub node_id: String, pub title: String, pub file_path: Option<String>, pub score: f64 }

pub(crate) async fn graph_semantic_search(
    pool: &PgPool, client: &reqwest::Client, graph_id: &str, query: &str, top_k: i64,
) -> Result<Vec<SemanticHit>, String> {
    let qvec = crate::mimir_ingest::get_embeddings_batch(client, &[query.to_string()]).await?
        .into_iter().next().ok_or("no query embedding")?;
    let rows = sqlx::query(
        "SELECT id, title, file_path, 1 - (embedding <=> $2::vector) AS score \
         FROM concept_graph_nodes WHERE graph_id = $1 AND embedding IS NOT NULL \
         ORDER BY embedding <=> $2::vector LIMIT $3"
    ).bind(graph_id).bind(crate::mimir_ingest::vector_str(&qvec)).bind(top_k)
     .fetch_all(pool).await.map_err(|e| e.to_string())?;
    Ok(rows.iter().map(|r| SemanticHit {
        node_id: r.try_get("id").unwrap_or_default(),
        title: r.try_get("title").unwrap_or_default(),
        file_path: r.try_get("file_path").ok(),
        score: r.try_get("score").unwrap_or(0.0),
    }).collect())
}
```

- [ ] **Step 2: `ToolCtx.project_root_id`.** Add `pub project_root_id: Option<String>` to `ToolCtx` (mimir_agent.rs:419-433) and populate it where `ToolCtx` is built in `mimir_retrieval.rs` (from the new `mimir_chat` param — Task 7; until then default `None`).

- [ ] **Step 3: Graph resolution helper for tools.** In `mimir_agent.rs`, add a local `async fn graph_for_ctx(ctx) -> Option<String>`: prefer `graph_id_for_project(ctx.project_root_id)`, else `graph_id_for_tree(ctx.tree_id)`. Route `query_graph`/`path_between`/`explain_node` tool arms through it (they currently resolve by `tree_id`).

- [ ] **Step 4: New agent tool `graph_semantic_search`.** Add its schema in `tool_schemas(...)` gated on `!ctx.project_roots.is_empty()`; dispatch in the tool match: resolve `graph_for_ctx`, call `graph_semantic_search`, format hits (`title — file_path (score)`) as the tool output.

- [ ] **Step 5: System-prompt roster.** Where the roots line is assembled (mimir_retrieval.rs ~:1165), when `project_root_id` is set append: `You are grounded in project "<label>" at <path>. Prefer graph_semantic_search then read_file to verify claims, and cite file:line.`

- [ ] **Step 6: Verify.** `cargo build` clean. Runtime (checklist): from a project-scoped chat (Phase 5) ask a concept phrase → `graph_semantic_search` returns the right symbol.

- [ ] **Step 7: Commit.**

```bash
git add src-tauri/src/concept_graph.rs src-tauri/src/mimir_agent.rs src-tauri/src/mimir_retrieval.rs
git commit -m "feat(agent): semantic graph search + project-aware verbs + ToolCtx.project_root_id"
```

---

## Phase 5 — Project-scoped chat sessions

### Task 6: Migration 059 + shared `resolve_session_id`

**Files:**
- Create: `src-tauri/migrations/059_project_sessions.sql`
- Modify: `src-tauri/src/mimir_retrieval.rs` (add `resolve_session_id`; replace the two inline upserts at ~:1006 and ~:1548)
- Modify: `src-tauri/src/project_scanner.rs` (`post_scan_message` uses the shared helper)

**Interfaces:**
- Produces: `resolve_session_id(pool, project_root_id: Option<&str>, tree_id: Option<&str>, node_id: Option<&str>) -> Result<Option<String>, String>`.

- [ ] **Step 1: Migration 059** — `src-tauri/migrations/059_project_sessions.sql`:

```sql
ALTER TABLE mimir_chat_sessions
    ADD COLUMN IF NOT EXISTS project_root_id TEXT REFERENCES project_roots(id) ON DELETE CASCADE;
-- Project-only sessions dedupe on project_root_id (the (tree_id,node_id) UNIQUE
-- treats NULLs as distinct, so it can't).
CREATE UNIQUE INDEX IF NOT EXISTS uq_chat_session_project
    ON mimir_chat_sessions(project_root_id) WHERE project_root_id IS NOT NULL;
```

- [ ] **Step 2: Shared `resolve_session_id`.** Picks the one scope the caller has:

```rust
pub(crate) async fn resolve_session_id(
    pool: &sqlx::PgPool, project_root_id: Option<&str>, tree_id: Option<&str>, node_id: Option<&str>,
) -> Result<Option<String>, String> {
    use sqlx::Row;
    if let Some(pr) = project_root_id {
        let row = sqlx::query(
            "INSERT INTO mimir_chat_sessions (id, project_root_id) VALUES ($1, $2) \
             ON CONFLICT (project_root_id) DO UPDATE SET updated_at = NOW() RETURNING id"
        ).bind(uuid::Uuid::new_v4().to_string()).bind(pr)
         .fetch_one(pool).await.map_err(|e| e.to_string())?;
        return Ok(row.try_get("id").ok());
    }
    if let (Some(tid), Some(nid)) = (tree_id, node_id) {
        let row = sqlx::query(
            "INSERT INTO mimir_chat_sessions (id, tree_id, node_id) VALUES ($1, $2, $3) \
             ON CONFLICT (tree_id, node_id) DO UPDATE SET updated_at = NOW() RETURNING id"
        ).bind(uuid::Uuid::new_v4().to_string()).bind(tid).bind(nid)
         .fetch_one(pool).await.map_err(|e| e.to_string())?;
        return Ok(row.try_get("id").ok());
    }
    Ok(None)
}
```

- [ ] **Step 3: Replace the two inline upserts** at mimir_retrieval.rs ~:1006 (in `mimir_chat`) and ~:1548 (in `get_chat_session`) with `resolve_session_id(&database.pool, project_root_id.as_deref(), tree_id.as_deref(), node_id.as_deref()).await?`. Replace `post_scan_message`'s inline session upsert (project_scanner.rs) with the same helper (`resolve_session_id(pool, None, Some(tree_id), node_id)`).

- [ ] **Step 4: Verify.** `cargo build` clean + existing tests pass.

- [ ] **Step 5: Commit.**

```bash
git add src-tauri/migrations/059_project_sessions.sql src-tauri/src/mimir_retrieval.rs src-tauri/src/project_scanner.rs
git commit -m "feat(chat): project-scoped sessions + shared resolve_session_id"
```

### Task 7: `mimir_chat` project scope + frontend project picker

**Files:**
- Modify: `src-tauri/src/mimir_retrieval.rs` (`mimir_chat`, `get_chat_session`, `clear_chat_session` gain `project_root_id`)
- Modify: `src/contexts/MimirContext.tsx` (`projectRootId` + `projectLabel`)
- Modify: `src/components/MimirChat.tsx` (project picker; scope selection; remove the tree+node bail)

**Interfaces:**
- Consumes: `resolve_session_id` (Task 6), `get_project_roots_cmd` (project_roots.rs), `ToolCtx.project_root_id` (Task 5).

- [ ] **Step 1: Backend params.** Add `project_root_id: Option<String>` to `mimir_chat` (after `node_id`), pass it into `resolve_session_id` and into `ToolCtx`. Add `project_root_id: Option<String>` to `get_chat_session`/`clear_chat_session` and their session resolution. Keep tree/node params working unchanged.

- [ ] **Step 2: MimirContext.** Add `projectRootId: string | null`, `projectLabel: string | null`, their setters in `setMimirContext`, and defaults. (Mirror the existing `projectName` wiring.)

- [ ] **Step 3: MimirChat scope + picker.** Add a project `<select>` populated from `invoke("get_project_roots_cmd")`. Compute the session scope:
  - if `treeId && nodeId` → tree session (today's behavior);
  - else if `projectRootId` → project session.
  Change the bail at MimirChat.tsx:~184 so a selected project (no node) still loads/sends. `sessionKey` becomes `projectRootId ? "proj:"+projectRootId : treeId+":"+nodeId`. Pass `projectRootId` to `mimir_chat`, `get_chat_session`, `clear_chat_session`.

- [ ] **Step 4: Verify.** `npx tsc --noEmit` exit 0; `cargo build` clean.

- [ ] **Step 5: Commit.**

```bash
git add src-tauri/src/mimir_retrieval.rs src/contexts/MimirContext.tsx src/components/MimirChat.tsx
git commit -m "feat(chat): project picker + project-scoped mimir_chat (no node required)"
```

---

## Phase 6 — Runtime verification (user checklist)

**Files:** none. Requires `bash dev.sh` (applies migrations 057–059).

- [ ] Scan `kelvin` from its tree twice → **one** graph carrying `project_root_id`; second scan reports "N new, M refreshed" (not "0 concepts added"); no duplicate "Scan complete" line on dev reload.
- [ ] `backfill_scan_embeddings_cmd` (or a fresh scan) → `concept_graph_nodes.embedding` non-NULL for that project.
- [ ] Coverage chip shows `X/total covered · P partial · G gaps`; `project_coverage` rows persist; re-run idempotent.
- [ ] Open Mimir with **no tree/node selected**, pick a project → ask "explain <concept>" → answer cites `graph_semantic_search` hits + `read_file` lines as SourceCards.
- [ ] "Talk to me about all my projects" works from the project picker; switching projects re-scopes the chat.

---

## Self-Review

**Spec coverage:** Component 1 (project graphs, go-forward) → Phase 1; Component 2 (embeddings) → Phase 2; Component 5 (coverage) → Phase 3; Component 3 (semantic + project verbs) → Phase 4; Component 4 (project sessions) → Phase 5; Component 6 polish — StrictMode dedupe + "refreshed N" → Phase 0, shared `resolve_session_id` → Phase 5 Task 6, persisted scan message → already shipped (`1d3381b`). The scan tool-call **chip rendering** (Comp 6.4) is intentionally deferred as cosmetic (noted in the earlier scan-spec discussion) — the persisted text message already informs user + agent.

**Corrections baked in (from code verification 2026-08-02):** no backfill UPDATE (locked decision — go-forward); migration 059 adds the **partial unique index** on `project_root_id` so project sessions dedupe (the `(tree_id,node_id)` UNIQUE can't, NULLs distinct); StrictMode fix lives in `MimirChat.tsx`'s async listener, not `main.tsx`.

**Placeholder scan:** two "confirm during implementation" notes remain and are legitimate signature confirmations, not design gaps — `call_llm`'s exact arg order (llm_client.rs) and the roots-line insertion point (mimir_retrieval.rs ~:1165). All new files/functions have full code.

**Type consistency:** `resolve_project_root_id -> Option<String>` (root id) consumed by `resolve_or_create_project_graph(project_root_id: Option<&str>)`; `graph_id_for_project -> Result<Option<String>>` matches `graph_id_for_tree`; `resolve_session_id(project_root_id, tree_id, node_id)` used identically in `mimir_chat`, `get_chat_session`, `post_scan_message`; `CoverageSummary { covered, partial, gap, total }` emitted on `ygg-project-coverage` and consumed by the chip.

**Altitude caveat (carried from brainstorming):** embedding symbol *names* is a weak signal — the coverage LLM pass (Phase 3) is the real tree↔repo bridge; `graph_semantic_search` is a secondary signal. This is why Phase 3 (coverage) is the Milestone-1 payoff, not `graph_semantic_search`.

## Explicitly out of scope

Editing/writing files; remote/Docker scanning; multi-repo diffing; replacing the learning-tree concept graphs (they stay tree-scoped — only code-grounded graphs become project-scoped); importing `graphify-out/graph.json` (dev-time reference only); native graph force-layout; per-turn model routing (stays `deepseek/deepseek-v4-flash`); the scan tool-call chip rendering (deferred cosmetic).
