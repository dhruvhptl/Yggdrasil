CREATE TABLE IF NOT EXISTS prompt_logs (
  id TEXT PRIMARY KEY,
  command TEXT NOT NULL,
  model TEXT NOT NULL,
  prompt_version TEXT NOT NULL DEFAULT 'v1',
  input_tokens INT,
  output_tokens INT,
  latency_ms INT,
  success BOOLEAN NOT NULL DEFAULT true,
  error TEXT,
  metadata JSONB,
  created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_prompt_logs_command ON prompt_logs(command, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_prompt_logs_model ON prompt_logs(model, created_at DESC);
