-- Polling reconciler (GH-012).
--
-- The reconciler scans every reviewable repository across tenants before it knows any tenant,
-- so it reads its work list through this narrow SECURITY DEFINER function (owned by rg_ops,
-- search_path pinned, like the other pre-tenant lookups). It returns ids and provider
-- coordinates only; everything else is read per repository under that repository's tenant.

CREATE OR REPLACE FUNCTION rg_reconcile_targets(p_provider text)
RETURNS TABLE (
  repository_id uuid, organization_id uuid, provider_installation_id bigint,
  provider_repo_id text, full_name text)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public, pg_temp AS $$
  SELECT r.id, r.organization_id, i.provider_installation_id, r.provider_repo_id, r.full_name
  FROM repositories r
  JOIN provider_installations i
    ON i.id = r.installation_id AND i.organization_id = r.organization_id
  LEFT JOIN repository_settings s ON s.repository_id = r.id
  WHERE r.provider = p_provider
    AND r.enabled
    AND r.access_state = 'active'
    AND NOT r.archived
    AND i.state = 'active'
    AND coalesce(s.enabled, true)
  ORDER BY r.id
$$;

REVOKE ALL ON FUNCTION rg_reconcile_targets(text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION rg_reconcile_targets(text) TO rg_api, rg_ops;
ALTER FUNCTION rg_reconcile_targets(text) OWNER TO rg_ops;
