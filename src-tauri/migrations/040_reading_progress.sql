CREATE TABLE IF NOT EXISTS resource_reading_progress (
  id              TEXT PRIMARY KEY,
  resource_id     TEXT NOT NULL REFERENCES mimir_resources(id) ON DELETE CASCADE,
  section_title   TEXT NOT NULL,
  page_start      INT,
  page_end        INT,
  completed_at    TIMESTAMPTZ DEFAULT NOW(),
  UNIQUE(resource_id, section_title)
);

CREATE INDEX IF NOT EXISTS idx_reading_progress_resource
  ON resource_reading_progress(resource_id);
