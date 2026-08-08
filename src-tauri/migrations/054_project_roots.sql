CREATE TABLE IF NOT EXISTS project_roots (
    id         TEXT PRIMARY KEY,
    label      TEXT NOT NULL,
    path       TEXT NOT NULL UNIQUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
