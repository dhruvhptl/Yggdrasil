# Concept Graphs (Phase 1)

**Date:** 2026-07-23
**Status:** Design approved — ready for implementation plan
**Depends on:** Phase 0 (mimir_memory.rs, mimir_agent.rs, agent loop, tool registry — all merged)

---

## Goal

Promote the current `concept_graph` JSONB blob (migration 033) into first-class tables so the
concept graph is queryable — by the agent, by the user, and eventually by tree-sitter (Phase 5).
Add three graph verbs (query, path, explain) as both Tauri commands and agent tools.

Tree generation already produces a concept graph during Phase 1 of tree gen
(`extract_concept_graph` in `prompt_builders.rs`) and persists it as a JSONB blob on `trees`
(`save_tree_to_database` in `tree_persistence.rs`). That blob is read back only by
`get_tree_concept_graph` today. Phase 1 routes it into structured tables and makes it useful.

---

## Decisions locked during brainstorming

| Fork | Decision | Why |
|---|---|---|
| JSONB migration | **Option A — write-once backfill.** Read every tree's blob on startup, insert into new tables (idempotent), keep the column for a safety window. Drop in a later migration (051+) after verification. | Minimal tech debt; old trees survive migration edge cases. |
| Verb interface | **Both Tauri commands + agent tools.** Shared backend functions with thin wrappers. Agent gets `query_graph`, `path_between`, `explain_node`; frontend gets matching commands. | Backend logic is identical; wrappers are ~15 lines. |
| `query` verb method | **Keyword match first, embedding fallback if available.** No LLM rewrite — the agent reformulates its own query. | Fast, deterministic, offline. |
| Edge confidence tags | **Everything `inferred` day one.** Phase 5 (tree-sitter) writes `extracted`. | Honest; schema CHECK is future-proof. |
| Embeddings on nodes | **Optional column `embedding vector(1024)`, left NULL in Phase 1** (see Grounding correction 4). NULL = keyword-only. | Zero cost now; incremental benefit later. |
| Relationship to `read_models.rs` | **Coexist.** `get_prereq_path`/`compute_learning_path` walk the tree hierarchy; concept-graph verbs walk the flexible DAG. | One is a curriculum, the other a knowledge map. |
| UI surface | **Minimal panel on tree page.** Text input + nested-list results. The agent is the primary interface; D3 deferred to Phase 2. | |

---

## Grounding corrections (verified against code 2026-07-23)

These override any conflicting detail elsewhere in this doc. They are technical facts about the
existing codebase, not design changes.

1. **Blob shape.** `trees.concept_graph` is a serialized `Vec<Concept>` — a flat JSON **array**,
   not `{nodes, edges}`. `Concept` (`prompt_builders.rs:18`) =
   `{ id, name, description, prerequisites: [concept_id...], concept_type, supporting_files: [], project_relevance }`.
   Nodes and edges are **derived**:
   - **node** per concept: `title = name`, `description = description`. `concept_type`,
     `supporting_files`, `project_relevance` are NOT migrated to columns in Phase 1 (they remain in
     the retained JSONB blob; add a metadata column later if needed). `file_path`/`line_*` stay NULL.
   - **edge** per prerequisite: for concept `C` with `prerequisites = [P1, P2]`, emit edges
     `source = P1 → target = C` and `source = P2 → target = C`, `relationship = 'prerequisite'`,
     `confidence = 'inferred'`. (Source is the prerequisite; target needs it.) Prerequisite ids
     that don't resolve to a concept in the same graph are skipped.
   - Both backfill and dual-write share ONE derivation routine keyed on a `concept.id → new node uuid` map.

2. **`path_between` must be graph-scoped.** Backend signature is
   `path_between(pool, tree_id, source_title, target_title)` — resolve `tree_id → graph_id`, and
   constrain BFS to that graph's nodes/edges. Without scoping, BFS crosses every tree's concepts.

3. **`explain_node` title resolution + resource bridge.**
   - The agent tool passes a `title`; the backend `explain_node` takes a `node_id`. The agent arm
     (and a `tree_id`-scoped lookup) resolves title → node within the tree's graph first.
   - `mimir_node_links.node_id` references **`tree_nodes.id`, not `concept_graph_nodes.id`**. To
     surface linked resources for a concept, bridge concept → `tree_nodes` in the same tree by
     matching `tree_nodes.title = concept title` (fallback `tree_nodes.concept_id = concept.id` /
     `concept_slug`), then join `mimir_node_links`. If no bridge matches, `resources` is empty.

4. **Embeddings NULL in Phase 1.** Nothing populates `concept_graph_nodes.embedding` this phase
   (the source `Concept` has no vector). The cosine-fallback branch of `query_graph` is written but
   dormant until a later enhancement embeds nodes. Keyword matching is the only live path.

