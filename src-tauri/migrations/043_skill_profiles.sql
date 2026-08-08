-- Migration 043 — skill_profiles, skill_trees, mimir_skill_links, skill_project_links
--
-- Note on types: every PK/FK column is TEXT to match the existing schema
-- (universal_skills.id, trees.id, mimir_resources.id, projects.id are all TEXT
-- holding UUID strings). Declaring these as UUID would fail the FK with a
-- type-mismatch error in Postgres.

-- ── Special "Skills" project (Option C) ─────────────────────────────────────
-- All skill-seeded trees use this as their project_id so the existing
-- trees.project_id NOT NULL + REFERENCES projects(id) FK is satisfied.
INSERT INTO projects (id, name, description, discipline_ids, skill_ids, status, progress)
VALUES (
  '00000000-0000-0000-0000-000000000001',
  'Skills',
  'Container project for skill-seeded trees generated from the Skills page.',
  '[]'::jsonb,
  '[]'::jsonb,
  'active',
  0
)
ON CONFLICT (id) DO NOTHING;

-- ── skill_profiles ──────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS skill_profiles (
  skill_id   TEXT PRIMARY KEY REFERENCES universal_skills(id) ON DELETE CASCADE,
  status     TEXT NOT NULL DEFAULT 'untouched'
             CHECK (status IN ('untouched','in_progress','practiced','mastered')),
  notes      TEXT,
  created_at TIMESTAMPTZ DEFAULT now(),
  updated_at TIMESTAMPTZ DEFAULT now()
);

-- ── skill_trees ─────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS skill_trees (
  skill_id TEXT REFERENCES universal_skills(id) ON DELETE CASCADE,
  tree_id  TEXT REFERENCES trees(id) ON DELETE CASCADE,
  PRIMARY KEY (skill_id, tree_id)
);

CREATE INDEX IF NOT EXISTS idx_skill_trees_tree
  ON skill_trees(tree_id);

-- ── mimir_skill_links ───────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS mimir_skill_links (
  skill_id        TEXT REFERENCES universal_skills(id) ON DELETE CASCADE,
  resource_id     TEXT REFERENCES mimir_resources(id) ON DELETE CASCADE,
  relevance_score FLOAT,
  added_at        TIMESTAMPTZ DEFAULT now(),
  PRIMARY KEY (skill_id, resource_id)
);

CREATE INDEX IF NOT EXISTS idx_mimir_skill_links_skill
  ON mimir_skill_links(skill_id);
CREATE INDEX IF NOT EXISTS idx_mimir_skill_links_resource
  ON mimir_skill_links(resource_id);

-- ── skill_project_links ─────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS skill_project_links (
  skill_id   TEXT REFERENCES universal_skills(id) ON DELETE CASCADE,
  project_id TEXT REFERENCES projects(id) ON DELETE CASCADE,
  PRIMARY KEY (skill_id, project_id)
);

CREATE INDEX IF NOT EXISTS idx_skill_project_links_project
  ON skill_project_links(project_id);

-- ── 5. Backfill skill_profiles ──────────────────────────────────────────────
INSERT INTO skill_profiles (skill_id)
SELECT id FROM universal_skills
ON CONFLICT DO NOTHING;

-- ── 6. Backfill skill_trees (active trees only, non-null slugs) ─────────────
INSERT INTO skill_trees (skill_id, tree_id)
SELECT DISTINCT u.id, t.id
FROM universal_skills u
JOIN tree_nodes n ON n.concept_slug = u.concept_slug
JOIN trees t ON t.id = n.tree_id
WHERE u.concept_slug IS NOT NULL
  AND n.concept_slug IS NOT NULL
  AND t.archived_at IS NULL
ON CONFLICT DO NOTHING;

-- ── 7. Backfill mimir_skill_links from existing node links ──────────────────
INSERT INTO mimir_skill_links (skill_id, resource_id, relevance_score)
SELECT DISTINCT u.id, mnl.resource_id, mnl.relevance_score
FROM mimir_node_links mnl
JOIN tree_nodes n ON n.id = mnl.node_id
JOIN universal_skills u ON u.concept_slug = n.concept_slug
WHERE n.concept_slug IS NOT NULL
  AND u.concept_slug IS NOT NULL
ON CONFLICT DO NOTHING;

-- ── 8. Backfill skill_project_links from tree_nodes concept_slug walk ───────
INSERT INTO skill_project_links (skill_id, project_id)
SELECT DISTINCT u.id, t.project_id
FROM universal_skills u
JOIN tree_nodes n ON n.concept_slug = u.concept_slug
JOIN trees t ON t.id = n.tree_id
WHERE u.concept_slug IS NOT NULL
  AND n.concept_slug IS NOT NULL
  AND t.project_id IS NOT NULL
  AND t.archived_at IS NULL
ON CONFLICT DO NOTHING;
