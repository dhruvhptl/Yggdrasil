ALTER TABLE mimir_resources ADD COLUMN IF NOT EXISTS parent_id TEXT REFERENCES mimir_resources(id) ON DELETE SET NULL;
