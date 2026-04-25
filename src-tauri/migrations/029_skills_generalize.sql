-- migrations/029_skills_generalize.sql
-- Make the skill system fully domain-agnostic:
--   * Widen universal_skills.kind constraint
--   * Widen skill_dependencies.relationship constraint
--   * Widen skill_evidence.source_type constraint
--   * Add review_needed + status to universal_skills
--   * Seed broad top-level domains into skill_domains
-- Safe to re-run: every DDL uses IF [NOT] EXISTS guards.

-- ─────────────────────────────────────────────────────────────────────────────
-- universal_skills.kind — drop narrow check, add wide one
-- ─────────────────────────────────────────────────────────────────────────────

ALTER TABLE universal_skills DROP CONSTRAINT IF EXISTS universal_skills_kind_check;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'universal_skills_kind_check_v2'
    ) THEN
        ALTER TABLE universal_skills
            ADD CONSTRAINT universal_skills_kind_check_v2
            CHECK (kind IN ('concept', 'technical', 'soft', 'practical', 'domain', 'unclassified'));
    END IF;
END $$;

-- ─────────────────────────────────────────────────────────────────────────────
-- universal_skills.review_needed + status — new human-curation columns
-- ─────────────────────────────────────────────────────────────────────────────

ALTER TABLE universal_skills
    ADD COLUMN IF NOT EXISTS review_needed BOOLEAN NOT NULL DEFAULT false;

ALTER TABLE universal_skills
    ADD COLUMN IF NOT EXISTS status TEXT NOT NULL DEFAULT 'active';

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'universal_skills_status_check'
    ) THEN
        ALTER TABLE universal_skills
            ADD CONSTRAINT universal_skills_status_check
            CHECK (status IN ('active', 'unclassified', 'archived'));
    END IF;
END $$;

CREATE INDEX IF NOT EXISTS idx_universal_skills_review_needed
    ON universal_skills(review_needed)
    WHERE review_needed = true;

CREATE INDEX IF NOT EXISTS idx_universal_skills_status ON universal_skills(status);

-- ─────────────────────────────────────────────────────────────────────────────
-- skill_dependencies.relationship — drop narrow check, add wide one
-- ─────────────────────────────────────────────────────────────────────────────

ALTER TABLE skill_dependencies DROP CONSTRAINT IF EXISTS skill_dep_relationship_check;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'skill_dep_relationship_check_v2'
    ) THEN
        ALTER TABLE skill_dependencies
            ADD CONSTRAINT skill_dep_relationship_check_v2
            CHECK (relationship IN ('prerequisite', 'related', 'part_of', 'specialization', 'co_occurs'));
    END IF;
END $$;

-- ─────────────────────────────────────────────────────────────────────────────
-- skill_evidence.source_type — drop narrow check, add wide one
-- ─────────────────────────────────────────────────────────────────────────────

ALTER TABLE skill_evidence DROP CONSTRAINT IF EXISTS skill_evidence_source_type_check;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'skill_evidence_source_type_check_v2'
    ) THEN
        ALTER TABLE skill_evidence
            ADD CONSTRAINT skill_evidence_source_type_check_v2
            CHECK (source_type IN ('resume', 'tree_quest', 'work_resource', 'manual', 'mimir_resource', 'job_demand', 'external'));
    END IF;
END $$;

-- ─────────────────────────────────────────────────────────────────────────────
-- skill_domains — seed broad top-level domains
-- ─────────────────────────────────────────────────────────────────────────────

INSERT INTO skill_domains (id, name, description)
VALUES
    ('dom-engineering',   'Engineering',    'Applied engineering disciplines including software, hardware, and systems'),
    ('dom-mathematics',   'Mathematics',    'Pure and applied mathematics, statistics, and formal reasoning'),
    ('dom-science',       'Science',        'Natural sciences: physics, chemistry, biology, and related fields'),
    ('dom-computing',     'Computing',      'Computer science, algorithms, data structures, and programming'),
    ('dom-communication', 'Communication',  'Writing, speaking, presenting, and interpersonal communication'),
    ('dom-design',        'Design',         'Product design, UX/UI, visual design, and creative disciplines'),
    ('dom-business',      'Business',       'Business strategy, finance, operations, and management'),
    ('dom-research',      'Research',       'Research methodology, academic writing, and knowledge synthesis')
ON CONFLICT (name) DO NOTHING;
