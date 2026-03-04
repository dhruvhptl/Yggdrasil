-- 011_skill_dependencies.sql
-- Skill dependency edges for the Universal Skill Tree galaxy

CREATE TABLE IF NOT EXISTS skill_dependencies (
    id TEXT PRIMARY KEY,
    source_skill_id TEXT NOT NULL REFERENCES universal_skills(id) ON DELETE CASCADE,
    target_skill_id TEXT NOT NULL REFERENCES universal_skills(id) ON DELETE CASCADE,
    relationship TEXT NOT NULL DEFAULT 'prerequisite',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(source_skill_id, target_skill_id)
);

CREATE INDEX IF NOT EXISTS idx_skill_deps_source ON skill_dependencies(source_skill_id);
CREATE INDEX IF NOT EXISTS idx_skill_deps_target ON skill_dependencies(target_skill_id);

-- Add UNIQUE on name for upsert support
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'universal_skills_name_unique'
    ) THEN
        ALTER TABLE universal_skills ADD CONSTRAINT universal_skills_name_unique UNIQUE (name);
    END IF;
END$$;
