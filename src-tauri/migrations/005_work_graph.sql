-- migrations/005_work_graph.sql
-- Work graph: co-op terms → research topics → resources → AI-extracted skill tags

CREATE TABLE IF NOT EXISTS coop_terms (
    id TEXT PRIMARY KEY,
    company TEXT NOT NULL,
    role TEXT NOT NULL,
    start_date TEXT NOT NULL,
    end_date TEXT NOT NULL,
    color TEXT NOT NULL DEFAULT '#10b981',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS research_topics (
    id TEXT PRIMARY KEY,
    coop_id TEXT NOT NULL REFERENCES coop_terms(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS work_resources (
    id TEXT PRIMARY KEY,
    topic_id TEXT NOT NULL REFERENCES research_topics(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    url TEXT,
    notes TEXT,
    completed BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS work_resource_skills (
    id TEXT PRIMARY KEY,
    resource_id TEXT NOT NULL REFERENCES work_resources(id) ON DELETE CASCADE,
    skill_name TEXT NOT NULL,
    tree_id TEXT REFERENCES trees(id) ON DELETE SET NULL
);
