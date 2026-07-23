// src-tauri/src/concept_graph.rs
//
// Concept graphs as first-class tables (migration 050). A concept graph is
// derived from the tree's Vec<Concept>: one node per concept, one 'prerequisite'
// edge per resolvable prerequisite (source = prereq, target = the concept).
// Read verbs: query_graph / path_between / explain_node (also agent tools).

use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use sqlx::Row;
use std::collections::HashMap;
use tauri::State;

use crate::database::Database;
use crate::prompt_builders::Concept;

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConceptNode {
    pub id: String,
    pub title: String,
    pub description: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConceptEdge {
    pub source_node_id: String,
    pub target_node_id: String,
    pub relationship: String,
    pub confidence: String,
}

#[derive(Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConceptSubgraph {
    pub nodes: Vec<ConceptNode>,
    pub edges: Vec<ConceptEdge>,
    pub match_method: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PathNode {
    pub title: String,
    pub depth: i32,
    pub is_source: bool,
    pub is_target: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EdgeWithNode {
    pub node_id: String,
    pub title: String,
    pub relationship: String,
    pub confidence: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LinkedResource {
    pub resource_id: String,
    pub title: String,
    pub url: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NodeDetail {
    pub node: ConceptNode,
    pub prerequisites: Vec<EdgeWithNode>,
    pub dependents: Vec<EdgeWithNode>,
    pub references: Vec<EdgeWithNode>,
    pub resources: Vec<LinkedResource>,
}

// ─── Derivation: Vec<Concept> → nodes + edges ────────────────────────────────

pub(crate) struct DerivedGraph {
    /// (concept_id, title, description)
    pub nodes: Vec<(String, String, String)>,
    /// (source_concept_id, target_concept_id, relationship, confidence)
    pub edges: Vec<(String, String, &'static str, &'static str)>,
}

/// Pure: one node per concept; one 'prerequisite' edge per prerequisite that
/// resolves to a concept in this set and is not the concept itself.
pub(crate) fn derive_graph(concepts: &[Concept]) -> DerivedGraph {
    let id_set: std::collections::HashSet<&str> =
        concepts.iter().map(|c| c.id.as_str()).collect();
    let mut nodes = Vec::with_capacity(concepts.len());
    let mut edges = Vec::new();
    for c in concepts {
        nodes.push((c.id.clone(), c.name.clone(), c.description.clone()));
        for prereq in &c.prerequisites {
            if prereq != &c.id && id_set.contains(prereq.as_str()) {
                edges.push((prereq.clone(), c.id.clone(), "prerequisite", "inferred"));
            }
        }
    }
    DerivedGraph { nodes, edges }
}

// ─── Graph creation (idempotent) ─────────────────────────────────────────────

/// Persist a derived graph for a tree. Idempotent: if a concept_graphs row
/// already exists for tree_id, returns its id without re-inserting.
pub(crate) async fn create_concept_graph_from_concepts(
    pool: &PgPool,
    tree_id: &str,
    concepts: &[Concept],
) -> Result<String, String> {
    if let Some(row) = sqlx::query("SELECT id FROM concept_graphs WHERE tree_id = $1")
        .bind(tree_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?
    {
        return row.try_get("id").map_err(|e| e.to_string());
    }
    if concepts.is_empty() {
        return Err("no concepts to persist".into());
    }

    let derived = derive_graph(concepts);
    let graph_id = uuid::Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO concept_graphs (id, tree_id) VALUES ($1, $2)")
        .bind(&graph_id)
        .bind(tree_id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;

    let mut id_map: HashMap<String, String> = HashMap::new();
    for (concept_id, title, description) in &derived.nodes {
        let node_id = uuid::Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO concept_graph_nodes (id, graph_id, title, description) \
             VALUES ($1, $2, $3, $4)",
        )
        .bind(&node_id)
        .bind(&graph_id)
        .bind(title)
        .bind(description)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
        id_map.insert(concept_id.clone(), node_id);
    }

    for (src_cid, tgt_cid, rel, conf) in &derived.edges {
        let (Some(src), Some(tgt)) = (id_map.get(src_cid), id_map.get(tgt_cid)) else {
            continue;
        };
        let edge_id = uuid::Uuid::new_v4().to_string();
        let _ = sqlx::query(
            "INSERT INTO concept_graph_edges \
               (id, graph_id, source_node_id, target_node_id, relationship, confidence) \
             VALUES ($1, $2, $3, $4, $5, $6) \
             ON CONFLICT (graph_id, source_node_id, target_node_id, relationship) DO NOTHING",
        )
        .bind(&edge_id)
        .bind(&graph_id)
        .bind(src)
        .bind(tgt)
        .bind(*rel)
        .bind(*conf)
        .execute(pool)
        .await;
    }

    println!("🕸  [graph] created concept graph {} for tree {} ({} nodes, {} edges)",
        graph_id, tree_id, derived.nodes.len(), derived.edges.len());
    Ok(graph_id)
}

// ─── query_graph ─────────────────────────────────────────────────────────────

const STOPWORDS: &[&str] = &[
    "the", "a", "an", "of", "to", "and", "or", "is", "are", "in", "on", "for",
    "how", "does", "do", "what", "with", "between", "relate",
];

pub(crate) fn tokenize(query: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in query.split(|c: char| !c.is_alphanumeric()) {
        let w = raw.to_lowercase();
        if w.len() < 2 || STOPWORDS.contains(&w.as_str()) {
            continue;
        }
        if !out.contains(&w) {
            out.push(w);
        }
    }
    out
}

/// exact title = query → 1.0; any token substring in title → 0.8;
/// any token substring in description → 0.4; else 0.0.
pub(crate) fn score_node(title: &str, description: &str, query_lc: &str, tokens: &[String]) -> f64 {
    let tl = title.to_lowercase();
    if tl == query_lc {
        return 1.0;
    }
    let dl = description.to_lowercase();
    let mut best = 0.0f64;
    for tok in tokens {
        if tl.contains(tok.as_str()) {
            best = best.max(0.8);
        } else if dl.contains(tok.as_str()) {
            best = best.max(0.4);
        }
    }
    best
}

pub(crate) async fn graph_id_for_tree(
    pool: &PgPool,
    tree_id: &str,
) -> Result<Option<String>, String> {
    let row = sqlx::query("SELECT id FROM concept_graphs WHERE tree_id = $1 ORDER BY created_at DESC LIMIT 1")
        .bind(tree_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(row.and_then(|r| r.try_get::<String, _>("id").ok()))
}

/// Keyword search over the tree's concept graph. Embedding fallback is dormant
/// (Phase 1 leaves node embeddings NULL). Empty subgraph when nothing matches.
pub(crate) async fn query_graph(
    pool: &PgPool,
    tree_id: &str,
    query: &str,
) -> Result<ConceptSubgraph, String> {
    let Some(graph_id) = graph_id_for_tree(pool, tree_id).await? else {
        return Ok(ConceptSubgraph::default());
    };
    let tokens = tokenize(query);
    let query_lc = query.trim().to_lowercase();

    let rows = sqlx::query("SELECT id, title, description FROM concept_graph_nodes WHERE graph_id = $1")
        .bind(&graph_id)
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;

    let mut scored: Vec<(f64, ConceptNode)> = Vec::new();
    for r in &rows {
        let id: String = r.try_get("id").unwrap_or_default();
        let title: String = r.try_get("title").unwrap_or_default();
        let description: String = r.try_get("description").unwrap_or_default();
        let s = score_node(&title, &description, &query_lc, &tokens);
        if s >= 0.4 {
            scored.push((s, ConceptNode { id, title, description }));
        }
    }
    if scored.is_empty() {
        // Embedding fallback would go here once nodes are embedded (dormant).
        return Ok(ConceptSubgraph { nodes: vec![], edges: vec![], match_method: "keyword".into() });
    }
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(30);
    let nodes: Vec<ConceptNode> = scored.into_iter().map(|(_, n)| n).collect();
    let node_ids: Vec<String> = nodes.iter().map(|n| n.id.clone()).collect();

    let edge_rows = sqlx::query(
        "SELECT source_node_id, target_node_id, relationship, confidence \
         FROM concept_graph_edges \
         WHERE graph_id = $1 AND (source_node_id = ANY($2) OR target_node_id = ANY($2)) \
         LIMIT 100",
    )
    .bind(&graph_id)
    .bind(&node_ids)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let edges: Vec<ConceptEdge> = edge_rows
        .iter()
        .map(|r| ConceptEdge {
            source_node_id: r.try_get("source_node_id").unwrap_or_default(),
            target_node_id: r.try_get("target_node_id").unwrap_or_default(),
            relationship: r.try_get("relationship").unwrap_or_default(),
            confidence: r.try_get("confidence").unwrap_or_default(),
        })
        .collect();

    Ok(ConceptSubgraph { nodes, edges, match_method: "keyword".into() })
}

#[tauri::command]
pub async fn query_graph_cmd(
    tree_id: String,
    query: String,
    database: State<'_, Database>,
) -> Result<ConceptSubgraph, String> {
    query_graph(&database.pool, &tree_id, &query).await
}

// ─── path_between ────────────────────────────────────────────────────────────

use std::collections::VecDeque;

/// Pure BFS over an adjacency map. Returns the ordered node keys from source to
/// target inclusive, or empty if unreachable. source == target → [source].
pub(crate) fn bfs_path(
    adj: &HashMap<String, Vec<String>>,
    source: &str,
    target: &str,
) -> Vec<String> {
    if source == target {
        return vec![source.to_string()];
    }
    let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut prev: HashMap<String, String> = HashMap::new();
    let mut q: VecDeque<String> = VecDeque::new();
    visited.insert(source.to_string());
    q.push_back(source.to_string());
    while let Some(cur) = q.pop_front() {
        if let Some(neighbors) = adj.get(&cur) {
            for n in neighbors {
                if visited.insert(n.clone()) {
                    prev.insert(n.clone(), cur.clone());
                    if n == target {
                        let mut path = vec![target.to_string()];
                        let mut c = target.to_string();
                        while let Some(p) = prev.get(&c) {
                            path.push(p.clone());
                            c = p.clone();
                        }
                        path.reverse();
                        return path;
                    }
                    q.push_back(n.clone());
                }
            }
        }
    }
    Vec::new()
}

pub(crate) async fn resolve_node_by_title(
    pool: &PgPool,
    tree_id: &str,
    title: &str,
) -> Result<Option<String>, String> {
    let Some(graph_id) = graph_id_for_tree(pool, tree_id).await? else {
        return Ok(None);
    };
    // exact (case-insensitive) first, then substring
    let row = sqlx::query(
        "SELECT id FROM concept_graph_nodes \
         WHERE graph_id = $1 AND LOWER(title) = LOWER($2) LIMIT 1",
    )
    .bind(&graph_id)
    .bind(title)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;
    if let Some(r) = row {
        return Ok(r.try_get::<String, _>("id").ok());
    }
    let row2 = sqlx::query(
        "SELECT id FROM concept_graph_nodes \
         WHERE graph_id = $1 AND title ILIKE '%' || $2 || '%' LIMIT 1",
    )
    .bind(&graph_id)
    .bind(title)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(row2.and_then(|r| r.try_get::<String, _>("id").ok()))
}

pub(crate) async fn path_between(
    pool: &PgPool,
    tree_id: &str,
    source_title: &str,
    target_title: &str,
) -> Result<Vec<PathNode>, String> {
    let Some(graph_id) = graph_id_for_tree(pool, tree_id).await? else {
        return Ok(vec![]);
    };
    // node id -> title, and prerequisite adjacency (source -> target)
    let node_rows = sqlx::query("SELECT id, title FROM concept_graph_nodes WHERE graph_id = $1")
        .bind(&graph_id)
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;
    let mut title_of: HashMap<String, String> = HashMap::new();
    let mut id_of_title: HashMap<String, String> = HashMap::new();
    for r in &node_rows {
        let id: String = r.try_get("id").unwrap_or_default();
        let title: String = r.try_get("title").unwrap_or_default();
        id_of_title.insert(title.to_lowercase(), id.clone());
        title_of.insert(id, title);
    }
    let (Some(src), Some(tgt)) = (
        id_of_title.get(&source_title.to_lowercase()).cloned(),
        id_of_title.get(&target_title.to_lowercase()).cloned(),
    ) else {
        return Ok(vec![]);
    };

    let edge_rows = sqlx::query(
        "SELECT source_node_id, target_node_id FROM concept_graph_edges \
         WHERE graph_id = $1 AND relationship = 'prerequisite'",
    )
    .bind(&graph_id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    let mut adj: HashMap<String, Vec<String>> = HashMap::new();
    for r in &edge_rows {
        let s: String = r.try_get("source_node_id").unwrap_or_default();
        let t: String = r.try_get("target_node_id").unwrap_or_default();
        adj.entry(s).or_default().push(t);
    }

    let path_ids = bfs_path(&adj, &src, &tgt);
    if path_ids.len() > 20 {
        return Ok(vec![]); // guard; graphs are small so this is defensive
    }
    let last = path_ids.len().saturating_sub(1);
    let out = path_ids
        .iter()
        .enumerate()
        .map(|(i, id)| PathNode {
            title: title_of.get(id).cloned().unwrap_or_default(),
            depth: i as i32,
            is_source: i == 0,
            is_target: i == last && !path_ids.is_empty(),
        })
        .collect();
    Ok(out)
}

// ─── explain_node ────────────────────────────────────────────────────────────

async fn edges_with_nodes(
    pool: &PgPool,
    node_id: &str,
    incoming: bool,
    rels: &[&str],
) -> Vec<EdgeWithNode> {
    // incoming: edges where target = node_id, join the SOURCE node.
    // outgoing: edges where source = node_id, join the TARGET node.
    let sql = if incoming {
        "SELECT e.relationship, e.confidence, n.id AS nid, n.title AS ntitle \
         FROM concept_graph_edges e JOIN concept_graph_nodes n ON n.id = e.source_node_id \
         WHERE e.target_node_id = $1 AND e.relationship = ANY($2)"
    } else {
        "SELECT e.relationship, e.confidence, n.id AS nid, n.title AS ntitle \
         FROM concept_graph_edges e JOIN concept_graph_nodes n ON n.id = e.target_node_id \
         WHERE e.source_node_id = $1 AND e.relationship = ANY($2)"
    };
    let rels_vec: Vec<String> = rels.iter().map(|s| s.to_string()).collect();
    let rows = sqlx::query(sql)
        .bind(node_id)
        .bind(&rels_vec)
        .fetch_all(pool)
        .await
        .unwrap_or_default();
    rows.iter()
        .map(|r| EdgeWithNode {
            node_id: r.try_get("nid").unwrap_or_default(),
            title: r.try_get("ntitle").unwrap_or_default(),
            relationship: r.try_get("relationship").unwrap_or_default(),
            confidence: r.try_get("confidence").unwrap_or_default(),
        })
        .collect()
}

pub(crate) async fn explain_node(pool: &PgPool, node_id: &str) -> Result<NodeDetail, String> {
    let node_row = sqlx::query(
        "SELECT n.id, n.title, n.description, g.tree_id \
         FROM concept_graph_nodes n JOIN concept_graphs g ON g.id = n.graph_id \
         WHERE n.id = $1",
    )
    .bind(node_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?
    .ok_or_else(|| "concept node not found".to_string())?;

    let node = ConceptNode {
        id: node_row.try_get("id").unwrap_or_default(),
        title: node_row.try_get("title").unwrap_or_default(),
        description: node_row.try_get("description").unwrap_or_default(),
    };
    let tree_id: String = node_row.try_get("tree_id").unwrap_or_default();

    let prerequisites = edges_with_nodes(pool, node_id, true, &["prerequisite"]).await;
    let dependents = edges_with_nodes(pool, node_id, false, &["prerequisite"]).await;
    let references = edges_with_nodes(pool, node_id, false, &["references", "implements"]).await;

    // Resource bridge: concept title -> tree_nodes in the same tree -> mimir_node_links.
    let res_rows = sqlx::query(
        "SELECT DISTINCT mr.id AS rid, mr.title AS rtitle, mr.url AS rurl \
         FROM tree_nodes tn \
         JOIN mimir_node_links mnl ON mnl.node_id = tn.id \
         JOIN mimir_resources mr ON mr.id = mnl.resource_id \
         WHERE tn.tree_id = $1 AND LOWER(tn.title) = LOWER($2) \
         ORDER BY mr.title LIMIT 10",
    )
    .bind(&tree_id)
    .bind(&node.title)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let resources: Vec<LinkedResource> = res_rows
        .iter()
        .map(|r| LinkedResource {
            resource_id: r.try_get("rid").unwrap_or_default(),
            title: r.try_get("rtitle").unwrap_or_default(),
            url: r.try_get("rurl").ok(),
        })
        .collect();

    Ok(NodeDetail { node, prerequisites, dependents, references, resources })
}

#[tauri::command]
pub async fn path_between_cmd(
    tree_id: String,
    source: String,
    target: String,
    database: State<'_, Database>,
) -> Result<Vec<PathNode>, String> {
    path_between(&database.pool, &tree_id, &source, &target).await
}

#[tauri::command]
pub async fn explain_node_cmd(
    node_id: String,
    database: State<'_, Database>,
) -> Result<NodeDetail, String> {
    explain_node(&database.pool, &node_id).await
}

// ─── One-shot backfill from the JSONB blob ───────────────────────────────────

/// For each tree with a concept_graph blob and no concept_graphs row, parse the
/// Vec<Concept> and create a graph. Idempotent, best-effort, never panics.
/// Returns the number of graphs created.
pub(crate) async fn backfill_concept_graphs(pool: &PgPool) -> usize {
    let rows = match sqlx::query(
        "SELECT t.id, t.concept_graph FROM trees t \
         WHERE t.concept_graph IS NOT NULL \
           AND NOT EXISTS (SELECT 1 FROM concept_graphs cg WHERE cg.tree_id = t.id)",
    )
    .fetch_all(pool)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            println!("⚠️  [graph] backfill query failed: {}", e);
            return 0;
        }
    };

    let mut created = 0usize;
    for row in &rows {
        let tree_id: String = match row.try_get("id") {
            Ok(v) => v,
            Err(_) => continue,
        };
        let blob: serde_json::Value = match row.try_get("concept_graph") {
            Ok(v) => v,
            Err(_) => continue,
        };
        // The blob is a flat array of Concept. Tolerate malformed/old shapes.
        let concepts: Vec<Concept> = match serde_json::from_value(blob) {
            Ok(c) => c,
            Err(_) => continue,
        };
        if concepts.is_empty() {
            continue;
        }
        match create_concept_graph_from_concepts(pool, &tree_id, &concepts).await {
            Ok(_) => created += 1,
            Err(e) => println!("⚠️  [graph] backfill tree {} failed: {}", tree_id, e),
        }
    }
    if created > 0 {
        println!("🕸  [graph] backfilled {} concept graph(s)", created);
    }
    created
}

#[cfg(test)]
mod tests {
    use super::*;

    fn concept(id: &str, name: &str, prereqs: &[&str]) -> Concept {
        Concept {
            id: id.into(),
            name: name.into(),
            description: format!("desc of {}", name),
            prerequisites: prereqs.iter().map(|s| s.to_string()).collect(),
            concept_type: String::new(),
            supporting_files: vec![],
            project_relevance: String::new(),
        }
    }

    #[test]
    fn derive_graph_builds_nodes_and_prereq_edges() {
        let concepts = vec![
            concept("a", "SQL", &[]),
            concept("b", "Sharding", &["a"]),
        ];
        let g = derive_graph(&concepts);
        assert_eq!(g.nodes.len(), 2);
        assert_eq!(g.edges.len(), 1);
        // edge points prereq -> dependent: a -> b
        assert_eq!(g.edges[0].0, "a");
        assert_eq!(g.edges[0].1, "b");
        assert_eq!(g.edges[0].2, "prerequisite");
        assert_eq!(g.edges[0].3, "inferred");
    }

    #[test]
    fn derive_graph_skips_unresolved_and_self_prereqs() {
        let concepts = vec![
            concept("a", "A", &["a", "ghost"]), // self + dangling → both dropped
            concept("b", "B", &["a"]),
        ];
        let g = derive_graph(&concepts);
        assert_eq!(g.nodes.len(), 2);
        assert_eq!(g.edges.len(), 1);
        assert_eq!((g.edges[0].0.as_str(), g.edges[0].1.as_str()), ("a", "b"));
    }

    #[test]
    fn derive_graph_empty_input() {
        let g = derive_graph(&[]);
        assert!(g.nodes.is_empty() && g.edges.is_empty());
    }

    #[test]
    fn tokenize_lowercases_splits_and_drops_stopwords() {
        let t = tokenize("How does RRF connect to Hybrid-Search?");
        assert!(t.contains(&"rrf".to_string()));
        assert!(t.contains(&"connect".to_string()));
        assert!(t.contains(&"hybrid".to_string()));
        assert!(t.contains(&"search".to_string()));
        assert!(!t.contains(&"how".to_string()));   // stopword
        assert!(!t.contains(&"to".to_string()));    // stopword
    }

    #[test]
    fn score_node_ranks_exact_over_title_over_desc() {
        let toks = tokenize("sharding");
        assert_eq!(score_node("Sharding", "irrelevant", "sharding", &toks), 1.0);
        assert_eq!(score_node("Database Sharding", "irrelevant", "sharding", &toks), 0.8);
        assert_eq!(score_node("Replication", "uses sharding internally", "sharding", &toks), 0.4);
        assert_eq!(score_node("Replication", "nothing here", "sharding", &toks), 0.0);
    }

    fn adj(pairs: &[(&str, &str)]) -> std::collections::HashMap<String, Vec<String>> {
        let mut m: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
        for (s, t) in pairs {
            m.entry(s.to_string()).or_default().push(t.to_string());
        }
        m
    }

    #[test]
    fn bfs_path_finds_shortest_chain() {
        let a = adj(&[("a", "b"), ("b", "c"), ("a", "d"), ("d", "c")]);
        assert_eq!(bfs_path(&a, "a", "c").len(), 3); // a -> (b|d) -> c
        assert_eq!(bfs_path(&a, "a", "c")[0], "a");
        assert_eq!(bfs_path(&a, "a", "c")[2], "c");
    }

    #[test]
    fn bfs_path_no_path_is_empty_and_self_is_singleton() {
        let a = adj(&[("a", "b")]);
        assert!(bfs_path(&a, "b", "a").is_empty());
        assert_eq!(bfs_path(&a, "a", "a"), vec!["a".to_string()]);
    }
}
