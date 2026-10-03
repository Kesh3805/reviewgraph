-- Installation lifecycle (GH-013): state on installations, access state on repositories, and the
-- pre-tenant functions the webhook uses (an `installation.created` delivery arrives before any
-- organization exists, so it cannot run under a tenant).

ALTER TABLE provider_installations
  ADD COLUMN state text NOT NULL DEFAULT 'active'
    CHECK (state IN ('active', 'suspended', 'deleted')),
  ADD COLUMN deleted_at timestamptz;
UPDATE provider_installations SET state = 'suspended' WHERE suspended_at IS NOT NULL;

-- `enabled` is the provider-access flag driven by installation events; the per-repository
-- operator toggle (API-008 repository_settings.enabled) is separate.
ALTER TABLE repositories
  ADD COLUMN enabled boolean NOT NULL DEFAULT true,
  ADD COLUMN access_state text NOT NULL DEFAULT 'active'
    CHECK (access_state IN ('active', 'removed', 'access_lost', 'installation_deleted'));

-- Creates or refreshes an installation and its organization; returns the organization id.
-- A new installation of an account that already has an organization (a reinstall gets a new
-- provider id) reuses that organization. `p_reactivate` is true for `installation.created` and
-- makes a previously deleted or suspended installation active again.
CREATE OR REPLACE FUNCTION rg_upsert_installation(
  p_provider text, p_installation_id bigint, p_account_login text, p_account_type text,
  p_permissions jsonb, p_reactivate boolean
) RETURNS uuid
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE
  v_org uuid;
  v_base text;
  v_slug text;
BEGIN
  SELECT organization_id INTO v_org FROM provider_installations
  WHERE provider = p_provider AND provider_installation_id = p_installation_id;

  IF v_org IS NOT NULL THEN
    UPDATE provider_installations
    SET account_login = p_account_login,
        account_type = p_account_type,
        permissions = coalesce(p_permissions, permissions),
        state = CASE WHEN p_reactivate THEN 'active' ELSE state END,
        suspended_at = CASE WHEN p_reactivate THEN NULL ELSE suspended_at END,
        deleted_at = CASE WHEN p_reactivate THEN NULL ELSE deleted_at END
    WHERE provider = p_provider AND provider_installation_id = p_installation_id;
    RETURN v_org;
  END IF;

  SELECT organization_id INTO v_org FROM provider_installations
  WHERE provider = p_provider AND lower(account_login) = lower(p_account_login)
  ORDER BY created_at LIMIT 1;

  IF v_org IS NULL THEN
    v_base := trim(BOTH '-' FROM regexp_replace(lower(p_account_login), '[^a-z0-9]+', '-', 'g'));
    IF v_base = '' THEN v_base := 'org'; END IF;
    v_base := left(v_base, 50);
    v_slug := v_base;
    IF EXISTS (SELECT 1 FROM organizations WHERE slug = v_slug) THEN
      v_slug := v_base || '-' || p_installation_id::text;
    END IF;
    INSERT INTO organizations (slug, display_name)
    VALUES (v_slug, left(p_account_login, 200))
    RETURNING id INTO v_org;
  END IF;

  INSERT INTO provider_installations
    (organization_id, provider, provider_installation_id, account_login, account_type, permissions)
  VALUES
    (v_org, p_provider, p_installation_id, p_account_login, p_account_type,
     coalesce(p_permissions, '{}'::jsonb))
  ON CONFLICT (provider, provider_installation_id) DO NOTHING;

  -- If a concurrent delivery won the insert, its organization is the right one.
  SELECT organization_id INTO v_org FROM provider_installations
  WHERE provider = p_provider AND provider_installation_id = p_installation_id;
  RETURN v_org;
END
$$;

-- Moves an installation to `active` / `suspended` / `deleted` (and optionally records new
-- permissions). Returns the organization id, or NULL for an unknown installation.
CREATE OR REPLACE FUNCTION rg_set_installation_state(
  p_provider text, p_installation_id bigint, p_state text, p_permissions jsonb
) RETURNS uuid
LANGUAGE sql SECURITY DEFINER SET search_path = public, pg_temp AS $$
  UPDATE provider_installations
  SET state = p_state,
      suspended_at = CASE p_state WHEN 'suspended' THEN coalesce(suspended_at, now()) ELSE NULL END,
      deleted_at = CASE p_state WHEN 'deleted' THEN coalesce(deleted_at, now()) ELSE NULL END,
      permissions = coalesce(p_permissions, permissions)
  WHERE provider = p_provider AND provider_installation_id = p_installation_id
  RETURNING organization_id
$$;

-- The state used by gates (clone-credential broker, job claims); NULL when unknown.
CREATE OR REPLACE FUNCTION rg_installation_state(p_provider text, p_installation_id bigint)
RETURNS text
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public, pg_temp AS $$
  SELECT state FROM provider_installations
  WHERE provider = p_provider AND provider_installation_id = p_installation_id
$$;

REVOKE ALL ON FUNCTION rg_upsert_installation(text, bigint, text, text, jsonb, boolean) FROM PUBLIC;
REVOKE ALL ON FUNCTION rg_set_installation_state(text, bigint, text, jsonb) FROM PUBLIC;
REVOKE ALL ON FUNCTION rg_installation_state(text, bigint) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION rg_upsert_installation(text, bigint, text, text, jsonb, boolean)
  TO rg_api, rg_ops;
GRANT EXECUTE ON FUNCTION rg_set_installation_state(text, bigint, text, jsonb) TO rg_api, rg_ops;
GRANT EXECUTE ON FUNCTION rg_installation_state(text, bigint) TO rg_api, rg_engine, rg_ops;
ALTER FUNCTION rg_upsert_installation(text, bigint, text, text, jsonb, boolean) OWNER TO rg_ops;
ALTER FUNCTION rg_set_installation_state(text, bigint, text, jsonb) OWNER TO rg_ops;
ALTER FUNCTION rg_installation_state(text, bigint) OWNER TO rg_ops;
