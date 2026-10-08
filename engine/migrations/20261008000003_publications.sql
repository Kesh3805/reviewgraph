-- Publication (GH-009) and stale comment resolution (GH-011).
--
--   publications   at most one review per run: the PK plus the marker lookup make a retried
--                  publish adopt the review it already posted instead of posting another.
--   check_runs     the provider check run of a run (`external_id = review_run_id`).
--   published_findings gains the diff side, the lifecycle status across re-reviews and the run
--                  in which a finding was resolved.

CREATE TABLE publications (
  review_run_id uuid PRIMARY KEY,
  organization_id uuid NOT NULL,
  state text NOT NULL CHECK (state IN ('posting', 'posted', 'skipped', 'failed')),
  attempt integer NOT NULL DEFAULT 1 CHECK (attempt >= 1),
  provider_review_id text,
  outcome text CHECK (length(outcome) <= 200),
  posted_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  FOREIGN KEY (review_run_id, organization_id)
    REFERENCES review_runs (id, organization_id) ON DELETE CASCADE,
  CHECK ((state = 'posted') = (provider_review_id IS NOT NULL))
);

CREATE TABLE check_runs (
  review_run_id uuid PRIMARY KEY,
  organization_id uuid NOT NULL,
  provider_check_run_id text NOT NULL,
  conclusion text CHECK (conclusion IN ('success', 'neutral', 'cancelled', 'skipped')),
  title text CHECK (length(title) <= 200),
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  FOREIGN KEY (review_run_id, organization_id)
    REFERENCES review_runs (id, organization_id) ON DELETE CASCADE
);

ALTER TABLE published_findings
  ADD COLUMN side text CHECK (side IN ('LEFT', 'RIGHT')),
  ADD COLUMN status text NOT NULL DEFAULT 'open'
    CHECK (status IN ('open', 'carried_over', 'resolved', 'unknown')),
  ADD COLUMN resolved_in_run_id uuid,
  ADD FOREIGN KEY (resolved_in_run_id, organization_id)
    REFERENCES review_runs (id, organization_id) ON DELETE SET NULL (resolved_in_run_id);
CREATE INDEX published_findings_pr_status_idx ON published_findings (pull_request_id, status);

CREATE TRIGGER publications_set_updated_at BEFORE UPDATE ON publications
  FOR EACH ROW EXECUTE FUNCTION rg_set_updated_at();
CREATE TRIGGER check_runs_set_updated_at BEFORE UPDATE ON check_runs
  FOR EACH ROW EXECUTE FUNCTION rg_set_updated_at();

DO $$
DECLARE
  t text;
BEGIN
  FOREACH t IN ARRAY ARRAY['publications', 'check_runs'] LOOP
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
