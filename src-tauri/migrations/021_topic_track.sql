-- migrations/021_topic_track.sql
-- Add hardware/software/general track to research topics.
ALTER TABLE research_topics ADD COLUMN IF NOT EXISTS track TEXT NOT NULL DEFAULT 'general';
