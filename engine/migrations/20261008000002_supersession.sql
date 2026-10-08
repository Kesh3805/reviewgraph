-- Supersession on a new head (SUP-001).
--
-- `startReview` runs one transaction: lock the pull request, move the head, supersede every
-- non-terminal run of another head, cancel their queued jobs and insert the new run. The new
-- run's id is chosen up front so the superseded rows can point at it before it is inserted (the
-- one-active-run-per-PR index forbids inserting it first); the self reference is therefore
-- checked at commit.
--
-- The jobs expression index on payload->>'review_run_id' ships with the jobs table (PIPE-001).

ALTER TABLE review_runs
  ADD COLUMN superseded_at timestamptz,
  ADD COLUMN superseded_by_head text
    CHECK (superseded_by_head ~ '^([0-9a-f]{40}|[0-9a-f]{64})$'),
  -- `pr-review:{provider}:{provider_repo_id}:{pr}:{head_sha}[:{suffix}]`, equal to the job key.
  ADD COLUMN idempotency_key text,
  ADD COLUMN depth text NOT NULL DEFAULT 'standard' CHECK (depth IN ('standard', 'full'));

ALTER TABLE review_runs
  ADD CONSTRAINT review_runs_idempotency_key_key UNIQUE (idempotency_key);

DO $$
DECLARE
  c text;
BEGIN
  SELECT conname INTO c FROM pg_constraint
  WHERE conrelid = 'review_runs'::regclass AND contype = 'f'
    AND pg_get_constraintdef(oid) LIKE 'FOREIGN KEY (superseded_by, organization_id)%';
  IF c IS NULL THEN
    RAISE EXCEPTION 'review_runs superseded_by foreign key not found';
  END IF;
  EXECUTE format('ALTER TABLE review_runs DROP CONSTRAINT %I', c);
END
$$;

ALTER TABLE review_runs
  ADD CONSTRAINT review_runs_superseded_by_fkey
    FOREIGN KEY (superseded_by, organization_id)
    REFERENCES review_runs (id, organization_id) ON DELETE SET NULL (superseded_by)
    DEFERRABLE INITIALLY DEFERRED;

CREATE INDEX review_runs_pr_state_idx ON review_runs (pull_request_id, state);
