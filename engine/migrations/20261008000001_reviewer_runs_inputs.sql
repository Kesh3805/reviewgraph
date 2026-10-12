-- Reviewer run inputs and outcomes (REV-001).
--
-- reviewer_runs already exists (DOM-009, 20261002000003) with its RLS policy (20261002000007).
-- This adds what the reviewer framework records for reproducibility and cost attribution: the
-- prompt sha, active focus profiles, the reviewer input hash (PRD §76 stage key) and context
-- package hash, the route decision and the request hash, and candidate counts. Additive only.

ALTER TABLE reviewer_runs
  ADD COLUMN prompt_sha text,
  ADD COLUMN focus_profiles text[] NOT NULL DEFAULT '{}',
  ADD COLUMN input_hash text CHECK (input_hash ~ '^[0-9a-f]{64}$'),
  ADD COLUMN context_package_hash text,
  ADD COLUMN route jsonb,
  ADD COLUMN request_hash text,
  ADD COLUMN candidates_raw integer NOT NULL DEFAULT 0 CHECK (candidates_raw >= 0),
  ADD COLUMN candidates_accepted integer NOT NULL DEFAULT 0 CHECK (candidates_accepted >= 0);

CREATE INDEX reviewer_runs_input_hash_idx ON reviewer_runs (review_run_id, reviewer, input_hash);
