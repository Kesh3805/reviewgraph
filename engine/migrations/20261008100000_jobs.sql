-- The shared PostgreSQL job queue (PIPE-001, ADR-012). Producers and consumers in both languages
-- (NestJS API-007, Rust PIPE-001/002) use this table and the claim statement of
-- target-architecture section 5. Every enqueue also issues `pg_notify('jobs_' || queue, id)` in
-- the same transaction, so a wake-up is delivered only once the job is committed.
--
-- Not tenant scoped by RLS (see the RLS migration): claims span tenants, and payloads carry ids
-- only. organization_id is still recorded so a handler can set the tenant for its own work.

CREATE TABLE jobs (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  organization_id uuid NOT NULL REFERENCES organizations (id) ON DELETE CASCADE,
  queue text NOT NULL CHECK (queue IN (
    'repository-index', 'incremental-index', 'pr-review', 'review-publish', 'history-ingest')),
  idempotency_key text NOT NULL UNIQUE,
  payload jsonb NOT NULL
    CHECK (jsonb_typeof(payload) = 'object' AND pg_column_size(payload) <= 16384),
  state text NOT NULL DEFAULT 'queued' CHECK (state IN (
    'queued', 'running', 'succeeded', 'failed', 'dead', 'cancelled')),
  priority integer NOT NULL DEFAULT 0,
  attempts integer NOT NULL DEFAULT 0,
  max_attempts integer NOT NULL DEFAULT 5 CHECK (max_attempts BETWEEN 1 AND 20),
  rate_limit_requeues integer NOT NULL DEFAULT 0,
  run_after timestamptz NOT NULL DEFAULT now(),
  locked_by text,
  locked_until timestamptz,
  last_error text CHECK (length(last_error) <= 2000),
  trace_parent text,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  CHECK ((state = 'running') = (locked_by IS NOT NULL))
);

CREATE INDEX jobs_claim_idx ON jobs (queue, priority DESC, created_at) WHERE state = 'queued';
CREATE INDEX jobs_lease_idx ON jobs (locked_until) WHERE state = 'running';
CREATE INDEX jobs_review_run_idx ON jobs ((payload ->> 'review_run_id'))
  WHERE state IN ('queued', 'running');

CREATE TRIGGER jobs_set_updated_at BEFORE UPDATE ON jobs
  FOR EACH ROW EXECUTE FUNCTION rg_set_updated_at();
