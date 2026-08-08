ALTER TABLE universal_skills
  ADD COLUMN IF NOT EXISTS origin TEXT NOT NULL DEFAULT 'tree_quest'
    CHECK(origin IN ('resume','ontology','job_gap','resource','tree_quest','work')),
  ADD COLUMN IF NOT EXISTS state TEXT NOT NULL DEFAULT 'adjacent'
    CHECK(state IN ('seed','adjacent'));

-- Backfill: resume evidence → seed
UPDATE universal_skills SET origin = 'resume', state = 'seed'
WHERE evidence::text LIKE '%"type":"resume"%';

-- Backfill: work evidence (no resume) → work/adjacent
UPDATE universal_skills SET origin = 'work', state = 'adjacent'
WHERE evidence::text LIKE '%"type":"work_resource"%'
  AND NOT evidence::text LIKE '%"type":"resume"%';
