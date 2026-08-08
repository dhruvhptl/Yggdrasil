-- Add display_name on universal_skills and backfill from name.
-- display_name is the user-facing label; name remains the canonical key.
-- Idempotent: safe to re-run.

ALTER TABLE universal_skills
  ADD COLUMN IF NOT EXISTS display_name TEXT;

UPDATE universal_skills
SET display_name = name
WHERE display_name IS NULL OR display_name = '';
