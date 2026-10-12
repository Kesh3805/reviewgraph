-- HIST-003: advisory, history-derived signals (PRD section 68: "signals, not rules"). Aggregates
-- and ids only: no comment text, no author identities. The refresh job replaces a repository's
-- whole set in one transaction, so readers see the old or the new full set.

CREATE TABLE repository_signals (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  organization_id uuid NOT NULL,
  repository_id uuid NOT NULL,
  kind text NOT NULL CHECK (kind IN (
    'module_regression', 'finding_density', 'accepted_pattern', 'dismissal_pattern',
    'change_class_needs_tests')),
  scope jsonb NOT NULL,
  statistic jsonb NOT NULL,
  n integer NOT NULL CHECK (n >= 0),
  evidence jsonb NOT NULL DEFAULT '[]'::jsonb,
  computed_at timestamptz NOT NULL,
  valid_until timestamptz NOT NULL,
  version integer NOT NULL DEFAULT 1,
  FOREIGN KEY (repository_id, organization_id)
    REFERENCES repositories (id, organization_id) ON DELETE CASCADE,
  UNIQUE (id, organization_id),
  CHECK (valid_until > computed_at)
);
CREATE INDEX repository_signals_repo_kind_idx ON repository_signals (repository_id, kind, valid_until);

ALTER TABLE repository_signals ENABLE ROW LEVEL SECURITY;
ALTER TABLE repository_signals FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON repository_signals
  USING (organization_id = NULLIF(current_setting('app.organization_id', true), '')::uuid)
  WITH CHECK (organization_id = NULLIF(current_setting('app.organization_id', true), '')::uuid);
