-- Finding lifecycle persistence (VER-001).
--
-- candidate_findings / verified_findings exist (DOM-009, 20261002000004). This adds the CAS
-- version column, the REJECTED_INVALID state (PRD §150 extension), and the verification,
-- evidence and state-transition audit tables. Additive only.

ALTER TABLE candidate_findings
  ADD COLUMN state_version integer NOT NULL DEFAULT 0 CHECK (state_version >= 0),
  ADD COLUMN reason_code text,
  ADD COLUMN raw_output_hash text,
  ADD COLUMN idempotency_key text UNIQUE;

ALTER TABLE candidate_findings DROP CONSTRAINT candidate_findings_state_check;
ALTER TABLE candidate_findings ADD CONSTRAINT candidate_findings_state_check CHECK (state IN (
  'GENERATED', 'EVIDENCE_COLLECTED', 'VERIFIED', 'DEDUPLICATED', 'PRIORITIZED', 'PUBLISHED',
  'SUPPRESSED_LOW_CONFIDENCE', 'SUPPRESSED_DUPLICATE', 'SUPPRESSED_PREEXISTING',
  'SUPPRESSED_NOT_ACTIONABLE', 'SUPPRESSED_POLICY', 'REJECTED_INVALID', 'INVALIDATED'));

-- A rejected candidate carries a reason_code, not a suppression object. The original CHECK is
-- unnamed, so find it by definition.
DO $$
DECLARE
  c text;
BEGIN
  FOR c IN
    SELECT conname FROM pg_constraint
    WHERE conrelid = 'candidate_findings'::regclass AND contype = 'c'
      AND pg_get_constraintdef(oid) LIKE '%suppression IS NOT NULL%'
  LOOP
    EXECUTE format('ALTER TABLE candidate_findings DROP CONSTRAINT %I', c);
  END LOOP;
END
$$;
ALTER TABLE candidate_findings ADD CONSTRAINT candidate_findings_suppression_check CHECK (
  state = 'REJECTED_INVALID'
  OR (state LIKE 'SUPPRESSED\_%' OR state = 'INVALIDATED') = (suppression IS NOT NULL));

CREATE TABLE finding_verifications (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  organization_id uuid NOT NULL,
  candidate_finding_id uuid NOT NULL,
  verification_version text NOT NULL,
  base_snapshot_id uuid NOT NULL,
  head_snapshot_id uuid NOT NULL,
  depth text NOT NULL,
  stage_outcomes jsonb NOT NULL,
  signals jsonb NOT NULL,
  confidence numeric(4,3) CHECK (confidence BETWEEN 0 AND 1),
  eligibility text CHECK (eligibility IN ('publish', 'internal', 'suppress')),
  placement text CHECK (placement IN ('inline', 'summary')),
  cache_key text NOT NULL,
  from_cache boolean NOT NULL DEFAULT false,
  duration_ms integer NOT NULL CHECK (duration_ms >= 0),
  created_at timestamptz NOT NULL DEFAULT now(),
  FOREIGN KEY (candidate_finding_id, organization_id)
    REFERENCES candidate_findings (id, organization_id) ON DELETE CASCADE,
  UNIQUE (id, organization_id),
  -- PRD §76 key verification:{candidateFindingId}:{verificationVersion}.
  UNIQUE (candidate_finding_id, verification_version)
);

CREATE TABLE finding_evidence (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  organization_id uuid NOT NULL,
  candidate_finding_id uuid NOT NULL,
  verification_id uuid,
  kind text NOT NULL,
  source text NOT NULL CHECK (source IN (
    'reviewer_cited', 'graph', 'repository', 'deterministic_tool', 'base_head', 'contradiction',
    'verifier_model')),
  strength text NOT NULL CHECK (strength IN (
    'strong', 'supporting', 'weak', 'counter', 'unverified')),
  polarity text NOT NULL CHECK (polarity IN ('supports', 'refutes')),
  path text,
  side text CHECK (side IN ('head', 'base')),
  start_line integer,
  end_line integer,
  symbol_keys text[] NOT NULL DEFAULT '{}',
  payload jsonb NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  FOREIGN KEY (candidate_finding_id, organization_id)
    REFERENCES candidate_findings (id, organization_id) ON DELETE CASCADE,
  FOREIGN KEY (verification_id, organization_id)
    REFERENCES finding_verifications (id, organization_id) ON DELETE CASCADE
);
CREATE INDEX finding_evidence_candidate_idx ON finding_evidence (candidate_finding_id);

CREATE TABLE finding_state_transitions (
  id bigserial PRIMARY KEY,
  organization_id uuid NOT NULL,
  candidate_finding_id uuid NOT NULL,
  from_state text NOT NULL,
  to_state text NOT NULL,
  reason_code text,
  stage text NOT NULL,
  detail jsonb,
  at timestamptz NOT NULL DEFAULT now(),
  FOREIGN KEY (candidate_finding_id, organization_id)
    REFERENCES candidate_findings (id, organization_id) ON DELETE CASCADE
);
CREATE INDEX finding_state_transitions_candidate_idx
  ON finding_state_transitions (candidate_finding_id, at);

DO $$
DECLARE
  t text;
BEGIN
  FOREACH t IN ARRAY ARRAY['finding_verifications', 'finding_evidence', 'finding_state_transitions'] LOOP
    EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
    EXECUTE format('ALTER TABLE %I FORCE ROW LEVEL SECURITY', t);
    EXECUTE format(
      'CREATE POLICY tenant_isolation ON %I
         USING (organization_id = NULLIF(current_setting(''app.organization_id'', true), '''')::uuid)
         WITH CHECK (organization_id = NULLIF(current_setting(''app.organization_id'', true), '''')::uuid)',
      t);
  END LOOP;
END
$$;
