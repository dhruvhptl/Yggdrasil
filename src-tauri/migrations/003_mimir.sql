-- migrations/003_mimir.sql
-- Mimir resource library tables + Universal Skill Tree
-- pgvector extension required (enable in Neon dashboard or via SQL)

CREATE EXTENSION IF NOT EXISTS vector;

CREATE TABLE IF NOT EXISTS mimir_resources (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    url TEXT,
    type TEXT NOT NULL DEFAULT 'other',
    status TEXT NOT NULL DEFAULT 'unread',
    user_notes TEXT,
    content_hash TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS mimir_chunks (
    id TEXT PRIMARY KEY,
    resource_id TEXT NOT NULL REFERENCES mimir_resources(id) ON DELETE CASCADE,
    content TEXT NOT NULL,
    chunk_index INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS mimir_embeddings (
    id TEXT PRIMARY KEY,
    chunk_id TEXT NOT NULL REFERENCES mimir_chunks(id) ON DELETE CASCADE,
    embedding vector(384)
);

CREATE TABLE IF NOT EXISTS mimir_node_links (
    id TEXT PRIMARY KEY,
    resource_id TEXT NOT NULL REFERENCES mimir_resources(id) ON DELETE CASCADE,
    node_id TEXT NOT NULL REFERENCES tree_nodes(id) ON DELETE CASCADE,
    relevance_score REAL,
    UNIQUE(resource_id, node_id)
);

CREATE TABLE IF NOT EXISTS universal_skills (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    domain TEXT,
    level INTEGER NOT NULL DEFAULT 1 CHECK(level BETWEEN 1 AND 5),
    evidence JSONB NOT NULL DEFAULT '[]',
    last_updated TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_mimir_chunks_resource_id ON mimir_chunks(resource_id);
CREATE INDEX IF NOT EXISTS idx_mimir_node_links_node_id ON mimir_node_links(node_id);
CREATE INDEX IF NOT EXISTS idx_universal_skills_name ON universal_skills(name);
