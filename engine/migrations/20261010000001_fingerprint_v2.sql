-- DED-001: accept both v1 and v2 fingerprints. Existing rows are untouched.
ALTER TABLE candidate_findings
  DROP CONSTRAINT IF EXISTS candidate_findings_fingerprint_check;
ALTER TABLE candidate_findings
  ADD CONSTRAINT candidate_findings_fingerprint_check
  CHECK (fingerprint ~ '^v[12]:[0-9a-f]{32}$');
