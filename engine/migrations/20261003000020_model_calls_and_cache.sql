-- Model gateway accounting ledger and tenant-scoped response cache (GW-008).
--
-- model_calls: one row per provider attempt (retries are billed, so they are rows). No prompt
-- text is ever stored, only the request hash. model_cache: validated output of cache-allowed
-- tasks, keyed per organisation; the key includes the organisation id and every read repeats the
-- predicate, on top of RLS. Both tables repeat the RLS block from 20261002000007.

CREATE TABLE model_calls (
  id uuid PRIMARY KEY,
  organization_id uuid NOT NULL,
  repository_id uuid NOT NULL,
  review_run_id uuid,
  reviewer_run_id uuid,
  task text NOT NULL,
  tier text NOT NULL,
  provider text NOT NULL,
  model text NOT NULL,
  attempt smallint NOT NULL,
  request_hash text NOT NULL,
  served_from text NOT NULL CHECK (served_from IN ('live', 'response_cache', 'replay')),
  outcome text NOT NULL, -- 'ok' or a GatewayError class
  input_uncached int NOT NULL,
  cache_write int NOT NULL,
  cache_read int NOT NULL,
  output_tokens int NOT NULL,
  cost_usd_micros bigint,
  latency_ms int NOT NULL,
  prices_as_of date,
  created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX model_calls_run_idx ON model_calls (review_run_id);
CREATE INDEX model_calls_org_time_idx ON model_calls (organization_id, created_at);

CREATE TABLE model_cache (
  organization_id uuid NOT NULL,
  cache_key text NOT NULL,
  request_hash text NOT NULL,
  provider text NOT NULL,
  model text NOT NULL,
  prompt_version text NOT NULL,
  schema_hash text,
  output jsonb NOT NULL,
  usage jsonb NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  expires_at timestamptz NOT NULL,
  PRIMARY KEY (organization_id, cache_key)
);
CREATE INDEX model_cache_expiry_idx ON model_cache (expires_at);

DO $$
DECLARE
  t text;
BEGIN
  FOREACH t IN ARRAY ARRAY['model_calls', 'model_cache'] LOOP
    EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
    EXECUTE format('ALTER TABLE %I FORCE ROW LEVEL SECURITY', t);
    EXECUTE format(
      'CREATE POLICY tenant_isolation ON %I
         USING (organization_id = NULLIF(current_setting(''app.organization_id'', true), '''')::uuid)
         WITH CHECK (organization_id = NULLIF(current_setting(''app.organization_id'', true), '''')::uuid)',
      t);
  END LOOP;
END
$$;
