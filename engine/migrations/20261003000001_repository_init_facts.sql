-- Repository init facts (INIT-013): the output of `review init`, versioned per commit.
--
-- `repositories.default_branch` already exists (DOM-009), so only the pointer and the summary
-- columns are added. The facts table is a tenant table: it repeats the RLS block of the
-- API-003 migration (ENABLE + FORCE + tenant_isolation on app.organization_id).

CREATE TABLE repository_init_facts (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  organization_id uuid NOT NULL,
  repository_id uuid NOT NULL,
  commit_sha text NOT NULL CHECK (commit_sha ~ '^([0-9a-f]{40}|[0-9a-f]{64})$'),
  facts_schema_version integer NOT NULL CHECK (facts_schema_version > 0),
  tool_version text NOT NULL,
  facts_hash text NOT NULL,
  fingerprint text,
  primary_language text,
  is_monorepo boolean NOT NULL,
  frameworks text[] NOT NULL DEFAULT '{}',
  warnings_count integer NOT NULL DEFAULT 0 CHECK (warnings_count >= 0),
  facts jsonb NOT NULL,
  detected_at timestamptz NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  FOREIGN KEY (repository_id, organization_id)
    REFERENCES repositories (id, organization_id) ON DELETE CASCADE,
  CONSTRAINT repository_init_facts_dedup UNIQUE (repository_id, commit_sha, facts_hash),
  CONSTRAINT repository_init_facts_size CHECK (pg_column_size(facts) <= 1048576),
  UNIQUE (id, organization_id)
);

CREATE INDEX repository_init_facts_latest
  ON repository_init_facts (repository_id, detected_at DESC);
CREATE INDEX repository_init_facts_org_idx ON repository_init_facts (organization_id);

ALTER TABLE repositories
  ADD COLUMN latest_init_facts_id uuid REFERENCES repository_init_facts (id) ON DELETE SET NULL,
  ADD COLUMN primary_language text,
  ADD COLUMN initialized_at timestamptz;

ALTER TABLE repository_init_facts ENABLE ROW LEVEL SECURITY;
ALTER TABLE repository_init_facts FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON repository_init_facts
  USING (organization_id = NULLIF(current_setting('app.organization_id', true), '')::uuid)
  WITH CHECK (organization_id = NULLIF(current_setting('app.organization_id', true), '')::uuid);
