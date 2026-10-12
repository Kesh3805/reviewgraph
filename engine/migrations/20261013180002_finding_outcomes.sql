-- HIST-002: the outcome of every published finding (PRD section 67). Events are append-only; the
-- current outcome is derived by source precedence (explicit feedback and suppression > code
-- change > thread state > merge with no action), so a concurrent writer cannot lose an explicit
-- verdict. `finding_id` is the published finding.

CREATE TABLE finding_outcome_events (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  organization_id uuid NOT NULL,
  finding_id uuid NOT NULL,
  outcome text NOT NULL CHECK (outcome IN (
    'accepted', 'rejected', 'dismissed', 'resolved_unattributed', 'ignored', 'pending')),
  source text NOT NULL CHECK (source IN (
    'explicit_feedback', 'suppression', 'code_change', 'thread_state', 'merge_no_action')),
  -- Ids and hashes only (commit sha, thread id): never comment bodies.
  evidence jsonb NOT NULL DEFAULT '{}'::jsonb,
  evidence_hash text NOT NULL,
  inferred boolean NOT NULL,
  confidence_of_inference real NOT NULL CHECK (confidence_of_inference BETWEEN 0 AND 1),
  created_at timestamptz NOT NULL DEFAULT now(),
  FOREIGN KEY (finding_id, organization_id)
    REFERENCES published_findings (id, organization_id) ON DELETE CASCADE,
  -- Replaying the same webhook adds nothing.
  UNIQUE (finding_id, source, evidence_hash),
  -- Explicit events are certain, inferred ones are not.
  CHECK ((inferred AND confidence_of_inference < 1) OR (NOT inferred AND confidence_of_inference = 1))
);
CREATE INDEX finding_outcome_events_finding_idx ON finding_outcome_events (finding_id, created_at);

CREATE FUNCTION finding_outcome_events_append_only() RETURNS trigger
  LANGUAGE plpgsql AS $$
BEGIN
  RAISE EXCEPTION 'finding_outcome_events is append-only' USING ERRCODE = 'integrity_constraint_violation';
END
$$;
CREATE TRIGGER finding_outcome_events_no_update BEFORE UPDATE ON finding_outcome_events
  FOR EACH ROW EXECUTE FUNCTION finding_outcome_events_append_only();

ALTER TABLE finding_outcome_events ENABLE ROW LEVEL SECURITY;
ALTER TABLE finding_outcome_events FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON finding_outcome_events
  USING (organization_id = NULLIF(current_setting('app.organization_id', true), '')::uuid)
  WITH CHECK (organization_id = NULLIF(current_setting('app.organization_id', true), '')::uuid);

-- The current state: highest-precedence source, then the latest event. Runs with the caller's
-- privileges, so the tenant policy above applies to readers of the view.
CREATE VIEW finding_outcomes_current WITH (security_invoker = true) AS
SELECT DISTINCT ON (finding_id)
  finding_id,
  organization_id,
  outcome,
  source,
  evidence,
  inferred,
  confidence_of_inference,
  created_at AS determined_at
FROM finding_outcome_events
ORDER BY
  finding_id,
  CASE source
    WHEN 'explicit_feedback' THEN 4
    WHEN 'suppression' THEN 4
    WHEN 'code_change' THEN 3
    WHEN 'thread_state' THEN 2
    ELSE 1
  END DESC,
  created_at DESC,
  id DESC;
