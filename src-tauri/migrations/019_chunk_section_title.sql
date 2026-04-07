-- migrations/019_chunk_section_title.sql
-- Add section metadata to chunks for PDF chapter-aware ingestion.
ALTER TABLE mimir_chunks ADD COLUMN IF NOT EXISTS section_title TEXT;
ALTER TABLE mimir_chunks ADD COLUMN IF NOT EXISTS page_start INT;
ALTER TABLE mimir_chunks ADD COLUMN IF NOT EXISTS page_end INT;
