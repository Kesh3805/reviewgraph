-- IDX-002: progress and outcome of index runs, one row per snapshot being indexed.
CREATE TABLE index_runs (
  id              uuid PRIMARY KEY,
  organization_id uuid NOT NULL REFERENCES organizations (id),
  repository_id   uuid NOT NULL,
  snapshot_id     uuid NOT NULL REFERENCES snapshots (id) ON DELETE CASCADE,
  job_id          uuid,
  kind            text NOT NULL CHECK (kind IN ('full', 'incremental', 'compaction')),
  phase           text NOT NULL,
  files_total     integer,
  files_done      integer NOT NULL DEFAULT 0,
  started_at      timestamptz NOT NULL DEFAULT now(),
  updated_at      timestamptz NOT NULL DEFAULT now(),
  finished_at     timestamptz,
  stats           jsonb NOT NULL DEFAULT '{}'::jsonb,
  error           text
);

CREATE INDEX index_runs_repo_started ON index_runs (repository_id, started_at DESC);
CREATE UNIQUE INDEX index_runs_snapshot ON index_runs (snapshot_id);

ALTER TABLE index_runs ENABLE ROW LEVEL SECURITY;
ALTER TABLE index_runs FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON index_runs
  USING (organization_id = NULLIF(current_setting('app.organization_id', true), '')::uuid)
  WITH CHECK (organization_id = NULLIF(current_setting('app.organization_id', true), '')::uuid);
