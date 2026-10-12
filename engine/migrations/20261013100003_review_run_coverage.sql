-- PIPE-008: the computed coverage report of a run (complete or degraded, missing reviewers,
-- unreviewed regions). Derived only from persisted rows, never from model output.
-- `degraded_reviewers` already exists (20261002000003).

ALTER TABLE review_runs ADD COLUMN IF NOT EXISTS coverage jsonb NOT NULL DEFAULT '{}';
