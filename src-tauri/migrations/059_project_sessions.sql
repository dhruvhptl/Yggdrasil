ALTER TABLE mimir_chat_sessions
    ADD COLUMN IF NOT EXISTS project_root_id TEXT REFERENCES project_roots(id) ON DELETE CASCADE;
-- Project-only sessions dedupe on project_root_id (the (tree_id,node_id) UNIQUE
-- treats NULLs as distinct, so it can't).
CREATE UNIQUE INDEX IF NOT EXISTS uq_chat_session_project
    ON mimir_chat_sessions(project_root_id) WHERE project_root_id IS NOT NULL;
