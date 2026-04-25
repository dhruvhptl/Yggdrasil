-- migrations/026_foundation.sql
-- Foundation schema for V2 work:
--   * Tree/project versioning groundwork (parent_tree_id, version, archived_at, active_tree_id)
--   * Stable concept identity on tree nodes (concept_id, concept_slug)
--   * Hardened daily_quest_links FK
--   * Knowledge-graph skeleton for skills (domains, kind, concept_slug, parent, aliases,
--     widened dependency relationships, first-class evidence table)
-- Safe to re-run: every DDL uses IF [NOT] EXISTS guards.

-- ─────────────────────────────────────────────────────────────────────────────
-- Trees / Projects — versioning groundwork
-- ─────────────────────────────────────────────────────────────────────────────

ALTER TABLE trees ADD COLUMN IF NOT EXISTS parent_tree_id TEXT REFERENCES trees(id) ON DELETE SET NULL;
ALTER TABLE trees ADD COLUMN IF NOT EXISTS version INT NOT NULL DEFAULT 1;
ALTER TABLE trees ADD COLUMN IF NOT EXISTS archived_at TIMESTAMPTZ;

ALTER TABLE projects ADD COLUMN IF NOT EXISTS active_tree_id TEXT REFERENCES trees(id) ON DELETE SET NULL;

-- Backfill trees.version: row_number per project ordered by created_at (oldest = 1).
-- Only runs for rows still at the default (1) so re-running is a no-op once real data exists.
WITH ranked AS (
    SELECT id,
           ROW_NUMBER() OVER (PARTITION BY project_id ORDER BY created_at, id) AS rn
    FROM trees
)
UPDATE trees t
SET version = ranked.rn
FROM ranked
WHERE t.id = ranked.id
  AND t.version = 1
  AND ranked.rn <> 1;

-- Backfill projects.active_tree_id: most recent tree per project.
WITH latest AS (
    SELECT DISTINCT ON (project_id) project_id, id AS tree_id
    FROM trees
    ORDER BY project_id, created_at DESC, id DESC
)
UPDATE projects p
SET active_tree_id = latest.tree_id
FROM latest
WHERE p.id = latest.project_id
  AND p.active_tree_id IS NULL;

CREATE INDEX IF NOT EXISTS idx_trees_project_id_created ON trees(project_id, created_at);
CREATE INDEX IF NOT EXISTS idx_trees_parent_tree_id ON trees(parent_tree_id);

-- ─────────────────────────────────────────────────────────────────────────────
-- Tree nodes — stable concept identity
-- ─────────────────────────────────────────────────────────────────────────────

ALTER TABLE tree_nodes ADD COLUMN IF NOT EXISTS concept_id TEXT;
ALTER TABLE tree_nodes ADD COLUMN IF NOT EXISTS concept_slug TEXT;

CREATE INDEX IF NOT EXISTS idx_tree_nodes_concept_slug ON tree_nodes(tree_id, concept_slug);
CREATE INDEX IF NOT EXISTS idx_tree_nodes_concept_id ON tree_nodes(concept_id);

-- ─────────────────────────────────────────────────────────────────────────────
-- Daily quest links — FK to tree_nodes (was orphan-prone TEXT column)
-- ─────────────────────────────────────────────────────────────────────────────

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'fk_daily_node'
    ) THEN
        ALTER TABLE daily_quest_links
            ADD CONSTRAINT fk_daily_node
            FOREIGN KEY (node_id) REFERENCES tree_nodes(id) ON DELETE CASCADE;
    END IF;
END $$;

-- ─────────────────────────────────────────────────────────────────────────────
-- Skill domains
-- ─────────────────────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS skill_domains (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    description TEXT,
    parent_domain_id TEXT REFERENCES skill_domains(id) ON DELETE SET NULL,
    color TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_skill_domains_parent ON skill_domains(parent_domain_id);

-- ─────────────────────────────────────────────────────────────────────────────
-- Universal skills — knowledge-graph upgrades
-- ─────────────────────────────────────────────────────────────────────────────

ALTER TABLE universal_skills
    ADD COLUMN IF NOT EXISTS domain_id TEXT REFERENCES skill_domains(id) ON DELETE SET NULL;

ALTER TABLE universal_skills
    ADD COLUMN IF NOT EXISTS kind TEXT NOT NULL DEFAULT 'technical';

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'universal_skills_kind_check'
    ) THEN
        ALTER TABLE universal_skills
            ADD CONSTRAINT universal_skills_kind_check
            CHECK (kind IN ('concept', 'technical'));
    END IF;
