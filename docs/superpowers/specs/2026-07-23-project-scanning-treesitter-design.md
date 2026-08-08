# Project Scanning via tree-sitter (Phase 5) — Design

**Date:** 2026-07-23
**Status:** Design approved — ready for implementation plan
**Depends on:** Phase 1 (concept graph tables + confidence tag on edges).

---

## Goal

Ground concept graphs in real code. Today every concept/edge is LLM-inferred. Phase 5 adds a native
Rust tree-sitter parser that walks a project directory, parses source files into ASTs, and extracts
actual code structure as **code-grounded** concept nodes (carrying `file_path`/line) and
`confidence='extracted'` edges — interleaved with the existing inferred graph, so the user can trust
edges differently depending on whether they came from code or from an LLM guess.

---

## Decisions locked during brainstorming

| Fork | Decision | Why |
|---|---|---|
| Grammars | **5 bundled: Python, JavaScript, TypeScript, Rust, Go.** | Cover >90% of projects; more are a Cargo.toml add. |
| Trigger | **`scan_project` agent tool (synchronous) + extension `/api/library` hook.** | Scanning is fast; no background job day one. |
| Output | **Merge into the tree's existing concept graph**, tagged extracted. One graph, two tiers. | User queries one graph; confidence tells trust. |
| Duplicate resolution | **By title (case-insensitive).** Extracted node matching an inferred node ENRICHES it (adds file_path/line, flips it to extracted); edges supplement. | No duplicate nodes; inferred nodes gain code locations. |
| Ignored paths | **Respect `.gitignore` (`ignore` crate) + always skip node_modules/target/venv/__pycache__/.git.** | Standard, robust. |

---

## Grounding corrections (verified against Phase 1 schema + Windows host 2026-07-23 — AUTHORITATIVE)

These override any conflicting SQL/claims below.

1. **`concept_graph_nodes` has NO `confidence` column** (migration 050 columns: id, graph_id, title,
   description, file_path, line_start, line_end, embedding, created_at; +concept_id from 051).
   Confidence exists only on EDGES. → **Migration 052 adds** `confidence TEXT NOT NULL DEFAULT
   'inferred' CHECK(confidence IN ('extracted','inferred','ambiguous'))` to `concept_graph_nodes`.
   Existing nodes become `'inferred'` (correct — they were LLM-inferred). This makes node-level
   extracted/inferred explicit and queryable, matching the design's intent. **So "no migrations" is
   WRONG — Phase 5 needs migration 052.**
2. **No UNIQUE index on `(graph_id, LOWER(title))`** exists (only the non-unique `idx_cgn_title_lower`),
   and one CANNOT be safely added because Phase 1's `derive_graph` can emit two nodes with the same
   title in one graph. → **Node merge is an application-level upsert** (SELECT id WHERE graph_id=$1 AND
   LOWER(title)=LOWER($2) LIMIT 1; if found UPDATE file_path/line + set confidence='extracted'; else
   INSERT with confidence='extracted'). The spec's `ON CONFLICT (graph_id, LOWER(title))` is NOT usable.
3. **Edge merge uses the existing `idx_cge_unique`** UNIQUE(graph_id, source_node_id, target_node_id,
   relationship) → `INSERT ... ON CONFLICT (graph_id, source_node_id, target_node_id, relationship) DO
   NOTHING` works as written.
4. **Graph resolution:** use `crate::concept_graph::graph_id_for_tree(pool, tree_id)`. If it returns
   `None` (tree has no graph yet), INSERT a bare `concept_graphs` row (id + tree_id) directly and use
   that id — do NOT call `create_concept_graph_from_concepts` (it errors on empty concepts).
