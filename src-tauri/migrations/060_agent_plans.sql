CREATE TABLE IF NOT EXISTS agent_plans (
    session_id TEXT PRIMARY KEY REFERENCES mimir_chat_sessions(id) ON DELETE CASCADE,
    content    TEXT NOT NULL,
    updated_at TIMESTAMPTZ DEFAULT NOW()
);
