CREATE TABLE IF NOT EXISTS project_coverage (
    id                TEXT PRIMARY KEY,
    project_root_id   TEXT NOT NULL REFERENCES project_roots(id) ON DELETE CASCADE,
    tree_id           TEXT REFERENCES trees(id) ON DELETE CASCADE,
    checkpoint_node_id TEXT NOT NULL,
    checkpoint_title  TEXT NOT NULL,
    status            TEXT NOT NULL CHECK(status IN ('covered','partial','gap')),
    evidence_node_id  TEXT,
    reason            TEXT NOT NULL DEFAULT '',
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(project_root_id, checkpoint_node_id)
);
CREATE INDEX IF NOT EXISTS idx_pcov_project ON project_coverage(project_root_id);