5. **Windows C toolchain is a real go/no-go.** Host is `x86_64-pc-windows-msvc`; the 5 tree-sitter
   grammar crates compile C via `cc`, needing MSVC `cl.exe` (VS Build Tools, "Desktop development with
   C++"). This project is pure-Rust today, so that toolchain may be unexercised. **Task 1 is a canary:
   add the deps and `cargo build`; if the C compile/link fails, STOP and report — the user may need to
   install VS Build Tools. Do not proceed to later tasks until Task 1 builds.**
6. **API traps for the builder (version-dependent — follow the actual installed crate):**
   - tree-sitter 0.24's `QueryCursor::matches(...)` returns a **`StreamingIterator`**, so
     `for m in cursor.matches(...)` does NOT compile — use `while let Some(m) = it.next()` with
     `use streaming_iterator::StreamingIterator;` (add the `streaming-iterator` crate if needed).
   - TypeScript grammar exposes **`LANGUAGE_TYPESCRIPT`** and `LANGUAGE_TSX`, not `LANGUAGE`.
   - `Parser::set_language` takes `&Language`; grammar entry points are `tree_sitter_x::LANGUAGE` (a
     `LanguageFn`) → `.into()`. Pin tree-sitter core + grammar versions that are ABI-compatible; if the
     canary shows a type/ABI mismatch, align versions (Task 1 resolves this).

---

## Architecture

```
"Scan my project at <path>"  →  agent scan_project tool  →  project_scanner::scan_project(pool, path, tree_id)
    1. walk dir (ignore crate: .gitignore + always-skip node_modules/target/venv/__pycache__/.git)
    2. per source file: grammar-by-extension → parse AST → extract functions/classes (nodes),
       calls ('implements'), imports ('references'), inheritance/impl ('prerequisite')
    3. merge_into_graph(graph_id, nodes, edges): app-level node upsert (enrich or insert, confidence='extracted');
       edge insert ON CONFLICT DO NOTHING (confidence='extracted')
    4. return ScanResult summary
Also: ext_server POST /api/library with local_path → after ingest, fire-and-forget scan_project.
```

## Component 1 — Dependencies (`Cargo.toml`)

```toml
tree-sitter = "0.24"
tree-sitter-python = "0.23"
tree-sitter-javascript = "0.23"
tree-sitter-typescript = "0.23"
tree-sitter-rust = "0.23"
tree-sitter-go = "0.23"
ignore = "0.4"
streaming-iterator = "0.1"   # for QueryCursor::matches (tree-sitter 0.24)
```
(`walkdir` is optional — `ignore::WalkBuilder` already does recursive, gitignore-aware traversal.)
Task 1 verifies these compile on the MSVC host BEFORE any code is written (see correction 5).

## Component 2 — Migration 052

`migrations/052_concept_node_confidence.sql`:
```sql
ALTER TABLE concept_graph_nodes
    ADD COLUMN IF NOT EXISTS confidence TEXT NOT NULL DEFAULT 'inferred'
    CHECK (confidence IN ('extracted', 'inferred', 'ambiguous'));
```

## Component 3 — Scanner (`src-tauri/src/project_scanner.rs`)

```rust
pub(crate) struct ScanResult {
    pub files_scanned: u32,
    pub files_skipped: u32,      // unsupported language
    pub nodes_added: u32,
    pub nodes_enriched: u32,
    pub edges_added: u32,
    pub errors: Vec<String>,     // per-file parse errors (non-fatal)
}

pub(crate) async fn scan_project(pool: &PgPool, path: &str, tree_id: &str) -> Result<ScanResult, String>;
```
- `grammar_for_extension(ext) -> Option<Language>`: py/pyw/pyx→Python; js/jsx/mjs/cjs→JavaScript;
  ts/tsx/mts/cts→TypeScript (`LANGUAGE_TYPESCRIPT`/`LANGUAGE_TSX`); rs→Rust; go→Go; else None.
- Extraction (per language, own query strings): functions/classes/structs/traits/interfaces → nodes
  (title=name, description e.g. "Python function", file_path, line_start/line_end); calls → `implements`
  edges (caller→callee); imports/use → `references` edges; inheritance/impl → `prerequisite` edges. Edge
  endpoints are resolved to node ids after nodes are upserted (match by title within the graph); an edge
  whose endpoint title has no node is skipped.
- Pure helpers get unit tests: `grammar_for_extension`, and per-language name-extraction from a small
  source snippet (parse → assert extracted function/struct names). DB merge is verified in the runtime
  checklist.

## Component 4 — Merge (`project_scanner.rs`)

`merge_into_graph(pool, graph_id, nodes, edges) -> Result<MergeStats,String>` — node upsert per
correction 2; edge insert per correction 3. All INSERTs supply `confidence='extracted'` (nodes now
have the column from migration 052).

## Component 5 — Agent tool (`mimir_agent.rs`)

`scan_project { path }` schema (appended in `tool_schemas`, unconditional). execute arm:
resolves `ctx.tree_id` (error if none), calls `scan_project(ctx.pool, path, tree_id)`, returns the
summary string. Update the tool-count unit test accordingly. Add one line to the system-prompt tool
guidance.

## Component 6 — Extension hook (`ext_server.rs`)

`create_library_handler` / `CreateLibraryBody` (line ~244/453): add optional `local_path: Option<String>`
(and `tree_id: Option<String>` if not already present). After ingest succeeds, if BOTH `local_path` and
a `tree_id` are non-empty, `tokio::spawn` a fire-and-forget `scan_project(&pool, &local_path, &tree_id)`
(log result). The HTTP response returns immediately with the normal ingest result.

---

## Files

**Create:** `src-tauri/migrations/052_concept_node_confidence.sql`; `src-tauri/src/project_scanner.rs`.
**Modify:** `src-tauri/Cargo.toml` (deps); `src-tauri/src/main.rs` (`mod project_scanner;`);
`src-tauri/src/mimir_agent.rs` (schema + arm + test); `src-tauri/src/mimir_retrieval.rs` (tool-guidance
line); `src-tauri/src/ext_server.rs` (local_path hook).
**No change:** mimir_memory / orchestrator / concept_graph (verbs already query the enriched graph) /
hitl / hound_client / dev.sh.

---

## Error handling

Path missing → `Err("Directory not found: <path>")`. Path is a file → scan just that file. No supported
files → friendly message. Per-file parse error → skip, push to `errors`, continue. Graph absent → create
a bare `concept_graphs` row (correction 4). Large repo (>10s) → synchronous; agent warns; background
variant deferred.

---

## Verification checklist

- **Task 1 canary:** `cargo build` compiles the 5 C grammars on the MSVC host. If not → STOP, report toolchain.
- `cargo test` passes (grammar selection + per-language name extraction + any pure merge helpers).
- `scan_project` on a small mock project returns correct counts; nodes appear with `confidence='extracted'`,
  file_path, line_start/line_end; edges appear with `confidence='extracted'`.
- A title matching an existing inferred node ENRICHES it (file/line added, confidence→extracted), no duplicate.
- `query_graph`/`explain_node` return the code-grounded node with location + code edges.
- Unsupported-only dir → graceful; syntax-error file → skipped + logged, scan continues; missing path → clear error.
- `scan_project` tool visible/callable; extension `/api/library` with `local_path` → ingest returns + scan runs (server log).

## Build order (for the plan)

1. **Cargo.toml deps + `cargo build` canary (go/no-go on the C toolchain).**
2. Migration 052 (node confidence column).
3. `project_scanner.rs` — walker, grammar selection, per-language extraction, app-level merge, unit tests.
4. `scan_project` agent tool (schema + arm + tool-count test + guidance line).
5. Extension hook in `ext_server.rs`.
6. Register `mod project_scanner;` in main.rs; `cargo build` + verification.

## Explicitly out of scope

More grammars; background job for huge repos; file watching; remote/Docker scanning; removing inferred
nodes (extracted supplements, never replaces).
