-- Explicit-rule violations of a review run (POL-005).
--
-- Written in the ANALYZING stage. The unique key (NULLS NOT DISTINCT, so a violation without a
-- symbol is still unique) makes a retried stage a no-op. candidate_finding_id links the
-- deterministic candidate created from the violation once it exists.

CREATE TABLE rule_violations (
  id                   uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  organization_id      uuid NOT NULL,
  repository_id        uuid NOT NULL,
  review_run_id        uuid NOT NULL,
  rule_id              text NOT NULL,
  rule_key             text NOT NULL,
  source               text NOT NULL CHECK (source IN ('explicit', 'documented', 'convention')),
  path                 text,
  start_line           integer CHECK (start_line >= 1),
  end_line             integer CHECK (end_line >= start_line),
  symbol_key           text,
  message              text NOT NULL CHECK (length(message) <= 8000),
  evidence             jsonb NOT NULL,
  severity             text NOT NULL CHECK (severity IN ('info', 'low', 'medium', 'high', 'critical')),
  candidate_finding_id uuid NULL,
  created_at           timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT rule_violations_unique
    UNIQUE NULLS NOT DISTINCT (review_run_id, rule_id, path, start_line, symbol_key),
  CONSTRAINT rule_violations_run_fk
    FOREIGN KEY (review_run_id, organization_id)
    REFERENCES review_runs (id, organization_id) ON DELETE CASCADE,
  CONSTRAINT rule_violations_repository_fk
    FOREIGN KEY (repository_id, organization_id)
    REFERENCES repositories (id, organization_id) ON DELETE CASCADE
);
CREATE INDEX rule_violations_run_idx ON rule_violations (review_run_id);

ALTER TABLE rule_violations ENABLE ROW LEVEL SECURITY;
ALTER TABLE rule_violations FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON rule_violations
  USING (organization_id = NULLIF(current_setting('app.organization_id', true), '')::uuid)
  WITH CHECK (organization_id = NULLIF(current_setting('app.organization_id', true), '')::uuid);
