-- PIPE-005: stage outputs keyed (review_run_id, stage, input_hash) for idempotent resume.
--
-- A stage's output is immutable once written; a retried job resumes from the stored outputs
-- instead of recomputing. Outputs above 256 KiB are spilled to the blob store (`blob_key`).
-- `output` can hold code excerpts, so it is covered by the retention policy and never logged.

CREATE TABLE stage_outputs (
  review_run_id uuid NOT NULL,
  organization_id uuid NOT NULL,
  stage text NOT NULL CHECK (stage ~ '^[a-z_:0-9-]{1,80}$'),
  input_hash text NOT NULL CHECK (input_hash ~ '^[0-9a-f]{64}$'),
  output_hash text NOT NULL CHECK (output_hash ~ '^[0-9a-f]{64}$'),
  output jsonb,
  blob_key text,
  status text NOT NULL CHECK (status IN ('succeeded', 'failed_permanent')),
  schema_version integer NOT NULL,
  duration_ms integer NOT NULL CHECK (duration_ms >= 0),
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (review_run_id, stage, input_hash),
  FOREIGN KEY (review_run_id, organization_id)
    REFERENCES review_runs (id, organization_id) ON DELETE CASCADE,
  CHECK ((output IS NOT NULL) <> (blob_key IS NOT NULL))
);

CREATE INDEX stage_outputs_org_idx ON stage_outputs (organization_id, review_run_id);

ALTER TABLE stage_outputs ENABLE ROW LEVEL SECURITY;
ALTER TABLE stage_outputs FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON stage_outputs
  USING (organization_id = NULLIF(current_setting('app.organization_id', true), '')::uuid)
  WITH CHECK (organization_id = NULLIF(current_setting('app.organization_id', true), '')::uuid);
