-- Multiple resume versions with a single active switcher.
-- label: human-friendly version name; is_active: exactly one row true at a time.
ALTER TABLE resume_profile ADD COLUMN IF NOT EXISTS label TEXT NOT NULL DEFAULT '';
ALTER TABLE resume_profile ADD COLUMN IF NOT EXISTS is_active BOOLEAN NOT NULL DEFAULT FALSE;

-- Partial unique index: at most one active resume.
CREATE UNIQUE INDEX IF NOT EXISTS uq_resume_profile_active
    ON resume_profile (is_active) WHERE is_active;

-- Backfill: promote the newest existing resume to active so boot never shows empty.
UPDATE resume_profile SET is_active = TRUE
    WHERE id = (SELECT id FROM resume_profile ORDER BY created_at DESC LIMIT 1);
