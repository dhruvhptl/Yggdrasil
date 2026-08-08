-- Store tailored projects recommendation for each job
ALTER TABLE job_applications ADD COLUMN IF NOT EXISTS tailored_projects JSONB;
