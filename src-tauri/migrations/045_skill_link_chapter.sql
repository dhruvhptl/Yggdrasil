-- Migration 045 — chapter-granular skill→resource linking
--
-- Brings mimir_skill_links up to parity with mimir_node_links (mig 025):
--  - Denormalized chunk + section + page columns
--  - PK widened so one skill can link to multiple sections of the same resource
--
-- Idempotent: PK swap uses IF EXISTS / IF NOT EXISTS guards.

ALTER TABLE mimir_skill_links
  ADD COLUMN IF NOT EXISTS matched_chunk_id      TEXT REFERENCES mimir_chunks(id) ON DELETE SET NULL,
  ADD COLUMN IF NOT EXISTS matched_section_title TEXT,
  ADD COLUMN IF NOT EXISTS matched_page_start    INT,
  ADD COLUMN IF NOT EXISTS matched_page_end      INT;

-- Replace the (skill_id, resource_id) PK with a synthetic id and a wider
-- UNIQUE that includes the section. NULL section_title means "whole resource"
-- and Postgres treats NULLs as distinct in UNIQUE constraints, which is the
-- behaviour we want — but we also explicitly want at most ONE whole-resource
-- row per (skill, resource), so add a partial unique index for that case.

ALTER TABLE mimir_skill_links
  ADD COLUMN IF NOT EXISTS id TEXT;

UPDATE mimir_skill_links
SET id = skill_id || ':' || resource_id || ':' || COALESCE(matched_section_title, '')
WHERE id IS NULL;

ALTER TABLE mimir_skill_links
  ALTER COLUMN id SET NOT NULL;

ALTER TABLE mimir_skill_links
  DROP CONSTRAINT IF EXISTS mimir_skill_links_pkey;

ALTER TABLE mimir_skill_links
  ADD CONSTRAINT mimir_skill_links_pkey PRIMARY KEY (id);

-- Section-aware uniqueness: one row per (skill, resource, section_title).
-- Two NULL section_titles would be considered distinct under default UNIQUE
-- semantics, so we add a separate partial index for the whole-resource case.
CREATE UNIQUE INDEX IF NOT EXISTS uq_mimir_skill_links_skill_resource_section
  ON mimir_skill_links (skill_id, resource_id, matched_section_title)
  WHERE matched_section_title IS NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS uq_mimir_skill_links_skill_resource_whole
  ON mimir_skill_links (skill_id, resource_id)
  WHERE matched_section_title IS NULL;
