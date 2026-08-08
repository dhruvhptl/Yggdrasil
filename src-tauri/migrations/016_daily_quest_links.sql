CREATE TABLE IF NOT EXISTS daily_quest_links (
    id TEXT PRIMARY KEY,
    date DATE NOT NULL,
    node_id TEXT,
    free_text TEXT,
    quadrant TEXT NOT NULL CHECK (quadrant IN ('do', 'schedule', 'delegate', 'eliminate')),
    sort_order INTEGER NOT NULL DEFAULT 0,
    added_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(date, node_id)
);