END $$;

ALTER TABLE universal_skills ADD COLUMN IF NOT EXISTS concept_slug TEXT;
ALTER TABLE universal_skills
    ADD COLUMN IF NOT EXISTS parent_skill_id TEXT REFERENCES universal_skills(id) ON DELETE SET NULL;

CREATE UNIQUE INDEX IF NOT EXISTS idx_universal_skills_concept_slug
    ON universal_skills(concept_slug)
    WHERE concept_slug IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_universal_skills_domain_id ON universal_skills(domain_id);
CREATE INDEX IF NOT EXISTS idx_universal_skills_parent_skill_id ON universal_skills(parent_skill_id);
CREATE INDEX IF NOT EXISTS idx_universal_skills_kind ON universal_skills(kind);

-- ─────────────────────────────────────────────────────────────────────────────
-- Skill aliases — canonical form deduplication
-- ─────────────────────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS skill_aliases (
    id TEXT PRIMARY KEY,
    canonical_skill_id TEXT NOT NULL REFERENCES universal_skills(id) ON DELETE CASCADE,
    alias TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(alias)
);

CREATE INDEX IF NOT EXISTS idx_skill_aliases_canonical ON skill_aliases(canonical_skill_id);
CREATE INDEX IF NOT EXISTS idx_skill_aliases_alias_lower ON skill_aliases(LOWER(alias));

-- ─────────────────────────────────────────────────────────────────────────────
-- Skill dependencies — widened relationship vocabulary + manual-curation flag
-- ─────────────────────────────────────────────────────────────────────────────

ALTER TABLE skill_dependencies
    ADD COLUMN IF NOT EXISTS is_manual BOOLEAN NOT NULL DEFAULT false;

ALTER TABLE skill_dependencies DROP CONSTRAINT IF EXISTS skill_dependencies_relationship_check;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'skill_dep_relationship_check'
    ) THEN
        ALTER TABLE skill_dependencies
            ADD CONSTRAINT skill_dep_relationship_check
            CHECK (relationship IN ('prerequisite', 'specialization', 'related', 'co_occurs'));
    END IF;
END $$;

CREATE INDEX IF NOT EXISTS idx_skill_dependencies_source ON skill_dependencies(source_skill_id);
CREATE INDEX IF NOT EXISTS idx_skill_dependencies_target ON skill_dependencies(target_skill_id);
CREATE INDEX IF NOT EXISTS idx_skill_dependencies_relationship ON skill_dependencies(relationship);

-- ─────────────────────────────────────────────────────────────────────────────
-- Skill evidence — first-class rows replace JSONB accumulation
-- ─────────────────────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS skill_evidence (
    id TEXT PRIMARY KEY,
    skill_id TEXT NOT NULL REFERENCES universal_skills(id) ON DELETE CASCADE,
    source_type TEXT NOT NULL CHECK (source_type IN ('resume', 'tree_quest', 'work_resource', 'manual')),
    source_id TEXT,
    payload JSONB NOT NULL,
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Composite unique index to emulate UNIQUE(skill_id, source_type, COALESCE(source_id, '')):
-- Postgres table constraints can't embed expressions, so a unique index is the idiomatic form.
CREATE UNIQUE INDEX IF NOT EXISTS idx_skill_evidence_unique
    ON skill_evidence (skill_id, source_type, COALESCE(source_id, ''));

CREATE INDEX IF NOT EXISTS idx_skill_evidence_skill_id ON skill_evidence(skill_id);
CREATE INDEX IF NOT EXISTS idx_skill_evidence_source ON skill_evidence(source_type, source_id);
