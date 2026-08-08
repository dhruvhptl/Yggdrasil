-- Dedup index for jobs created via the browser extension.
-- NULL links are excluded (each NULL is treated as distinct; dedup logic is handled in application code).
CREATE UNIQUE INDEX IF NOT EXISTS jobs_company_position_link_unique
    ON job_applications (company, position, link)
    WHERE link IS NOT NULL;
