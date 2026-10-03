-- Webhook delivery idempotency (GH-003).
--
-- `webhook_deliveries` (DOM-009) is the durable truth behind the Redis SETNX fast path:
-- UNIQUE (provider, delivery_id). Its tenant is unknown at ingress (organization_id is NULL until
-- the event is resolved), and RLS hides NULL-organization rows from rg_api, so ingress goes
-- through these narrow SECURITY DEFINER functions (owned by rg_ops, search_path pinned) instead
-- of touching the table directly. Only a payload hash is stored, never the payload.

-- True when this call recorded the delivery, false when it was already recorded (a duplicate).
-- A concurrent insert of the same delivery blocks until the first transaction ends: if that one
-- commits the loser sees no row (duplicate); if it rolls back the loser inserts and processes.
CREATE OR REPLACE FUNCTION rg_record_webhook_delivery(
  p_provider text, p_delivery_id text, p_event text, p_action text,
  p_installation_id bigint, p_payload_sha256 text, p_signature_valid boolean
) RETURNS boolean
LANGUAGE sql SECURITY DEFINER SET search_path = public, pg_temp AS $$
  WITH ins AS (
    INSERT INTO webhook_deliveries
      (provider, delivery_id, event, action, provider_installation_id, payload_sha256,
       signature_valid, status)
    VALUES
      (p_provider, p_delivery_id, p_event, p_action, p_installation_id, p_payload_sha256,
       p_signature_valid, 'received')
    ON CONFLICT (provider, delivery_id) DO NOTHING
    RETURNING 1
  )
  SELECT EXISTS (SELECT 1 FROM ins)
$$;

-- Cheap read used when Redis says "seen": PG has the final word.
CREATE OR REPLACE FUNCTION rg_webhook_delivery_exists(p_provider text, p_delivery_id text)
RETURNS boolean
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public, pg_temp AS $$
  SELECT EXISTS (
    SELECT 1 FROM webhook_deliveries WHERE provider = p_provider AND delivery_id = p_delivery_id)
$$;

-- Records the outcome and, once known, the organization the delivery belongs to.
CREATE OR REPLACE FUNCTION rg_finish_webhook_delivery(
  p_provider text, p_delivery_id text, p_status text, p_error text, p_organization_id uuid
) RETURNS void
LANGUAGE sql SECURITY DEFINER SET search_path = public, pg_temp AS $$
  UPDATE webhook_deliveries
  SET status = p_status,
      error = left(p_error, 2000),
      organization_id = coalesce(p_organization_id, organization_id),
      processed_at = now()
  WHERE provider = p_provider AND delivery_id = p_delivery_id
$$;

REVOKE ALL ON FUNCTION rg_record_webhook_delivery(text, text, text, text, bigint, text, boolean)
  FROM PUBLIC;
REVOKE ALL ON FUNCTION rg_webhook_delivery_exists(text, text) FROM PUBLIC;
REVOKE ALL ON FUNCTION rg_finish_webhook_delivery(text, text, text, text, uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION rg_record_webhook_delivery(text, text, text, text, bigint, text, boolean)
  TO rg_api, rg_ops;
GRANT EXECUTE ON FUNCTION rg_webhook_delivery_exists(text, text) TO rg_api, rg_ops;
GRANT EXECUTE ON FUNCTION rg_finish_webhook_delivery(text, text, text, text, uuid)
  TO rg_api, rg_ops;
ALTER FUNCTION rg_record_webhook_delivery(text, text, text, text, bigint, text, boolean)
  OWNER TO rg_ops;
ALTER FUNCTION rg_webhook_delivery_exists(text, text) OWNER TO rg_ops;
ALTER FUNCTION rg_finish_webhook_delivery(text, text, text, text, uuid) OWNER TO rg_ops;
