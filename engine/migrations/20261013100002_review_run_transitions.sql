-- PIPE-007: audit trail of review-run state transitions. A row is inserted in the same
-- transaction as the compare-and-set that applied the transition, and only when it applied.

CREATE TABLE review_run_transitions (
  id bigserial PRIMARY KEY,
  review_run_id uuid NOT NULL,
  organization_id uuid NOT NULL,
  from_state text NOT NULL CHECK (from_state IN (
    'RECEIVED', 'INDEXING', 'ANALYZING', 'REVIEWING', 'VERIFYING', 'PUBLISHING', 'COMPLETED',
    'FAILED_INDEXING', 'FAILED_ANALYSIS', 'FAILED_REVIEW', 'FAILED_PUBLISH',
    'SUPERSEDED', 'CANCELLED')),
  to_state text NOT NULL CHECK (to_state IN (
    'RECEIVED', 'INDEXING', 'ANALYZING', 'REVIEWING', 'VERIFYING', 'PUBLISHING', 'COMPLETED',
    'FAILED_INDEXING', 'FAILED_ANALYSIS', 'FAILED_REVIEW', 'FAILED_PUBLISH',
    'SUPERSEDED', 'CANCELLED')),
  job_id uuid,
  detail jsonb,
  at timestamptz NOT NULL DEFAULT now(),
  FOREIGN KEY (review_run_id, organization_id)
    REFERENCES review_runs (id, organization_id) ON DELETE CASCADE
);

CREATE INDEX review_run_transitions_run_idx ON review_run_transitions (review_run_id, id);

ALTER TABLE review_run_transitions ENABLE ROW LEVEL SECURITY;
ALTER TABLE review_run_transitions FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON review_run_transitions
  USING (organization_id = NULLIF(current_setting('app.organization_id', true), '')::uuid)
  WITH CHECK (organization_id = NULLIF(current_setting('app.organization_id', true), '')::uuid);
