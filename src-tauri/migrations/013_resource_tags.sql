ALTER TABLE mimir_resources ADD COLUMN IF NOT EXISTS tags TEXT[] NOT NULL DEFAULT '{}';
CREATE INDEX IF NOT EXISTS idx_mimir_resources_tags ON mimir_resources USING gin(tags);
