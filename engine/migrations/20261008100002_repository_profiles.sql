-- Repository profiles (PROF-001).
--
-- A profile is immutable per (repository_id, snapshot_id, profile_version, config_hash); writers
-- use INSERT ... ON CONFLICT DO NOTHING so concurrent computations of the same key converge.
-- profile_conventions repeats each convention of the profile as a row (the profile jsonb stays
-- the source of truth). Profiles hold code facts only, never source bodies.

CREATE TABLE repository_profiles (
  id              uuid PRIMARY KEY,
  organization_id uuid NOT NULL,
  repository_id   uuid NOT NULL,
  snapshot_id     uuid NOT NULL,
  profile_version integer NOT NULL CHECK (profile_version >= 0),
  config_hash     text NOT NULL CHECK (config_hash ~ '^[0-9a-f]{64}$'),
  profile         jsonb NOT NULL,
  computed_at     timestamptz NOT NULL DEFAULT now(),
  UNIQUE (repository_id, snapshot_id, profile_version, config_hash),
  UNIQUE (id, organization_id),
  CONSTRAINT repository_profiles_repository_fk
    FOREIGN KEY (repository_id, organization_id)
    REFERENCES repositories (id, organization_id) ON DELETE CASCADE,
  CONSTRAINT repository_profiles_snapshot_fk
    FOREIGN KEY (snapshot_id, organization_id)
    REFERENCES snapshots (id, organization_id) ON DELETE CASCADE
);
CREATE INDEX repository_profiles_snapshot_idx
  ON repository_profiles (repository_id, snapshot_id, computed_at DESC);

CREATE TABLE profile_conventions (
  profile_id      uuid NOT NULL,
  organization_id uuid NOT NULL,
  convention_id   text NOT NULL,
  rule            text NOT NULL,
  scope           text NOT NULL,
  samples         integer NOT NULL CHECK (samples >= 0),
  violations      integer NOT NULL CHECK (violations >= 0),
  consistency     real NOT NULL CHECK (consistency >= 0 AND consistency <= 1),
  confidence      real NOT NULL CHECK (confidence >= 0 AND confidence <= 1),
  enforceable     boolean NOT NULL DEFAULT false,
  exceptions      jsonb NOT NULL DEFAULT '[]'::jsonb,
  source          text NOT NULL CHECK (source IN ('inferred', 'documented', 'explicit')),
  PRIMARY KEY (profile_id, convention_id),
  CONSTRAINT profile_conventions_profile_fk
    FOREIGN KEY (profile_id, organization_id)
    REFERENCES repository_profiles (id, organization_id) ON DELETE CASCADE
);

DO $$
DECLARE
  t text;
BEGIN
  FOREACH t IN ARRAY ARRAY['repository_profiles', 'profile_conventions'] LOOP
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
