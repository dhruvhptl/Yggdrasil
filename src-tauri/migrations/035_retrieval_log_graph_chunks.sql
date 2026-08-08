ALTER TABLE mimir_retrieval_logs ADD COLUMN IF NOT EXISTS graph_chunks_used INT DEFAULT 0;
