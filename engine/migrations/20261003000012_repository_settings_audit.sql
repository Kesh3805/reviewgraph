-- Repository settings and the audit log baseline (API-008).

-- Per-repository review settings. They port the legacy guard options (`github.rs:305-347`).
-- A repository without a row behaves as the defaults below.
CREATE TABLE repository_settings (
  repository_id uuid PRIMARY KEY,
  organization_id uuid NOT NULL,
  enabled boolean NOT NULL DEFAULT true,
  -- Base-branch patterns; empty means every branch. A trailing `*` is a prefix match.
  target_branches text[] NOT NULL DEFAULT '{}',
  skip_drafts boolean NOT NULL DEFAULT true,
  skip_bots boolean NOT NULL DEFAULT true,
  -- Reviewer toggles over the policy defaults: {"security": false, ...}.
  reviewer_overrides jsonb NOT NULL DEFAULT '{}'::jsonb
    CHECK (jsonb_typeof(reviewer_overrides) = 'object'),
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  FOREIGN KEY (repository_id, organization_id)
    REFERENCES repositories (id, organization_id) ON DELETE CASCADE
);
CREATE INDEX repository_settings_org_idx ON repository_settings (organization_id);

CREATE TRIGGER repository_settings_set_updated_at BEFORE UPDATE ON repository_settings
  FOR EACH ROW EXECUTE FUNCTION rg_set_updated_at();

ALTER TABLE repository_settings ENABLE ROW LEVEL SECURITY;
ALTER TABLE repository_settings FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON repository_settings
  USING (organization_id = NULLIF(current_setting('app.organization_id', true), '')::uuid)
  WITH CHECK (organization_id = NULLIF(current_setting('app.organization_id', true), '')::uuid);

-- Append-only audit log baseline (SEC-008 adds the hash chain, the verification function and the
-- read API on top of this table; prev_hash/hash are reserved for it). Configuration changes
-- write a row in the same transaction as the change.
CREATE TABLE audit_log (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  organization_id uuid NOT NULL REFERENCES organizations (id) ON DELETE CASCADE,
  repository_id uuid,
  occurred_at timestamptz NOT NULL DEFAULT now(),
  actor_type text NOT NULL CHECK (actor_type IN ('user', 'service', 'system')),
  actor_id text,
  action text NOT NULL CHECK (length(action) BETWEEN 1 AND 100),
  target_type text NOT NULL,
  target_id text,
  outcome text NOT NULL DEFAULT 'success' CHECK (outcome IN ('success', 'denied', 'failure')),
  -- Ids, enum values and before/after of non-secret fields only: never source, tokens or prompts.
  metadata jsonb NOT NULL DEFAULT '{}'::jsonb,
  request_id text,
  trace_id text,
  prev_hash bytea,
  hash bytea
);
CREATE INDEX audit_log_org_time_idx ON audit_log (organization_id, occurred_at DESC);
CREATE INDEX audit_log_org_action_idx ON audit_log (organization_id, action, occurred_at);

ALTER TABLE audit_log ENABLE ROW LEVEL SECURITY;
ALTER TABLE audit_log FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON audit_log
  USING (organization_id = NULLIF(current_setting('app.organization_id', true), '')::uuid)
  WITH CHECK (organization_id = NULLIF(current_setting('app.organization_id', true), '')::uuid);

-- Append-only: the application roles can only insert and read, and a trigger rejects UPDATE and
-- DELETE for everyone except the retention role (rg_ops) used by the documented purge procedure.
REVOKE UPDATE, DELETE, TRUNCATE ON audit_log FROM rg_api, rg_engine;

CREATE FUNCTION rg_audit_log_immutable() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF current_user = 'rg_ops'
     OR (current_user NOT IN ('rg_api', 'rg_engine') AND current_setting('is_superuser') = 'on') THEN
    RETURN COALESCE(OLD, NEW);
  END IF;
  RAISE EXCEPTION 'audit_log is append-only' USING ERRCODE = 'insufficient_privilege';
END
$$;
CREATE TRIGGER audit_log_append_only BEFORE UPDATE OR DELETE ON audit_log
  FOR EACH ROW EXECUTE FUNCTION rg_audit_log_immutable();

-- The effective review settings of a repository by provider coordinates, for webhook
-- normalization, which runs before any tenant is known. `enabled` is false when the repository
-- lost access, its installation is not active or an operator disabled it. An unknown repository
-- (not synced yet) gets the defaults.
CREATE OR REPLACE FUNCTION rg_repository_settings(
  p_provider text, p_installation_id bigint, p_full_name text
) RETURNS TABLE (enabled boolean, target_branches text[], skip_drafts boolean, skip_bots boolean)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public, pg_temp AS $$
  SELECT
    r.enabled AND i.state = 'active' AND coalesce(s.enabled, true),
    coalesce(s.target_branches, '{}'::text[]),
    coalesce(s.skip_drafts, true),
    coalesce(s.skip_bots, true)
  FROM repositories r
  JOIN provider_installations i
    ON i.id = r.installation_id AND i.organization_id = r.organization_id
  LEFT JOIN repository_settings s ON s.repository_id = r.id
  WHERE i.provider = p_provider
    AND i.provider_installation_id = p_installation_id
    AND lower(r.full_name) = lower(p_full_name)
$$;

REVOKE ALL ON FUNCTION rg_repository_settings(text, bigint, text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION rg_repository_settings(text, bigint, text)
  TO rg_api, rg_engine, rg_ops;
ALTER FUNCTION rg_repository_settings(text, bigint, text) OWNER TO rg_ops;
