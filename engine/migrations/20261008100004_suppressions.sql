-- Suppressions and their matches (POL-006, PRD §124).
--
-- Two sources: rows synced from the base policy's `.review/config.yaml` (source = 'config',
-- keyed by the config hash) and rows created through the API (source = 'api'). Revoking is a
-- soft delete (revoked_at) so the audit trail survives. Every match is recorded with the
-- suppression id; the insert is ON CONFLICT DO NOTHING.

CREATE TABLE suppressions (
  id              uuid PRIMARY KEY,
  organization_id uuid NOT NULL,
  repository_id   uuid NOT NULL,
  external_id     text,
  kind            text NOT NULL CHECK (kind IN ('type', 'path', 'symbol', 'rule', 'fingerprint')),
  value           text NOT NULL CHECK (length(value) BETWEEN 1 AND 1000),
  source          text NOT NULL CHECK (source IN ('config', 'api')),
  config_hash     text CHECK (config_hash ~ '^[0-9a-f]{64}$'),
  reason          text NOT NULL CHECK (length(btrim(reason)) BETWEEN 1 AND 2000),
  allow_critical  boolean NOT NULL DEFAULT false,
  created_by      uuid NULL,
  expires_at      timestamptz NULL,
  revoked_at      timestamptz NULL,
  created_at      timestamptz NOT NULL DEFAULT now(),
  CHECK ((source = 'config') = (config_hash IS NOT NULL)),
  UNIQUE (id, organization_id),
  CONSTRAINT suppressions_repository_fk
    FOREIGN KEY (repository_id, organization_id)
    REFERENCES repositories (id, organization_id) ON DELETE CASCADE
);
CREATE INDEX suppressions_active_idx
  ON suppressions (repository_id) WHERE revoked_at IS NULL;

CREATE TABLE suppression_matches (
  suppression_id       uuid NOT NULL,
  organization_id      uuid NOT NULL,
  candidate_finding_id uuid NOT NULL,
  review_run_id        uuid NOT NULL,
  matched_at           timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (suppression_id, candidate_finding_id),
  CONSTRAINT suppression_matches_suppression_fk
    FOREIGN KEY (suppression_id, organization_id)
    REFERENCES suppressions (id, organization_id) ON DELETE CASCADE
);
CREATE INDEX suppression_matches_run_idx ON suppression_matches (review_run_id);

DO $$
DECLARE
  t text;
BEGIN
  FOREACH t IN ARRAY ARRAY['suppressions', 'suppression_matches'] LOOP
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
