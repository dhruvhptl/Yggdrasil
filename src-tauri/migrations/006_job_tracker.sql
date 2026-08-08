CREATE TABLE IF NOT EXISTS job_applications (
    id TEXT PRIMARY KEY,
    company TEXT NOT NULL,
    position TEXT NOT NULL,
    location TEXT,
    source TEXT,
    status TEXT NOT NULL DEFAULT 'saved'
        CHECK(status IN ('saved','applied','interviewing','offer','rejected')),
    date_applied TIMESTAMPTZ,
    date_follow_up TIMESTAMPTZ,
    job_description TEXT,
    link TEXT,
    notes TEXT,
    rating_overall   INTEGER CHECK(rating_overall   BETWEEN 1 AND 5),
    rating_location  INTEGER CHECK(rating_location  BETWEEN 1 AND 5),
    rating_alignment INTEGER CHECK(rating_alignment BETWEEN 1 AND 5),
    rating_salary    INTEGER CHECK(rating_salary    BETWEEN 1 AND 5),
    rating_role      INTEGER CHECK(rating_role      BETWEEN 1 AND 5),
    season TEXT NOT NULL DEFAULT 'Winter 2026',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS job_skills (
    id TEXT PRIMARY KEY,
    job_id TEXT NOT NULL REFERENCES job_applications(id) ON DELETE CASCADE,
    skill_name TEXT NOT NULL,
    is_required BOOLEAN NOT NULL DEFAULT true
);
