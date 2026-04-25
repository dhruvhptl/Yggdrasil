-- migrations/030_fts.sql
-- Hybrid retrieval: add full-text search index to mimir_chunks + log hybrid stats.

-- Generated TSVECTOR column — maintained automatically by Postgres.
ALTER TABLE mimir_chunks
    ADD COLUMN IF NOT EXISTS fts_vector TSVECTOR
        GENERATED ALWAYS AS (to_tsvector('english', content)) STORED;

CREATE INDEX IF NOT EXISTS idx_mimir_chunks_fts ON mimir_chunks USING GIN(fts_vector);

-- Hybrid retrieval log columns
ALTER TABLE mimir_retrieval_logs ADD COLUMN IF NOT EXISTS lexical_candidates INT;
ALTER TABLE mimir_retrieval_logs ADD COLUMN IF NOT EXISTS hybrid_candidates_merged INT;
