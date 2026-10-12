-- Finding feedback (API-012, PRD section 70): explicit verdicts from the web UI and weak signals
-- from the provider (reactions and replies on our comments). It feeds the acceptance and
-- false-positive KPIs and calibration. A user's repeated feedback on a finding is an upsert on
-- (finding_id, user_id, source): the latest verdict wins and the history is in audit_log.
-- `finding_id` is the verified finding (the id the findings API exposes).

CREATE TABLE feedback (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  organization_id uuid NOT NULL,
  repository_id uuid NOT NULL,
  finding_id uuid NOT NULL,
  user_id uuid REFERENCES users (id) ON DELETE SET NULL,
  source text NOT NULL CHECK (source IN ('web', 'provider')),
  verdict text NOT NULL CHECK (verdict IN (
    'useful', 'false_positive', 'already_handled', 'not_relevant', 'intentional')),
  -- Plain text, HTML-escaped by the UI.
  comment text CHECK (length(comment) <= 2000),
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  FOREIGN KEY (finding_id, organization_id)
    REFERENCES verified_findings (id, organization_id) ON DELETE CASCADE,
  FOREIGN KEY (repository_id, organization_id)
    REFERENCES repositories (id, organization_id) ON DELETE CASCADE,
  UNIQUE (id, organization_id),
  UNIQUE (finding_id, user_id, source)
);
CREATE INDEX feedback_repo_created_idx ON feedback (organization_id, repository_id, created_at);

CREATE TRIGGER feedback_set_updated_at BEFORE UPDATE ON feedback
  FOR EACH ROW EXECUTE FUNCTION rg_set_updated_at();

ALTER TABLE feedback ENABLE ROW LEVEL SECURITY;
ALTER TABLE feedback FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON feedback
  USING (organization_id = NULLIF(current_setting('app.organization_id', true), '')::uuid)
  WITH CHECK (organization_id = NULLIF(current_setting('app.organization_id', true), '')::uuid);
