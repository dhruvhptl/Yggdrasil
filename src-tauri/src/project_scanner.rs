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
        "js" | "jsx" | "mjs" | "cjs" => extract_js_like(&lang, path, source),
        "ts" | "mts" | "cts" | "tsx" => extract_ts_like(&lang, path, source),
        "rs" => extract_rust(&lang, path, source),
        "go" => extract_go(&lang, path, source),
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

/// JavaScript (also used as the base pattern for TypeScript below).
fn extract_js_like(lang: &Language, path: &str, source: &str) -> (Vec<ExtractedNode>, Vec<ExtractedEdge>) {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    // Functions
    for_each_named(lang, source,
        "(function_declaration name: (identifier) @name)",
        |name, s, e| nodes.push(ExtractedNode {
            title: name.to_string(), description: "JS function".into(),
            file_path: path.to_string(), line_start: s, line_end: e,
        }));
    // Classes
    for_each_named(lang, source,
        "(class_declaration name: (identifier) @name)",
        |name, s, e| nodes.push(ExtractedNode {
            title: name.to_string(), description: "JS class".into(),
            file_path: path.to_string(), line_start: s, line_end: e,
        }));
    // Calls → implements edges
    let file_stem = std::path::Path::new(path).file_stem().and_then(|s| s.to_str()).unwrap_or(path).to_string();
    for_each_named(lang, source,
        "(call_expression function: (identifier) @name)",
        |name, _s, _e| edges.push(ExtractedEdge {
            source_title: file_stem.clone(), target_title: name.to_string(), relationship: "implements",
        }));
    // Imports → references edges
    for_each_named(lang, source,
        "(import_statement source: (string) @name)",
        |name, _s, _e| edges.push(ExtractedEdge {
            source_title: file_stem.clone(), target_title: name.to_string(), relationship: "references",
        }));
    (nodes, edges)
}

/// TypeScript: same shape as JS, but `class_declaration`'s name field is a
/// `type_identifier` in the TS/TSX grammar (unlike plain JS, where it is an
/// `identifier`) — so class/interface/type-alias queries all key on
/// `type_identifier` while functions/calls stay on `identifier`.
fn extract_ts_like(lang: &Language, path: &str, source: &str) -> (Vec<ExtractedNode>, Vec<ExtractedEdge>) {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    // Functions
    for_each_named(lang, source,
        "(function_declaration name: (identifier) @name)",
        |name, s, e| nodes.push(ExtractedNode {
            title: name.to_string(), description: "TS function".into(),
            file_path: path.to_string(), line_start: s, line_end: e,
        }));
    // Classes
    for_each_named(lang, source,
        "(class_declaration name: (type_identifier) @name)",
        |name, s, e| nodes.push(ExtractedNode {
            title: name.to_string(), description: "TS class".into(),
            file_path: path.to_string(), line_start: s, line_end: e,
        }));
    // Interfaces
    for_each_named(lang, source,
        "(interface_declaration name: (type_identifier) @name)",
        |name, s, e| nodes.push(ExtractedNode {
            title: name.to_string(), description: "TS interface".into(),
            file_path: path.to_string(), line_start: s, line_end: e,
        }));
    // Type aliases
    for_each_named(lang, source,
        "(type_alias_declaration name: (type_identifier) @name)",
        |name, s, e| nodes.push(ExtractedNode {
            title: name.to_string(), description: "TS type alias".into(),
            file_path: path.to_string(), line_start: s, line_end: e,
        }));
    // Calls → implements edges
    let file_stem = std::path::Path::new(path).file_stem().and_then(|s| s.to_str()).unwrap_or(path).to_string();
    for_each_named(lang, source,
        "(call_expression function: (identifier) @name)",
        |name, _s, _e| edges.push(ExtractedEdge {
            source_title: file_stem.clone(), target_title: name.to_string(), relationship: "implements",
        }));
    // Imports → references edges
    for_each_named(lang, source,
        "(import_statement source: (string) @name)",
        |name, _s, _e| edges.push(ExtractedEdge {
            source_title: file_stem.clone(), target_title: name.to_string(), relationship: "references",
        }));
    (nodes, edges)
}

