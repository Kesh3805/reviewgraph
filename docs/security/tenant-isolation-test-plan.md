# Tenant isolation test plan (SEC-001)

Tenant isolation is enforced twice: the API guards (`TenancyGuard`, see [tenancy.md](tenancy.md))
decide who may act, and PostgreSQL row-level security makes a missing `WHERE` unable to leak rows.
This suite proves both, and fails when a new route or table ships without coverage.

## Route level (`apps/api`)

- `test/security/isolation-matrix.ts` lists every route with its isolation behavior
  (`foreign_id_404`, `list_filtered`, `session_only`, `public`, `service_auth_only`).
- `test/security/coverage-guard.spec.ts` (unit, CI `check` job) reads the OpenAPI document and
  fails on any route missing from the matrix, on stale entries, and on a route with a tenant id
  parameter that is not `foreign_id_404`.
- `integration/security/routes-isolation.int.spec.ts` (CI `integration` job) builds two
  organizations with identical-looking data (`integration/security/tenancy.fixture.ts`) and, as
  organization A's owner, calls every `foreign_id_404` route with each of organization B's ids:
  - every call answers 404, never 403, and `tenancy_denied_total` grows;
  - the 404 for a foreign id is identical (status, problem body without `instance` and
    `request_id`, content type, `Retry-After`) to the 404 for an id that does not exist;
  - lists never contain the other organization's rows, and a cursor issued to organization B
    neither opens B's data nor leaks it into A's pages;
  - mutations on foreign ids (settings, cancel, feedback, manual review) change nothing.

## Database level

`integration/security/rls-tables.int.spec.ts` enumerates every table with an `organization_id`
column from `information_schema` (plus `organizations`) and checks, as the real non-superuser
roles `rg_api` and `rg_engine`:

- RLS is enabled and forced, with at least one policy;
- with no `app.organization_id`, no row of any table is visible;
- acting as organization A, organization B's rows can be neither read, updated nor deleted;
- inserting a copy of one of B's rows while acting as A fails the policy's `WITH CHECK`;
- the `jobs` table is visible across tenants only inside a worker transaction that sets
  `app.job_worker = on`, and neither setting outlives its transaction on a pooled connection.

## Exemptions

- `users` and `sessions` are global identity, not tenant data.
- `webhook_deliveries.organization_id` is nullable by design: rows without an organization are
  visible to `rg_ops` only.
- Queue workers (`rg_api` consumers and the Rust worker) opt into cross-tenant `jobs` access per
  transaction; the reaper runs as `rg_ops`.

## Not covered here

The engine-side role test (`graph-storage`, `rg_engine` reading snapshots) belongs with the graph
storage tables (GS-005), Qdrant filters with SEC-002 and the object store with SEC-009.
