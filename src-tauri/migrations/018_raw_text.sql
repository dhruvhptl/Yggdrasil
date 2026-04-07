-- migrations/018_raw_text.sql
-- Store the full extracted text from PDF ingestion so reembed_pdfs can
-- re-chunk without requiring the user to re-upload the file.
ALTER TABLE mimir_resources ADD COLUMN IF NOT EXISTS raw_text TEXT;
