-- DED-002: persisted cross-reviewer merge records. Singletons get no record.
-- RLS policies follow the project template and are applied by the RLS migration sweep.
CREATE TABLE finding_merge_records (
  id uuid PRIMARY KEY,
  organization_id uuid NOT NULL,
  review_run_id uuid NOT NULL,
  primary_candidate_id uuid NOT NULL,
  rule text NOT NULL CHECK (rule IN ('R1', 'R2', 'R3')),
  similarity real NOT NULL,
  confidence_before real NOT NULL,
  confidence_after real NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (review_run_id, primary_candidate_id)
);

CREATE TABLE finding_merge_members (
  merge_record_id uuid NOT NULL REFERENCES finding_merge_records (id) ON DELETE CASCADE,
  candidate_finding_id uuid NOT NULL,
  reviewer text NOT NULL,
  role text NOT NULL CHECK (role IN ('primary', 'duplicate')),
  PRIMARY KEY (merge_record_id, candidate_finding_id)
);
