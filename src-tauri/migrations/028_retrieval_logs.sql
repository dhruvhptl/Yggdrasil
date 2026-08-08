CREATE TABLE mimir_retrieval_logs (
  id TEXT PRIMARY KEY,
  query TEXT NOT NULL,
  node_id TEXT,
  tree_id TEXT,
  top_k INT,
  threshold FLOAT,
  candidates_before_rerank INT,
  candidates_after_rerank INT,
  prematch_chunks_used INT,
  rerank_fallback_used BOOLEAN,
  sources JSONB,
  created_at TIMESTAMPTZ DEFAULT NOW()
);
CREATE INDEX idx_retrieval_logs_created ON mimir_retrieval_logs(created_at DESC);
