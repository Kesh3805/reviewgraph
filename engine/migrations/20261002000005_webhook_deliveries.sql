-- Webhook ingress log (DOM-009).
--
-- The tenant is resolved only after normalization, so organization_id is NULLABLE by design;
-- SEC-001 restricts this table to the service role. Raw payloads are not stored, only their
-- SHA-256, to minimise retained provider data.

CREATE TABLE webhook_deliveries (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  provider text NOT NULL CHECK (provider IN ('github')),
  delivery_id text NOT NULL,
  event text NOT NULL,
  action text,
  organization_id uuid REFERENCES organizations (id) ON DELETE CASCADE,
  provider_installation_id bigint,
  payload_sha256 text NOT NULL CHECK (payload_sha256 ~ '^[0-9a-f]{64}$'),
  signature_valid boolean NOT NULL,
  status text NOT NULL CHECK (status IN (
    'received', 'processed', 'ignored', 'rejected', 'failed')),
  error text CHECK (length(error) <= 2000),
  received_at timestamptz NOT NULL DEFAULT now(),
  processed_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  -- Durable dedup backing the Redis SETNX fast path (target-arch §5).
  UNIQUE (provider, delivery_id)
);
CREATE INDEX webhook_deliveries_org_received_idx
  ON webhook_deliveries (organization_id, received_at DESC);

CREATE TRIGGER webhook_deliveries_set_updated_at BEFORE UPDATE ON webhook_deliveries
  FOR EACH ROW EXECUTE FUNCTION rg_set_updated_at();
