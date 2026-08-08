-- Migration 049: Mimir memory system — durable facts + rolling session summaries

-- Long-term memory: one unified facts table, scoped user/tree/project.
-- entity_id makes per-entity facts (mastered_concept per node) accumulate
-- instead of overwriting; NULL entity_id = singular fact that upserts in place.
CREATE TABLE IF NOT EXISTS mimir_memory_facts (
    id          TEXT PRIMARY KEY,
    scope       TEXT NOT NULL CHECK(scope IN ('user','tree','project')),
    tree_id     TEXT REFERENCES trees(id) ON DELETE CASCADE,
    project_id  TEXT REFERENCES projects(id) ON DELETE CASCADE,
    fact_key    TEXT NOT NULL,
    entity_id   TEXT,
    fact_value  JSONB NOT NULL,
    confidence  FLOAT NOT NULL DEFAULT 0.7 CHECK(confidence >= 0.0 AND confidence <= 1.0),
    source      TEXT NOT NULL CHECK(source IN (
                    'agent_extraction','checkpoint_complete','resource_complete',
                    'user_said','manual','session_consolidation')),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Collision-safe uniqueness: same (scope+owner+key+entity) upserts in place;
-- different entities coexist. COALESCE handles NULL owners/entities.
CREATE UNIQUE INDEX IF NOT EXISTS idx_memory_facts_unique ON mimir_memory_facts (
    scope, COALESCE(tree_id,''), COALESCE(project_id,''), fact_key, COALESCE(entity_id,'')
);
CREATE INDEX IF NOT EXISTS idx_memory_facts_tree  ON mimir_memory_facts(tree_id);
CREATE INDEX IF NOT EXISTS idx_memory_facts_scope ON mimir_memory_facts(scope);

-- Short-term memory: ONE rolling summary row per chat session, updated
-- incrementally. covered_through = created_at of the last message summarized.
CREATE TABLE IF NOT EXISTS mimir_memory_shortterm (
    id              TEXT PRIMARY KEY,
    session_id      TEXT NOT NULL REFERENCES mimir_chat_sessions(id) ON DELETE CASCADE,
    summary         TEXT NOT NULL,
    key_topics      TEXT[] NOT NULL DEFAULT '{}',
    covered_through TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(session_id)
);
CREATE INDEX IF NOT EXISTS idx_memory_shortterm_topics ON mimir_memory_shortterm USING GIN(key_topics);
