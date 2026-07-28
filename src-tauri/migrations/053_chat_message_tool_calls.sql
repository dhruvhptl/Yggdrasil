ALTER TABLE mimir_chat_messages
    ADD COLUMN IF NOT EXISTS tool_calls JSONB,
    ADD COLUMN IF NOT EXISTS reasoning  TEXT;