---

## Architecture overview

```
Tree generation (tree_persistence.rs::save_tree_to_database)
    │  already: INSERT trees(... concept_graph=JSONB[Vec<Concept>])   ← unchanged (safety net)
    └─ Phase 1: also call create_concept_graph_from_concepts(pool, tree_id, &concepts)
                → concept_graphs + concept_graph_nodes + concept_graph_edges

Agent (mimir_agent.rs)
    └─ new tools query_graph / path_between / explain_node → concept_graph.rs backend

Tauri commands (main.rs)
    ├─ query_graph_cmd(tree_id, query) -> ConceptSubgraph
    ├─ path_between_cmd(tree_id, source, target) -> Vec<PathNode>
    └─ explain_node_cmd(node_id) -> NodeDetail

Migration 050 backfill (database.rs, post-migrate)
    └─ for each tree with JSONB blob and no concept_graphs row: parse Vec<Concept>,
       run the shared derivation, INSERT rows, log counts. Never blocks startup.
```

---

## Component 1 — Tables (`migrations/050_concept_graphs.sql`)

```sql
-- Migration 050: Concept graphs — first-class tables replacing the JSONB blob

CREATE TABLE IF NOT EXISTS concept_graphs (
    id          TEXT PRIMARY KEY,
    tree_id     TEXT NOT NULL REFERENCES trees(id) ON DELETE CASCADE,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_cg_tree ON concept_graphs(tree_id);

CREATE TABLE IF NOT EXISTS concept_graph_nodes (
    id          TEXT PRIMARY KEY,
    graph_id    TEXT NOT NULL REFERENCES concept_graphs(id) ON DELETE CASCADE,
    title       TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    file_path   TEXT,
    line_start  INT,
    line_end    INT,
    embedding   vector(1024),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_cgn_graph  ON concept_graph_nodes(graph_id);
CREATE INDEX IF NOT EXISTS idx_cgn_title  ON concept_graph_nodes(title);
CREATE INDEX IF NOT EXISTS idx_cgn_embedding ON concept_graph_nodes
    USING hnsw (embedding vector_cosine_ops) WITH (m = 16, ef_construction = 64);

CREATE TABLE IF NOT EXISTS concept_graph_edges (
    id              TEXT PRIMARY KEY,
    graph_id        TEXT NOT NULL REFERENCES concept_graphs(id) ON DELETE CASCADE,
    source_node_id  TEXT NOT NULL REFERENCES concept_graph_nodes(id) ON DELETE CASCADE,
    target_node_id  TEXT NOT NULL REFERENCES concept_graph_nodes(id) ON DELETE CASCADE,
    relationship    TEXT NOT NULL CHECK(relationship IN (
                        'prerequisite', 'implements', 'references', 'related')),
    confidence      TEXT NOT NULL CHECK(confidence IN (
                        'extracted', 'inferred', 'ambiguous')),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_cge_graph  ON concept_graph_edges(graph_id);
CREATE INDEX IF NOT EXISTS idx_cge_source ON concept_graph_edges(source_node_id);
CREATE INDEX IF NOT EXISTS idx_cge_target ON concept_graph_edges(target_node_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_cge_unique ON concept_graph_edges(
    graph_id, source_node_id, target_node_id, relationship);
```

---

## Component 2 — Backend (`src-tauri/src/concept_graph.rs`)

`create_concept_graph_from_concepts(pool, tree_id, concepts: &[Concept]) -> Result<String, String>`
- INSERT a `concept_graphs` row; build a `concept.id → node uuid` map while inserting nodes;
  then insert edges from each concept's `prerequisites` (skip unresolved ids). Returns graph id.
- Idempotent guard: if a `concept_graphs` row already exists for `tree_id`, return its id without
  re-inserting (protects against dual-write + backfill both firing).

`query_graph(pool, tree_id, query) -> ConceptSubgraph`
`ConceptSubgraph { nodes: Vec<ConceptNode>, edges: Vec<ConceptEdge>, match_method: "keyword"|"embedding" }`
1. Tokenize query (lowercase, split on non-alphanumeric, drop stopwords/dupes).
2. Score `concept_graph_nodes` in the tree's graph: exact title = 1.0; `title ILIKE '%word%'` = 0.8;
   description contains word = 0.4.
3. If any node ≥ 0.4, return matched nodes + all their edges (both directions), cap 30 nodes / 100 edges.
4. Else, if the graph has any non-NULL embedding, embed the query (`get_embedding`) and cosine top-5
   above 0.7. (Dormant in Phase 1 — see correction 4.)
5. Else empty subgraph.

