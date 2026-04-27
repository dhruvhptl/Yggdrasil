-- migrations/033_tree_concept_graph.sql
-- Persist the extracted concept graph per tree so it can be used for
-- diff-based regeneration, concept_slug backfill, and frontend inspection.

ALTER TABLE trees ADD COLUMN IF NOT EXISTS concept_graph JSONB;
