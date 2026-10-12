-- HIST-004: anti-reinforcement audit trail (PRD section 69). Every history-derived signal gets an
-- immutable, versioned admission decision with reasons; the latest decision wins. Counts and ids
-- only. `signal_id` is the signal's stable key (a signal row is replaced on every refresh), so
-- it is text, not a foreign key.

CREATE TABLE signal_admissions (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  organization_id uuid NOT NULL,
  repository_id uuid NOT NULL,
  signal_id text NOT NULL,
  input_hash text NOT NULL,
  version integer NOT NULL CHECK (version >= 1),
  decision text NOT NULL CHECK (decision IN ('admitted', 'rejected', 'demoted')),
  reasons jsonb NOT NULL,
  evidence_summary jsonb NOT NULL DEFAULT '{}'::jsonb,
  decided_at timestamptz NOT NULL,
  expires_at timestamptz,
  FOREIGN KEY (repository_id, organization_id)
    REFERENCES repositories (id, organization_id) ON DELETE CASCADE,
  UNIQUE (signal_id, version)
);
-- Re-evaluating the same inputs does not add a version: the writer skips an input_hash equal to
-- the latest version's, under a per-signal advisory lock.
CREATE INDEX signal_admissions_hash_idx ON signal_admissions (signal_id, input_hash);
CREATE INDEX signal_admissions_signal_idx ON signal_admissions (repository_id, signal_id, version DESC);

CREATE FUNCTION signal_admissions_immutable() RETURNS trigger
  LANGUAGE plpgsql AS $$
BEGIN
  RAISE EXCEPTION 'signal_admissions is immutable' USING ERRCODE = 'integrity_constraint_violation';
END
$$;
CREATE TRIGGER signal_admissions_no_update BEFORE UPDATE ON signal_admissions
  FOR EACH ROW EXECUTE FUNCTION signal_admissions_immutable();

ALTER TABLE signal_admissions ENABLE ROW LEVEL SECURITY;
ALTER TABLE signal_admissions FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON signal_admissions
  USING (organization_id = NULLIF(current_setting('app.organization_id', true), '')::uuid)
  WITH CHECK (organization_id = NULLIF(current_setting('app.organization_id', true), '')::uuid);
