-- Repository profiles (PROF-001): immutable per (repository, snapshot, profile_version, config_hash).
-- Writers use INSERT ... ON CONFLICT DO NOTHING. The profile holds code facts only.

CREATE TABLE repository_profiles (
  id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  organization_id uuid NOT NULL,
  repository_id   uuid NOT NULL,
  snapshot_id     uuid NOT NULL REFERENCES snapshots (id),
  profile_version integer NOT NULL CHECK (profile_version > 0),
  config_hash     text NOT NULL CHECK (config_hash ~ '^[0-9a-f]{64}$'),
  profile         jsonb NOT NULL,
  computed_at     timestamptz NOT NULL DEFAULT now(),
  UNIQUE (repository_id, snapshot_id, profile_version, config_hash)
);

CREATE TABLE profile_conventions (
  profile_id    uuid NOT NULL REFERENCES repository_profiles (id) ON DELETE CASCADE,
  convention_id text NOT NULL,
  rule          text NOT NULL,
  scope         text NOT NULL,
  samples       integer NOT NULL,
  violations    integer NOT NULL,
  consistency   real NOT NULL,
  confidence    real NOT NULL,
  exceptions    jsonb NOT NULL DEFAULT '[]'::jsonb,
  source        text NOT NULL CHECK (source IN ('inferred', 'documented', 'explicit')),
  PRIMARY KEY (profile_id, convention_id)
);

ALTER TABLE repository_profiles ENABLE ROW LEVEL SECURITY;
ALTER TABLE repository_profiles FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON repository_profiles
  USING (organization_id = NULLIF(current_setting('app.organization_id', true), '')::uuid)
  WITH CHECK (organization_id = NULLIF(current_setting('app.organization_id', true), '')::uuid);

-- Conventions inherit tenancy from their profile (visible only through the RLS-filtered parent).
ALTER TABLE profile_conventions ENABLE ROW LEVEL SECURITY;
ALTER TABLE profile_conventions FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON profile_conventions
  USING (EXISTS (SELECT 1 FROM repository_profiles p WHERE p.id = profile_id))
  WITH CHECK (EXISTS (SELECT 1 FROM repository_profiles p WHERE p.id = profile_id));
