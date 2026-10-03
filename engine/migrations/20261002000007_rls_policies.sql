-- Row-level security for every tenant table (API-003). Defense in depth: the application guards
-- (TenancyGuard) decide who may act, and these policies make a missing WHERE unable to leak rows.
--
--   * Every statement is evaluated against `app.organization_id`, set transaction-locally by the
--     API/engine (`set_config('app.organization_id', $1, true)`).
--   * A missing or empty setting yields NULL, so no row matches: the policies fail closed.
--   * FORCE ROW LEVEL SECURITY applies the policy to the table owner as well; only superusers
--     and BYPASSRLS roles (rg_ops) are exempt.
--   * `jobs` (not created yet) is deliberately NOT tenant scoped: claims span tenants and
--     payloads carry ids only.
--   * `webhook_deliveries.organization_id` is nullable (the tenant is known only after
--     normalization): rows without an organization are visible to rg_ops alone, and ingress
--     writes them through the SECURITY DEFINER functions below.
--   * `users` is global identity and is not tenant scoped.
--
-- Tables added by later migrations must repeat this block for their own tenant tables.

DO $$
DECLARE
  t text;
  tenant_tables text[] := ARRAY[
    'provider_installations', 'memberships', 'repositories', 'pull_requests', 'review_runs',
    'reviewer_runs', 'candidate_findings', 'verified_findings', 'published_findings',
    'finding_feedback', 'webhook_deliveries'];
BEGIN
  FOREACH t IN ARRAY tenant_tables LOOP
    EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
    EXECUTE format('ALTER TABLE %I FORCE ROW LEVEL SECURITY', t);
    IF NOT EXISTS (
      SELECT 1 FROM pg_policies
      WHERE schemaname = 'public' AND tablename = t AND policyname = 'tenant_isolation'
    ) THEN
      EXECUTE format(
        'CREATE POLICY tenant_isolation ON %I
           USING (organization_id = NULLIF(current_setting(''app.organization_id'', true), '''')::uuid)
           WITH CHECK (organization_id = NULLIF(current_setting(''app.organization_id'', true), '''')::uuid)',
        t);
    END IF;
  END LOOP;

  -- The organization row is its own tenant.
  ALTER TABLE organizations ENABLE ROW LEVEL SECURITY;
  ALTER TABLE organizations FORCE ROW LEVEL SECURITY;
  IF NOT EXISTS (
    SELECT 1 FROM pg_policies
    WHERE schemaname = 'public' AND tablename = 'organizations' AND policyname = 'tenant_isolation'
  ) THEN
    CREATE POLICY tenant_isolation ON organizations
      USING (id = NULLIF(current_setting('app.organization_id', true), '')::uuid)
      WITH CHECK (id = NULLIF(current_setting('app.organization_id', true), '')::uuid);
  END IF;
END
$$;

-- Narrow lookups that must work before a tenant is known. They run as the function owner (the
-- migration role, which is exempt from RLS), return only what the caller needs, and pin
-- search_path so a caller cannot shadow a table.

-- Resolves a route resource id to its organization. NULL when the id is unknown, so the guard
-- answers 404 for both "does not exist" and "belongs to someone else".
CREATE OR REPLACE FUNCTION resolve_org(kind text, resource_id uuid) RETURNS uuid
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public, pg_temp AS $$
  SELECT CASE kind
    WHEN 'repository' THEN (SELECT organization_id FROM repositories WHERE id = resource_id)
    WHEN 'pull_request' THEN (SELECT organization_id FROM pull_requests WHERE id = resource_id)
    WHEN 'review' THEN (SELECT organization_id FROM review_runs WHERE id = resource_id)
    WHEN 'finding' THEN (SELECT organization_id FROM verified_findings WHERE id = resource_id)
    WHEN 'installation' THEN (SELECT organization_id FROM provider_installations WHERE id = resource_id)
    WHEN 'organization' THEN (SELECT id FROM organizations WHERE id = resource_id)
  END
$$;

-- Organization of a provider installation (webhooks and login only know the provider's id).
CREATE OR REPLACE FUNCTION rg_installation_org(p_provider text, p_installation_id bigint) RETURNS uuid
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public, pg_temp AS $$
  SELECT organization_id FROM provider_installations
  WHERE provider = p_provider AND provider_installation_id = p_installation_id
$$;

-- The caller's role in an organization; NULL when not a member.
CREATE OR REPLACE FUNCTION rg_membership_role(p_user_id uuid, p_organization_id uuid) RETURNS text
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public, pg_temp AS $$
  SELECT role FROM memberships WHERE user_id = p_user_id AND organization_id = p_organization_id
$$;

-- Every organization a user belongs to, with the role.
CREATE OR REPLACE FUNCTION rg_user_memberships(p_user_id uuid)
RETURNS TABLE (organization_id uuid, slug text, display_name text, role text)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public, pg_temp AS $$
  SELECT m.organization_id, o.slug, o.display_name, m.role
  FROM memberships m JOIN organizations o ON o.id = m.organization_id
  WHERE m.user_id = p_user_id
  ORDER BY o.slug
$$;

REVOKE ALL ON FUNCTION resolve_org(text, uuid) FROM PUBLIC;
REVOKE ALL ON FUNCTION rg_installation_org(text, bigint) FROM PUBLIC;
REVOKE ALL ON FUNCTION rg_membership_role(uuid, uuid) FROM PUBLIC;
REVOKE ALL ON FUNCTION rg_user_memberships(uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION resolve_org(text, uuid) TO rg_api, rg_engine, rg_ops;
GRANT EXECUTE ON FUNCTION rg_installation_org(text, bigint) TO rg_api, rg_engine, rg_ops;
GRANT EXECUTE ON FUNCTION rg_membership_role(uuid, uuid) TO rg_api, rg_ops;
GRANT EXECUTE ON FUNCTION rg_user_memberships(uuid) TO rg_api, rg_ops;

-- FORCE RLS also binds the table owner, so the lookups above must be owned by a role that
-- bypasses RLS (rg_ops) to see across tenants. The migration role must be a member of rg_ops
-- (a superuser, as in development, always is).
GRANT CREATE ON SCHEMA public TO rg_ops;
ALTER FUNCTION resolve_org(text, uuid) OWNER TO rg_ops;
ALTER FUNCTION rg_installation_org(text, bigint) OWNER TO rg_ops;
ALTER FUNCTION rg_membership_role(uuid, uuid) OWNER TO rg_ops;
ALTER FUNCTION rg_user_memberships(uuid) OWNER TO rg_ops;
