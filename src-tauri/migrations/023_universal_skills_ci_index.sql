-- migrations/023_universal_skills_ci_index.sql
-- Add case-insensitive unique index so upsert_skill can use ON CONFLICT (LOWER(name)).
-- Drop the old case-sensitive unique constraint on name first if it exists.
ALTER TABLE universal_skills DROP CONSTRAINT IF EXISTS universal_skills_name_key;
CREATE UNIQUE INDEX IF NOT EXISTS idx_universal_skills_name_lower ON universal_skills (LOWER(name));
