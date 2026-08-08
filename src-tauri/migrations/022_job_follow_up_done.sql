-- migrations/022_job_follow_up_done.sql
-- Track when a follow-up has been marked done so it leaves the follow-ups view.
ALTER TABLE job_applications ADD COLUMN IF NOT EXISTS follow_up_done BOOLEAN NOT NULL DEFAULT FALSE;
