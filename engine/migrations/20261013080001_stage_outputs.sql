-- stage_outputs: one row per completed pipeline stage output, keyed (review_run_id, stage,
-- input_hash) so a retried run resumes instead of recomputing (target-architecture section 4.1).
-- CTX-009 stores context packages here (stage 'context'; the cluster and reviewer are part of the
-- input hash). repository_id supports reuse by a superseding run of the same repository.
--
-- Written to be idempotent so that it composes with a PIPE-005 migration that defines the same
-- table: every statement tolerates the table, column, index or policy already existing.

CREATE TABLE IF NOT EXISTS stage_outputs (
  review_run_id uuid NOT NULL,
  organization_id uuid NOT NULL,
  repository_id uuid,
  stage text NOT NULL CHECK (stage ~ '^[a-z_]{1,40}$'),
  input_hash text NOT NULL CHECK (input_hash ~ '^[0-9a-f]{64}$'),
  output_hash text NOT NULL CHECK (output_hash ~ '^[0-9a-f]{64}$'),
  output jsonb,
  blob_key text,
  status text NOT NULL DEFAULT 'succeeded' CHECK (status IN ('succeeded', 'failed_permanent')),
  schema_version int NOT NULL DEFAULT 1,
  duration_ms int NOT NULL DEFAULT 0,
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (review_run_id, stage, input_hash),
  FOREIGN KEY (review_run_id, organization_id)
    REFERENCES review_runs (id, organization_id) ON DELETE CASCADE,
  CHECK ((output IS NOT NULL) <> (blob_key IS NOT NULL))
);

ALTER TABLE stage_outputs ADD COLUMN IF NOT EXISTS repository_id uuid;

CREATE INDEX IF NOT EXISTS stage_outputs_repo_stage_input
  ON stage_outputs (repository_id, stage, input_hash);

ALTER TABLE stage_outputs ENABLE ROW LEVEL SECURITY;
ALTER TABLE stage_outputs FORCE ROW LEVEL SECURITY;

DO $$
BEGIN
  IF NOT EXISTS (
    SELECT 1 FROM pg_policies WHERE tablename = 'stage_outputs' AND policyname = 'tenant_isolation'
  ) THEN
    CREATE POLICY tenant_isolation ON stage_outputs
      USING (organization_id = NULLIF(current_setting('app.organization_id', true), '')::uuid)
      WITH CHECK (organization_id = NULLIF(current_setting('app.organization_id', true), '')::uuid);
  END IF;
END
$$;
