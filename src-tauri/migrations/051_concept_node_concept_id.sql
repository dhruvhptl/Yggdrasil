-- Migration 051: durable concept id on graph nodes + usable title index

ALTER TABLE concept_graph_nodes ADD COLUMN IF NOT EXISTS concept_id TEXT;
CREATE INDEX IF NOT EXISTS idx_cgn_concept ON concept_graph_nodes(concept_id);

-- idx_cgn_title (plain btree on title) is unusable by LOWER()/ILIKE queries; replace it.
DROP INDEX IF EXISTS idx_cgn_title;
CREATE INDEX IF NOT EXISTS idx_cgn_title_lower ON concept_graph_nodes(LOWER(title));
