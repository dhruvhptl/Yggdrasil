-- Migration 044 — skill embeddings for ANN-based skill similarity search.
-- Same model as Mimir (pplx-embed-v1-0.6b, 1024d) so chunk and skill spaces
-- are compatible. HNSW cosine index mirrors the one on tree_nodes.title_embedding.

ALTER TABLE universal_skills
  ADD COLUMN IF NOT EXISTS embedding vector(1024);

CREATE INDEX IF NOT EXISTS idx_universal_skills_embedding
  ON universal_skills
  USING hnsw (embedding vector_cosine_ops)
  WITH (m = 16, ef_construction = 64);
