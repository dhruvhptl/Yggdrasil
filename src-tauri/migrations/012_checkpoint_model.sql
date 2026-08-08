-- 012_checkpoint_model.sql
-- Migrate leaf node tasks from quest array format to checkpoint object format.
-- Before: [{ id, title, description, estimated_hours, difficulty, completed, notes }]
-- After:  { mastery_criteria, exercises, notes, completed }

UPDATE tree_nodes
SET tasks = jsonb_build_object(
  'mastery_criteria', COALESCE(tasks->0->>'description', ''),
  'exercises', '[]'::jsonb,
  'notes', COALESCE(tasks->0->>'notes', ''),
  'completed', COALESCE((tasks->0->>'completed')::boolean, false)
)
WHERE type = 'leaf'
  AND tasks IS NOT NULL
  AND jsonb_typeof(tasks) = 'array';
