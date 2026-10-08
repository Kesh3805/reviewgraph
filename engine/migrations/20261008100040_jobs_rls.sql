-- Row-level security on the job queue (SEC-001; PIPE-001 left it to this task).
--
-- Request paths always run with a tenant (`app.organization_id`) and see, enqueue and cancel only
-- that tenant's jobs. Queue workers claim across tenants, so their transactions opt in
-- explicitly with `set_config('app.job_worker', 'on', true)` (transaction-local, like the tenant
-- setting). The claim, heartbeat, complete, fail and release statements of both languages (API-007,
-- PIPE-001/002) run in such transactions; nothing else may set it. rg_ops bypasses RLS as before.

ALTER TABLE jobs ENABLE ROW LEVEL SECURITY;
ALTER TABLE jobs FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON jobs
  USING (organization_id = NULLIF(current_setting('app.organization_id', true), '')::uuid
         OR current_setting('app.job_worker', true) = 'on')
  WITH CHECK (organization_id = NULLIF(current_setting('app.organization_id', true), '')::uuid
              OR current_setting('app.job_worker', true) = 'on');