fn extract_rust(lang: &Language, path: &str, source: &str) -> (Vec<ExtractedNode>, Vec<ExtractedEdge>) {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    // Functions
    for_each_named(lang, source,
        "(function_item name: (identifier) @name)",
        |name, s, e| nodes.push(ExtractedNode {
            title: name.to_string(), description: "Rust function".into(),
            file_path: path.to_string(), line_start: s, line_end: e,
        }));
    // Structs
    for_each_named(lang, source,
        "(struct_item name: (type_identifier) @name)",
        |name, s, e| nodes.push(ExtractedNode {
            title: name.to_string(), description: "Rust struct".into(),
            file_path: path.to_string(), line_start: s, line_end: e,
        }));
    // Traits
    for_each_named(lang, source,
        "(trait_item name: (type_identifier) @name)",
        |name, s, e| nodes.push(ExtractedNode {
            title: name.to_string(), description: "Rust trait".into(),
            file_path: path.to_string(), line_start: s, line_end: e,
        }));
    // Calls → implements edges
    let file_stem = std::path::Path::new(path).file_stem().and_then(|s| s.to_str()).unwrap_or(path).to_string();
    for_each_named(lang, source,
        "(call_expression function: (identifier) @name)",
        |name, _s, _e| edges.push(ExtractedEdge {
            source_title: file_stem.clone(), target_title: name.to_string(), relationship: "implements",
        }));
    // `use` paths → references edges
    for_each_named(lang, source,
        "(use_declaration argument: (scoped_identifier name: (identifier) @name))",
        |name, _s, _e| edges.push(ExtractedEdge {
            source_title: file_stem.clone(), target_title: name.to_string(), relationship: "references",
        }));
    // `impl Trait for Type` → prerequisite edges (trait must exist before the impl)
    for_each_named(lang, source,
        "(impl_item trait: (type_identifier) @name)",
        |name, _s, _e| edges.push(ExtractedEdge {
            source_title: file_stem.clone(), target_title: name.to_string(), relationship: "prerequisite",
        }));
    (nodes, edges)
}

fn extract_go(lang: &Language, path: &str, source: &str) -> (Vec<ExtractedNode>, Vec<ExtractedEdge>) {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    // Functions
    for_each_named(lang, source,
        "(function_declaration name: (identifier) @name)",
        |name, s, e| nodes.push(ExtractedNode {
            title: name.to_string(), description: "Go function".into(),
            file_path: path.to_string(), line_start: s, line_end: e,
        }));
    // Methods (receiver functions)
    for_each_named(lang, source,
        "(method_declaration name: (field_identifier) @name)",
        |name, s, e| nodes.push(ExtractedNode {
            title: name.to_string(), description: "Go method".into(),
            file_path: path.to_string(), line_start: s, line_end: e,
        }));
    // Type declarations (struct/interface/alias)
    for_each_named(lang, source,
        "(type_declaration (type_spec name: (type_identifier) @name))",
        |name, s, e| nodes.push(ExtractedNode {
            title: name.to_string(), description: "Go type".into(),
            file_path: path.to_string(), line_start: s, line_end: e,
        }));
    // Calls → implements edges
    let file_stem = std::path::Path::new(path).file_stem().and_then(|s| s.to_str()).unwrap_or(path).to_string();
    for_each_named(lang, source,
        "(call_expression function: (identifier) @name)",
        |name, _s, _e| edges.push(ExtractedEdge {
            source_title: file_stem.clone(), target_title: name.to_string(), relationship: "implements",
        }));
    // Imports → references edges
    for_each_named(lang, source,
        "(import_spec path: (interpreted_string_literal) @name)",
        |name, _s, _e| edges.push(ExtractedEdge {
            source_title: file_stem.clone(), target_title: name.to_string(), relationship: "references",
        }));
    (nodes, edges)
}

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

pub(crate) async fn scan_project_inner(pool: &PgPool, path: &str, tree_id: &str) -> Result<ScanResult, String> {
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
}

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
