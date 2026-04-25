CREATE TABLE IF NOT EXISTS mimir_chat_sessions (
  id TEXT PRIMARY KEY,
  tree_id TEXT REFERENCES trees(id) ON DELETE CASCADE,
  node_id TEXT REFERENCES tree_nodes(id) ON DELETE CASCADE,
  created_at TIMESTAMPTZ DEFAULT NOW(),
  updated_at TIMESTAMPTZ DEFAULT NOW(),
  UNIQUE(tree_id, node_id)
);

CREATE TABLE IF NOT EXISTS mimir_chat_messages (
  id TEXT PRIMARY KEY,
  session_id TEXT NOT NULL REFERENCES mimir_chat_sessions(id) ON DELETE CASCADE,
  role TEXT NOT NULL CHECK(role IN ('user', 'assistant')),
  content TEXT NOT NULL,
  sources JSONB,
  created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_chat_messages_session ON mimir_chat_messages(session_id, created_at);
