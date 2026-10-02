-- Review runs and reviewer runs (DOM-009). The state and enum CHECK values mirror
-- review_core::review::ReviewState, ReviewTrigger, ReviewerRunState, ReviewerType and ErrorClass.

CREATE TABLE review_runs (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  organization_id uuid NOT NULL,
  repository_id uuid NOT NULL,
  pull_request_id uuid NOT NULL,
  base_sha text NOT NULL CHECK (base_sha ~ '^([0-9a-f]{40}|[0-9a-f]{64})$'),
  head_sha text NOT NULL CHECK (head_sha ~ '^([0-9a-f]{40}|[0-9a-f]{64})$'),
  merge_base_sha text CHECK (merge_base_sha ~ '^([0-9a-f]{40}|[0-9a-f]{64})$'),
  state text NOT NULL CHECK (state IN (
    'RECEIVED', 'INDEXING', 'ANALYZING', 'REVIEWING', 'VERIFYING', 'PUBLISHING', 'COMPLETED',
    'FAILED_INDEXING', 'FAILED_ANALYSIS', 'FAILED_REVIEW', 'FAILED_PUBLISH',
    'SUPERSEDED', 'CANCELLED')),
  trigger text NOT NULL CHECK (trigger IN ('webhook', 'manual', 'reconciler', 'cli')),
  superseded_by uuid,
  retry_of uuid,
  failure_class text CHECK (failure_class IN (
    'invalid_input', 'not_found', 'conflict', 'transient', 'rate_limited', 'permanent',
    'cancelled', 'internal')),
  failure_detail text CHECK (length(failure_detail) <= 2000),
  degraded_reviewers text[] NOT NULL DEFAULT '{}',
  provenance jsonb NOT NULL DEFAULT '{}'::jsonb,
  trace_parent text
    CHECK (trace_parent ~ '^[0-9a-f]{2}-[0-9a-f]{32}-[0-9a-f]{16}-[0-9a-f]{2}$'),
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  completed_at timestamptz,
  FOREIGN KEY (pull_request_id, organization_id)
    REFERENCES pull_requests (id, organization_id) ON DELETE CASCADE,
  FOREIGN KEY (repository_id, organization_id)
    REFERENCES repositories (id, organization_id) ON DELETE CASCADE,
  UNIQUE (id, organization_id),
  -- Self references are tenant-scoped too. SET NULL clears only the run id (PG 15+).
  FOREIGN KEY (superseded_by, organization_id)
    REFERENCES review_runs (id, organization_id) ON DELETE SET NULL (superseded_by),
  FOREIGN KEY (retry_of, organization_id)
    REFERENCES review_runs (id, organization_id) ON DELETE SET NULL (retry_of),
  CHECK ((state LIKE 'FAILED\_%') = (failure_class IS NOT NULL)),
  CHECK ((state = 'SUPERSEDED') = (superseded_by IS NOT NULL))
);

-- At most one active run per pull request. A new head's run can only be inserted in the same
-- transaction that moves the old active run to SUPERSEDED (SUP-001); a concurrent loser gets a
-- unique violation, which maps to ErrorClass::Conflict.
CREATE UNIQUE INDEX review_runs_one_active_per_pr ON review_runs (pull_request_id)
  WHERE state IN ('RECEIVED', 'INDEXING', 'ANALYZING', 'REVIEWING', 'VERIFYING', 'PUBLISHING');
-- A replayed event for the same head cannot create a second first run.
CREATE UNIQUE INDEX review_runs_first_run_per_head ON review_runs (pull_request_id, head_sha)
  WHERE retry_of IS NULL;
CREATE INDEX review_runs_org_repo_created_idx
  ON review_runs (organization_id, repository_id, created_at DESC);

CREATE TABLE reviewer_runs (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  organization_id uuid NOT NULL,
  review_run_id uuid NOT NULL,
  reviewer text NOT NULL CHECK (reviewer IN (
    'correctness', 'security', 'test', 'architecture', 'performance', 'maintainability')),
  cluster_key text CHECK (cluster_key ~ '^[0-9a-f]{32}$'),
  state text NOT NULL CHECK (state IN (
    'pending', 'running', 'succeeded', 'failed', 'skipped', 'timed_out')),
  provider text,
  model text,
  prompt_version text,
  reviewer_version text,
  input_tokens bigint NOT NULL DEFAULT 0,
  output_tokens bigint NOT NULL DEFAULT 0,
  cached_read_tokens bigint NOT NULL DEFAULT 0,
  cached_write_tokens bigint NOT NULL DEFAULT 0,
  cost_usd_micros bigint NOT NULL DEFAULT 0,
  latency_ms integer,
  error_class text CHECK (error_class IN (
    'invalid_input', 'not_found', 'conflict', 'transient', 'rate_limited', 'permanent',
    'cancelled', 'internal')),
  started_at timestamptz,
  finished_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  FOREIGN KEY (review_run_id, organization_id)
    REFERENCES review_runs (id, organization_id) ON DELETE CASCADE,
  UNIQUE NULLS NOT DISTINCT (review_run_id, reviewer, cluster_key),
  UNIQUE (id, organization_id),
  CHECK ((state IN ('failed', 'timed_out')) = (error_class IS NOT NULL))
);

CREATE TRIGGER review_runs_set_updated_at BEFORE UPDATE ON review_runs
  FOR EACH ROW EXECUTE FUNCTION rg_set_updated_at();
CREATE TRIGGER reviewer_runs_set_updated_at BEFORE UPDATE ON reviewer_runs
  FOR EACH ROW EXECUTE FUNCTION rg_set_updated_at();
