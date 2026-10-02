-- Repositories and pull requests (DOM-009).

CREATE TABLE repositories (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  organization_id uuid NOT NULL REFERENCES organizations (id) ON DELETE CASCADE,
  installation_id uuid NOT NULL,
  provider text NOT NULL CHECK (provider IN ('github')),
  provider_repo_id text NOT NULL,
  full_name text NOT NULL,
  default_branch text NOT NULL,
  visibility text NOT NULL CHECK (visibility IN ('public', 'private', 'internal')),
  archived boolean NOT NULL DEFAULT false,
  settings jsonb NOT NULL DEFAULT '{}'::jsonb,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  FOREIGN KEY (installation_id, organization_id)
    REFERENCES provider_installations (id, organization_id) ON DELETE CASCADE,
  UNIQUE (organization_id, provider, provider_repo_id),
  UNIQUE (id, organization_id)
);
CREATE INDEX repositories_installation_idx ON repositories (installation_id);

CREATE TABLE pull_requests (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  organization_id uuid NOT NULL,
  repository_id uuid NOT NULL,
  provider_number integer NOT NULL CHECK (provider_number > 0),
  title text NOT NULL,
  author_login text NOT NULL,
  base_ref text NOT NULL,
  head_ref text NOT NULL,
  base_sha text NOT NULL CHECK (base_sha ~ '^([0-9a-f]{40}|[0-9a-f]{64})$'),
  head_sha text NOT NULL CHECK (head_sha ~ '^([0-9a-f]{40}|[0-9a-f]{64})$'),
  merge_base_sha text CHECK (merge_base_sha ~ '^([0-9a-f]{40}|[0-9a-f]{64})$'),
  state text NOT NULL CHECK (state IN ('open', 'closed', 'merged')),
  draft boolean NOT NULL DEFAULT false,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  FOREIGN KEY (repository_id, organization_id)
    REFERENCES repositories (id, organization_id) ON DELETE CASCADE,
  UNIQUE (repository_id, provider_number),
  UNIQUE (id, organization_id)
);
CREATE INDEX pull_requests_org_repo_state_idx
  ON pull_requests (organization_id, repository_id, state);

CREATE TRIGGER repositories_set_updated_at BEFORE UPDATE ON repositories
  FOR EACH ROW EXECUTE FUNCTION rg_set_updated_at();
CREATE TRIGGER pull_requests_set_updated_at BEFORE UPDATE ON pull_requests
  FOR EACH ROW EXECUTE FUNCTION rg_set_updated_at();
