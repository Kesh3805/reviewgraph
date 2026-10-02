-- Findings: candidate -> verified -> published, plus feedback (DOM-009).
-- The CHECK values mirror review_core::finding (FindingState, Severity, FindingCategory,
-- ReviewerType, PublicationBand, Placement). Every table has organization_id, a composite FK to
-- its parent, UNIQUE (id, organization_id) and an updated_at trigger.

CREATE TABLE candidate_findings (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  organization_id uuid NOT NULL,
  review_run_id uuid NOT NULL,
  reviewer_run_id uuid NOT NULL,
  reviewer text NOT NULL CHECK (reviewer IN (
    'correctness', 'security', 'test', 'architecture', 'performance', 'maintainability')),
  category text NOT NULL CHECK (category IN (
    'correctness', 'security', 'performance', 'testing', 'architecture', 'maintainability',
    'data_integrity', 'concurrency', 'api_contract', 'error_handling')),
  title text NOT NULL CHECK (length(title) BETWEEN 1 AND 200),
  description text NOT NULL CHECK (length(description) <= 8000),
  changed_path text NOT NULL,
  changed_side text NOT NULL CHECK (changed_side IN ('head', 'base')),
  changed_start_line integer NOT NULL,
  changed_end_line integer NOT NULL,
  severity_candidate text NOT NULL CHECK (severity_candidate IN (
    'info', 'low', 'medium', 'high', 'critical')),
  -- The model's self-report: informational only, never a publication input (PRD §54).
  confidence_candidate real CHECK (confidence_candidate BETWEEN 0 AND 1),
  affected_symbols text[] NOT NULL DEFAULT '{}',
  evidence jsonb NOT NULL DEFAULT '[]'::jsonb,
  reasoning_artifacts jsonb NOT NULL DEFAULT '[]'::jsonb,
  fingerprint text NOT NULL CHECK (fingerprint ~ '^v1:[0-9a-f]{32}$'),
  state text NOT NULL CHECK (state IN (
    'GENERATED', 'EVIDENCE_COLLECTED', 'VERIFIED', 'DEDUPLICATED', 'PRIORITIZED', 'PUBLISHED',
    'SUPPRESSED_LOW_CONFIDENCE', 'SUPPRESSED_DUPLICATE', 'SUPPRESSED_PREEXISTING',
    'SUPPRESSED_NOT_ACTIONABLE', 'SUPPRESSED_POLICY', 'INVALIDATED')),
  suppression jsonb,
  suppressed_at_stage smallint CHECK (suppressed_at_stage BETWEEN 1 AND 8),
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  FOREIGN KEY (review_run_id, organization_id)
    REFERENCES review_runs (id, organization_id) ON DELETE CASCADE,
  FOREIGN KEY (reviewer_run_id, organization_id)
    REFERENCES reviewer_runs (id, organization_id) ON DELETE CASCADE,
  UNIQUE (id, organization_id),
  CHECK (changed_start_line >= 1 AND changed_end_line >= changed_start_line),
  CHECK ((state LIKE 'SUPPRESSED\_%' OR state = 'INVALIDATED') = (suppression IS NOT NULL)),
  -- Makes re-inserting a retried reviewer's candidates ON CONFLICT DO NOTHING.
  UNIQUE (reviewer_run_id, fingerprint)
);
CREATE INDEX candidate_findings_run_state_idx ON candidate_findings (review_run_id, state);

CREATE TABLE verified_findings (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  organization_id uuid NOT NULL,
  candidate_finding_id uuid NOT NULL UNIQUE,
  review_run_id uuid NOT NULL,
  computed_confidence real NOT NULL CHECK (computed_confidence BETWEEN 0 AND 1),
  severity text NOT NULL CHECK (severity IN ('info', 'low', 'medium', 'high', 'critical')),
  band text NOT NULL CHECK (band IN (
    'suppress', 'internal', 'publish_if_medium_or_above', 'publish')),
  verification_version integer NOT NULL,
  stage_outcomes jsonb NOT NULL,
  evidence jsonb NOT NULL,
  priority_score real,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  FOREIGN KEY (candidate_finding_id, organization_id)
    REFERENCES candidate_findings (id, organization_id) ON DELETE CASCADE,
  FOREIGN KEY (review_run_id, organization_id)
    REFERENCES review_runs (id, organization_id) ON DELETE CASCADE,
  UNIQUE (id, organization_id)
);
CREATE INDEX verified_findings_run_idx ON verified_findings (review_run_id);

CREATE TABLE published_findings (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  organization_id uuid NOT NULL,
  verified_finding_id uuid NOT NULL UNIQUE,
  review_run_id uuid NOT NULL,
  pull_request_id uuid NOT NULL,
  provider text NOT NULL CHECK (provider IN ('github')),
  placement text NOT NULL CHECK (placement IN ('inline', 'summary')),
  path text,
  start_line integer,
  end_line integer,
  head_sha text NOT NULL CHECK (head_sha ~ '^([0-9a-f]{40}|[0-9a-f]{64})$'),
  provider_review_id text,
  provider_comment_id text,
  published_at timestamptz NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  FOREIGN KEY (verified_finding_id, organization_id)
    REFERENCES verified_findings (id, organization_id) ON DELETE CASCADE,
  FOREIGN KEY (review_run_id, organization_id)
    REFERENCES review_runs (id, organization_id) ON DELETE CASCADE,
  FOREIGN KEY (pull_request_id, organization_id)
    REFERENCES pull_requests (id, organization_id) ON DELETE CASCADE,
  UNIQUE (id, organization_id),
  CHECK ((placement = 'inline') = (path IS NOT NULL AND start_line IS NOT NULL)),
  CHECK (start_line IS NULL OR end_line IS NULL OR (start_line >= 1 AND end_line >= start_line)),
  -- NULL comment ids (summary placement) never conflict.
  UNIQUE (provider, provider_comment_id)
);
CREATE INDEX published_findings_pr_idx ON published_findings (pull_request_id);

CREATE TABLE finding_feedback (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  organization_id uuid NOT NULL,
  published_finding_id uuid NOT NULL,
  user_id uuid REFERENCES users (id) ON DELETE SET NULL,
  source text NOT NULL CHECK (source IN ('web', 'provider_reaction', 'provider_reply', 'cli')),
  verdict text NOT NULL CHECK (verdict IN (
    'useful', 'false_positive', 'already_handled', 'not_relevant', 'intentional')),
  comment text CHECK (length(comment) <= 2000),
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  FOREIGN KEY (published_finding_id, organization_id)
    REFERENCES published_findings (id, organization_id) ON DELETE CASCADE,
  UNIQUE (id, organization_id),
  -- A user's verdict on a finding is upserted.
  UNIQUE (published_finding_id, user_id)
);

CREATE TRIGGER candidate_findings_set_updated_at BEFORE UPDATE ON candidate_findings
  FOR EACH ROW EXECUTE FUNCTION rg_set_updated_at();
CREATE TRIGGER verified_findings_set_updated_at BEFORE UPDATE ON verified_findings
  FOR EACH ROW EXECUTE FUNCTION rg_set_updated_at();
CREATE TRIGGER published_findings_set_updated_at BEFORE UPDATE ON published_findings
  FOR EACH ROW EXECUTE FUNCTION rg_set_updated_at();
CREATE TRIGGER finding_feedback_set_updated_at BEFORE UPDATE ON finding_feedback
  FOR EACH ROW EXECUTE FUNCTION rg_set_updated_at();
