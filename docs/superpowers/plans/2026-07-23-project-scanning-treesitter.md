# Project Scanning via tree-sitter (Phase 5) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a native Rust tree-sitter scanner that parses a local project directory and merges code-grounded concept nodes (`file_path`/line, `confidence='extracted'`) and edges into the tree's existing concept graph, exposed as a `scan_project` agent tool + an extension `/api/library` hook.

**Architecture:** A new `project_scanner.rs` walks a directory (gitignore-aware via the `ignore` crate), parses each supported source file with tree-sitter, extracts functions/classes/calls/imports/inheritance, and merges them into the Phase 1 concept graph via an application-level node upsert (enrich-or-insert) + edge `ON CONFLICT DO NOTHING`. Migration 052 adds a `confidence` column to `concept_graph_nodes`. Agent tool + extension hook drive it.

**Tech Stack:** Rust (Tauri 2, sqlx, tree-sitter 0.24 + 5 grammar crates that compile C, `ignore`), Postgres.

**Spec:** `docs/superpowers/specs/2026-07-23-project-scanning-treesitter-design.md` — its **Grounding corrections** section is binding.

## Global Constraints

- **Task 1 is a go/no-go canary.** The 5 tree-sitter grammar crates compile C via `cc`; host is `x86_64-pc-windows-msvc`. If `cargo build` fails on the C compile/link, STOP and report (likely missing VS Build Tools "Desktop development with C++"). Do not attempt later tasks until Task 1 builds.
- SQL `$N`, non-macro `sqlx::query()` + `try_get`. IDs via `uuid::Uuid::new_v4().to_string()`.
- `concept_graph_nodes` gains a `confidence` column in migration 052; new/enriched code nodes use `confidence='extracted'`. Node merge is an APP-LEVEL upsert (SELECT then UPDATE/INSERT) — there is NO unique index on (graph_id, LOWER(title)) and one must not be added.
- Edge merge uses the existing `idx_cge_unique`: `ON CONFLICT (graph_id, source_node_id, target_node_id, relationship) DO NOTHING`.
- Graph resolution via `crate::concept_graph::graph_id_for_tree`; if None, INSERT a bare `concept_graphs` row (id+tree_id) — do NOT call `create_concept_graph_from_concepts` (errors on empty).
- tree-sitter 0.24 API traps: `QueryCursor::matches` returns a `StreamingIterator` (use `while let Some(m) = it.next()`, `use streaming_iterator::StreamingIterator;`); TypeScript grammar exposes `LANGUAGE_TYPESCRIPT`/`LANGUAGE_TSX` (not `LANGUAGE`). Verify exact node-type names + entry points against the ACTUAL installed crate versions — make it compile, don't blind-transcribe.
- New migration is **052** (051 is highest). Never modify an existing migration.
- All shell commands from `C:/Users/dhruv/projects/Yggdrasil/src-tauri`; branch `fix-compilation-errors`.
- Tests before this phase: 24. This phase adds grammar-selection + per-language extraction unit tests and updates one agent tool-count test.
- `cargo build` after Task 1 is permanently ~30-60s slower (C grammar compilation) — normal, do not kill it.

---

### Task 1: Dependencies + build canary (GO/NO-GO)

**Files:**
- Modify: `src-tauri/Cargo.toml`

**Interfaces:** Produces a compiling crate with tree-sitter + 5 grammars + `ignore` + `streaming-iterator` available.

- [ ] **Step 1: Add the dependencies**

In `src-tauri/Cargo.toml`, under `[dependencies]`, add:

```toml
tree-sitter = "0.24"
tree-sitter-python = "0.23"
tree-sitter-javascript = "0.23"
tree-sitter-typescript = "0.23"
tree-sitter-rust = "0.23"
tree-sitter-go = "0.23"
ignore = "0.4"
streaming-iterator = "0.1"
```

- [ ] **Step 2: Build — THE CANARY**

Run: `cargo build 2>&1 | tail -30`
Expected: `Finished`. First build is ~30-60s slower (C grammars).

