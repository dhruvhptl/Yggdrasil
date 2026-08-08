-- Migration 046: add provenance columns to mimir_skill_links
-- source: how this link was created (manual | ann | tree_bridge | llm_extraction)
-- confidence: model confidence score (NULL for non-LLM sources)

ALTER TABLE mimir_skill_links
  ADD COLUMN IF NOT EXISTS source TEXT NOT NULL DEFAULT 'ann'
    CHECK (source IN ('manual', 'ann', 'tree_bridge', 'llm_extraction')),
  ADD COLUMN IF NOT EXISTS confidence FLOAT;

-- Backfill existing rows with approximate provenance
UPDATE mimir_skill_links SET source = 'manual'       WHERE relevance_score = 1.0;
UPDATE mimir_skill_links SET source = 'tree_bridge'  WHERE matched_chunk_id IS NULL AND relevance_score != 1.0;
-- Rows with matched_chunk_id and relevance_score != 1.0 remain 'ann' (default)
