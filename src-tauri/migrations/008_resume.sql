-- Resume profile and projects
CREATE TABLE IF NOT EXISTS resume_profile (
    id TEXT PRIMARY KEY,
    raw_text TEXT NOT NULL,
    name TEXT,
    email TEXT,
    education JSONB NOT NULL DEFAULT '[]',
    work_experience JSONB NOT NULL DEFAULT '[]',
    skills JSONB NOT NULL DEFAULT '[]',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS resume_projects (
    id TEXT PRIMARY KEY,
    resume_id TEXT NOT NULL REFERENCES resume_profile(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    description TEXT,
    tech_stack JSONB NOT NULL DEFAULT '[]',
    github_url TEXT,
    linked_project_id TEXT REFERENCES projects(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
