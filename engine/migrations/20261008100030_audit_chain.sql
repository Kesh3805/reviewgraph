-- Audit log tamper evidence and deduplication (SEC-008) on top of the API-008 baseline table.
--
--   * Per-organization hash chain: every row gets the next `chain_seq` of its organization and
--     `hash = sha256(prev_hash || rg_audit_payload(row))`, computed by a BEFORE INSERT trigger
--     under a transaction-scoped advisory lock per organization, so the chain is gap-free even
--     with concurrent writers. Rows written before this migration have no chain_seq and are not
--     part of the chain.
--   * `dedupe_key`: a retried job (publication, provider feedback) audits an event once.
--   * `rg_audit_verify(org)` recomputes the chain and reports the first row that does not match.

ALTER TABLE audit_log ADD COLUMN chain_seq bigint;
ALTER TABLE audit_log ADD COLUMN dedupe_key text CHECK (length(dedupe_key) <= 300);
CREATE UNIQUE INDEX audit_log_org_chain_idx ON audit_log (organization_id, chain_seq)
  WHERE chain_seq IS NOT NULL;
CREATE UNIQUE INDEX audit_log_org_dedupe_idx ON audit_log (organization_id, dedupe_key)
  WHERE dedupe_key IS NOT NULL;

-- The canonical, time-zone independent serialization that is hashed. jsonb renders keys in a
-- fixed order, and the timestamp is rendered as microseconds since the epoch.
CREATE FUNCTION rg_audit_payload(a audit_log) RETURNS bytea
LANGUAGE sql IMMUTABLE SET search_path = public, pg_temp AS $$
  SELECT convert_to(jsonb_build_object(
    'id', a.id,
    'organization_id', a.organization_id,
    'repository_id', a.repository_id,
    'occurred_at_us', floor(extract(epoch FROM a.occurred_at) * 1000000)::bigint,
    'actor_type', a.actor_type,
    'actor_id', a.actor_id,
    'action', a.action,
    'target_type', a.target_type,
    'target_id', a.target_id,
    'outcome', a.outcome,
    'metadata', a.metadata,
    'request_id', a.request_id,
    'trace_id', a.trace_id,
    'dedupe_key', a.dedupe_key,
    'chain_seq', a.chain_seq)::text, 'UTF8')
$$;

-- Runs as rg_ops so the chain head is found whatever the caller's tenant setting is.
CREATE FUNCTION rg_audit_chain() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE
  head record;
BEGIN
  PERFORM pg_advisory_xact_lock(hashtextextended('rg_audit:' || NEW.organization_id::text, 0));
  SELECT chain_seq, hash INTO head FROM audit_log
   WHERE organization_id = NEW.organization_id AND chain_seq IS NOT NULL
   ORDER BY chain_seq DESC LIMIT 1;
  NEW.chain_seq := coalesce(head.chain_seq, 0) + 1;
  NEW.prev_hash := head.hash;
  NEW.hash := sha256(coalesce(head.hash, ''::bytea) || rg_audit_payload(NEW));
  RETURN NEW;
END
$$;
ALTER FUNCTION rg_audit_chain() OWNER TO rg_ops;

CREATE TRIGGER audit_log_hash_chain BEFORE INSERT ON audit_log
  FOR EACH ROW EXECUTE FUNCTION rg_audit_chain();

-- Recomputes the chain of an organization (RLS applies: callers see their own tenant only).
CREATE FUNCTION rg_audit_verify(p_organization_id uuid)
RETURNS TABLE (ok boolean, checked bigint, first_invalid_id uuid)
LANGUAGE plpgsql STABLE SET search_path = public, pg_temp AS $$
DECLARE
  r audit_log;
  prev bytea := NULL;
  expected_seq bigint := 1;
  n bigint := 0;
BEGIN
  FOR r IN SELECT * FROM audit_log
            WHERE organization_id = p_organization_id AND chain_seq IS NOT NULL
            ORDER BY chain_seq LOOP
    n := n + 1;
    IF r.chain_seq <> expected_seq
       OR r.prev_hash IS DISTINCT FROM prev
       OR r.hash IS DISTINCT FROM sha256(coalesce(prev, ''::bytea) || rg_audit_payload(r)) THEN
      RETURN QUERY SELECT false, n, r.id;
      RETURN;
    END IF;
    prev := r.hash;
    expected_seq := expected_seq + 1;
  END LOOP;
  RETURN QUERY SELECT true, n, NULL::uuid;
END
$$;

REVOKE ALL ON FUNCTION rg_audit_verify(uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION rg_audit_verify(uuid) TO rg_api, rg_ops;
