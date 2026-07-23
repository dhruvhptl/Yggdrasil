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
}
