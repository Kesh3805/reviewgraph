-- PIPE-006: per-run budget accounting snapshot.
ALTER TABLE review_runs ADD COLUMN IF NOT EXISTS budget_usage jsonb NOT NULL DEFAULT '{}';
