-- Migration 047: mimir_proposals — LLM-proposed graph repairs (proposal-based, never auto-applied)

CREATE TABLE IF NOT EXISTS mimir_proposals (
  id            TEXT PRIMARY KEY,
  type          TEXT NOT NULL CHECK (type IN (
                  'merge_skills', 'delete_skill', 'add_edge',
                  'remove_edge', 'rename_skill')),
  payload       JSONB NOT NULL,
  llm_reasoning TEXT NOT NULL,
  status        TEXT NOT NULL DEFAULT 'pending'
                  CHECK (status IN ('pending', 'approved', 'rejected')),
  created_at    TIMESTAMPTZ DEFAULT now(),
  reviewed_at   TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_mimir_proposals_status ON mimir_proposals(status);
