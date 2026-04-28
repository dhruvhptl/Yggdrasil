-- Replace exact cosine index with HNSW for ANN search
DROP INDEX IF EXISTS mimir_embeddings_embedding_idx;
CREATE INDEX IF NOT EXISTS idx_mimir_embeddings_hnsw
ON mimir_embeddings USING hnsw (embedding vector_cosine_ops)
WITH (m = 16, ef_construction = 64);

-- Add embedding cache on tree_nodes
ALTER TABLE tree_nodes ADD COLUMN IF NOT EXISTS title_embedding vector(1024);
CREATE INDEX IF NOT EXISTS idx_tree_nodes_embedding
ON tree_nodes USING hnsw (title_embedding vector_cosine_ops)
WITH (m = 16, ef_construction = 64);
