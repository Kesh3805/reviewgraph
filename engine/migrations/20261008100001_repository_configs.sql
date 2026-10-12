-- Repository review configuration bound to snapshots (POL-002).
--
-- One row per distinct normalized `.review/config.yaml` of a repository, content-addressed by
-- the review-config hash (blake3 of the canonical normalized JSON, 64 hex characters). An invalid
-- file is stored with `normalized` = the defaults and its validation errors, under a hash that
-- differs from the defaults' hash. Writers use INSERT ... ON CONFLICT DO NOTHING.

CREATE TABLE repository_configs (
  organization_id uuid NOT NULL,
  repository_id   uuid NOT NULL,
  config_hash     text NOT NULL CHECK (config_hash ~ '^[0-9a-f]{64}$'),
  status          text NOT NULL CHECK (status IN ('missing', 'valid', 'invalid')),
  normalized      jsonb NOT NULL,
  raw_blob_sha    text,
  validation      jsonb NOT NULL DEFAULT '[]'::jsonb,
  created_at      timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (repository_id, config_hash),
  CONSTRAINT repository_configs_repository_fk
    FOREIGN KEY (repository_id, organization_id)
    REFERENCES repositories (id, organization_id) ON DELETE CASCADE
);

-- Where the snapshot's config came from (NULL: no file, defaults applied).
ALTER TABLE snapshots ADD COLUMN config_source_path text;

ALTER TABLE repository_configs ENABLE ROW LEVEL SECURITY;
ALTER TABLE repository_configs FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON repository_configs
  USING (organization_id = NULLIF(current_setting('app.organization_id', true), '')::uuid)
  WITH CHECK (organization_id = NULLIF(current_setting('app.organization_id', true), '')::uuid);
