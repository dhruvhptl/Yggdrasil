-- Migration 052: node-level confidence (extracted vs inferred) on concept graph nodes.
-- Existing nodes were all LLM-inferred → default 'inferred'. tree-sitter scanning
-- writes 'extracted'.
ALTER TABLE concept_graph_nodes
    ADD COLUMN IF NOT EXISTS confidence TEXT NOT NULL DEFAULT 'inferred'
    CHECK (confidence IN ('extracted', 'inferred', 'ambiguous'));
