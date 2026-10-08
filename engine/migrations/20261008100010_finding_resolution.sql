-- Findings API (API-010): a finding is addressed by its verified id, or by its candidate id when
-- it never reached verification (suppressed early). The tenancy lookup resolves both, so the
-- guard still answers 404 for unknown and foreign ids alike.

CREATE OR REPLACE FUNCTION resolve_org(kind text, resource_id uuid) RETURNS uuid
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public, pg_temp AS $$
  SELECT CASE kind
    WHEN 'repository' THEN (SELECT organization_id FROM repositories WHERE id = resource_id)
    WHEN 'pull_request' THEN (SELECT organization_id FROM pull_requests WHERE id = resource_id)
    WHEN 'review' THEN (SELECT organization_id FROM review_runs WHERE id = resource_id)
    WHEN 'finding' THEN coalesce(
      (SELECT organization_id FROM verified_findings WHERE id = resource_id),
      (SELECT organization_id FROM candidate_findings WHERE id = resource_id))
    WHEN 'installation' THEN (SELECT organization_id FROM provider_installations WHERE id = resource_id)
    WHEN 'organization' THEN (SELECT id FROM organizations WHERE id = resource_id)
  END
$$;
