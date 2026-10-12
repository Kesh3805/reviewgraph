-- DED-003: cross-run finding identity.
CREATE TABLE finding_identities (
  identity_key text NOT NULL,
  organization_id uuid NOT NULL,
  repository_id uuid NOT NULL,
  pull_request_id uuid NOT NULL,
  first_review_run_id uuid NOT NULL,
  last_review_run_id uuid NOT NULL,
  last_status text NOT NULL CHECK (last_status IN ('new', 'persisting', 'fixed', 'not_reevaluated')),
  last_seen_head_sha text NOT NULL,
  published_fingerprint text,
  provider_comment_id text,
  PRIMARY KEY (pull_request_id, identity_key)
);