`path_between(pool, tree_id, source_title, target_title) -> Vec<PathNode>`
`PathNode { title, depth, is_source, is_target }`
- BFS constrained to the tree's graph over `relationship='prerequisite'` edges only. Ordered
  source→target, cap 20. No path → empty vec.

`explain_node(pool, node_id) -> NodeDetail`
`NodeDetail { node, prerequisites: Vec<EdgeWithNode>, dependents: Vec<EdgeWithNode>, references: Vec<EdgeWithNode>, resources: Vec<LinkedResource> }`
- `prerequisites` = incoming `prerequisite` edges (target = node); `dependents` = outgoing (source = node);
  `references` = `references`/`implements` edges. `resources` via the tree_node bridge (correction 3).

---

## Component 3 — Agent tools (`mimir_agent.rs`, alongside the existing 7)

- `query_graph { query }` → `query_graph(pool, ctx.tree_id, query)`; format subgraph as bullet nodes+edges.
- `path_between { source, target }` → `path_between(pool, ctx.tree_id, source, target)`.
- `explain_node { title }` → resolve title → node id in ctx.tree_id's graph, then `explain_node(pool, node_id)`.
- All arms require `ctx.tree_id`; return a friendly message when no tree is active.

---

## Component 4 — Backfill (`database.rs`, after migrations run)

For each tree with a non-null `concept_graph` and no `concept_graphs` row: deserialize the blob to
`Vec<Concept>` (skip on parse failure or empty), call `create_concept_graph_from_concepts`, log the
count. Never blocks startup — per-tree failures are logged and skipped. One-shot and idempotent.

---

## Component 5 — Tree-generation dual-write (`tree_persistence.rs::save_tree_to_database`)

Immediately after the existing `INSERT INTO trees (... concept_graph ...)`, if
`sorted_concepts` is `Some`, call `create_concept_graph_from_concepts(&database.pool, &tree_id, concepts)`.
Keep writing the JSONB blob (safety net). Failure logs a warning and does NOT abort tree gen.
Dual-write ends when migration 051 drops the JSONB column (out of scope here).

---

## Tauri commands (register in `main.rs`)

`query_graph_cmd(tree_id, query) -> ConceptSubgraph`, `path_between_cmd(tree_id, source, target) -> Vec<PathNode>`,
`explain_node_cmd(node_id) -> NodeDetail`. All structs `#[serde(rename_all = "camelCase")]`;
commands `Result<T, String>`, `database: State<'_, Database>` last.

---

## Files

**Create:** `migrations/050_concept_graphs.sql`; `src-tauri/src/concept_graph.rs` (backend + commands + tests).
**Modify:** `main.rs` (`mod concept_graph;` + 3 commands); `mimir_agent.rs` (3 tool schemas + execute arms);
`tree_persistence.rs` (dual-write in `save_tree_to_database`); `database.rs` (backfill); optionally the tree page (minimal panel).
**No change:** `dev.sh`, `orchestrator.rs`, `mimir_memory.rs`.

---

## Error handling

- `create_concept_graph_from_concepts` failure → log, tree gen continues (JSONB is fallback).
- `query_graph` no match → empty subgraph (not an error). `path_between` no path → empty vec.
- `explain_node` bad title (agent arm) → resolution fails → friendly "concept not found".
- Backfill per-tree failure → skip + log, never block launch.

---

## Verification checklist

- `cargo build` clean; `cargo test` passes (unit tests: concept→node/edge derivation, BFS pathfinding, keyword scoring, blob parsing).
- Migration 050 applies; three tables + indexes exist.
- Generate a new tree → rows in all three new tables AND the JSONB blob still present (dual-write).
- Restart with an existing tree → backfill inserts rows for it; re-restart inserts nothing (idempotent).
- `query_graph` returns a correct subgraph for a known topic; unknown topic → empty, no crash.
- `path_between` returns a prerequisite chain; unrelated pair → empty vec.
- `explain_node` returns detail + prerequisites + dependents + linked resources (bridged).
- Agent shows 3 new tools; "how does X connect to Y?" triggers query_graph/path_between and references the graph.

---

## Build order (for the implementation plan)

1. Migration 050.
2. `concept_graph.rs` — structs, `create_concept_graph_from_concepts` (+ shared derivation), `query_graph`, `path_between`, `explain_node`, Tauri commands, unit tests.
3. Backfill in `database.rs`.
4. Dual-write in `tree_persistence.rs`.
5. Agent tools in `mimir_agent.rs`.
6. Register commands in `main.rs`.
7. (Optional) minimal frontend panel.
8. `cargo build` + verification.

---

## Explicitly out of scope (later phases)

D3 visualization + community detection (Phase 2); tree-sitter `extracted` edges (Phase 5); LLM query
rewriting; `delete_graph`/`update_graph` (graphs are immutable — regenerate the tree); dropping the
JSONB column (migration 051).