**If it FAILS:**
- On a `cc`/`cl.exe`/linker error (C toolchain): STOP. Report BLOCKED with the error. The host needs MSVC C++ build tools. Do not continue.
- On a version/ABI mismatch (e.g., `expected Language, found ...`, or a grammar crate requiring a different tree-sitter core): adjust the version pins so the grammar crates and `tree-sitter` core are ABI-compatible (e.g., bump/lower grammar versions, or pin `tree-sitter` to the version the grammars target), then rebuild. Record the working version set in the report.

- [ ] **Step 3: Commit (only if Step 2 succeeded)**

```bash
git add Cargo.toml Cargo.lock
git commit -m "build(scanner): add tree-sitter + 5 grammars + ignore deps"
```

Report the exact working version set (from `Cargo.lock`) so later tasks match the real API.

---

### Task 2: Migration 052 — node confidence column

**Files:**
- Create: `src-tauri/migrations/052_concept_node_confidence.sql`

**Interfaces:** Produces `concept_graph_nodes.confidence TEXT NOT NULL DEFAULT 'inferred'` used by Tasks 3–4.

- [ ] **Step 1: Write the migration**

Create `src-tauri/migrations/052_concept_node_confidence.sql`:

```sql
-- Migration 052: node-level confidence (extracted vs inferred) on concept graph nodes.
-- Existing nodes were all LLM-inferred → default 'inferred'. tree-sitter scanning
-- writes 'extracted'.
ALTER TABLE concept_graph_nodes
    ADD COLUMN IF NOT EXISTS confidence TEXT NOT NULL DEFAULT 'inferred'
    CHECK (confidence IN ('extracted', 'inferred', 'ambiguous'));
```

- [ ] **Step 2: Verify numbering**

Run: `ls migrations | tail -3`
Expected: `050_concept_graphs.sql`, `051_concept_node_concept_id.sql`, `052_concept_node_confidence.sql`

- [ ] **Step 3: Commit**

```bash
git add migrations/052_concept_node_confidence.sql
git commit -m "feat(scanner): migration 052 — concept_graph_nodes.confidence column"
```

---

### Task 3: Scanner core + Python extraction + merge (prove the pipeline)

**Files:**
- Create: `src-tauri/src/project_scanner.rs`
- Modify: `src-tauri/src/main.rs` (`mod project_scanner;` after `mod hitl;`)

**Interfaces:**
- Consumes: `crate::concept_graph::graph_id_for_tree`, `crate::database::Database`, tree-sitter crates.
- Produces (used by Tasks 4–6): `pub(crate) struct ScanResult {...}` (camelCase serde), `pub(crate) fn grammar_for_extension(ext: &str) -> Option<tree_sitter::Language>`, `pub(crate) struct ExtractedNode { title: String, description: String, file_path: String, line_start: i32, line_end: i32 }`, `pub(crate) struct ExtractedEdge { source_title: String, target_title: String, relationship: &'static str }`, `pub(crate) fn extract_from_file(path: &str, source: &str) -> (Vec<ExtractedNode>, Vec<ExtractedEdge>)`, `pub(crate) async fn scan_project(pool: &sqlx::PgPool, path: &str, tree_id: &str) -> Result<ScanResult, String>`.

- [ ] **Step 1: Create the module with structs + `grammar_for_extension` + a failing test**

Create `src-tauri/src/project_scanner.rs`:

