# Tenancy

ReviewGraph is multi-tenant: an organization owns installations, repositories, pull requests, review runs and findings. Isolation is enforced twice (task API-003, risk R9): the application decides who may act, and Postgres makes a missing `WHERE` unable to leak rows.

## Layers

1. **Session** (API-004) authenticates the user and sets `req.rgUser`.
2. **TenancyGuard** (`apps/api/src/tenancy`) runs on every route carrying `@RequireRole(...)`:
   - resolves the organization from a resource route param (`:repoId`, `:reviewId`, `:findingId`, `:pullRequestId`, `:installationId`, `:organizationId`), else from `organization_id` in the query or body, else from the only membership of the caller;
   - checks membership and role, then stores `{organizationId, role}` on the request (`@Tenant()`).
3. **`DbService.withTx(orgId, fn)`** opens a transaction that always runs `set_config('app.organization_id', $1, true)`. The setting is transaction-local, so a pooled connection never carries a tenant to the next request.
4. **Row-level security** (`engine/migrations/20261002000007_rls_policies.sql`) filters and checks every row against that setting.

## Roles

| Route role   | Stored roles that satisfy it |
| ------------ | ---------------------------- |
| `viewer`     | viewer, member, admin, owner |
| `maintainer` | member, admin, owner         |
| `admin`      | admin, owner                 |

`maintainer` is the stored `member` role: the schema role set is `owner | admin | member | viewer`.

## Responses

- An unknown id, a malformed id and an id owned by another organization all answer **404**. The guard never answers 403 for a resource the caller cannot see, so ids cannot be probed.
- **403** is returned only to a member whose role is too low.
- Counter `tenancy_denied_total{reason=not_member|role}`.

## Database roles (`20261002000006_db_roles.sql`)

| Role         | Purpose                                                    | RLS      |
| ------------ | ---------------------------------------------------------- | -------- |
| `rg_migrator`| owns the schema, used by `sqlx migrate` only               | n/a      |
| `rg_api`     | the control plane                                          | enforced |
| `rg_engine`  | the Rust engine/worker; sets the tenant per job            | enforced |
| `rg_ops`     | reaper and admin scripts; **never** on a request path      | bypassed |

All roles are `NOLOGIN`; deployments grant membership to their logins. In development the login is a superuser, which bypasses RLS, so `DB_APP_ROLE=rg_api` makes every API transaction `SET LOCAL ROLE rg_api`.

Policies are `FORCE`d, so they bind the table owner too. Only superusers and `BYPASSRLS` roles are exempt.

## Fail closed

- No `app.organization_id` (or an empty one) matches no row. A write without a matching organization fails `WITH CHECK` (SQLSTATE 42501).
- `webhook_deliveries.organization_id` is nullable (the tenant is known after normalization). Rows without an organization are invisible to `rg_api`; ingress writes them through SECURITY DEFINER functions.
- `jobs` is deliberately not tenant scoped: claims span tenants and payloads carry ids only. Consumers set the organization before touching tenant rows.

## Pre-tenant lookups

Resolving "which organization owns this id" needs to see rows across tenants. These lookups are SECURITY DEFINER functions owned by `rg_ops`, with a pinned `search_path`, `EXECUTE` revoked from `PUBLIC`, and a return value limited to what the caller needs:

- `resolve_org(kind, id)` returns only the organization id.
- `rg_installation_org(provider, installation_id)`
- `rg_membership_role(user_id, organization_id)`
- `rg_user_memberships(user_id)`

## Adding a tenant table

Every new tenant table needs `organization_id`, composite foreign keys to its parents, and the `tenant_isolation` policy with `ENABLE` and `FORCE ROW LEVEL SECURITY`, repeated in the migration that creates it. `engine/migrations` stays the only schema source.

## Tests

- `apps/api/integration/rls.int.spec.ts` (`pnpm -F @reviewgraph/api test:integration`): rows of other organizations are hidden, a missing setting returns nothing, `WITH CHECK` blocks cross-organization writes, the setting does not leak across a pooled connection, RLS is enabled and forced on every tenant table, and the guard answers 404 and 403 as above against the real database.
- `apps/api/test/tenancy/tenancy.guard.spec.ts`: guard behavior with fake lookups.
- SEC-001 builds the full isolation suite on these fixtures.
