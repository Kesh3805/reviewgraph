-- Pull request sync (GH-005).
--
-- `pull_requests.updated_at` is bumped by the rg_set_updated_at trigger on every write, so it
-- cannot order provider events. The provider's own `updated_at` is stored separately and guards
-- the upsert: an out-of-order older event never overwrites a newer row, and SUP-001 refuses to
-- move the head for a stale event.

ALTER TABLE pull_requests ADD COLUMN provider_updated_at timestamptz;
