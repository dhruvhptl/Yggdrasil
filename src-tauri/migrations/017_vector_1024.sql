-- migrations/017_vector_1024.sql
-- Fix embedding dimensions: pplx-embed-v1-0.6b natively outputs 1024 dims.
-- The previous vector(384) column was based on an incorrect MRL truncation assumption.
-- All existing embeddings are corrupted (truncated/mismatched), so we truncate them first.

-- Remove all existing (corrupted) embeddings and chunks — they must be re-ingested.
-- Both tables are listed together so Postgres handles FK ordering automatically.
TRUNCATE TABLE mimir_chunks, mimir_embeddings;

-- Widen the embedding column from vector(384) to vector(1024).
ALTER TABLE mimir_embeddings ALTER COLUMN embedding TYPE vector(1024);
