-- migrations/034_tree_versioning.sql
-- Supports diff-based tree regeneration with user-state preservation.

-- Track each regeneration event: which nodes were carried over vs dropped.
CREATE TABLE IF NOT EXISTS tree_regenerations (
  id TEXT PRIMARY KEY,
  old_tree_id TEXT REFERENCES trees(id) ON DELETE SET NULL,
  new_tree_id TEXT REFERENCES trees(id) ON DELETE SET NULL,
  regenerated_at TIMESTAMPTZ DEFAULT NOW(),
  nodes_carried INT NOT NULL DEFAULT 0,
  nodes_dropped INT NOT NULL DEFAULT 0,
  user_state_losses JSONB
);

-- pg_trgm is needed for title-similarity fallback matching during diff.
CREATE EXTENSION IF NOT EXISTS pg_trgm;