```rust
// src-tauri/src/project_scanner.rs
//
// Native tree-sitter scanner: walk a project dir, parse source files, extract
// code structure, and merge it into the tree's concept graph as code-grounded
// (confidence='extracted') nodes and edges. See Phase 5 spec.
//
// NOTE: exact tree-sitter node-type names and entry points are version-dependent
// (Task 1 pins the versions). QueryCursor::matches returns a StreamingIterator in
// tree-sitter 0.24 — iterate with `while let Some(m) = it.next()`.

use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use sqlx::Row;
use std::collections::HashMap;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Parser, Query, QueryCursor};

#[derive(Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ScanResult {
    pub files_scanned: u32,
    pub files_skipped: u32,
    pub nodes_added: u32,
    pub nodes_enriched: u32,
    pub edges_added: u32,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ExtractedNode {
    pub title: String,
    pub description: String,
    pub file_path: String,
    pub line_start: i32,
    pub line_end: i32,
}

#[derive(Debug, Clone)]
pub(crate) struct ExtractedEdge {
    pub source_title: String,
    pub target_title: String,
    pub relationship: &'static str, // "implements" | "references" | "prerequisite"
}

/// Map a file extension to a tree-sitter Language, or None to skip.
/// TypeScript uses LANGUAGE_TYPESCRIPT / LANGUAGE_TSX (verify against the crate).
pub(crate) fn grammar_for_extension(ext: &str) -> Option<Language> {
    match ext.to_lowercase().as_str() {
        "py" | "pyw" | "pyx" => Some(tree_sitter_python::LANGUAGE.into()),
        "js" | "jsx" | "mjs" | "cjs" => Some(tree_sitter_javascript::LANGUAGE.into()),
        "ts" | "mts" | "cts" => Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        "tsx" => Some(tree_sitter_typescript::LANGUAGE_TSX.into()),
        "rs" => Some(tree_sitter_rust::LANGUAGE.into()),
        "go" => Some(tree_sitter_go::LANGUAGE.into()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grammar_selection_maps_known_extensions_and_skips_others() {
        for ext in ["py", "PY", "js", "jsx", "ts", "tsx", "rs", "go", "mjs"] {
            assert!(grammar_for_extension(ext).is_some(), "{} should map", ext);
        }
        for ext in ["java", "rb", "c", "cpp", "txt", "md", ""] {
            assert!(grammar_for_extension(ext).is_none(), "{} should skip", ext);
        }
    }
}
```

Add `mod project_scanner;` to `main.rs` after `mod hitl;`.

- [ ] **Step 2: Run the test (confirms crates + API compile)**

Run: `cargo test project_scanner 2>&1 | tail -15`
Expected: `grammar_selection_maps_known_extensions_and_skips_others` passes. If `LANGUAGE`/`LANGUAGE_TYPESCRIPT`/`.into()` don't compile, adjust to the actual crate API (per Task 1's version set) until this test passes — this test is the API-shape gate.

- [ ] **Step 3: Add Python extraction + a test**

Add above the tests. This is the worked template for all languages (Task 4 repeats the pattern):

```rust
/// Extract nodes (functions/classes) and edges (calls/imports/inheritance) from
/// one file, dispatching on extension. Unknown extension → empty.
pub(crate) fn extract_from_file(path: &str, source: &str) -> (Vec<ExtractedNode>, Vec<ExtractedEdge>) {
    let ext = std::path::Path::new(path)
        .extension().and_then(|e| e.to_str()).unwrap_or("");
    let Some(lang) = grammar_for_extension(ext) else {
        return (Vec::new(), Vec::new());
    };
    match ext.to_lowercase().as_str() {
        "py" | "pyw" | "pyx" => extract_python(&lang, path, source),
        // Task 4 adds: js/jsx/mjs/cjs, ts/mts/cts, tsx, rs, go
        _ => (Vec::new(), Vec::new()),
    }
}

/// Run a captured-name query, calling `f(name, start_line, end_line)` per match.
fn for_each_named<F: FnMut(&str, i32, i32)>(
    lang: &Language, source: &str, query_src: &str, mut f: F,
) {
    let Ok(query) = Query::new(lang, query_src) else { return };
    let mut parser = Parser::new();
    if parser.set_language(lang).is_err() { return; }
    let Some(tree) = parser.parse(source, None) else { return };
    // capture index 0 is the @name capture in the queries below
    let mut cursor = QueryCursor::new();
    let mut it = cursor.matches(&query, tree.root_node(), source.as_bytes());
    while let Some(m) = it.next() {
        if let Some(cap) = m.captures.iter().find(|c| query.capture_names()[c.index as usize] == "name") {
            if let Ok(text) = cap.node.utf8_text(source.as_bytes()) {
                let s = cap.node.start_position().row as i32 + 1;
                let e = cap.node.end_position().row as i32 + 1;
                f(text, s, e);
            }
        }
    }
}

fn extract_python(lang: &Language, path: &str, source: &str) -> (Vec<ExtractedNode>, Vec<ExtractedEdge>) {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    // Functions
    for_each_named(lang, source,
        "(function_definition name: (identifier) @name)",
        |name, s, e| nodes.push(ExtractedNode {
            title: name.to_string(), description: "Python function".into(),
            file_path: path.to_string(), line_start: s, line_end: e,
        }));
    // Classes (+ inheritance → prerequisite edges)
    for_each_named(lang, source,
        "(class_definition name: (identifier) @name)",
        |name, s, e| nodes.push(ExtractedNode {
            title: name.to_string(), description: "Python class".into(),
            file_path: path.to_string(), line_start: s, line_end: e,
        }));
    // Calls → implements edges (caller unknown at this granularity → use file stem as source)
    let file_stem = std::path::Path::new(path).file_stem().and_then(|s| s.to_str()).unwrap_or(path).to_string();
    for_each_named(lang, source,
        "(call function: (identifier) @name)",
        |name, _s, _e| edges.push(ExtractedEdge {
            source_title: file_stem.clone(), target_title: name.to_string(), relationship: "implements",
        }));
    // Imports → references edges
    for_each_named(lang, source,
        "(import_from_statement name: (dotted_name (identifier) @name))",
        |name, _s, _e| edges.push(ExtractedEdge {
            source_title: file_stem.clone(), target_title: name.to_string(), relationship: "references",
        }));
    (nodes, edges)
}
```

