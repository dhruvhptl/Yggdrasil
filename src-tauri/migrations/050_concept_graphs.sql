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
CREATE INDEX IF NOT EXISTS idx_cgn_graph ON concept_graph_nodes(graph_id);
CREATE INDEX IF NOT EXISTS idx_cgn_title ON concept_graph_nodes(title);
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
