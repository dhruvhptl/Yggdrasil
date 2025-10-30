-- migrations/001_initial.sql - Initial database schema

CREATE TABLE IF NOT EXISTS projects (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    description TEXT NOT NULL,
    discipline_ids TEXT NOT NULL DEFAULT '[]', -- JSON array of discipline IDs
    skill_ids TEXT NOT NULL DEFAULT '[]',      -- JSON array of skill IDs
    status TEXT NOT NULL DEFAULT 'active',
    created_at TEXT NOT NULL,
    progress INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS disciplines (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL UNIQUE,
    description TEXT NOT NULL,
    color TEXT
);

CREATE TABLE IF NOT EXISTS skills (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    description TEXT NOT NULL,
    discipline_id TEXT NOT NULL,
    proficiency_level TEXT NOT NULL DEFAULT 'beginner',
    progress INTEGER NOT NULL DEFAULT 0,
    is_unlocked BOOLEAN NOT NULL DEFAULT FALSE,
    prerequisites TEXT NOT NULL DEFAULT '[]', -- JSON array of prerequisite skill IDs
    project_ids TEXT NOT NULL DEFAULT '[]',   -- JSON array of project IDs
    FOREIGN KEY (discipline_id) REFERENCES disciplines (id)
);

CREATE TABLE IF NOT EXISTS quests (
    id TEXT PRIMARY KEY NOT NULL,
    title TEXT NOT NULL,
    description TEXT NOT NULL,
    skill_id TEXT NOT NULL,
    project_id TEXT NOT NULL,
    is_completed BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TEXT NOT NULL,
    due_date TEXT,
    priority TEXT NOT NULL DEFAULT 'medium',
    FOREIGN KEY (skill_id) REFERENCES skills (id),
    FOREIGN KEY (project_id) REFERENCES projects (id)
);

CREATE TABLE IF NOT EXISTS resources (
    id TEXT PRIMARY KEY NOT NULL,
    title TEXT NOT NULL,
    description TEXT NOT NULL,
    url TEXT NOT NULL,
    type TEXT NOT NULL DEFAULT 'other',
    skill_ids TEXT NOT NULL DEFAULT '[]',   -- JSON array of skill IDs
    project_ids TEXT NOT NULL DEFAULT '[]', -- JSON array of project IDs
    source TEXT,
    notion_id TEXT
);

-- Create indexes for better performance
CREATE INDEX IF NOT EXISTS idx_projects_status ON projects(status);
CREATE INDEX IF NOT EXISTS idx_projects_created_at ON projects(created_at);
CREATE INDEX IF NOT EXISTS idx_skills_discipline_id ON skills(discipline_id);
CREATE INDEX IF NOT EXISTS idx_skills_proficiency_level ON skills(proficiency_level);
CREATE INDEX IF NOT EXISTS idx_quests_skill_id ON quests(skill_id);
CREATE INDEX IF NOT EXISTS idx_quests_project_id ON quests(project_id);
CREATE INDEX IF NOT EXISTS idx_quests_is_completed ON quests(is_completed);