Add a test (parse a Python snippet, assert names). NOTE: if a query string fails to compile against the actual grammar's node types, adjust the S-expression to the grammar's real node names (check via the grammar's `node-types.json` or docs) until the test passes:

```rust
    #[test]
    fn python_extraction_finds_functions_and_classes() {
        let src = "class VectorStore:\n    pass\n\ndef rrf_merge(a, b):\n    return cosine(a, b)\n";
        let (nodes, edges) = extract_from_file("search.py", src);
        let titles: Vec<&str> = nodes.iter().map(|n| n.title.as_str()).collect();
        assert!(titles.contains(&"VectorStore"));
        assert!(titles.contains(&"rrf_merge"));
        assert!(nodes.iter().any(|n| n.title == "rrf_merge" && n.line_start >= 1));
        assert!(edges.iter().any(|e| e.target_title == "cosine" && e.relationship == "implements"));
    }
```

- [ ] **Step 4: Add the walker + merge + `scan_project` orchestration**

Add above the tests:

```rust
struct MergeStats { added: u32, enriched: u32, edges: u32 }

/// App-level node upsert (no unique index exists on (graph_id, LOWER(title))):
/// match by LOWER(title) → enrich file/line + set confidence='extracted'; else INSERT.
/// Edges: INSERT ON CONFLICT (idx_cge_unique) DO NOTHING. Returns a title→node_id map
/// so edges can resolve endpoints.
async fn merge_into_graph(
    pool: &PgPool, graph_id: &str,
    nodes: Vec<ExtractedNode>, edges: Vec<ExtractedEdge>,
) -> Result<MergeStats, String> {
    let mut stats = MergeStats { added: 0, enriched: 0, edges: 0 };
    let mut id_by_title: HashMap<String, String> = HashMap::new();

    for n in &nodes {
        let existing = sqlx::query(
            "SELECT id FROM concept_graph_nodes WHERE graph_id = $1 AND LOWER(title) = LOWER($2) LIMIT 1"
        )
        .bind(graph_id).bind(&n.title)
        .fetch_optional(pool).await.map_err(|e| e.to_string())?;

        let node_id = if let Some(row) = existing {
            let id: String = row.try_get("id").map_err(|e| e.to_string())?;
            sqlx::query(
                "UPDATE concept_graph_nodes SET file_path = $1, line_start = $2, line_end = $3, \
                 confidence = 'extracted' WHERE id = $4"
            )
            .bind(&n.file_path).bind(n.line_start).bind(n.line_end).bind(&id)
            .execute(pool).await.map_err(|e| e.to_string())?;
            stats.enriched += 1;
            id
        } else {
            let id = uuid::Uuid::new_v4().to_string();
            sqlx::query(
                "INSERT INTO concept_graph_nodes \
                   (id, graph_id, title, description, file_path, line_start, line_end, confidence) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, 'extracted')"
            )
            .bind(&id).bind(graph_id).bind(&n.title).bind(&n.description)
            .bind(&n.file_path).bind(n.line_start).bind(n.line_end)
            .execute(pool).await.map_err(|e| e.to_string())?;
            stats.added += 1;
            id
        };
        id_by_title.insert(n.title.to_lowercase(), node_id);
    }

    // Resolve existing graph nodes too (edge endpoints may be pre-existing inferred nodes).
    let rows = sqlx::query("SELECT id, title FROM concept_graph_nodes WHERE graph_id = $1")
        .bind(graph_id).fetch_all(pool).await.map_err(|e| e.to_string())?;
    for r in &rows {
        let id: String = r.try_get("id").unwrap_or_default();
        let title: String = r.try_get("title").unwrap_or_default();
        id_by_title.entry(title.to_lowercase()).or_insert(id);
    }

    for e in &edges {
        let (Some(src), Some(tgt)) = (
            id_by_title.get(&e.source_title.to_lowercase()),
            id_by_title.get(&e.target_title.to_lowercase()),
        ) else { continue };
        if src == tgt { continue; }
        let edge_id = uuid::Uuid::new_v4().to_string();
        let res = sqlx::query(
            "INSERT INTO concept_graph_edges \
               (id, graph_id, source_node_id, target_node_id, relationship, confidence) \
             VALUES ($1, $2, $3, $4, $5, 'extracted') \
             ON CONFLICT (graph_id, source_node_id, target_node_id, relationship) DO NOTHING"
        )
        .bind(&edge_id).bind(graph_id).bind(src).bind(tgt).bind(e.relationship)
        .execute(pool).await.map_err(|e| e.to_string())?;
        stats.edges += res.rows_affected() as u32;
    }
    Ok(stats)
}

/// Resolve (or create) the tree's concept graph id.
async fn resolve_or_create_graph(pool: &PgPool, tree_id: &str) -> Result<String, String> {
    if let Some(id) = crate::concept_graph::graph_id_for_tree(pool, tree_id).await? {
        return Ok(id);
    }
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO concept_graphs (id, tree_id) VALUES ($1, $2)")
        .bind(&id).bind(tree_id)
        .execute(pool).await.map_err(|e| e.to_string())?;
    Ok(id)
}

pub(crate) async fn scan_project(pool: &PgPool, path: &str, tree_id: &str) -> Result<ScanResult, String> {
    let root = std::path::Path::new(path);
    if !root.exists() {
        return Err(format!("Directory not found: {}", path));
    }
    let graph_id = resolve_or_create_graph(pool, tree_id).await?;

    let mut result = ScanResult::default();
    let mut all_nodes: Vec<ExtractedNode> = Vec::new();
    let mut all_edges: Vec<ExtractedEdge> = Vec::new();

    // ignore::WalkBuilder: gitignore-aware + always skip heavy dirs.
    let walker = ignore::WalkBuilder::new(root)
        .standard_filters(true)
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !matches!(name.as_ref(), "node_modules" | "target" | "venv" | "__pycache__" | ".git")
        })
        .build();

    for entry in walker.flatten() {
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) { continue; }
        let p = entry.path();
        let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
        if grammar_for_extension(ext).is_none() {
            result.files_skipped += 1;
            continue;
        }
        let file_path = p.to_string_lossy().to_string();
        let source = match std::fs::read_to_string(p) {
            Ok(s) => s,
            Err(e) => { result.errors.push(format!("{}: {}", file_path, e)); continue; }
        };
        let (nodes, edges) = extract_from_file(&file_path, &source);
        result.files_scanned += 1;
        all_nodes.extend(nodes);
        all_edges.extend(edges);
    }

    let merged = merge_into_graph(pool, &graph_id, all_nodes, all_edges).await?;
    result.nodes_added = merged.added;
    result.nodes_enriched = merged.enriched;
    result.edges_added = merged.edges;
    Ok(result)
}
```

- [ ] **Step 5: Build + test**

Run: `cargo build 2>&1 | tail -8` → `Finished`.
Run: `cargo test project_scanner 2>&1 | tail -8` → 2 passed (grammar selection + python extraction). If the Python query strings didn't match the grammar's node types, fix them until the extraction test passes.

- [ ] **Step 6: Commit**

```bash
git add src/project_scanner.rs src/main.rs
git commit -m "feat(scanner): scanner core + Python extraction + graph merge + scan_project"
```

---

### Task 4: JavaScript / TypeScript / Rust / Go extraction

**Files:**
- Modify: `src-tauri/src/project_scanner.rs`

**Interfaces:** Consumes `for_each_named`, `ExtractedNode/Edge` (Task 3). Extends `extract_from_file` dispatch.

- [ ] **Step 1: Write failing per-language tests**

Add to the `tests` module (one per language; adjust expected node types only if the grammar differs):

```rust
    #[test]
    fn rust_extraction_finds_fn_struct_trait() {
        let src = "struct SearchResult;\ntrait Scorer {}\nfn cosine_similarity(a: f64) -> f64 { helper(a) }\n";
        let (nodes, edges) = extract_from_file("lib.rs", src);
        let t: Vec<&str> = nodes.iter().map(|n| n.title.as_str()).collect();
        assert!(t.contains(&"SearchResult") && t.contains(&"Scorer") && t.contains(&"cosine_similarity"));
        assert!(edges.iter().any(|e| e.target_title == "helper"));
    }

    #[test]
    fn go_js_ts_extraction_find_functions() {
        let (gn, _) = extract_from_file("main.go", "func HybridSearch(q string) {}\ntype Store struct{}\n");
        assert!(gn.iter().any(|n| n.title == "HybridSearch"));
        let (jn, _) = extract_from_file("app.js", "function render() {}\nclass Widget {}\n");
        assert!(jn.iter().any(|n| n.title == "render"));
        let (tn, _) = extract_from_file("api.ts", "interface Provider {}\nfunction fetchIt() {}\n");
        assert!(tn.iter().any(|n| n.title == "fetchIt"));
    }
```

Run: `cargo test project_scanner 2>&1 | tail -8` → these FAIL (dispatch arms not added; only Python wired).

- [ ] **Step 2: Add the four extractors + dispatch**

In `extract_from_file`, replace the `_ => (Vec::new(), Vec::new())` dispatch tail with arms for the four languages:

```rust
        "js" | "jsx" | "mjs" | "cjs" => extract_js_like(&lang, path, source),
        "ts" | "mts" | "cts" | "tsx" => extract_ts_like(&lang, path, source),
        "rs" => extract_rust(&lang, path, source),
        "go" => extract_go(&lang, path, source),
        _ => (Vec::new(), Vec::new()),
```

Add the four functions, each mirroring `extract_python` with language-appropriate query S-expressions and node descriptions. Starting query strings (VERIFY node-type names against each grammar's `node-types.json`; adjust until the tests pass):

- **Rust:** functions `(function_item name: (identifier) @name)`; structs `(struct_item name: (type_identifier) @name)`; traits `(trait_item name: (type_identifier) @name)`; calls `(call_expression function: (identifier) @name)`; use `(use_declaration argument: (scoped_identifier name: (identifier) @name))`; impl-for → prerequisite `(impl_item trait: (type_identifier) @name)`. Descriptions "Rust function/struct/trait".
- **JavaScript (`extract_js_like`):** `(function_declaration name: (identifier) @name)`, `(class_declaration name: (identifier) @name)`, calls `(call_expression function: (identifier) @name)`, imports `(import_statement source: (string) @name)`. Descriptions "JS function/class".
- **TypeScript (`extract_ts_like`):** same as JS plus `(interface_declaration name: (type_identifier) @name)` and `(type_alias_declaration name: (type_identifier) @name)`. Descriptions "TS function/class/interface".
- **Go:** `(function_declaration name: (identifier) @name)`, `(method_declaration name: (field_identifier) @name)`, `(type_declaration (type_spec name: (type_identifier) @name))`, calls `(call_expression function: (identifier) @name)`, imports `(import_spec path: (interpreted_string_literal) @name)`. Descriptions "Go function/type".

For calls/imports use the same `source_title = file_stem` pattern as `extract_python`. For inheritance/impl use `relationship: "prerequisite"`, for calls `"implements"`, for imports `"references"`.

- [ ] **Step 3: Build + test until green**

Run: `cargo test project_scanner 2>&1 | tail -12` → 4 passed (grammar, python, rust, go_js_ts). Fix query node-type names per the actual grammars until all pass.

- [ ] **Step 4: Commit**

```bash
git add src/project_scanner.rs
git commit -m "feat(scanner): JS/TS/Rust/Go extraction"
```

---

### Task 5: `scan_project` agent tool

**Files:**
- Modify: `src-tauri/src/mimir_agent.rs` (schema + arm + tool-count test)
- Modify: `src-tauri/src/mimir_retrieval.rs` (one tool-guidance line)

**Interfaces:** Consumes `crate::project_scanner::scan_project`.

- [ ] **Step 1: Update the tool-count test first**

The current test `tool_schemas_registers_web_tools_only_when_available` asserts 10 (hound off) / 12 (hound on) with the 3 destructive tools last. `scan_project` is appended after them. Update: `tool_schemas(false)` = 11 names ending `..., delete_fact, delete_resource, merge_skills, scan_project`; `tool_schemas(true)` = 13 with `[12] == "scan_project"`. Edit the test's two assertions accordingly (add `"scan_project"` to the `base` vec end; change `with_web.len()` to 13 and assert `with_web[12] == "scan_project"`).

Run: `cargo test mimir_agent 2>&1 | tail -6` → FAIL (scan_project not in schemas yet).

- [ ] **Step 2: Append the schema**

In `tool_schemas`, after the 3 destructive schemas and before the final `schemas` return, push:

```rust
    schemas.push(json!({
        "type": "function",
        "function": {
            "name": "scan_project",
            "description": "Parse a local project directory and extract concept nodes and edges from source code into the concept graph (confidence='extracted'). Supports Python, JavaScript, TypeScript, Rust, Go.",
            "parameters": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Absolute or relative path to the project directory." }
                },
                "required": ["path"]
            }
        }
    }));
```

- [ ] **Step 3: Add the execute arm**

In `execute_tool`, before the `other =>` arm:

```rust
        "scan_project" => {
            let path = args["path"].as_str().ok_or("scan_project requires a 'path' argument")?;
            let Some(tree_id) = ctx.tree_id.as_deref() else {
                return Ok("No active tree to scan into — open a tree first.".to_string());
            };
            let r = crate::project_scanner::scan_project(ctx.pool, path, tree_id).await?;
            let mut msg = format!(
                "Scanned {} files ({} skipped). Added {} new concepts, enriched {} existing, added {} edges.",
                r.files_scanned, r.files_skipped, r.nodes_added, r.nodes_enriched, r.edges_added
            );
            if !r.errors.is_empty() {
                msg.push_str(&format!(" {} file(s) had errors and were skipped.", r.errors.len()));
            }
            Ok(msg)
        }
```

- [ ] **Step 4: Add the tool-guidance line**

In `mimir_retrieval.rs`, the agent tool-guidance string appended to `agent_system_prompt` (the block listing the tools) — add a sentence: ` scan_project: parse source code from a local directory into the concept graph when the user wants concepts grounded in actual code or has added a project locally.`

- [ ] **Step 5: Build + full test suite**

Run: `cargo build 2>&1 | tail -5` → `Finished`.
Run: `cargo test 2>&1 | tail -8` → all pass (24 prior + grammar/python/rust/go_js_ts = 28).

- [ ] **Step 6: Commit**

```bash
git add src/mimir_agent.rs src/mimir_retrieval.rs
git commit -m "feat(scanner): scan_project agent tool + guidance"
```

---

### Task 6: Extension hook (`/api/library` local_path)

**Files:**
- Modify: `src-tauri/src/ext_server.rs`

**Interfaces:** Consumes `crate::project_scanner::scan_project`.

- [ ] **Step 1: Read the handler + body struct**

Read `ext_server.rs` `CreateLibraryBody` (~line 244) and `create_library_handler` (~line 453). Note whether `tree_id` is already a field and how the handler accesses the pool (it has `database`/pool via state or a struct). Report the exact shapes.

- [ ] **Step 2: Add `local_path` (and `tree_id` if absent) to the body**

Add to `CreateLibraryBody`:

```rust
    #[serde(default)]
    local_path: Option<String>,
    #[serde(default)]
    tree_id: Option<String>,
```

(If `tree_id` already exists, add only `local_path`.)

- [ ] **Step 3: Fire-and-forget scan after ingest**

In `create_library_handler`, after the existing ingest logic succeeds and before returning the response, add:

```rust
    if let (Some(lp), Some(tid)) = (
        body.local_path.as_ref().filter(|s| !s.is_empty()),
        body.tree_id.as_ref().filter(|s| !s.is_empty()),
    ) {
        let pool = /* the pool handle used elsewhere in this handler */ .clone();
        let lp = lp.clone();
        let tid = tid.clone();
        tokio::spawn(async move {
            match crate::project_scanner::scan_project(&pool, &lp, &tid).await {
                Ok(r) => println!("📡 [ext] auto-scan: {} files, {} nodes, {} edges", r.files_scanned, r.nodes_added, r.edges_added),
                Err(e) => println!("⚠️  [ext] auto-scan failed: {}", e),
            }
        });
    }
```

(Use whatever pool handle the handler already has — read Step 1's findings; the ingest code in the same handler already clones a pool for its background work, mirror that.)

- [ ] **Step 4: Build**

Run: `cargo build 2>&1 | tail -5` → `Finished`, no errors.

- [ ] **Step 5: Commit**

```bash
git add src/ext_server.rs
git commit -m "feat(scanner): extension /api/library local_path → fire-and-forget scan"
```

---

### Task 7: Final verification (inline — controller runs this)

**Files:** none.

- [ ] **Step 1: Build + tests**

Run: `cargo build 2>&1 | tail -5` → `Finished`.
Run: `cargo test 2>&1 | tail -8` → 28 passed, 0 failed.

- [ ] **Step 2: Diff review**

Run: `git log --oneline -8` → task commits.
Run: `git diff <base>..HEAD --stat` → `Cargo.toml`, `Cargo.lock`, `migrations/052_*.sql`, `src/project_scanner.rs`, `src/main.rs`, `src/mimir_agent.rs`, `src/mimir_retrieval.rs`, `src/ext_server.rs`.

- [ ] **Step 3: Runtime checklist (requires `bash dev.sh` — user-driven; do NOT fake)**

- [ ] App builds + starts (first build slower — C grammars).
- [ ] Open a tree; ask Mimir "scan my project at <path-to-a-real-repo>" → `🛠 [agent] tool call scan_project`; summary reports files/nodes/edges.
- [ ] `SELECT count(*) FROM concept_graph_nodes WHERE confidence='extracted';` > 0; those rows have file_path + line_start/line_end.
- [ ] `SELECT count(*) FROM concept_graph_edges WHERE confidence='extracted';` > 0.
- [ ] A concept whose title matches an existing inferred node is ENRICHED (file/line filled, confidence flipped), not duplicated.
- [ ] `query_graph`/`explain_node` on an extracted concept show its code location + code edges.
- [ ] Scan a dir with no supported files → graceful message; a syntax-error file → skipped + counted in errors; a bad path → clear error.
- [ ] Extension: POST /api/library with `local_path` + `tree_id` → ingest returns immediately, scan runs (server log shows `📡 [ext] auto-scan`).

- [ ] **Step 4: Hand off**

Use superpowers:finishing-a-development-branch. (No Fable — deferred to the consolidated v3 audit.)

---

## Deferred (not in this plan)

More grammars; background `OrchestratorJob::ScanProject` for huge repos; file watching; remote/Docker
scanning; removing inferred nodes; frontend extracted/inferred badges on the graph panel.
