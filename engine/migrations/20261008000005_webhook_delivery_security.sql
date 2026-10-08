-- Webhook replay protection (SEC-006).
--
-- `webhook_deliveries` already stores the body hash (`payload_sha256`) and the outcome
-- (`status`), so no column is added. The replay guard reads the recorded hash of a delivery id
-- before any tenant is known, through this narrow SECURITY DEFINER lookup, and the retention
-- sweep (SEC-007, `WEBHOOK_REPLAY_RETENTION_DAYS`) scans by `received_at`.

CREATE INDEX webhook_deliveries_received_idx ON webhook_deliveries (received_at);

CREATE OR REPLACE FUNCTION rg_webhook_delivery_hash(p_provider text, p_delivery_id text)
RETURNS text
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public, pg_temp AS $$
  SELECT payload_sha256 FROM webhook_deliveries
  WHERE provider = p_provider AND delivery_id = p_delivery_id
$$;

REVOKE ALL ON FUNCTION rg_webhook_delivery_hash(text, text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION rg_webhook_delivery_hash(text, text) TO rg_api, rg_ops;
ALTER FUNCTION rg_webhook_delivery_hash(text, text) OWNER TO rg_ops;
