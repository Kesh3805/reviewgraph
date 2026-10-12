-- HIST-001: provider review history (PRD section 67). Author identities are salted hashes, never
-- raw logins; comment bodies are redacted, capped at 4 KiB and only stored for repositories that
-- opted in with `history.enabled` in .review/config.yaml.

CREATE TABLE history_pull_requests (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  organization_id uuid NOT NULL,
  repository_id uuid NOT NULL,
  provider_pr_id text NOT NULL,
  number integer NOT NULL CHECK (number >= 1),
  title_hash text NOT NULL,
  author_ref_hash text NOT NULL,
  state text NOT NULL CHECK (state IN ('open', 'closed', 'merged')),
  merged boolean NOT NULL,
  merge_commit text,
  opened_at timestamptz NOT NULL,
  closed_at timestamptz,
  size_stats jsonb NOT NULL DEFAULT '{}'::jsonb,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  FOREIGN KEY (repository_id, organization_id)
    REFERENCES repositories (id, organization_id) ON DELETE CASCADE,
  UNIQUE (id, organization_id),
  UNIQUE (repository_id, provider_pr_id)
);
CREATE INDEX history_pull_requests_repo_closed_idx
  ON history_pull_requests (repository_id, closed_at);

CREATE TABLE history_review_comments (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  organization_id uuid NOT NULL,
  repository_id uuid NOT NULL,
  history_pr_id uuid NOT NULL,
  provider_comment_id text NOT NULL,
  thread_id text,
  path text,
  line integer CHECK (line IS NULL OR line >= 1),
  commit_sha text,
  author_kind text NOT NULL CHECK (author_kind IN ('human', 'bot', 'reviewgraph')),
  author_ref_hash text NOT NULL,
  body_text_redacted text
    CHECK (body_text_redacted IS NULL OR octet_length(body_text_redacted) <= 4096),
  resolved boolean NOT NULL DEFAULT false,
  resolved_by_kind text CHECK (resolved_by_kind IS NULL
    OR resolved_by_kind IN ('human', 'bot', 'reviewgraph')),
  link_status text NOT NULL CHECK (link_status IN ('linked', 'path_only', 'unlinked')),
  symbol_key text,
  -- Set for ReviewGraph's own comments: the published finding with this provider comment id.
  published_finding_id uuid,
  created_at timestamptz NOT NULL,
  ingested_at timestamptz NOT NULL DEFAULT now(),
  FOREIGN KEY (history_pr_id, organization_id)
    REFERENCES history_pull_requests (id, organization_id) ON DELETE CASCADE,
  UNIQUE (history_pr_id, provider_comment_id)
);
CREATE INDEX history_review_comments_pr_idx ON history_review_comments (history_pr_id);
CREATE INDEX history_review_comments_repo_path_idx ON history_review_comments (repository_id, path);

CREATE TABLE history_ingest_cursors (
  repository_id uuid PRIMARY KEY,
  organization_id uuid NOT NULL,
  cursor text,
  last_pr_updated_at timestamptz,
  prs_ingested integer NOT NULL DEFAULT 0,
  completed boolean NOT NULL DEFAULT false,
  status text NOT NULL DEFAULT 'idle'
    CHECK (status IN ('idle', 'running', 'completed', 'failed', 'skipped')),
  failure_reason text,
  updated_at timestamptz NOT NULL DEFAULT now(),
  FOREIGN KEY (repository_id, organization_id)
    REFERENCES repositories (id, organization_id) ON DELETE CASCADE
);

CREATE TRIGGER history_pull_requests_set_updated_at BEFORE UPDATE ON history_pull_requests
  FOR EACH ROW EXECUTE FUNCTION rg_set_updated_at();
CREATE TRIGGER history_ingest_cursors_set_updated_at BEFORE UPDATE ON history_ingest_cursors
  FOR EACH ROW EXECUTE FUNCTION rg_set_updated_at();

DO $$
DECLARE
  t text;
BEGIN
  FOREACH t IN ARRAY ARRAY['history_pull_requests', 'history_review_comments', 'history_ingest_cursors'] LOOP
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
