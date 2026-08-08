-- Async transcript fetch job queue for YouTube resources.
-- Workers poll this table every 5 minutes and retry with backoff.
CREATE TABLE IF NOT EXISTS transcript_jobs (
  id              TEXT PRIMARY KEY,
  resource_id     TEXT NOT NULL REFERENCES mimir_resources(id) ON DELETE CASCADE,
  status          TEXT NOT NULL DEFAULT 'pending'
                    CHECK(status IN ('pending', 'processing', 'done', 'failed', 'skipped')),
  attempts        INT  NOT NULL DEFAULT 0,
  last_error      TEXT,
  next_retry_at   TIMESTAMPTZ,
  created_at      TIMESTAMPTZ DEFAULT NOW(),
  updated_at      TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_transcript_jobs_pending
  ON transcript_jobs(status, next_retry_at)
  WHERE status IN ('pending', 'failed');
