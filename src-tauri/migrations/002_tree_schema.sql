-- migrations/002_tree_schema.sql

CREATE TABLE IF NOT EXISTS trees (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL,
    name TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS tree_nodes (
    id TEXT PRIMARY KEY NOT NULL,
    tree_id TEXT NOT NULL,
    parent_id TEXT,
    type TEXT NOT NULL CHECK(type IN ('trunk', 'branch', 'leaf')),
    title TEXT NOT NULL,
    description TEXT,
    progress INTEGER DEFAULT 0,
    tasks JSONB NOT NULL DEFAULT '[]',
    resources JSONB DEFAULT NULL,
    x FLOAT8,
    y FLOAT8,
    order_index INTEGER DEFAULT 0,
    FOREIGN KEY (tree_id) REFERENCES trees(id) ON DELETE CASCADE,
    FOREIGN KEY (parent_id) REFERENCES tree_nodes(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS tree_edges (
    id TEXT PRIMARY KEY NOT NULL,
    tree_id TEXT NOT NULL,
    source_node_id TEXT NOT NULL,
    target_node_id TEXT NOT NULL,
    FOREIGN KEY (tree_id) REFERENCES trees(id) ON DELETE CASCADE,
    FOREIGN KEY (source_node_id) REFERENCES tree_nodes(id) ON DELETE CASCADE,
    FOREIGN KEY (target_node_id) REFERENCES tree_nodes(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_tree_nodes_tree_id ON tree_nodes(tree_id);
CREATE INDEX IF NOT EXISTS idx_tree_nodes_parent_id ON tree_nodes(parent_id);
CREATE INDEX IF NOT EXISTS idx_tree_edges_tree_id ON tree_edges(tree_id);
