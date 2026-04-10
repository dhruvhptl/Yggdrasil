-- migrations/024_daily_task_completed.sql
-- Add completed flag to daily_quest_links so free-text task completion persists.
ALTER TABLE daily_quest_links ADD COLUMN IF NOT EXISTS completed BOOLEAN NOT NULL DEFAULT FALSE;
