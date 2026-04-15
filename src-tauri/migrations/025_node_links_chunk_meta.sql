-- migrations/025_node_links_chunk_meta.sql
-- Add chunk-level match metadata to mimir_node_links so the node panel
-- can show which section/pages matched, not just which resource.

ALTER TABLE mimir_node_links
    ADD COLUMN IF NOT EXISTS matched_chunk_id TEXT,
    ADD COLUMN IF NOT EXISTS matched_section_title TEXT,
    ADD COLUMN IF NOT EXISTS matched_page_start INT,
    ADD COLUMN IF NOT EXISTS matched_page_end INT;
