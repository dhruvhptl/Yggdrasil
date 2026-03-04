CREATE TABLE IF NOT EXISTS ideas (
    id TEXT PRIMARY KEY,
    content TEXT NOT NULL,
    tag TEXT NOT NULL DEFAULT 'random'
        CHECK(tag IN ('project_idea', 'resource', 'random', 'learning')),
    pinned BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
