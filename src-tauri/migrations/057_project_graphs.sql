-- Bind a concept graph to a registered project (go-forward, set at scan time).
ALTER TABLE concept_graphs
    ADD COLUMN IF NOT EXISTS project_root_id TEXT REFERENCES project_roots(id) ON DELETE SET NULL;
ALTER TABLE concept_graphs ALTER COLUMN tree_id DROP NOT NULL;
CREATE INDEX IF NOT EXISTS idx_cg_project ON concept_graphs(project_root_id);
