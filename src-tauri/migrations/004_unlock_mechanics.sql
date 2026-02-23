-- migrations/004_unlock_mechanics.sql
-- Add per-node lock state for progressive skill unlocking.
-- trunk (phase) and leaf (quest) nodes default false (always unlocked).
-- branch (skill) nodes: first skill of each phase = false, rest = true (set by brain.rs at generation).
-- Existing trees: all false → all nodes remain accessible. No regression.

ALTER TABLE tree_nodes ADD COLUMN IF NOT EXISTS is_locked BOOLEAN NOT NULL DEFAULT false;
