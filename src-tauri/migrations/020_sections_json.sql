-- migrations/020_sections_json.sql
-- Store the full section structure extracted from PDFs for re-embedding without re-upload.
ALTER TABLE mimir_resources ADD COLUMN IF NOT EXISTS sections_json JSONB;
