# Phases 27A, 27, 28, 29 — Control plane, GitHub integration, supersession, CLI

**Parent:** [MASTER_IMPLEMENTATION_PLAN.md](../MASTER_IMPLEMENTATION_PLAN.md) §9 · **Architecture:** [target-architecture.md](../../architecture/target-architecture.md) §1, §4.1, §5 · **ADRs:** 002, 012, 013, 014, 015

Status markers: ☐ todo · ◐ in progress · ☑ done (acceptance criteria executed and passed).

**Environment constraints that shape these tasks.**
- **No public ingress and no repo admin.** Real GitHub webhooks need the user to create a GitHub App and run ngrok (DEV-006). Automated tests therefore use signed webhook replay and a fake GitHub API server (DEV-005). No task in this file may need live GitHub for its acceptance criteria. Live checks are confined to E2E-002.
- **No model API keys.** The CLI and pipeline tests use the replay provider.
- **The host cannot build the engine's Rust C dependencies.** Rust tasks (API-013, CLI-*) build and test through `engine/scripts/cargo.sh` (Linux container). The Windows `review.exe` is produced by CI-009.

**Control-plane conventions.**
- NestJS 11, Node 24, TypeScript strict.
- One Nest module per row of target-arch §5.
- Domain modules import provider *ports* only. An ESLint `no-restricted-imports` rule bans `@octokit/*` outside `src/providers/github/**`.
- All routes are under `/api/v1`.
- Errors use RFC 9457 problem+json.
- Every request runs in a Kysely transaction that sets `app.organization_id` (API-003).

---

## Phase 27A — Control-plane foundation

## Task index

| ID | Title |
|---|---|
| API-001 | NestJS app skeleton (config validation, health, graceful shutdown) |
| API-002 | Kysely + pg with types generated from the migrated database |
| API-003 | Tenancy guards and Postgres RLS policies |
| API-004 | GitHub OAuth login and session cookie |
| API-005 | Service-to-service auth (engine ↔ API) |
| API-006 | Provider ports RepositoryProvider and ReviewPublisher |
| API-007 | TS JobQueue adapter (same PG jobs table, transactional enqueue) |
| API-008 | Repositories API (PRD §106) |
| API-009 | Reviews API |
| API-010 | Findings API |
| API-011 | Graph proxy API |
| API-012 | Feedback API |
| API-013 | review-engine Axum internal API |
| GH-001 | GitHub App auth (JWT, installation tokens cached encrypted in Redis) |
| GH-002 | Webhook endpoint and HMAC verification |
| GH-003 | Delivery idempotency (Redis SETNX + webhook_deliveries) |
| GH-004 | Event normalization |
| GH-005 | PR fetch, normalize and repository sync |
| GH-006 | Clone credential broker internal endpoint |
| GH-007 | Inline comment rendering and anchoring |
| GH-008 | Summary rendering (PRD §61) |
| GH-009 | Publisher (atomic review, check run, published_findings, supersession gate) |
| GH-010 | No-merge guarantee (App permission manifest + static test) |
| GH-011 | Stale comment resolution on re-review |
| GH-012 | Polling reconciler fallback |
| GH-013 | Installation lifecycle events |
| SUP-001 | Supersession on new head (single transaction) |
| SUP-002 | Engine stage-boundary supersession checks |
| SUP-003 | Publish-time gate |
| SUP-004 | Concurrency tests |
| CLI-001 | clap skeleton and output (human/json) |
| CLI-002 | `review init` |
| CLI-003 | `review status` |
| CLI-004 | `review doctor` (PRD §83 checks) |
| CLI-005 | `review diff <base>..<head>` and `review branch <a> <b>` |
| CLI-006 | `review pr <id>` (local review via GitHub API token) |
| CLI-007 | `review graph symbol` / `inspect` |
| CLI-008 | `review graph callers` / `callees` / `tests` |
| CLI-009 | `review graph path` |
| CLI-010 | `review graph rebuild` |
| CLI-011 | `review impact` |
| CLI-012 | `review profile` |
| CLI-013 | `review migrate` and `contracts export` |

---

### API-001 — NestJS app skeleton (config validation, health, graceful shutdown)
Status: ☑
> **Implementation note:** Readiness probes use their own minimal `pg`/`ioredis` connections (the typed `DbModule` arrives with API-002); the Pino logger is a Nest `LoggerService` with the redaction injection point for OBS-006; the `api_ready{dependency}` gauge is deferred to OBS-005 (no metric instruments yet); `ShutdownService.register()` is the seam where the API-007 publish consumer stops and releases leases; the webhook raw-body (25 MiB) route is left to the webhooks module (only the 1 MiB JSON limit is wired). An unprefixed `GET /health` liveness alias is also served. The SIGTERM drain test closes the app programmatically (same path the signal hook runs). Config falls back to `.env` via `process.loadEnvFile` for local dev.

- **Task ID:** API-001
- **Title:** NestJS app skeleton (config validation, health, graceful shutdown)
- **Problem:** No control plane exists. Legacy `web.rs` is a `std::net` thread-per-connection server with no auth.
- **Why it exists:** Every API, GH and SUP task needs a bootable, validated and observable NestJS process.
- **Scope:**
  - `apps/api` package in the pnpm workspace.
  - `ConfigModule` with a zod schema; the process fails fast on invalid env.
  - `/api/v1/health/live` and `/api/v1/health/ready`. Ready checks PG, Redis and the review-engine `/internal/v1/health`.
  - Shutdown hooks:
    - On SIGTERM, stop accepting requests.
    - Drain in-flight requests (≤25 s).
    - Stop the publish consumer and release its leases (API-007).
    - Close the pools.
  - Pino JSON logger, wired to OBS-002/OBS-006 redaction.
- **Explicit non-scope:**
  - Auth (API-004/005).
  - Database types (API-002).
  - Business modules.
- **Files/modules expected to change:**
  - `pnpm-workspace.yaml`
  - `package.json` (root scripts)
  - `packages/config` (tsconfig/eslint presets)
- **New files/modules expected:**
  - `apps/api/{package.json,tsconfig.json,nest-cli.json,jest.config.ts}`
  - `apps/api/src/{main.ts,app.module.ts}`
  - `apps/api/src/config/{config.module.ts,env.schema.ts}`
  - `apps/api/src/health/{health.module.ts,health.controller.ts}`
  - `apps/api/src/common/{problem.filter.ts,request-id.middleware.ts}`
- **Dependencies:** FND-001 (pnpm workspace), FND-003 (TS toolchain), OBS-002 (wired when available; it is a no-op until then).
- **Implementation details:**
  - Env schema: `NODE_ENV`, `PORT=8080`, `DATABASE_URL`, `REDIS_URL`, `ENGINE_INTERNAL_URL`, `SERVICE_JWT_SECRET` (≥32 bytes), `SESSION_JWT_SECRET`, `TOKEN_CACHE_KEY` (32-byte base64), `GITHUB_APP_ID`, `GITHUB_APP_PRIVATE_KEY` | `GITHUB_APP_PRIVATE_KEY_FILE`, `GITHUB_WEBHOOK_SECRET`, `GITHUB_CLIENT_ID`, `GITHUB_CLIENT_SECRET`, `GITHUB_API_URL` (default `https://api.github.com`; tests point it at the fake server), `WEB_ORIGIN`, `OTEL_EXPORTER_OTLP_ENDPOINT`.
  - GitHub variables are optional when `GITHUB_ENABLED=false`, which allows CLI-only local development.
  - `app.enableShutdownHooks()`. A `ShutdownService` implements `beforeApplicationShutdown`.
  - Request id: `x-request-id`, or a generated UUIDv7, propagated to logs and spans.
  - Body limit 1 MiB, except the webhook route, which takes a raw body up to 25 MiB.
- **Data model changes:** None.
- **API/protocol changes:** `GET /api/v1/health/live` and `GET /api/v1/health/ready` → `{ status, checks: { pg, redis, engine } }`.
- **Concurrency semantics:** Stateless. Readiness checks are cached for 2 s to avoid probe storms.
- **Failure behavior:**
  - Invalid config exits with code 78 (EX_CONFIG) and prints the zod issues **with values redacted**.
  - Ready returns 503 when any dependency is down. Live stays 200 unless the event loop is blocked for more than 5 s.
- **Idempotency considerations:** N/A.
- **Security considerations:**
  - `helmet` headers.
  - CORS restricted to `WEB_ORIGIN`, with credentials.
  - Config errors never print secret values.
- **Observability additions:** Spans for HTTP server requests (auto). Gauge `api_ready{dependency}`. Log field `request_id`.
- **Tests required:**
  - `env_schema_rejects_missing_database_url`
  - `env_schema_redacts_values_in_errors`
  - `health_ready_503_when_pg_down`
  - `sigterm_drains_inflight_request` (e2e with supertest and a delayed handler)
  - `request_id_propagated`
- **Benchmarks if applicable:** None.
- **Acceptance criteria:**
  - `pnpm --filter api start` boots against compose services.
  - `/health/ready` reports all three checks.
  - SIGTERM exits within 30 s with zero dropped in-flight requests (test).
- **Definition of done:** Global DoD.

---

### API-002 — Kysely + pg with types generated from the migrated database
Status: ☑
> **Implementation note:** `pnpm db:types` is `scripts/db-types.mjs` (Node, not bash: the host is Windows). It starts a throwaway `postgres:16.15-alpine`, applies every `engine/migrations/*.sql` in order through `pg` (the same files `sqlx migrate run` applies; the engine image is not needed) and runs `kysely-codegen`; `pnpm db:types:check` regenerates to a temp file and fails on drift (CI-005 entry point). `DbService.withTx(orgId?, fn)` wraps `runInTx`; the optional `DB_APP_ROLE` env makes each transaction `SET LOCAL ROLE` (used by API-003 so RLS applies to the dev superuser login). The pool acquire timeout and statement timeout (`57014`) map to 503 + `Retry-After` in `ProblemFilter`. DB tests that need Postgres live in `apps/api/integration` (`pnpm test:integration`, separate jest project, dev services by default); the unit suite needs no services.

- **Task ID:** API-002
- **Title:** Kysely + pg + generated types from migrated DB
- **Problem:** The API needs typed SQL against a schema it does not own. ADR-014 makes `engine/migrations` the only schema source.
- **Why it exists:** It prevents drift between Rust migrations and TS queries (ADR-002), and replaces legacy's interpolated SQL through a psql subprocess.
- **Scope:**
  - `DbModule` providing `Kysely<DB>` on a `pg.Pool`.
  - The `pnpm db:types` script: it runs migrations into a throwaway PG and then `kysely-codegen --out-file apps/api/src/db/generated.ts`.
  - The generated file is committed. CI-005 checks for drift.
  - A transaction helper `withTx(orgId?, fn)`.
- **Explicit non-scope:**
  - Writing migrations (that belongs to the engine tasks).
  - RLS (API-003).
- **Files/modules expected to change:** `apps/api/src/app.module.ts`, `apps/api/package.json`.
- **New files/modules expected:**
  - `apps/api/src/db/{db.module.ts,kysely.provider.ts,tx.ts,generated.ts}`
  - `scripts/db-types.sh`
- **Dependencies:** API-001, DOM-009 (migrations exist), CLI-013 (`review migrate`) or `review-worker migrate`.
- **Implementation details:**
  - Pool settings: `max=20`, `idleTimeoutMillis=30000`, `statement_timeout=15s` (set per connection), `application_name=rg-api`.
  - `CamelCasePlugin` is **not** used: column names stay snake_case to match the Rust side and the contracts.
  - `withTx(orgId, fn)` runs `db.transaction().execute(async trx => { await sql\`select set_config('app.organization_id', ${orgId}, true)\`.execute(trx); return fn(trx); })`.
  - `db-types.sh` starts `postgres:16` with `--rm` on a random port, runs `review migrate --database-url` (CLI-013, via the engine image), and then runs `kysely-codegen`.
- **Data model changes:** None.
- **API/protocol changes:** None.
- **Concurrency semantics:** Pooled connections. Transactions are request-scoped, and there is no cross-request transaction.
- **Failure behavior:** If the pool is exhausted, the request fails with 503 after a 5 s acquire timeout. A statement timeout returns 503 with `Retry-After`.
- **Idempotency considerations:** N/A.
- **Security considerations:**
  - Parameterized queries only.
  - An ESLint rule bans `sql.raw` outside `src/db/`.
  - `DATABASE_URL` is never logged.
- **Observability additions:** pg auto-instrumentation spans. Gauges `db_pool_in_use` and `db_pool_waiting`.
- **Tests required:**
  - `with_tx_sets_org_setting` (integration)
  - `generated_types_compile_against_queries` (typecheck)
  - `pool_acquire_timeout_returns_503`
  - `sql_raw_lint_rule_enforced`
- **Benchmarks if applicable:** None.
- **Acceptance criteria:**
  - Running `pnpm db:types` on a clean checkout reproduces `generated.ts` byte for byte (the CI-005 check).
  - The integration test reads `current_setting('app.organization_id')` inside `withTx`.
- **Definition of done:** Global DoD.

---

### API-003 — Tenancy guards and Postgres RLS policies
Status: ☑
> **Implementation note:** Roles and RLS ship as two migrations (`20261002000006_db_roles.sql`, `20261002000007_rls_policies.sql`) and cover the tables that exist today (organizations, provider_installations, memberships, repositories, pull_requests, review_runs, reviewer_runs, candidate_findings, verified_findings, published_findings, finding_feedback, webhook_deliveries); tables from later migrations (jobs excluded by design, snapshots, symbols, audit_log, ...) must repeat the `tenant_isolation` block when they are created. Policies are generated by one idempotent `DO` loop. The stored role set is owner/admin/member/viewer, so `@RequireRole('maintainer')` means member or above. `resolve_org(kind, id)` is joined by `rg_installation_org`, `rg_membership_role` and `rg_user_memberships` (all SECURITY DEFINER, owned by `rg_ops`, `search_path` pinned); the guard resolves from a resource route param, else `organization_id` in query/body, else the only membership of the caller. `rg_migrator` is created but table ownership is not transferred (the migration login owns the schema; FORCE RLS binds owners anyway). The dev login is a superuser, so `DB_APP_ROLE=rg_api` makes `withTx` run `SET LOCAL ROLE rg_api`; the integration tests use the same option. The `tenancy_denied_total` counter is in; the PERF-008 p95 benchmark is deferred. Docs: `docs/security/tenancy.md`.

- **Task ID:** API-003
- **Title:** Tenancy guards + Postgres RLS policies migration
- **Problem:** PRD §112 requires every query that touches repository data to enforce tenant ownership. Legacy is single-tenant.
- **Why it exists:** It addresses risk R9 (critical). Defense in depth: application guards plus database RLS, so that a missing `WHERE` cannot leak data.
- **Scope:**
  - A migration that enables RLS on every tenant table, with policies on `organization_id`.
  - DB roles `rg_migrator` (owner), `rg_api` (RLS enforced), `rg_engine` (RLS enforced; sets the org per job) and `rg_ops` (BYPASSRLS, used only by the reaper and admin scripts).
  - Nest `TenancyGuard` and `@RequireRole('viewer'|'maintainer'|'admin')`.
  - A `memberships` lookup.
- **Explicit non-scope:**
  - Auth (API-004).
  - The isolation test suite (SEC-001).
- **Files/modules expected to change:** `apps/api/src/db/tx.ts` (always sets the org), `apps/api/src/app.module.ts`.
- **New files/modules expected:**
  - `engine/migrations/{seq}_rls_policies.sql`
  - `engine/migrations/{seq}_db_roles.sql`
  - `apps/api/src/tenancy/{tenancy.module.ts,tenancy.guard.ts,roles.decorator.ts,membership.service.ts}`
- **Dependencies:** API-002, DOM-009.
- **Implementation details:**

  ```sql
  ALTER TABLE repositories ENABLE ROW LEVEL SECURITY; ALTER TABLE repositories FORCE ROW LEVEL SECURITY;
  CREATE POLICY tenant_isolation ON repositories
    USING (organization_id = NULLIF(current_setting('app.organization_id', true), '')::uuid)
    WITH CHECK (organization_id = NULLIF(current_setting('app.organization_id', true), '')::uuid);
  -- repeated for: pull_requests, review_runs, reviewer_runs, candidate_findings, findings, finding_evidence, published_findings,
  -- feedback, suppressions, suppression_matches, rule_violations, repository_profiles, repository_configs, snapshots, file_versions,
  -- symbols, graph_edges, synthetic_nodes, symbol_lineage, audit_log, webhook_deliveries(org nullable → policy allows NULL only for rg_ops)
  ```

  - The `jobs` table is **not** RLS-scoped. Claims span tenants and payloads are IDs only. Consumers set the org before they touch tenant rows.
  - `TenancyGuard` resolves `:repoId`, `:reviewId`, `:findingId` or `:pullRequestId` route params to `organization_id` with a narrow lookup (as `rg_ops` via a SECURITY DEFINER function `resolve_org(kind, id)`). It then checks the membership role and stores the org on the request, and `withTx` uses it.
  - An unknown id returns 404 (never 403) to avoid existence oracles.
- **Data model changes:**
  - RLS policies.
  - Roles.
  - `memberships (organization_id, user_id, role)` if DOM-009 did not already create it.
- **API/protocol changes:** All tenant routes return 404 for resources outside the caller's organizations.
- **Concurrency semantics:** The setting is transaction-local (`set_config(..., true)`), so pooled connections never carry the org across requests.
- **Failure behavior:** A missing org setting makes RLS return zero rows (fail closed). Inserts without a matching org fail the `WITH CHECK`.
- **Idempotency considerations:** The migration is idempotent (`CREATE POLICY IF NOT EXISTS` pattern via a `DO` block).
- **Security considerations:**
  - This task is the core security control.
  - `rg_ops` credentials are never given to the API or the engine request paths.
  - The SECURITY DEFINER function returns only `organization_id`.
- **Observability additions:** Counter `tenancy_denied_total{reason=not_member|role}`.
- **Tests required:**
  - `rls_hides_other_org_rows` (integration)
  - `rls_without_setting_returns_nothing`
  - `rls_with_check_blocks_cross_org_insert`
  - `guard_404_for_foreign_repo`
  - `guard_role_maintainer_required`
  - `setting_does_not_leak_across_pooled_connections`
- **Benchmarks if applicable:** RLS overhead on `GET /reviews/:id/findings`: p95 delta < 2 ms (measured in PERF-008 setup).
- **Acceptance criteria:**
  - All tests pass.
  - `\d+` shows RLS enabled and forced on every listed table.
  - The SEC-001 suite builds on these fixtures.
- **Definition of done:** Global DoD, plus `docs/security/tenancy.md`.

---

### API-004 — GitHub OAuth login and session cookie
Status: ☑
> **Implementation note:** `users` already exists from DOM-009 (provider, provider_user_id, login, display_name, email), so the migration `20261002000008_users_sessions.sql` only adds `users.avatar_url` and the `sessions` table (no `github_user_id`/`name` columns). Sessions are global like users (no RLS). Memberships are synced atomically by the SECURITY DEFINER function `rg_sync_user_memberships` (owned by `rg_ops`): it upserts the roles derived from GitHub, deletes memberships for installations the user lost, and never downgrades an admin/owner assigned in the dashboard. Role derivation is `owner` for the personal installation of the user and `member` (maintainer) for an organization installation; GitHub does not expose org admin through `/user/installations`, so admin elevation is a dashboard/DB action for now. Only installations already recorded in `provider_installations` (GH-013) grant access. The OAuth client is plain `fetch` in `src/auth/github-oauth.client.ts` (the provider port has no "user installations" operation, and Octokit is confined to providers/github); `GITHUB_OAUTH_URL` (default https://github.com) and optional `GITHUB_OAUTH_REDIRECT_URI` are new env vars. Single-use state keeps the PKCE verifier in Redis (`rg:oauth:state:{state}`, GETDEL) while the signed `rg_oauth_state` cookie only binds the state to the browser. SessionGuard then CsrfGuard are global guards in `AuthModule` (before the TenancyGuard); `@Public()` exempts health, webhooks and OAuth login/callback, and `/internal/**` is skipped by path. Logout clears the Redis session cache entry so revocation is immediate; a revocation made directly in the database is honoured within the 60 s cache window. Tests: DB-free unit tests in `test/auth`, database/Redis flows in `integration/auth.int.spec.ts` against the fake GitHub OAuth server (`test/helpers/fake-oauth.ts`).

- **Task ID:** API-004
- **Title:** GitHub OAuth login + session cookie
- **Problem:** Humans need to authenticate to the dashboard. Legacy `web.rs:219-341` exposed **unauthenticated POST routes that published a review under the operator's identity**, which is a CSRF path (audit §1).
- **Why it exists:** It authenticates users and maps them to organizations and memberships. It closes the CSRF class by design.
- **Scope:**
  - The GitHub App user-authorization flow (OAuth web flow with the App's client id and secret).
  - User upsert. Organization memberships derived from the App installations the user can access (`GET /user/installations`).
  - Session: a signed JWT cookie that references a `sessions` row, so sessions are revocable.
  - CSRF protection on all mutating routes.
  - Logout.
- **Explicit non-scope:**
  - SSO/SAML.
  - Personal access tokens.
  - Fine-grained role editing UI (WEB-008).
- **Files/modules expected to change:** `apps/api/src/app.module.ts`.
- **New files/modules expected:**
  - `apps/api/src/auth/{auth.module.ts,auth.controller.ts,session.service.ts,session.guard.ts,csrf.guard.ts,github-oauth.client.ts}`
  - `engine/migrations/{seq}_users_sessions.sql`
- **Dependencies:** API-001, API-002, API-003, API-006 (the provider port gives the user's installations), GH-001.
- **Implementation details:**
  - Routes:
    - `GET /api/v1/auth/github/login`: redirect with `state` (random 32 bytes, stored in a signed short-lived cookie `rg_oauth_state`) and PKCE S256.
    - `GET /api/v1/auth/github/callback`: verify `state`, exchange the code, fetch `/user` and `/user/installations`, upsert `users` and `memberships`, create a `sessions` row, set the cookie.
    - `POST /api/v1/auth/logout`
    - `GET /api/v1/auth/me`
  - Cookie `rg_session`: HttpOnly, Secure (except `NODE_ENV=development`), SameSite=Lax, Path=/, 12 h. JWT HS256 `{ sub: user_id, sid, iat, exp }`.
  - `SessionGuard` verifies the JWT and checks `sessions.revoked_at IS NULL AND expires_at > now()`, cached in Redis for 60 s.
  - CSRF: the double-submit token `rg_csrf` cookie (not HttpOnly) must equal the `X-CSRF-Token` header on POST/PUT/PATCH/DELETE. The `Origin` header must equal `WEB_ORIGIN`. Both are required.
  - The user's GitHub OAuth token is **not persisted**. It is used only during the callback to list installations.
- **Data model changes:** `users (id, github_user_id UNIQUE, login, name, avatar_url)` and `sessions (id, user_id, created_at, expires_at, revoked_at, user_agent_hash)`.
- **API/protocol changes:** The auth routes above. All other `/api/v1/**` routes (except health, webhooks and internal) require a session.
- **Concurrency semantics:** Session creation is per login. Concurrent callbacks with the same `state` are rejected by single-use state (Redis `GETDEL`).
- **Failure behavior:** A state mismatch or replay returns 400 and no session. A GitHub API failure redirects to `/login?error=github_unavailable`.
- **Idempotency considerations:** Users and memberships are upserted on the natural keys.
- **Security considerations:**
  - CSRF double-submit plus an Origin check.
  - PKCE.
  - Single-use state.
  - No OAuth token storage.
  - Session revocation on logout.
  - Membership refreshed on each login.
- **Observability additions:** Counters `auth_logins_total{result}` and `csrf_rejections_total`.
- **Tests required:**
  - `oauth_state_mismatch_rejected`
  - `oauth_state_single_use`
  - `callback_creates_session_and_memberships` (fake GitHub)
  - `mutation_without_csrf_header_403`
  - `mutation_with_foreign_origin_403`
  - `revoked_session_401`
  - `oauth_token_not_persisted`
  - `legacy_csrf_regression_cross_site_form_post_rejected`
- **Benchmarks if applicable:** None.
- **Acceptance criteria:**
  - Login works end to end against the fake GitHub OAuth endpoints (DEV-005).
  - A cross-site form POST to any mutating route is rejected. This is the regression test for the legacy `web.rs` CSRF path.
- **Definition of done:** Global DoD.

---

### API-005 — Service-to-service auth (engine ↔ API)
Status: ☑
> **Implementation note:** TS side only (`apps/api/src/internal/{service-token.ts,service-auth.guard.ts,internal.module.ts}`); the Rust signer/verifier (`review-core::service_auth`, `pipeline::service_token`, the Axum extractor) is left to the engine work. The cross-language golden vectors live in `packages/contracts/fixtures/service-token/`: `ts-signed.json` is produced by the TS signer (the Rust verifier must accept it) and `rust-signed.json` is built with the `jsonwebtoken` field layout by a test helper and must be regenerated from the real Rust signer when it lands; no `service-token.json` schema was added (it would be picked up by the contracts generator, which is Rust-sourced). `SERVICE_JWT_KEYS` (`kid:base64,...`) is optional and falls back to `SERVICE_JWT_SECRET` as kid `default`. `ServiceAuthGuard` is a global guard that acts only on `/internal/**` (mounted outside the `/api/v1` prefix): a route without a `@ServiceAuth({scopes, repoParam?})` policy is denied (403), so a new internal route is authenticated by construction. The jti cache is Redis `SET rg:jti:{jti} NX EX 120`; a Redis outage fails closed with 503. The route-table test enumerates routes of a fake internal controller since the real internal routes (GH-006) do not exist yet. A shared Redis provider (`RedisModule`) and a counter/gauge helper (`common/metrics.ts`, OTel meter plus a local tally for tests) were added; `pnpm test` in apps/api now runs jest with `--experimental-vm-modules` so ESM-only dependencies (`jose`, `@octokit/*`) load.

- **Task ID:** API-005
- **Title:** Service-to-service auth (engine↔api, HMAC or signed JWT)
- **Problem:** Workers call the API credential broker (GH-006), and the API calls review-engine (API-011/013). Both need mutual authentication without user sessions.
- **Why it exists:** The internal endpoints hand out clone tokens and graph data, which are high-value targets.
- **Scope:**
  - Signed short-lived JWTs (HS256, shared secret with `kid` rotation), implemented in TS (`jose`) and Rust (`jsonwebtoken`), with a shared claim contract.
  - Nest `ServiceAuthGuard`.
  - An Axum `ServiceAuth` extractor.
- **Explicit non-scope:**
  - mTLS (revisit on GCP with Cloud Run identity tokens).
  - User auth.
- **Files/modules expected to change:**
  - `engine/crates/pipeline/src/http.rs` (outbound client adds the token)
  - `engine/apps/review-engine/src/main.rs`
- **New files/modules expected:**
  - `apps/api/src/internal/{service-auth.guard.ts,service-token.ts}`
  - `engine/crates/review-core/src/service_auth.rs` (claims type only, no I/O)
  - `engine/crates/pipeline/src/service_token.rs`
  - `packages/contracts/schemas/service-token.json`
- **Dependencies:** API-001, DOM-001.
- **Implementation details:**
  - Claims: `{ iss: "rg-api"|"rg-worker"|"rg-engine", aud: "rg-api"|"rg-engine", sub: service instance id, scope: ["clone-credentials"|"graph:read"], org?: uuid, repo?: uuid, iat, exp (≤ iat+60), jti }`.
  - Header `kid`. Secrets come from `SERVICE_JWT_KEYS = kid1:base64,kid2:base64`. The first key signs and all keys verify (rotation).
  - Verification rules:
    - `aud` must match the receiver.
    - The clock-skew allowance is 30 s.
    - The `jti` replay cache is Redis `SET rg:jti:{jti} NX EX 120` on the API side and an in-memory LRU of 10k entries in the engine.
  - Scopes are checked per route. For the credential broker, the `repo` claim must equal `:id`.
- **Data model changes:** None.
- **API/protocol changes:** Header `Authorization: Bearer <service-jwt>` on `/internal/**` (API) and `/internal/v1/**` (engine).
- **Concurrency semantics:** Stateless apart from the jti cache.
- **Failure behavior:** Any verification failure returns 401 with no detail in the body. The reason is logged and the token is not.
- **Idempotency considerations:** N/A. A replayed `jti` is rejected by design.
- **Security considerations:**
  - Tokens last ≤ 60 s.
  - Each token is scoped to one repository for credentials.
  - Keys come from env or the secret manager and are never logged.
  - Constant-time signature comparison is provided by the libraries.
- **Observability additions:** Counter `service_auth_failures_total{reason,route}`.
- **Tests required:**
  - `ts_signs_rust_verifies` (cross-language golden vector in `packages/contracts/fixtures/service-token/`)
  - `rust_signs_ts_verifies`
  - `expired_token_401`
  - `wrong_audience_401`
  - `replayed_jti_401`
  - `repo_claim_mismatch_403`
  - `rotation_old_kid_still_verifies`
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** The golden vectors pass in both languages, and every `/internal` route rejects unauthenticated calls (route-table test).
- **Definition of done:** Global DoD.

---

### API-006 — Provider ports RepositoryProvider and ReviewPublisher
Status: ☑
> **Implementation note:** Ports live in `apps/api/src/providers/ports/` (an `index.ts` barrel and a `provider-resolver.ts` port were added) and `apps/api/src/providers/provider.registry.ts`. Deviations: the ESLint config is the flat `apps/api/eslint.config.mjs` (not `.eslintrc.cjs`); the domain-module rule is a `no-restricted-imports` regex that allows only `providers/ports`, so domain modules obtain providers through the `PROVIDER_RESOLVER` port (implemented by `ProviderRegistry`) rather than importing the registry class. `RepositoryProvider.normalizeEvent` is async and returns `NormalizeResult` (`ProviderEvent | Ignored{reason}`) instead of `ProviderEvent | null`, because a `/review` command needs the commenter's permission lookup (GH-004 fails closed). The `ProviderEvent` union (head event, closed, review command) and the shared `Secret<T>` wrapper (`common/secret.ts`) are defined here. The "tsd" type test is a `@ts-expect-error` assertion compiled by `pnpm typecheck`; the lint fixture test spawns ESLint on stdin with a planted import. `CheckRunConclusion` deliberately omits `failure`/`action_required` so a check run cannot block a merge.

- **Task ID:** API-006
- **Title:** Provider ports RepositoryProvider/ReviewPublisher
- **Problem:** Invariant 4 says provider-specific code cannot leak into review domain logic, and PRD §11/§78 list the provider responsibilities.
- **Why it exists:** GitHub is first. GitLab and Bitbucket (MP-001/002) must be able to plug in without touching the reviews, publisher or findings modules.
- **Scope:**
  - TS interfaces and a normalized domain model (`ProviderPullRequest`, `ProviderChangedFile`, `ProviderEvent`, `PublishRequest`, `PublishResult`).
  - A Nest injection token per provider kind.
  - A registry keyed by `repositories.provider`.
  - ESLint boundaries.
- **Explicit non-scope:** The GitHub implementation (GH-*).
- **Files/modules expected to change:** `apps/api/.eslintrc.cjs` (restricted imports).
- **New files/modules expected:**
  - `apps/api/src/providers/ports/{repository-provider.port.ts,review-publisher.port.ts,provider-event.ts,types.ts}`
  - `apps/api/src/providers/provider.registry.ts`
- **Dependencies:** API-001, DOM-004 (domain entities, mirrored through contracts).
- **Implementation details:**

  ```ts
  interface RepositoryProvider {
    kind: 'github' | 'gitlab' | 'bitbucket';
    getRepository(ref: RepoRef): Promise<ProviderRepository>;
    getPullRequest(ref: PrRef): Promise<ProviderPullRequest>;           // base/head sha, author, draft, labels, state
    listChangedFiles(ref: PrRef): AsyncIterable<ProviderChangedFile>;    // paginated
    getCommit(ref: RepoRef, sha: string): Promise<ProviderCommit>;
    issueCloneCredential(ref: RepoRef, ttlSeconds: number): Promise<CloneCredential>; // read-only, single repo
    verifyWebhook(headers: Record<string,string>, rawBody: Buffer): WebhookVerification;
    normalizeEvent(headers, body): ProviderEvent | null;
    getActorPermission(ref: RepoRef, login: string): Promise<'admin'|'write'|'read'|'none'>;
  }
  interface ReviewPublisher {
    publishReview(req: PublishRequest): Promise<PublishResult>;      // ONE atomic review; event is always 'COMMENT'
    upsertCheckRun(req: CheckRunRequest): Promise<{ checkRunId: string }>;
    findExistingReview(ref: PrRef, marker: string): Promise<ExistingReview | null>;
    resolveThreads(ref: PrRef, providerCommentIds: string[]): Promise<ResolveResult>;
  }
  type ReviewEvent = 'COMMENT';    // the type admits no APPROVE/REQUEST_CHANGES/merge (INV-011/012)
  ```

  ESLint: `src/{reviews,publisher,findings,repositories}/**` may import only `providers/ports`.
- **Data model changes:** None.
- **API/protocol changes:** Internal TS contracts only.
- **Concurrency semantics:** N/A (interfaces).
- **Failure behavior:** Port methods throw the typed `ProviderError { kind: 'transient'|'rate_limited'|'not_found'|'forbidden'|'invalid', retryAfterMs? }`. Callers branch on `kind`, never on HTTP codes.
- **Idempotency considerations:** `findExistingReview(marker)` exists specifically so the publisher can be idempotent (GH-009).
- **Security considerations:** `CloneCredential` has a TTL and is typed as `Secret<string>`. Its `toJSON` returns `"[redacted]"`.
- **Observability additions:** None (implementations add spans).
- **Tests required:**
  - `eslint_blocks_octokit_in_domain_modules` (lint fixture)
  - `review_event_type_is_comment_only` (tsd type test)
  - `secret_tojson_redacted`
  - `registry_resolves_by_provider_kind`
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** The lint rule fails on a planted import, and the type test proves `'APPROVE'` is not assignable to `ReviewEvent`.
- **Definition of done:** Global DoD.

---

### API-007 — TS JobQueue adapter (same PG jobs table, transactional enqueue)
Status: ◐

> **Implementation note:** The `jobs` table did not exist yet, so this task adds
> `engine/migrations/20261008100000_jobs.sql` exactly as specified by PIPE-001 (the Rust lane reuses
> it). The adapter lives in `src/jobs/{job-queue.ts,pg-job-queue.ts,consumer.ts,job-store.ts,payloads.ts,jobs.module.ts}`
> and also implements the GH-013 `JobCanceller`. Payload schemas are zod schemas in `payloads.ts`
> (the contracts package exports JSON Schema from Rust types, and the Rust payload structs arrive
> with PIPE-001). LISTEN uses a dedicated pool client and polling covers lost notifications.
> Concurrency defaults live in `DEFAULT_CONCURRENCY` (`review-publish: 4`) and can be overridden per
> `consume` call. `ts_and_rust_claim_interoperate` runs the target-architecture claim statement
> verbatim on a plain connection; running it through the Rust engine test binary waits for
> PIPE-001, which is why the task stays ◐.

- **Task ID:** API-007
- **Title:** TS JobQueue adapter (same PG jobs table, transactional enqueue)
- **Problem:** NestJS produces `repository-index`, `incremental-index` and `pr-review` jobs, and consumes `review-publish`. ADR-012 requires enqueue in the same transaction as the state change.
- **Why it exists:** It removes dual-write bugs, such as a webhook stored with no job, and makes the TS side of the shared queue port.
- **Scope:**
  - `JobQueue` with `enqueue(trx, job)`, `cancelWhere(trx, predicate)` and `consume(queue, handler, opts)`.
  - A lease heartbeat.
  - LISTEN/NOTIFY wakeups.
  - Graceful release on shutdown.
  - Payload schemas from contracts.
- **Explicit non-scope:**
  - The Rust consumer (PIPE-001/002).
  - The reaper (PIPE-002 owns it, running in the worker).
- **Files/modules expected to change:** `apps/api/src/app.module.ts`.
- **New files/modules expected:**
  - `apps/api/src/jobs/{jobs.module.ts,job-queue.ts,pg-job-queue.ts,consumer.ts,payloads.ts}`
- **Dependencies:** API-002, PIPE-001 (jobs table and claim SQL), OBS-003 (traceparent).
- **Implementation details:**
  - Enqueue:

    ```sql
    INSERT INTO jobs (id, queue, idempotency_key, payload, priority, max_attempts, run_after, trace_parent, organization_id)
    VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)
    ON CONFLICT (idempotency_key) DO NOTHING RETURNING id;
    SELECT pg_notify('jobs_' || $2, $1);   -- same trx; delivered on commit
    ```

    It returns `{ id, created: boolean }`.
  - Consume: the target-arch §5 claim SQL. The heartbeat runs every `lease/3` (`UPDATE jobs SET locked_until=now()+$lease WHERE id=$1 AND locked_by=$w`). Completion is `state='succeeded'`. A failure sets `run_after = now() + backoff(attempts)` with jitter, or `dead` once `attempts >= max_attempts`.
  - Concurrency per queue comes from config (`review-publish: 4`).
  - Payloads are zod schemas generated from the contracts JSON Schema and carry IDs only, for example `{ review_run_id }`.
  - The `trace_parent` column is filled from the active span (OBS-003).
- **Data model changes:** None (uses PIPE-001's `jobs`). It adds an expression index if missing: `CREATE INDEX jobs_review_run_idx ON jobs ((payload->>'review_run_id')) WHERE state IN ('queued','running')` (owned by SUP-001's migration).
- **API/protocol changes:** None external.
- **Concurrency semantics:**
  - At-least-once delivery.
  - SKIP LOCKED claims.
  - Handlers must be idempotent.
  - On shutdown: stop claiming, wait up to 20 s for handlers, then set unfinished jobs back to `queued` with `locked_by=NULL`.
- **Failure behavior:** A handler throwing `ProviderError{rate_limited}` re-queues with `run_after = retryAfter` and does **not** increment attempts beyond the claim. Other errors use backoff.
- **Idempotency considerations:** `UNIQUE(idempotency_key)`. The key formats follow PRD §76, for example `pr-review:github:{repo_id}:{pr}:{head_sha}` and `publish:{review_run_id}`.
- **Security considerations:** Payloads carry no secrets or source (a test asserts that the payload schema has no free-text fields).
- **Observability additions:**
  - Span `job_enqueue`, and span `job_process{queue}` linked to `trace_parent`.
  - Gauge `queue_depth{queue}`.
  - Histograms `queue_wait_seconds{queue}` and `worker_duration_seconds{queue}`.
- **Tests required:**
  - `enqueue_rolls_back_with_transaction`
  - `duplicate_idempotency_key_returns_created_false`
  - `notify_delivered_after_commit_only`
  - `heartbeat_extends_lease`
  - `shutdown_releases_unfinished_job`
  - `rate_limited_requeues_with_retry_after`
  - `dead_after_max_attempts`
  - `ts_and_rust_claim_interoperate` (integration: TS enqueue, Rust claim via the engine test binary)
- **Benchmarks if applicable:** Enqueue-claim round trip p95 < 50 ms locally (smoke in CI-007).
- **Acceptance criteria:** The interop test passes. A rolled-back transaction leaves no job row, and a duplicate enqueue is a no-op.
- **Definition of done:** Global DoD.

---

### API-008 — Repositories API (PRD §106)
Status: ☐

- **Task ID:** API-008
- **Title:** Repositories API (PRD §106)
- **Problem:** Operators need to onboard repositories, trigger initialization and rebuilds, and read status and profile.
- **Why it exists:** The PRD §106 endpoints, plus what the web screens (WEB-003/004) need.
- **Scope:**
  - `POST /repositories` (enable an installation repository for review)
  - `GET /repositories`
  - `GET /repositories/:id`
  - `POST /repositories/:id/initialize` (enqueue `repository-index`)
  - `GET /repositories/:id/status` (fingerprint, versions, last snapshot, index state, config_hash, validation errors)
  - `GET /repositories/:id/profile` (PROF-001 JSON)
  - `POST /repositories/:id/graph/rebuild` (enqueue a forced full index)
  - `PATCH /repositories/:id/settings` (enabled, target branches, reviewers toggles override)
- **Explicit non-scope:**
  - Editing `.review/config.yaml` (it is repository-owned).
  - Graph queries (API-011).
- **Files/modules expected to change:** `apps/api/src/app.module.ts`.
- **New files/modules expected:**
  - `apps/api/src/repositories/{repositories.module.ts,repositories.controller.ts,repositories.service.ts,dto/*.ts}`
- **Dependencies:** API-003, API-004, API-007, GH-005 (repository sync), PROF-001, POL-002.
- **Implementation details:**
  - DTOs use zod with `nestjs-zod`, and the responses are typed from contracts.
  - Status assembles `snapshots` (latest full and latest delta), `jobs` (active index job), `repository_configs.validation` and `repository_profiles.computed_at`.
  - Initialize idempotency key: `repo-index:{repository_id}:{default_branch_head_sha}`.
  - Rebuild key: `repo-rebuild:{repository_id}:{head_sha}:{yyyyMMddHH}`, limited to 1 per hour.
  - Settings live in `repository_settings (repository_id PK, enabled, target_branches text[], skip_drafts bool default true, skip_bots bool default true, reviewer_overrides jsonb)`. These port the legacy guard options from `github.rs:305-347`.
- **Data model changes:** `repository_settings` (migration, with RLS).
- **API/protocol changes:** The routes above, under `/api/v1`. The OpenAPI document is generated by `@nestjs/swagger` and written to `packages/contracts/openapi/api.json`.
- **Concurrency semantics:** Enqueue happens in the same transaction as the state rows. A double-click on initialize produces a single job (idempotency key).
- **Failure behavior:**
  - 404 for foreign repositories (API-003).
  - 409 when an index is already running for the same head, with the existing job id returned.
  - 429 for rebuilds over the limit.
- **Idempotency considerations:** Idempotency keys as above. `PATCH` is naturally idempotent.
- **Security considerations:**
  - `viewer` for GETs. `maintainer` for initialize, rebuild and settings.
  - Settings changes write `audit_log` (SEC-008).
- **Observability additions:** Spans per handler (auto). Counter `repository_index_requests_total{kind}`.
- **Tests required:**
  - `create_repository_requires_installation_access`
  - `initialize_enqueues_once`
  - `rebuild_rate_limited`
  - `status_contains_fingerprint_and_versions`
  - `profile_404_before_first_index`
  - `settings_change_audited`
  - `openapi_snapshot`
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** All routes appear in the OpenAPI snapshot. The integration tests pass against compose PG, and the status response validates against its contracts schema.
- **Definition of done:** Global DoD.

---

### API-009 — Reviews API
Status: ☑

> **Implementation note:** The manual trigger calls SUP-001's `SupersessionService.startInTx` at
> the stored head with the suffix `manual:{yyyyMMddHHmm}` (one per head and minute), in the same
> transaction as its audit row; an active run of the same head is returned rather than replaced
> (SUP-001 semantics), so "supersedes the running run" applies to runs of an older head. The
> response's `job_id` is the run's `pr-review` job. Stage timings, risk assessment, change summary and coverage are read
> from `review_runs.provenance` (`stages`, `risk_assessment`, `change_summary`, `coverage`) until
> PIPE-007 records transitions; they are empty or null before that.

- **Task ID:** API-009
- **Title:** Reviews API
- **Problem:** Users need to list pull requests and review runs, inspect run state, coverage and degradation, and trigger a manual review.
- **Why it exists:** PRD §106 (`POST /pull-requests/:id/review`, `GET /pull-requests/:id/reviews/:reviewId`). It feeds WEB-005 and WEB-006.
- **Scope:**
  - `GET /repositories/:id/pull-requests?state=open|closed&cursor=`
  - `GET /pull-requests/:id`
  - `POST /pull-requests/:id/review` (manual trigger at the current head, using SUP-001)
  - `GET /pull-requests/:id/reviews`
  - `GET /pull-requests/:id/reviews/:reviewId` (state, stages with timings, reviewer runs with outcome, prompt and model versions, risk assessment, change summary, coverage: reviewed and unreviewed clusters, degraded reasons, counts by lifecycle state, `trace_id`)
  - `POST /reviews/:id/cancel`
- **Explicit non-scope:**
  - Findings detail (API-010).
  - Re-publishing (never offered).
- **Files/modules expected to change:** `apps/api/src/app.module.ts`.
- **New files/modules expected:**
  - `apps/api/src/reviews/{reviews.module.ts,reviews.controller.ts,reviews.service.ts,supersession.service.ts,dto/*.ts}`
- **Dependencies:** API-003, API-007, SUP-001, PIPE-007 (state machine rows), PIPE-008 (degraded), DOM-008.
- **Implementation details:**
  - Manual review calls `SupersessionService.startReview(prId, headSha, trigger='manual')`, the same code path as webhooks.
  - Cancel runs `UPDATE review_runs SET state='CANCELLED' WHERE id=$1 AND state NOT IN (terminal)` and cancels the queued jobs.
  - Pagination is cursor-based on `(created_at, id)`, with a limit of at most 100.
  - The response includes `completeness: { reviewers_planned, reviewers_succeeded, reviewers_failed[], not_executed[] }`. These values are computed from `reviewer_runs` rows and never self-reported (INV-013).
- **Data model changes:** None.
- **API/protocol changes:** The routes above.
- **Concurrency semantics:**
  - Manual trigger and webhook use the same transaction (SUP-001), so their order does not matter.
  - Cancel is a CAS. When a cancel races with publish, SUP-003's gate sees CANCELLED and does not post.
- **Failure behavior:**
  - A manual trigger on a closed PR returns 409.
  - Cancel on a terminal run returns 409 with the current state.
- **Idempotency considerations:** The manual trigger uses the key `pr-review:{provider}:{repo}:{pr}:{head}:manual:{minute}` (one per minute). Cancel is idempotent.
- **Security considerations:** `viewer` for reads, `maintainer` for trigger and cancel. Both mutations write audit entries.
- **Observability additions:** The response exposes `trace_id` so the UI can deep-link to OpenObserve. Counter `manual_reviews_total`.
- **Tests required:**
  - `manual_review_supersedes_running_run`
  - `cancel_running_run_cancels_jobs`
  - `cancel_terminal_409`
  - `review_detail_completeness_from_rows`
  - `degraded_reason_listed`
  - `pagination_cursor_stable`
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** Review detail for a degraded fixture run lists the failed reviewer, and `completeness` matches the rows.
- **Definition of done:** Global DoD.

---

### API-010 — Findings API
Status: ☐

- **Task ID:** API-010
- **Title:** Findings API
- **Problem:** Users and operators need findings by lifecycle state, and the full explainability trace from PR change to published finding (PRD §85/§86).
- **Why it exists:** It feeds WEB-006/007 and the evaluation work, since suppressed findings are evaluation data.
- **Scope:**
  - `GET /reviews/:id/findings?state=published|verified|suppressed|all&severity=&reviewer=`
  - `GET /findings/:id` (anchor, symbols, explanation, evidence items, severity, computed confidence with components, publication info)
  - `GET /findings/:id/trace`: change → symbol → context item refs → reviewer@version → candidate → per-stage verification outcomes and evidence → dedup merges → effective policy → publication. It never includes model reasoning or prompts.
- **Explicit non-scope:** Feedback (API-012).
- **Files/modules expected to change:** `apps/api/src/app.module.ts`.
- **New files/modules expected:** `apps/api/src/findings/{findings.module.ts,findings.controller.ts,findings.service.ts,trace.service.ts}`
- **Dependencies:** API-003, VER-002 (evidence persisted), DED-003 (merge records), POL-004, GH-009 (published_findings).
- **Implementation details:**
  - Confidence components are read from `findings.confidence_components jsonb` (VER-009).
  - Evidence items are typed (DOM-007). Code excerpts are **not** stored. The trace returns `{path, start_line, end_line, snapshot_id}`, and the UI fetches excerpts through API-011 with redaction.
  - Severity ordering is `critical > high > medium > low > info`.
- **Data model changes:** None.
- **API/protocol changes:** The routes above, with JSON Schemas `FindingDetail` and `FindingTrace` in contracts.
- **Concurrency semantics:** Read-only.
- **Failure behavior:** 404 for foreign or unknown findings. A trace with missing stage rows (an older run) returns the available items plus `incomplete: true`.
- **Idempotency considerations:** N/A.
- **Security considerations:**
  - Never returns prompts, model raw output or source bodies.
  - A test asserts that the response schema has no `prompt` or `raw_output` field.
- **Observability additions:** None beyond auto spans.
- **Tests required:**
  - `findings_filter_by_state`
  - `suppressed_findings_have_reason`
  - `trace_contains_all_stages`
  - `trace_excludes_prompt_and_raw_output`
  - `foreign_finding_404`
- **Benchmarks if applicable:** `GET /reviews/:id/findings` p95 < 100 ms for 200 findings.
- **Acceptance criteria:** The trace for the auth-bypass E2E finding lists the path `UserController.update → AdminService.updateUser → AuthService.authorize`, reviewer `security:v1`, and the verification stages, matching PRD §86.
- **Definition of done:** Global DoD.

---

### API-011 — Graph proxy API
Status: ☐

- **Task ID:** API-011
- **Title:** Graph proxy API
- **Problem:** The UI's graph explorer and Finding Detail need graph queries. review-engine is internal and has no notion of users.
- **Why it exists:** Target-arch §5 `graph` module: an authorized proxy that adds the tenant scope.
- **Scope:**
  - `GET /repositories/:id/graph/symbols?q=&kind=&snapshot=`
  - `GET /repositories/:id/graph/symbols/:key`
  - `GET .../symbols/:key/neighbors?dir=in|out&kinds=&min_confidence=`
  - `POST /repositories/:id/graph/subgraph` (`{seeds, depth≤3, kinds, max_nodes≤500}`)
  - `GET /repositories/:id/graph/path?from=&to=&max_depth≤6`
  - `GET /reviews/:id/impact/:symbolKey`
  - `GET /repositories/:id/source?path=&start=&end=&snapshot=` (redacted excerpt, ≤200 lines)
- **Explicit non-scope:** Graph computation, which lives in the engine (API-013).
- **Files/modules expected to change:** `apps/api/src/app.module.ts`.
- **New files/modules expected:** `apps/api/src/graph/{graph.module.ts,graph.controller.ts,engine.client.ts}`
- **Dependencies:** API-003, API-005, API-013, SEC-004 (redaction for excerpts).
- **Implementation details:**
  - `EngineClient` uses `undici` with keep-alive and a 5 s timeout. It adds a service JWT with `scope=graph:read`, `org` and `repo`.
  - The default snapshot is the latest full snapshot of the default branch. Any snapshot id passed in is validated as belonging to the repository.
  - Redis hot cache key: `rg:gq:{repo}:{snapshot}:{sha256(query)}`, TTL 300 s. Snapshots are immutable, so this is safe.
- **Data model changes:** None.
- **API/protocol changes:** The routes above.
- **Concurrency semantics:** Stateless proxy. Concurrent identical queries may both hit the engine (acceptable).
- **Failure behavior:**
  - Engine down returns 503.
  - Budget truncation passes `truncated: true` through.
  - `max_nodes > 500` returns 400.
- **Idempotency considerations:** GETs. The subgraph POST is side-effect free.
- **Security considerations:**
  - The tenant scope is injected server-side and is never taken from the client.
  - Source excerpts are redacted and length-capped.
  - The excerpt requests are audited (SEC-008: source access).
- **Observability additions:** Span `engine_proxy{route}`. Counter `graph_queries_total{route,cached}`.
- **Tests required:**
  - `subgraph_max_nodes_enforced`
  - `foreign_snapshot_rejected`
  - `tenant_claims_added_server_side`
  - `excerpt_redacted`
  - `cache_hit_on_repeat`
- **Benchmarks if applicable:** Proxy overhead p95 < 10 ms over the engine.
- **Acceptance criteria:** The UI explorer (GX-002) works against fixtures through the proxy. The redaction test shows `KEY="<redacted>"`.
- **Definition of done:** Global DoD.

---

### API-012 — Feedback API
Status: ☐

- **Task ID:** API-012
- **Title:** Feedback API
- **Problem:** PRD §70 feedback (useful, false positive, already handled, not relevant, intentional) has no storage or endpoint.
- **Why it exists:** It is the input to the acceptance and FP KPIs (PRD §116–117), QB-002 and calibration (QB-004).
- **Scope:**
  - `POST /findings/:id/feedback` `{ verdict, comment?, create_suppression?: { kind: 'fingerprint'|'symbol'|'path', reason } }`
  - `GET /findings/:id/feedback`
  - `GET /repositories/:id/feedback/summary?since=`
  - Ingestion of GitHub reactions and replies on our comments as weak signals (GH-011 path) is recorded with `source='provider'`.
- **Explicit non-scope:** Learning from feedback (HIST-*).
- **Files/modules expected to change:** `apps/api/src/findings/findings.module.ts`.
- **New files/modules expected:**
  - `apps/api/src/findings/{feedback.controller.ts,feedback.service.ts}`
  - `engine/migrations/{seq}_feedback.sql`
- **Dependencies:** API-010, POL-006 (suppression creation), SEC-008.
- **Implementation details:**

  ```sql
  CREATE TABLE feedback (id uuid PK, organization_id uuid NOT NULL, repository_id uuid NOT NULL, finding_id uuid NOT NULL REFERENCES findings(id),
    user_id uuid NULL, source text CHECK (source IN ('web','provider')), verdict text CHECK (verdict IN
    ('useful','false_positive','already_handled','not_relevant','intentional')), comment text, created_at timestamptz DEFAULT now(),
    UNIQUE (finding_id, user_id, source));
  ```

  - Repeat feedback from the same user upserts: the latest verdict wins, and history goes to `audit_log`.
  - `create_suppression` is allowed only with the `intentional` or `not_relevant` verdicts and requires `maintainer`.
- **Data model changes:** `feedback` (with RLS).
- **API/protocol changes:** The routes above.
- **Concurrency semantics:** The upsert on the unique key.
- **Failure behavior:** 422 for an invalid verdict, and 403 for a suppression without `maintainer`.
- **Idempotency considerations:** The unique key makes a repeated POST an update.
- **Security considerations:**
  - The comment is limited to 2,000 characters, stored as plain text and HTML-escaped by the UI.
  - Feedback is audited.
- **Observability additions:** Counter `finding_feedback_total{verdict,reviewer}`, which feeds `finding_acceptance_rate` (OBS-005).
- **Tests required:**
  - `feedback_upsert_latest_wins`
  - `suppression_requires_maintainer`
  - `suppression_only_for_intentional_or_not_relevant`
  - `feedback_summary_rates`
  - `feedback_audited`
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** WEB-009 actions persist rows, the summary endpoint returns acceptance and FP rates, and the suppression created is matched on the next run (POL-006 test).
- **Definition of done:** Global DoD.

---

### API-013 — review-engine Axum internal API
Status: ☐

- **Task ID:** API-013
- **Title:** review-engine Axum internal API (Rust) for graph/impact/subgraph queries
- **Problem:** Graph traversals run only in the Rust in-memory graph (ADR-014). The API and the UI need synchronous, read-only access to them.
- **Why it exists:** It is the synchronous half of the TS↔Rust boundary (target-arch §1).
- **Scope:**
  - The `engine/apps/review-engine` binary.
  - Axum on `:8081`, with routes under `/internal/v1/`:
    - `GET health`
    - `GET repos/{repo}/snapshots/{snap}/symbols?q&kind&limit≤50`
    - `GET .../symbols/{key}`
    - `GET .../symbols/{key}/neighbors`
    - `POST .../subgraph`
    - `GET .../path`
    - `GET reviews/{run}/impact/{key}`
    - `GET repos/{repo}/snapshots/{snap}/source?path&start&end`
  - A per-snapshot `Arc<Graph>` LRU loaded from `GraphStore`.
  - The ServiceAuth extractor (API-005).
- **Explicit non-scope:**
  - Writes of any kind.
  - Job processing (that is the worker).
- **Files/modules expected to change:** `engine/Cargo.toml` (workspace member).
- **New files/modules expected:**
  - `engine/apps/review-engine/src/{main.rs,routes/*.rs,state.rs,auth.rs,dto.rs}`
- **Dependencies:** CG-007 (queries), GS-005 (PG load), IMP-001..IMP-008, API-005, OBS-001.
- **Implementation details:**
  - The DTOs are serde types that also derive `schemars`, exported to contracts so the TS client is generated.
  - The graph LRU holds `ENGINE_GRAPH_CACHE=8` snapshots by default. A miss loads through a single-flight (`tokio::sync::OnceCell` per key) so concurrent requests share one load.
  - Every query passes an explicit budget (`max_nodes`, `max_depth`) and returns `truncated`.
  - Tenant check: `repo` in the path must equal the token's `repo` claim, and the snapshot must belong to the repository (PG check, cached).
  - Source excerpts are read from the object store or a bare mirror at `snapshot.commit_sha`, run through `telemetry::redact`, and capped at 200 lines.
  - Shutdown: graceful with a 10 s drain.
- **Data model changes:** None.
- **API/protocol changes:** The internal routes above and the OpenAPI document `packages/contracts/openapi/engine.json` (via `utoipa`).
- **Concurrency semantics:**
  - Read-only shared `Arc<Graph>`.
  - A Tokio multi-threaded runtime. CPU-heavy traversals run in `spawn_blocking` with a semaphore sized to the CPU count.
- **Failure behavior:**
  - 503 while a snapshot is loading beyond 2 s, with `Retry-After: 2`; the load continues.
  - 404 for an unknown snapshot.
  - 400 for budget limits over the maximum.
- **Idempotency considerations:** All routes are pure reads.
- **Security considerations:**
  - The service listens on the internal network only. Compose and Cloud Run ingress are set to internal.
  - Every route requires ServiceAuth.
  - Source excerpts are redacted.
- **Observability additions:**
  - Spans `engine_query{route}` and `graph_load`.
  - Histogram `graph_query_duration_seconds{route}`.
  - Gauges `graph_cache_entries` and `graph_nodes_total` / `graph_edges_total` per loaded snapshot.
- **Tests required:**
  - `routes_require_service_auth`
  - `repo_claim_mismatch_403`
  - `subgraph_budget_truncates`
  - `path_finds_auth_bypass_chain` (fixture)
  - `single_flight_load`
  - `excerpt_redacted_and_capped`
  - `openapi_generated_matches_snapshot`
- **Benchmarks if applicable:** PERF-004 neighbor and BFS latency through HTTP: p95 < 30 ms on the reference-api snapshot, warm.
- **Acceptance criteria:** The API-011 proxy tests pass against a running engine in compose, and the OpenAPI document shows no drift (CI-006).
- **Definition of done:** Global DoD, plus the Dockerfile entrypoint (CI-008).

---

---

### GH-001 — GitHub App auth (JWT, installation tokens cached encrypted in Redis)
Status: ☑
> **Implementation note:** Uses `@octokit/rest` with `@octokit/plugin-throttling` and `@octokit/plugin-retry`; the App JWT is signed with `jose` (RS256) and the key is parsed once into a `KeyObject` (PKCS1 or PKCS8). Tests run against an in-process fake GitHub HTTP server (`test/helpers/fake-github.ts`) and `ioredis-mock`; no real GitHub or Redis is called. A 401 on a cached token evicts it and retries once, implemented by mutating the request options inside an Octokit `request` hook (Octokit binds inner hooks to the original options object, so replacing it has no effect). The span `github_token_mint` uses the OTel API directly because it is not in the shared `SPAN_NAMES` list (kept identical to the Rust list). Primary rate limits are waited out at most twice via the throttling plugin; secondary limits surface as `ProviderError{rate_limited}` through `toProviderError`. The Redis lock is per installation (`rg:gh:itok-lock:{id}`) and waiters poll for their own cache key, so different scopes do not serialize for the 10 s timeout. `GithubModule` yields `null` when `GITHUB_ENABLED=false`; the `GithubAppAuth` is not yet registered with the `ProviderRegistry` (the provider implementation arrives with GH-005/GH-009). The `github_app_permissions_valid` boot check is GH-010.

- **Task ID:** GH-001
- **Title:** GitHub App auth (JWT, installation tokens cached encrypted in Redis)
- **Problem:** Legacy uses `gh` with a PAT that carries `push` scope (audit; `github.rs:5-13`). That is least-privilege failure and a single-user identity.
- **Why it exists:** A GitHub App gives least privilege (contents:read, pull_requests:write, checks:write, metadata:read), per-installation tokens and a bot identity.
- **Scope:**
  - `GithubAppAuth`: App JWT (RS256) creation, installation token minting (optionally repository- and permission-scoped), and a Redis cache encrypted with AES-256-GCM.
  - An Octokit factory per installation, with the throttling and retry plugins.
- **Explicit non-scope:**
  - Webhooks (GH-002).
  - The OAuth user flow (API-004).
- **Files/modules expected to change:** `apps/api/src/providers/github/github.module.ts`.
- **New files/modules expected:**
  - `apps/api/src/providers/github/{app-auth.service.ts,token-cache.ts,octokit.factory.ts}`
  - `apps/api/src/common/crypto/aead.ts`
- **Dependencies:** API-001, API-006, SEC-005.
- **Implementation details:**
  - The App JWT has `iat = now-60` and `exp = now+540`. It is cached in-process for 8 min.
  - Installation token: `POST /app/installations/{id}/access_tokens`. The general token is unscoped. The clone token is `{ repository_ids:[id], permissions:{ contents:'read' } }` (GH-006).
  - Cache key `rg:gh:itok:{installation_id}:{scope_hash}`. The value is `aead_encrypt(TOKEN_CACHE_KEY, json{token, expires_at})` with a 96-bit random nonce. TTL is `min(expires_at - now - 10min, 50min)`.
  - `@octokit/plugin-throttling`: on a primary rate limit, wait `retry-after` (at most twice). On a secondary rate limit, back off and surface `ProviderError{rate_limited}`.
  - `baseUrl` is `GITHUB_API_URL`, which allows the fake server (DEV-005).
- **Data model changes:** None. Tokens are never persisted in PG.
- **API/protocol changes:** None external.
- **Concurrency semantics:** Single-flight per cache key (an in-process promise map plus Redis `SET NX` lock `rg:gh:itok-lock:{id}` for 10 s), so N concurrent jobs mint one token.
- **Failure behavior:**
  - A Redis outage falls back to minting without the cache and logs a warning (availability over efficiency).
  - A private key parse failure fails at boot.
  - A 401 from GitHub on a cached token evicts the entry and retries once.
- **Idempotency considerations:** Token minting is safe to repeat.
- **Security considerations:**
  - The private key is loaded from env or file at boot and kept in memory only. It is never logged; the redaction test covers PEM blocks.
  - Tokens are encrypted at rest in Redis and are never in logs, spans or errors (the `Secret` type).
- **Observability additions:**
  - Span `github_token_mint`.
  - Counters `github_token_cache_hits_total` and `github_token_cache_misses_total`.
  - Gauge `github_rate_limit_remaining{installation}`.
  - Counter `github_rate_limited_total`.
- **Tests required:**
  - `app_jwt_claims_valid`
  - `token_cached_encrypted_not_plaintext` (reads raw Redis)
  - `token_ttl_below_expiry`
  - `single_flight_concurrent_mint`
  - `cached_token_401_evicts_and_retries`
  - `redis_down_falls_back`
  - `pem_never_logged`
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** Against the fake GitHub server, 20 concurrent `getOctokit(inst)` calls mint one token, and the Redis value does not contain the token substring.
- **Definition of done:** Global DoD.

---

### GH-002 — Webhook endpoint and HMAC verification
Status: ☑
> **Implementation note:** The endpoint (`apps/api/src/webhooks/`) is built against three ports in `webhook.ports.ts` because their owners do not exist yet: `DeliveryStore` (GH-003; an in-memory bounded set stands in), `EventNormalizer` (GH-004; a stub that ignores everything stands in) and `ProviderEventSink` (SUP-001; a stub that logs and drops stands in). Because there is no database transaction yet, the handoff to the sink is detached (`void sink.dispatch(...)` with failures logged and counted in `webhook_dispatch_failures_total`) rather than "verify, record, normalize, orchestrate in one transaction"; the atomic record-plus-enqueue wiring belongs to GH-003/API-007/SUP-001. The raw body comes from a route-specific `express.raw` parser registered in `configureApp` (`app.setup.ts`, 25 MiB, `express` added as a direct dependency) ahead of the 1 MiB JSON parsers; `ProblemFilter` now maps body-parser 4xx errors (for example 413) instead of answering 500. A bad or missing signature answers 401 with an empty body; once the signature is valid, missing `X-GitHub-Event`/`X-GitHub-Delivery` or invalid JSON answers 400. `GITHUB_WEBHOOK_SECRET_PREVIOUS` is accepted during rotation. `ping` and events outside `pull_request|issue_comment|installation|installation_repositories` answer 202 `accepted:false`. The endpoint is a 404 when `GITHUB_ENABLED=false`. The `webhook_received` span is created with the OTel API directly (the attribute set `event/action/delivery_id` is not in the shared `Attr` list); the span is a child of the HTTP server span rather than a separate trace root until OBS-003 defines propagation. The k6 benchmark and the DEV-005 signed replay tool are out of scope here.

- **Task ID:** GH-002
- **Title:** Webhook endpoint + HMAC verification
- **Problem:** No webhook ingestion exists. Legacy polls.
- **Why it exists:** PRD §79 and §107 (signature validation, fast acknowledgement).
- **Scope:**
  - `POST /api/v1/webhooks/github`, with a raw-body capture.
  - `X-Hub-Signature-256` verification (HMAC-SHA256, constant-time).
  - Header validation (`X-GitHub-Event`, `X-GitHub-Delivery`).
  - Handoff to idempotency (GH-003) and normalization (GH-004).
  - A 202 response in under 500 ms, with no review work in the request.
- **Explicit non-scope:**
  - Event semantics (GH-004).
  - The replay window (SEC-006 adds the timestamp check).
- **Files/modules expected to change:** `apps/api/src/main.ts` (raw-body parser for this route).
- **New files/modules expected:** `apps/api/src/webhooks/{webhooks.module.ts,github-webhook.controller.ts,signature.ts}`
- **Dependencies:** API-001, API-006, GH-003, GH-004.
- **Implementation details:**
  - Signature: `expected = 'sha256=' + hmac(secret, rawBody).hex`. Compare with `timingSafeEqual` on equal-length buffers; a length mismatch is rejected before the comparison.
  - Multiple secrets are supported during rotation (`GITHUB_WEBHOOK_SECRET`, `GITHUB_WEBHOOK_SECRET_PREVIOUS`).
  - The controller flow is verify → record delivery (GH-003) → normalize (GH-004) → orchestrate (SUP-001) inside one transaction → 202 `{ delivery_id, accepted: true|false, reason? }`.
  - Unsupported events return 202 with `accepted: false` (GitHub must not retry them).
- **Data model changes:** None (GH-003 owns `webhook_deliveries`).
- **API/protocol changes:** `POST /api/v1/webhooks/github`.
- **Concurrency semantics:** Stateless. Each delivery is handled in its own transaction.
- **Failure behavior:**
  - A bad or missing signature returns 401 with an empty body and increments a metric. It is never logged with the payload.
  - A database failure returns 503 so GitHub retries.
  - A body over 25 MiB returns 413.
- **Idempotency considerations:** Delegated to GH-003.
- **Security considerations:**
  - Constant-time comparison.
  - The raw body is used for HMAC. It is never re-serialized JSON.
  - No payload logging; only the event, action, delivery id and installation id are logged.
- **Observability additions:**
  - Root span `webhook_received{event,action,delivery_id}`, which starts the review trace (OBS-003).
  - Counter `webhook_signature_failures_total`, which drives the OBS-008 alert.
  - Counter `webhooks_received_total{event,accepted}`.
- **Tests required:**
  - `valid_signature_202`
  - `invalid_signature_401`
  - `length_mismatch_401_without_compare`
  - `rotated_previous_secret_accepted`
  - `unsupported_event_202_not_accepted`
  - `db_down_503`
  - `ack_under_500ms` (with orchestration mocked slow, the ack does not wait)
- **Benchmarks if applicable:** k6 load (PERF-008) at 50 events per minute sustained, with p95 ack < 200 ms.
- **Acceptance criteria:** The signed replay tool (DEV-005) gets 202 responses, a tampered payload gets 401, and a trace starts at `webhook_received`.
- **Definition of done:** Global DoD.

---

### GH-003 — Delivery idempotency (Redis SETNX + webhook_deliveries)
Status: ☑
> **Implementation note:** `webhook_deliveries` already exists (DOM-009, `20261002000005`) with `status` (received/processed/ignored/rejected/failed), `signature_valid`, `provider_installation_id` and a nullable `organization_id`, so no table is created: the new migration `20261003000010_webhook_delivery_functions.sql` adds three SECURITY DEFINER functions (`rg_record_webhook_delivery`, `rg_webhook_delivery_exists`, `rg_finish_webhook_delivery`, owned by `rg_ops`) because RLS hides NULL-organization rows from `rg_api`. The spec `outcome`/`review_run_id` columns are not added: the outcome is `status`, and the run link belongs to SUP-001. `DeliveryStore` changed from `record()` to `process(delivery, work)`: the insert, the handling (normalization) and the status update run in ONE transaction, so a handler failure rolls the row back (and clears the Redis key) and GitHub retry is processed fresh; `afterCommit` callbacks (dispatch to the orchestrator, the command reaction) run only once the transaction committed, and `work` receives `{trx}` for GH-013/SUP-001 to write in the same transaction. A Redis hit is only a hint: Postgres is asked (cheap read) and wins, and Redis being down degrades to Postgres alone. The in-memory store moved to `test/helpers/memory-delivery-store.ts`; SUP-004 `duplicate_webhook` is covered at the endpoint level (10 parallel replays, one accepted, one row) but the run/job half waits for SUP-001/API-007. The 30-day retention purge belongs to SEC-007.

- **Task ID:** GH-003
- **Title:** Delivery idempotency (Redis SETNX + webhook_deliveries)
- **Problem:** GitHub redelivers on timeouts, and operators can redeliver manually. Duplicate deliveries must not create duplicate runs.
- **Why it exists:** Production readiness §17, "Webhook idempotency".
- **Scope:**
  - A fast path: Redis `SET rg:wh:gh:{delivery_id} 1 NX EX 259200` (72 h).
  - The durable truth: `INSERT INTO webhook_deliveries ... ON CONFLICT (provider, delivery_id) DO NOTHING`, in the orchestration transaction.
  - A duplicate is a 202 `accepted:false reason:duplicate`.
- **Explicit non-scope:** Semantic dedup of different deliveries for the same head. SUP-001 handles that through run idempotency keys.
- **Files/modules expected to change:** `apps/api/src/webhooks/github-webhook.controller.ts`.
- **New files/modules expected:**
  - `apps/api/src/webhooks/delivery-store.ts`
  - `engine/migrations/{seq}_webhook_deliveries.sql`
- **Dependencies:** GH-002, API-002.
- **Implementation details:**

  ```sql
  CREATE TABLE webhook_deliveries (provider text NOT NULL, delivery_id text NOT NULL, event text NOT NULL, action text,
    installation_id bigint, organization_id uuid NULL, payload_sha256 text NOT NULL, received_at timestamptz DEFAULT now(),
    outcome text CHECK (outcome IN ('accepted','ignored','duplicate','failed')), review_run_id uuid NULL,
    PRIMARY KEY (provider, delivery_id));
  ```

  - If Redis says "seen" but PG has no row (a crash between the two), PG wins: the delivery is processed. The Redis check is an optimization only.
  - Retention: rows older than 30 days are purged by the SEC-007 job.
- **Data model changes:** `webhook_deliveries`.
- **API/protocol changes:** None.
- **Concurrency semantics:** Two simultaneous deliveries with the same id produce one PG insert. The loser's transaction sees `rowCount=0` and returns duplicate.
- **Failure behavior:**
  - With Redis down, the PG path alone suffices.
  - If the PG insert succeeds and a later step in the same transaction fails, everything rolls back and GitHub's retry is processed fresh.
- **Idempotency considerations:** This task is the idempotency mechanism itself.
- **Security considerations:** Only `payload_sha256` is stored. The payload body is not stored, which keeps source fragments and PII out of the database.
- **Observability additions:** Counter `webhook_duplicates_total`. Span attribute `duplicate=true`.
- **Tests required:**
  - `same_delivery_twice_one_run`
  - `concurrent_same_delivery_one_row`
  - `redis_seen_pg_missing_processes`
  - `rollback_allows_retry`
  - `redis_down_still_deduplicates`
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** SUP-004 `duplicate_webhook` passes: 10 parallel replays of one delivery produce exactly one `review_runs` row and one job.
- **Definition of done:** Global DoD.

---

### GH-004 — Event normalization
Status: ☑
> **Implementation note:** Split in two layers. `providers/github/normalize.ts` is the pure `(eventName, payload, deliveryId, {botLogin}) -> ProviderEvent | ReviewCommandCandidate | Ignored` function (zod subset validation); `providers/github/event-normalizer.service.ts` (`GithubEventNormalizer`, the webhook `EventNormalizer` port implementation that replaces the GH-002 stub) adds the I/O: repository-settings guards, the commenter permission lookup (`GET /repos/{o}/{r}/collaborators/{u}/permission`, `maintain` maps to write and `triage` to read; any lookup failure gives `Ignored{permission_unknown}`, fail closed) and the `provider_events_total{kind,outcome}` counter. The `eyes` reaction on an accepted command is sent best-effort by `GithubCommandAcknowledger` through a `CommandAcknowledger` port that the webhook controller calls after dispatch. Because repository settings (API-008) do not exist yet, `repositories/repository-settings.port.ts` defines the `RepositorySettingsPort` plus a `DefaultRepositorySettings` provider (enabled, all branches, skip drafts and bots); API-008 replaces it. Decisions beyond the spec: the bot guard checks the PR author (the legacy `[bot]`/`dependabot` rule) for head events and for non-cancel commands; an explicit command bypasses only the draft guard; the base-branch filter is not applied to commands because `issue_comment` payloads carry no base ref (the orchestrator re-checks after fetching the PR in GH-005); bot commenters are ignored; `review_requested` needs the new optional `GITHUB_APP_SLUG` env (bot login `<slug>[bot]`), otherwise it is ignored; `pull_request.closed` bypasses guards so runs are always cancelled. `IgnoreReason` gained `repository_disabled`, and `RepositoryProvider.normalizeEvent` is async (see API-006). Fixtures in `apps/api/test/fixtures/github/` are hand-trimmed to the documented GitHub webhook payload shape (extra fields included to prove subset parsing), not copied byte for byte from the docs; expected `ProviderEvent` snapshots are inline `toEqual` assertions. `ProviderEvent` itself was already defined in API-006, so `provider-event.ts` only gained the new reason.

- **Task ID:** GH-004
- **Title:** Event normalization (opened/reopened/synchronize/review_requested/issue_comment "/review" command)
- **Problem:** Core review logic must not depend on provider payloads (PRD §78).
- **Why it exists:** It turns GitHub events into `ProviderEvent`s that the orchestrator understands.
- **Scope:**
  - `pull_request` with `opened|reopened|synchronize|ready_for_review|review_requested` gives `PullRequestHeadEvent { kind, repo, pr, head_sha, base_sha, base_ref, author, draft, requested_reviewer? }`.
  - `pull_request.closed` gives `PullRequestClosed`, which cancels runs.
  - `issue_comment.created` on a PR with a body matching `^/review(\s+(full|cancel))?\s*$` gives `ReviewCommand`. It requires the commenter to have `write` or `admin` permission.
  - Pre-review guards ported from `github.rs:305-347`:
    - skip drafts (unless the command was explicit)
    - skip bots (`[bot]` suffix, dependabot)
    - skip base branches outside `repository_settings.target_branches` (glob `*` suffix semantics, as in `branch_matches`)
  - `review_requested` triggers only when the requested reviewer is the App's bot.
- **Explicit non-scope:**
  - Installation events (GH-013).
  - Self-review, which is not applicable: the App bot is never the PR author.
- **Files/modules expected to change:** `apps/api/src/webhooks/github-webhook.controller.ts`.
- **New files/modules expected:**
  - `apps/api/src/providers/github/normalize.ts`
  - `apps/api/src/providers/github/guards.ts`
  - `apps/api/src/providers/ports/provider-event.ts` (extended)
- **Dependencies:** API-006, GH-002, API-008 (repository settings).
- **Implementation details:**
  - Normalization is a pure function `(eventName, payload) => ProviderEvent | Ignored{reason}`, validated with zod against the subset of fields used.
  - `/review full` sets `depth=full` (it bypasses low-risk suppression). `/review cancel` cancels the active run.
  - The command reply is an emoji reaction (`eyes`) on the comment. No text comment.
- **Data model changes:** None. `webhook_deliveries.outcome` records `ignored` with the reason.
- **API/protocol changes:** Internal type `ProviderEvent`.
- **Concurrency semantics:** Pure.
- **Failure behavior:**
  - Missing fields give `Ignored{reason: 'malformed'}` and a metric.
  - A failed permission lookup gives `Ignored{reason:'permission_unknown'}`. The system fails closed: no review.
- **Idempotency considerations:** Deterministic. The run key comes from `head_sha`.
- **Security considerations:**
  - `/review` from a read-only user or an external contributor is ignored, which prevents cost abuse.
  - Comment bodies are never logged.
- **Observability additions:** Counter `provider_events_total{kind,outcome}`.
- **Tests required:**
  - `opened_normalized`
  - `synchronize_normalized_with_new_head`
  - `draft_skipped`
  - `draft_with_explicit_command_reviewed`
  - `bot_author_skipped`
  - `branch_glob_release_star`
  - `review_command_requires_write`
  - `review_cancel_command`
  - `review_requested_other_user_ignored`
  - `malformed_payload_ignored`

  The guard tests port legacy `self_authored_pr_is_skipped_case_insensitively`, `draft_pr_is_skipped` and `branch_globs_match`, adapted.
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** Golden payload fixtures in `apps/api/test/fixtures/github/*.json` (taken from GitHub docs examples) normalize to the expected snapshots.
- **Definition of done:** Global DoD.

---

### GH-005 — PR fetch, normalize and repository sync
Status: ☑
> **Implementation note:** `pull_requests.updated_at` is rewritten by the `rg_set_updated_at` trigger, so migration `20261008000001_pull_request_sync.sql` adds `provider_updated_at` (the provider's own `updated_at`) and the upsert guard compares that column. The upsert writes head/base SHAs only on insert (SUP-001 moves the head). `merge_base_sha` stays null: GitHub has no merge-base field on the PR and the engine computes it locally with gix, so no extra compare call is made. Transient errors are retried by the Octokit retry plugin (3 retries, GH-001 factory) and then surface as `ProviderError{transient}`. `RepositorySyncService` (`apps/api/src/repositories/sync.service.ts`) reads the provider only when the row is missing or on an explicit refresh (the orchestrator refreshes after a PR 404, which is how a repository 404 becomes `access_lost`). `collectChangedFiles(stream, cap)` in the ports reports `truncated`. The GitHub provider is registered in the `ProviderRegistry` by `GithubProviderRegistration`. Tests: unit `test/providers/github/repository-provider.spec.ts` (paging, cap, recorded-response contract in `test/fixtures/github/api/`), integration `integration/github-sync.int.spec.ts`.

- **Task ID:** GH-005
- **Title:** PR fetch/normalize + repository sync
- **Problem:** Webhook payloads can be stale, and the pipeline needs authoritative PR metadata plus repository rows.
- **Why it exists:** It implements the `RepositoryProvider` methods for GitHub and keeps the `repositories` and `pull_requests` rows current.
- **Scope:**
  - `GithubRepositoryProvider.getRepository`, `getPullRequest`, `listChangedFiles` (paginated, cap 3,000), `getCommit` and `getActorPermission`.
  - `RepositorySyncService.upsertFromEvent` and `PullRequestSyncService.upsert(pr)`, which records base, head and merge-base candidate SHAs.
- **Explicit non-scope:**
  - Computing the diff. The engine computes it locally with gix (DIFF). The provider file list is used for validation and for very large PR hints only.
- **Files/modules expected to change:** `apps/api/src/providers/github/github.module.ts`.
- **New files/modules expected:**
  - `apps/api/src/providers/github/repository-provider.ts`
  - `apps/api/src/repositories/sync.service.ts`
  - `apps/api/src/reviews/pull-request-sync.service.ts`
- **Dependencies:** GH-001, API-006, DOM-004.
- **Implementation details:**
  - `pull_requests (id, organization_id, repository_id, provider_number, title, author_login, base_ref, base_sha, head_ref, head_sha, draft, state, updated_at, UNIQUE(repository_id, provider_number))`. These come from DOM-009; this task adds columns if missing.
  - Upsert uses `ON CONFLICT (repository_id, provider_number) DO UPDATE ... WHERE pull_requests.updated_at <= EXCLUDED.updated_at`, so an out-of-order older event cannot overwrite a newer head. Head changes go through SUP-001, not this upsert.
  - Title and body are stored truncated (title ≤ 512). The body is **not** stored; it is fetched on demand for intent classification.
- **Data model changes:** Possible column additions to `pull_requests` (migration).
- **API/protocol changes:** None external.
- **Concurrency semantics:** The guarded upsert prevents regressions from out-of-order events.
- **Failure behavior:**
  - A 404 (repository removed or access revoked) marks the repository `access_lost` and ignores the event.
  - Transient errors are retried with backoff (at most 3), then fail the job.
- **Idempotency considerations:** Upserts.
- **Security considerations:** The PR body is not persisted (it may hold secrets). Logins are stored and are public data.
- **Observability additions:** Spans `github_get_pull_request` and `github_list_files`. Counter `github_api_calls_total{route,status}`.
- **Tests required:**
  - `pr_upsert_ignores_older_event`
  - `list_files_paginates`
  - `list_files_cap_marks_truncated`
  - `repo_404_marks_access_lost`
  - `fake_server_contract` (DEV-005 fake server replays recorded responses)
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** In the E2E flow, the PR row has the correct base and head from the fake server, and out-of-order events do not regress the head.
- **Definition of done:** Global DoD.

---

### GH-006 — Clone credential broker internal endpoint
Status: ◐
> **Implementation note:** API side done: `CloneCredentialsController` (`apps/api/src/internal/clone-credentials.controller.ts`, excluded from the public OpenAPI document) resolves the repository with `resolve_org`, refuses a token whose `org` claim differs, answers 409 `installation_suspended` / `installation_inactive` / `repository_access_lost`, 503 on transient provider errors, and mints through `RepositoryProvider.issueCloneCredential(ref, ttl, { providerRepoId })` (`repository_ids:[id]`, `contents:read`). `clone_url` is derived from `GITHUB_API_URL` (api.github.com -> github.com, otherwise the origin) and never carries the token. The test controller of `test/internal/service-auth.e2e.spec.ts` moved to `/internal/test/...` so it no longer shadows the real route. **Missing:** the Rust half (`engine/crates/pipeline/src/credentials.rs`, the gix credential callback in `repository`, `rust_secret_not_in_debug`) belongs to the engine pipeline lane (the `pipeline` crate is still a stub); `token_absent_from_logs` covers the API process logs only.

- **Task ID:** GH-006
- **Title:** Clone credential broker internal endpoint
- **Problem:** Workers must clone or fetch private repositories without holding the App private key.
- **Why it exists:** Target-arch §5 `internal` module. Least privilege: workers receive short-lived, single-repository, read-only tokens that are kept in memory only.
- **Scope:**
  - `POST /internal/repositories/:id/clone-credentials` returns `{ username: 'x-access-token', token, expires_at, clone_url }`.
  - It requires a service JWT with `scope=clone-credentials` and `repo=:id` (API-005).
  - The Rust client `pipeline::credentials::fetch_clone_credentials` uses the token via a `GIT_ASKPASS`-free gix credential callback, never in the URL.
- **Explicit non-scope:** The checkout logic (IDX or PIPE own the mirror management).
- **Files/modules expected to change:** `engine/crates/repository/src/git/fetch.rs` (credential callback).
- **New files/modules expected:**
  - `apps/api/src/internal/{internal.module.ts,clone-credentials.controller.ts}`
  - `engine/crates/pipeline/src/credentials.rs`
- **Dependencies:** API-005, GH-001, IDX-001 (bare mirror fetch).
- **Implementation details:**
  - The token is minted with `repository_ids:[repo.provider_repo_id]` and `permissions:{contents:'read'}` (GH-001 scoped cache).
  - The response has `Cache-Control: no-store`.
  - Rust holds the token as `secrecy::SecretString`, zeroized on drop, never in `Debug`.
  - The mirror path is `/{work}/{org}/{repo}.git`. The remote URL has no credentials embedded.
- **Data model changes:** None.
- **API/protocol changes:** The internal endpoint above.
- **Concurrency semantics:** The scoped-token single-flight comes from GH-001.
- **Failure behavior:**
  - The installation is suspended: 409 `installation_suspended`, and the worker fails the job as permanent.
  - A transient error: 503, and the worker retries.
- **Idempotency considerations:** Safe to call repeatedly.
- **Security considerations:**
  - Service auth, with a repo claim match.
  - No logging of the token.
  - A test greps worker logs from an integration run for the token value.
- **Observability additions:** Span `clone_credentials_issue`. Counter `clone_credentials_issued_total`.
- **Tests required:**
  - `requires_service_token`
  - `repo_claim_mismatch_403`
  - `token_scoped_read_only_single_repo` (asserts the fake server received the scoped request)
  - `no_store_header`
  - `rust_secret_not_in_debug`
  - `token_absent_from_logs`
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** In E2E-001 the worker fetches from the local bare repository using a credential obtained from the broker. Logs contain no token.
- **Definition of done:** Global DoD.

---

### GH-007 — Inline comment rendering and anchoring
Status: ☑
> **Implementation note:** Pure renderers in `apps/api/src/publisher/render/` (`inline-comment.ts`, `anchor.ts`, `diff-index.ts`, `severity.ts`, plus `escape.ts`, `plan.ts` and `types.ts`). The input is a local `RenderableFinding` view model (and a local `Severity` union mirroring the contract wire form) rather than the generated contract types, because `apps/api` does not depend on `@reviewgraph/contracts` yet and API-010 (finding detail) will map verified findings onto it. Hunks come from the provider `patch` text per changed file (`DiffIndex.build`) or stored hunks (`DiffIndex.fromHunks`, for `review_diff_hunks` once DIFF-* persists them). Anchoring: a range wholly inside one hunk becomes a multi-line comment; a range spanning hunks or only partly overlapping falls back to a single line (the first changed visible line, else the first visible line); `base`-side findings anchor LEFT with old-file numbers; no location, file absent from the diff or file without a patch relocates with `outside_diff`. `planPublication` applies the cap (default 25) and relocates overflow (`inline_cap`), out-of-diff and unrenderable (`render_error`) findings, counting `findings_relocated_to_summary_total{reason}`; nothing is dropped. Untrusted text is markdown-escaped (`<>&@` become entities, so markers cannot be closed and users cannot be pinged) and marker values are restricted to a safe alphabet. The hedging-phrase assertion throws when `NODE_ENV` is not `production`. The latent note text is adapted from `github.rs:288-297`.

- **Task ID:** GH-007
- **Title:** Inline comment rendering (PRD §58/59 format, evidence path) + anchoring (multi-line, LEFT side for deletions, out-of-diff → summary)
- **Problem:** Legacy `comment_body` (`github.rs:274-303`) renders `[HIGH] title / description / Evidence: text` with single-line RIGHT-side anchors only. PRD §58/§59 require five answers and an evidence path, and deletions need LEFT-side anchors.
- **Why it exists:** Comment quality and correct placement are what the developer sees.
- **Scope:**
  - `renderInlineComment(finding)`: a pure TS function.
  - `anchorFinding(finding, diffIndex)` returns `Inline{path, line, side, start_line?, start_side?}` or `Summary{reason}`.
  - The inline cap (default 25, from config `publish.inline_cap`).
  - Overflow, out-of-diff and anchoring failures are relocated to the summary (INV-015).
- **Explicit non-scope:**
  - The summary (GH-008).
  - Posting (GH-009).
- **Files/modules expected to change:** None existing.
- **New files/modules expected:** `apps/api/src/publisher/render/{inline-comment.ts,anchor.ts,severity.ts,diff-index.ts}`
- **Dependencies:** DED-004 (priority order), DIFF-001..DIFF-005 (hunks persisted as `review_diff_hunks`, or fetched from engine stage output), API-010 (finding detail).
- **Implementation details:** The template:

  ```
  {emoji} **{Severity} — {title}**                                   (🔴 critical/high, 🟠 medium, 🟡 low, ⚪ info)

  {what_changed}  {why_risky}                                         (§59 Q1, Q2)

  {behavior_result}                                                    (§59 Q4)

  **Evidence**
  ```text
  {path[0].display}()
    → {path[1].display}()
    → {path[2].display}()
  ```                                                                  (§59 Q3; omitted if no path, then cite evidence items)
  {corrective_direction}                                               (§59 Q5)

  <sub>{finding_short_id} · {reviewer} · confidence {c:.2}</sub>
  <!-- reviewgraph:finding={fingerprint} run={review_run_id} -->
  ```

  Anchoring:
  - When the anchor range intersects added or context lines in a new-side hunk, the comment goes on the RIGHT side, at `line = end`, `start_line = start` if start < end, and both lie within the same hunk.
  - When the anchor is on deleted lines only, it goes on the LEFT side with old-side line numbers.
  - Otherwise the finding goes to the summary with reason `outside_diff`.
  - Over the cap, the finding goes to the summary with reason `inline_cap`.
  - A latent finding (no current caller) gets the legacy latent note (ported from `github.rs:288-297`).
  - The PRD §58 banned hedging phrases have already been rejected upstream (VER stage 7). The renderer asserts this in development builds.
- **Data model changes:** None.
- **API/protocol changes:** None.
- **Concurrency semantics:** Pure.
- **Failure behavior:** Missing explanation fields mean a rendering error for that finding, which moves it to the summary with reason `render_error` and a metric. It is never dropped.
- **Idempotency considerations:** Deterministic output for the same finding.
- **Security considerations:**
  - Finding text is markdown-escaped except the template markup.
  - `<!-- -->` markers are sanitized so finding text cannot close them.
  - No source excerpts appear beyond the evidence path names.
- **Observability additions:** Counter `findings_relocated_to_summary_total{reason}`.
- **Tests required:**
  - `renders_prd58_example` (snapshot)
  - `answers_all_five_questions_fields_required`
  - `multiline_anchor_same_hunk`
  - `multiline_across_hunks_falls_back_single_line`
  - `deletion_anchors_left_side`
  - `outside_diff_goes_to_summary`
  - `cap_overflow_goes_to_summary`
  - `latent_note_present`
  - `marker_cannot_be_closed_by_text`
  - `only_inline_findings_become_anchored_comments` (ported from `github.rs:455-466`)
  - `inline_comments_respect_the_cap` (ported from `github.rs:468-472`)
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** The snapshot for the auth-bypass finding matches the PRD §58 example structure, and the anchor tests pass for added, modified and deleted fixtures.
- **Definition of done:** Global DoD.

---

### GH-008 — Summary rendering (PRD §61)
Status: ☑
> **Implementation note:** `renderSummary(SummaryInput)` in `apps/api/src/publisher/render/summary.ts` is pure and deterministic. The input is a local view model whose counts (findings by severity, verified/candidates, suppressed by reason) are supplied by the caller; the SQL aggregation over `candidate_findings`/`findings` and the RiskAssessment, completeness and POL-002 sources (API-009, RISK-004, DED-004, POL-002) do not exist yet and will populate it. `summary_counts_match_db` therefore asserts that the rendered numbers equal the supplied counts exactly; the end-to-end check against database rows belongs to E2E-001. Sections: header, optional degraded line, Changed, Risk areas (at most 5, "Risk areas: unavailable" when no assessment), Findings by severity or "No verified findings.", Verified X / Y, Suppressed by reason (the five PRD reasons), Findings outside the diff (relocated list with severity, title and `path:line`), Coverage (reviewers run and not run with reasons, unreviewed clusters, deterministic checks where a non-executed check renders `NOT EXECUTED — reason` and never PASS), optional policy notice, the no-merge footer and the hidden `<!-- reviewgraph:run=... head=... -->` marker. All free text is markdown-escaped and marker values are restricted to a safe alphabet. The posting step (GH-009) decides whether to post it from `publish.summary`.

- **Task ID:** GH-008
- **Title:** Summary rendering (PRD §61)
- **Problem:** The legacy summary (`report.rs:11-140`) has a decision section and validation lines, but no risk areas, no verified X/Y counts and no suppressed counts.
- **Why it exists:** PRD §61. It is also where out-of-diff findings and coverage gaps must appear (INV-013/014/015).
- **Scope:** `renderSummary(run)` produces these sections:
  - Changed: files, behavioral symbols, API contracts.
  - Risk areas (from RiskAssessment signals, at most 5).
  - Findings by severity.
  - Verified: published / candidates.
  - Suppressed: counts by reason (low confidence, duplicate, pre-existing, not actionable, policy).
  - Findings outside the diff (relocated list, with severity, title and `path:line`).
  - Coverage: reviewers run and not run with reasons, unreviewed clusters, and deterministic checks `NOT EXECUTED`, never shown as passed.
  - A policy-change notice (POL-002).
  - The footer "No merge performed. ReviewGraph never approves or merges."
  - A hidden marker `<!-- reviewgraph:run={id} head={sha} -->`.
- **Explicit non-scope:** Internal reasoning, which is never exposed (PRD §61, §86).
- **Files/modules expected to change:** None existing.
- **New files/modules expected:** `apps/api/src/publisher/render/summary.ts`
- **Dependencies:** GH-007, API-009 (completeness), RISK-004, DED-004, POL-002.
- **Implementation details:**
  - Counts come from `candidate_findings` and `findings` grouped by lifecycle state, and are computed in SQL, not in the model.
  - Validation and NOT_EXECUTED rendering ports `report.rs` behavior: `NOT EXECUTED — {reason}`, never PASS (`report.rs:96-107`).
  - Zero findings: the summary is "No verified findings." followed by the coverage section. It is still posted when `publish.summary: true`.
  - A degraded run gets a header line "Review completed with reduced coverage".
- **Data model changes:** None.
- **API/protocol changes:** None.
- **Concurrency semantics:** Pure.
- **Failure behavior:** A missing risk assessment renders the "Risk areas: unavailable" line. The summary never fails.
- **Idempotency considerations:** Deterministic.
- **Security considerations:** Text is escaped. No source or prompt content appears.
- **Observability additions:** None.
- **Tests required:**
  - `summary_prd61_example_snapshot`
  - `summary_lists_outside_diff_findings`
  - `summary_not_executed_never_pass`
  - `summary_degraded_header`
  - `summary_zero_findings_shows_coverage`
  - `summary_no_merge_footer`
  - `summary_counts_match_db`
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** The E2E-001 posted body contains every section, and its counts match the database rows.
- **Definition of done:** Global DoD.

---

### GH-009 — Publisher (atomic review, check run, published_findings, supersession gate)
Status: ☑
> **Implementation note:** `PublisherService.publish(runId)` (`apps/api/src/publisher/`) renders before the gate, then in one transaction acquires the SUP-003 gate, upserts `publications` (`posting`, attempt+1), ALWAYS looks for a review carrying `reviewgraph:run=<id>` first (a crash after GitHub accepted the POST rolls our row back, so the marker is the only reliable evidence), posts one `COMMENT` review (the POST is never retried in-process by Octokit, 15 s timeout) and marks the run COMPLETED while the locks are still held, so a waiting supersession never supersedes a run whose review was posted. `published_findings`, candidate `PUBLISHED`, stale resolution and the check run follow outside the transaction and are idempotent (a retry after a posted publication takes the `already_published` path). A 422 relocates every inline comment to the summary and posts once more; 403/404 (permanent) set `FAILED_PUBLISH` and a neutral "Review could not be published" check run. Migration `20261008000003_publications.sql` adds `publications`, `check_runs` (both with RLS) and `published_findings.side/status/resolved_in_run_id`. The PRD §59 explanation is read from `verified_findings.evidence.explanation` (`what_changed`, `why_risky`, `behavior_result`, `corrective_direction`, `evidence_path`, `latent`); a finding without it is relocated to the summary (`render_error`), never dropped — VER/DED must persist that shape. `PublishConsumer.handle(payload)` is ready but is registered with the queue by API-007 (`consume('review-publish', handler, { concurrency: 4 })`); until then nothing consumes `review-publish`. Summary counts for behavioral symbols/API contracts are 0 until the analysis outputs are persisted.

- **Task ID:** GH-009
- **Title:** Publisher (single atomic review POST event=COMMENT, check run, published_findings with comment ids, publish-time supersession gate)
- **Problem:** Publication must be atomic, so there is no comment spam (legacy `github.rs:197-241`). It must be idempotent under retries, never obsolete (PRD §77), and never an approval.
- **Why it exists:** It is on the critical path (`GH-009 → SUP-003 → E2E-001`).
- **Scope:**
  - The `review-publish` consumer (API-007).
  - Load the verified, prioritized findings, render them (GH-007/008), apply the SUP-003 gate, and make one `POST /repos/{o}/{r}/pulls/{n}/reviews` with `event:'COMMENT'`, `commit_id: head_sha`, `body`, and `comments[]`.
  - Create or update a check run named "ReviewGraph": conclusion `neutral` when findings were published, `success` when there were none and coverage was complete, and `neutral` with title "Review incomplete" when coverage was degraded or failed. It is **never** `success` on failure (INV-012).
  - Fetch the review's comments and map them to findings by marker. Insert `published_findings`.
- **Explicit non-scope:**
  - Stale resolution (GH-011).
  - APPROVE/REQUEST_CHANGES, which are never sent (API-006 type).
- **Files/modules expected to change:** `apps/api/src/app.module.ts`.
- **New files/modules expected:**
  - `apps/api/src/publisher/{publisher.module.ts,publish.consumer.ts,publisher.service.ts}`
  - `apps/api/src/providers/github/review-publisher.ts`
  - `engine/migrations/{seq}_published_findings.sql`
- **Dependencies:** GH-007, GH-008, SUP-003, API-007, GH-001, DED-004, PIPE-003 (enqueues `review-publish` on VERIFYING→PUBLISHING).
- **Implementation details:** The flow:
  1. Gate transaction (SUP-003). It locks the run and PR rows, checks state and head, and inserts `publications(review_run_id PK, state='posting', attempt)`. If a `posting` row already exists from a previous attempt, go to step 2b.
  2. a. Post the review. b. On retry, call `findExistingReview(marker run={id})` first. If found, adopt its id and do not post again.
  3. Inside the same gate transaction (the lock is held across this single HTTP call with a 15 s timeout, which bounds the supersession wait), mark the `publications` row `posted` with `provider_review_id`.
  4. Outside the transaction: list the review comments (`GET .../reviews/{id}/comments`), parse the markers, and insert `published_findings (finding_id, review_run_id, provider_review_id, provider_comment_id, path, line, side, published_at)` with `ON CONFLICT DO NOTHING`. Transition the run to PUBLISHING→COMPLETED (CAS).
  5. Upsert the check run (`external_id = review_run_id`).

  A GitHub 422 ("line must be part of the diff"): every inline comment is moved to the summary and the post is retried once (INV-015).
- **Data model changes:** `publications`, `published_findings` (with RLS) and `check_runs (review_run_id, provider_check_run_id)`.
- **API/protocol changes:** None external. The GitHub calls are as described.
- **Concurrency semantics:**
  - Publish-consumer concurrency is 4.
  - The row lock ensures that supersession and publish for the same PR serialize.
  - At most one review per run, guaranteed by the `publications` PK plus marker adoption.
- **Failure behavior:**
  - Transient or rate-limited: retry via the job (backoff).
  - Permanent (403 or 404): the run becomes `FAILED_PUBLISH`, with a check run "Review could not be published".
  - Superseded at the gate: the job succeeds with outcome `skipped_superseded`, and nothing is posted.
- **Idempotency considerations:**
  - Job key `publish:{review_run_id}`.
  - The `publications` row.
  - The marker lookup before reposting.
  - `published_findings` upserts.
- **Security considerations:**
  - The event is the constant `'COMMENT'`.
  - The installation token comes from GH-001.
  - Comment bodies contain no secrets (rendered from redacted, verified content).
- **Observability additions:**
  - Span `publication{review_run_id}`, with children `github_create_review` and `github_check_run`.
  - Counters `published_findings_total{severity,reviewer}` and `publish_outcomes_total{outcome}`.
- **Tests required:**
  - `single_review_post_with_all_comments`
  - `event_is_comment_constant`
  - `retry_after_crash_adopts_existing_review` (fake server: the first POST succeeds, then the consumer is killed)
  - `422_relocates_to_summary_and_retries_once`
  - `check_run_neutral_with_findings`
  - `check_run_success_only_when_clean_and_complete`
  - `check_run_never_success_on_failure`
  - `published_findings_mapped_by_marker`
  - `superseded_at_gate_posts_nothing`
- **Benchmarks if applicable:** None.
- **Acceptance criteria:**
  - E2E-001 receives exactly one review POST, with the expected body and comments, at the fake server.
  - The retry and supersession tests in SUP-004 pass.
- **Definition of done:** Global DoD.

---

### GH-010 — No-merge guarantee (App permission manifest + static test)
Status: ☑
> **Implementation note:** TS side only. Added `infra/github/app-manifest.json` (a GitHub App manifest whose URLs are `example.invalid` placeholders for DEV-006), `providers/github/app-manifest.ts` (the exact permission constants; a test keeps them identical to the JSON), `GithubAppAuth.getAppPermissions()` (`GET /app`) and `GithubPermissionsMonitor` (`permissions-monitor.ts`), which runs at `onApplicationBootstrap`, compares permissions for exact equality, logs a CRITICAL error on a mismatch, sets the `github_app_permissions_valid` gauge and exposes `canPublish()` (true only when valid; an unreachable GitHub leaves it `unknown`, which also keeps publishing off). `/health/ready` reports `checks.github_permissions` (only when GitHub is enabled) and returns 503 when it is `invalid`. The publish consumer that must consult `canPublish()` does not exist yet (GH-009/API-007). The static test (`apps/api/test/invariants/no-merge.spec.ts`, helper `scan.ts`) strips comments by collecting comment ranges from the TypeScript parser (not the bare scanner, which cannot tell a regex from a division and could hide code), scans every `apps/api/src/**/*.ts` for the forbidden strings (plus the `APPROVE`/`REQUEST_CHANGES` event literals), and a meta-test plants merge, approve and `contents: 'write'` code in a temp directory and expects the scan to fail. Deviation: the Rust half (`engine/crates/review-core/tests/no_merge_capability.rs`, `module_exposes_no_merge_capability_rust`) is NOT done here because `engine/` is being edited concurrently by another engineer; it remains open for the engine side, so GH-010 is complete for the TypeScript control plane only. The "required in CI" wiring for the new tests is covered by `pnpm -r test` already (CI-* tasks own the workflow).

- **Task ID:** GH-010
- **Title:** No-merge guarantee (App permission manifest + static test porting github.rs:356-375)
- **Problem:** PRD §6 non-goal and legacy invariant: the system structurally cannot merge. Legacy enforced it by a self-scanning test (`github.rs:356-375`) because the PAT had push scope.
- **Why it exists:** It carries over INV-011. In the new system it is guaranteed at two levels:
  - Token level: the App has no `contents:write`, so merge is impossible server-side.
  - Code level: no merge code path exists, guarded by a static test.
- **Scope:**
  - `infra/github/app-manifest.json` with permissions `{contents:'read', pull_requests:'write', checks:'write', metadata:'read', issues:'read'}` and events `[pull_request, issue_comment, installation, installation_repositories]`.
  - A boot-time check: `GET /app` permissions must equal the manifest (no `contents:write`, no `administration`, no `workflows`). On a mismatch the process refuses to start the publisher.
  - A static test scanning `apps/api/src/**` and `engine/**/src/**`.
- **Explicit non-scope:** None.
- **Files/modules expected to change:** `apps/api/src/providers/github/app-auth.service.ts` (permission check at boot).
- **New files/modules expected:**
  - `infra/github/app-manifest.json`
  - `apps/api/test/invariants/no-merge.spec.ts`
  - `engine/crates/review-core/tests/no_merge_capability.rs`
- **Dependencies:** GH-001, GH-009, DEV-006 (manifest used in setup).
- **Implementation details:**
  - The TS test reads every `.ts` file under `apps/api/src`, strips comments with the TypeScript scanner, and asserts the absence of `pulls.merge`, `.merge(`, `/merge`, `merge_method`, `squash`, `rebase_merge`, `enablePullRequestAutoMerge`, `mergePullRequest`, and `contents: 'write'`/`contents:"write"`.
  - The Rust test does the same over `engine/crates/*/src` and `engine/apps/*/src` for `"/merge"`, `"pr merge"`, `merge_pull`, `squash` and `rebase_merge`. Executable lines only, as in the legacy filter on `//`/`//!`.
  - A manifest test asserts the exact permission set.
- **Data model changes:** None.
- **API/protocol changes:** None.
- **Concurrency semantics:** N/A.
- **Failure behavior:** A permission mismatch at boot logs a critical message, disables publishing (the consumer does not start), and makes `/health/ready` report `github_permissions: invalid`.
- **Idempotency considerations:** N/A.
- **Security considerations:** This task is a least-privilege enforcement.
- **Observability additions:** Gauge `github_app_permissions_valid` (0/1).
- **Tests required:**
  - `module_exposes_no_merge_capability_ts`
  - `module_exposes_no_merge_capability_rust` (port of `github.rs:356-375`)
  - `app_manifest_permissions_exact`
  - `boot_rejects_contents_write_permission` (fake `GET /app`)
  - `planted_merge_call_fails_test` (meta-test using a temp fixture file)
- **Benchmarks if applicable:** None.
- **Acceptance criteria:**
  - Both static tests pass and are required in CI.
  - The planted-merge meta-test fails as expected.
  - The boot check rejects an over-privileged App.
- **Definition of done:** Global DoD, plus INV-011 referencing these tests.

---

### GH-011 — Stale comment resolution on re-review
Status: ☑
> **Implementation note:** `StaleResolutionService` classifies the PR's still-tracked earlier findings (`published_findings.status in open|carried_over|unknown`) before posting (carried-over findings are filtered from the new review; unknown ones are rendered under **Previously reported**) and applies the result after the post. Matching is fingerprint equality or same category plus a symbol mapped through the `SYMBOL_LINEAGE` port; the default has no lineage until the SID-005 store is wired to it. "Anchor changed" is approximated at file level through the optional `RepositoryProvider.listFilesBetween` (GitHub compare API); if the comparison is unavailable the finding is `unknown`, never `fixed`. Thread lookup and `resolveReviewThread` live in `providers/github/graphql.ts`; only threads whose first comment author is the App (`GITHUB_APP_SLUG`, GraphQL or REST form) are resolved. `publish.reply_on_resolve` is not implemented (no such setting exists yet), so no reply is posted.

- **Task ID:** GH-011
- **Title:** Stale comment resolution on re-review
- **Problem:** On a new head, findings from the previous review may be fixed (and their threads should resolve) or still present (and must not be reposted). Legacy matched by title (`report.rs:117-140`).
- **Why it exists:** PRD §78 says "resolve stale comments". Avoiding duplicate comments is part of noise control.
- **Scope:** After a successful publish for head B, compare it with the previous published run (head A) on the same PR:
  - **Still present:** same DED root-cause fingerprint, following symbol lineage. Do not repost. The renderer filters these before posting, using a `carried_over` state.
  - **Fixed:** absent at B, and its anchor symbol changed or was removed. Resolve the thread via the GraphQL `resolveReviewThread`, and reply "Addressed in `{short_sha}`" only if `publish.reply_on_resolve: true`.
  - **Unknown:** absent, but its anchor was unchanged. Leave the thread open and list it in the summary under "Previously reported".
- **Explicit non-scope:**
  - Human-authored threads, which are never touched.
  - Deleting comments, which never happens.
- **Files/modules expected to change:**
  - `apps/api/src/publisher/publisher.service.ts`
  - `apps/api/src/publisher/render/summary.ts`
- **New files/modules expected:**
  - `apps/api/src/publisher/stale-resolution.service.ts`
  - `apps/api/src/providers/github/graphql.ts`
- **Dependencies:** GH-009, DED-002 (fingerprints), SID-005 (lineage), API-006 (`resolveThreads`).
- **Implementation details:**
  - Thread ids are looked up through GraphQL `pullRequest.reviewThreads(first:100){ nodes{ id isResolved comments(first:1){ nodes{ databaseId author{login} } } } }`, matching `databaseId` to `published_findings.provider_comment_id`.
  - Only threads whose first comment's author is the App bot are resolved.
  - Matching state is recorded in `published_findings.status: open|carried_over|resolved|unknown` and `resolved_in_run_id`.
- **Data model changes:** `published_findings.status` and `resolved_in_run_id` columns (migration).
- **API/protocol changes:** None external.
- **Concurrency semantics:** Runs after publish in the same job. It is safe to re-run because resolving an already-resolved thread is a no-op.
- **Failure behavior:** A GraphQL failure is logged and counted. The publish still succeeds, and resolution is retried on the next run.
- **Idempotency considerations:** Status updates are monotonic (`open → resolved`).
- **Security considerations:** It acts only on the App's own threads (author check). `pull_requests:write` is sufficient.
- **Observability additions:** Counters `stale_threads_resolved_total` and `findings_carried_over_total`.
- **Tests required:**
  - `fixed_finding_thread_resolved`
  - `still_present_not_reposted`
  - `unknown_left_open_listed_in_summary`
  - `human_thread_never_resolved`
  - `renamed_symbol_matches_via_lineage`
  - `graphql_failure_does_not_fail_publish`
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** In the E2E variant with two heads (a fix pushed), the fake server receives a `resolveReviewThread` mutation for the fixed finding, and the second review omits the carried-over finding.
- **Definition of done:** Global DoD.

---

### GH-012 — Polling reconciler fallback
Status: ◐
> **Implementation note:** `GithubReconciler` (`apps/api/src/providers/github/reconciler.service.ts`, provided by `WebhooksModule` next to the event sink it feeds) runs on a plain `setInterval` instead of `@nestjs/schedule` to avoid a new dependency. Targets come from the new SECURITY DEFINER function `rg_reconcile_targets` (migration `20261008000004`). Config: `RECONCILER_ENABLED` (unset: on only in development without a webhook secret) and `RECONCILE_INTERVAL_SECONDS` (default 300). Synthetic deliveries are recorded in `webhook_deliveries` with `event='poll'`; a known head is one with a matching PR head and a non-superseded run, which is what makes repeated polls no-ops. Tests call `runCycle()` directly (`integration/reconciler.int.spec.ts`); the 1 s interval acceptance check and the DEV-006 documentation are not done.

- **Task ID:** GH-012
- **Title:** Polling reconciler fallback
- **Problem:** There is no public ingress in this environment, and webhooks can be missed in production too (outages, misconfiguration). Legacy relied on polling (`github.rs:80-99`).
- **Why it exists:** It is the environment gap response (gap analysis §Q) and gives a production safety net.
- **Scope:**
  - A scheduled job (every `RECONCILE_INTERVAL_SECONDS=300`, guarded by a Redis lock) that, for each active installation repository with review enabled, lists open PRs (`GET /repos/{o}/{r}/pulls?state=open&per_page=100`, with ETag conditional requests).
  - Compare each `head.sha` with `pull_requests.head_sha` and the existence of a non-superseded run.
  - Synthesize `PullRequestHeadEvent`s through the same orchestration path, with synthetic delivery id `poll:{repo_id}:{pr}:{head_sha}`.
- **Explicit non-scope:** Replacing webhooks. When webhooks work, the reconciler finds nothing.
- **Files/modules expected to change:** `apps/api/src/app.module.ts`.
- **New files/modules expected:** `apps/api/src/providers/github/reconciler.service.ts`
- **Dependencies:** GH-004, GH-005, SUP-001, GH-003.
- **Implementation details:**
  - `@nestjs/schedule` interval. Lock `SET rg:lock:reconciler NX EX 280`.
  - ETags are stored in Redis `rg:gh:etag:{repo}` (304 responses do not count against the rate limit).
  - The same guards as GH-004 apply (draft, bot, branches).
  - `RECONCILER_ENABLED` defaults to true in local development when `GITHUB_WEBHOOK_SECRET` is unset.
- **Data model changes:** None. `webhook_deliveries` stores the synthetic ids with `event='poll'`.
- **API/protocol changes:** None.
- **Concurrency semantics:** A single active reconciler cluster-wide (lock). A race with a real webhook for the same head is resolved by the SUP-001 run idempotency key.
- **Failure behavior:** Rate-limited: stop the cycle early and resume on the next tick. A per-repository error is logged, and the cycle continues.
- **Idempotency considerations:** The synthetic delivery id is deterministic, so repeated polls of the same head are duplicates.
- **Security considerations:** It uses the installation tokens only. No user tokens.
- **Observability additions:**
  - Span `reconcile_cycle`.
  - Counters `reconciler_events_synthesized_total` and `reconciler_repos_scanned_total`.
- **Tests required:**
  - `missed_webhook_head_enqueued`
  - `known_head_not_enqueued`
  - `etag_304_no_work`
  - `lock_prevents_parallel_cycles`
  - `race_with_webhook_single_run`
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** With webhooks disabled, a new head on the fake server leads to a review run within one interval (test with a 1 s interval).
- **Definition of done:** Global DoD, plus documentation in `docs/operations/github-app-setup.md` (DEV-006).

---

### GH-013 — Installation lifecycle events
Status: ☑
> **Implementation note:** The schema already had `provider_installations` (the spec `installations` table), so migration `20261003000011_installation_lifecycle.sql` only adds `state` (active|suspended|deleted) and `deleted_at` there, `enabled` and `access_state` (active|removed|access_lost|installation_deleted) on `repositories`, and the pre-tenant SECURITY DEFINER functions `rg_upsert_installation` (creates or reuses the organization: slug from the account login, a reinstall of the same account reuses its organization), `rg_set_installation_state` and `rg_installation_state`. Installation events are a new provider-neutral `InstallationEvent` (not part of `ProviderEvent`, so they never reach the orchestrator) parsed in `normalize.ts`; `InstallationService` (the lifecycle handler and the `InstallationGate`) runs inside the GH-003 delivery transaction, switches that transaction to the resolved tenant (`set_config`) so RLS applies to every repository write, and moves active review runs to CANCELLED by compare-and-set on `state`. `jobs` does not exist until API-007, so queued-job cancellation goes through a `JobCanceller` port (`JOB_CANCELLER`) with a no-op default that API-007 replaces; the 409 on clone credentials is provided as `InstallationGate.assertUsable` (throws `InstallationInactiveError`) for GH-006 to map, since the broker endpoint does not exist yet; the SEC-007 retention purge is left to that job (it reads `deleted_at`). The installation payload carries no default branch, so a repository created from an event gets the placeholder `main` until the GH-005 sync reads the real one. Cached tokens are purged with SCAN (`rg:gh:itok:{id}:*` plus the mint lock) before commit, best effort; `new_permissions_accepted` stores the permissions and re-runs the GH-010 check after commit. An unknown installation on `deleted`/`suspend`/removal is a recorded no-op answered `accepted:false reason:unknown_installation`.

- **Task ID:** GH-013
- **Title:** Installation lifecycle events
- **Problem:** Installing, uninstalling, suspending and changing repository access must update tenancy and stop work for revoked repositories.
- **Why it exists:** Tenancy correctness and security: there must be no processing after access is revoked.
- **Scope:**
  - `installation.created` upserts the `organizations` row (from the account) and the `installations` row, and lists the repositories.
  - `installation.deleted` marks the installation deleted, disables its repositories, cancels queued jobs, purges cached tokens, and schedules a retention purge (SEC-007).
  - `installation.suspend|unsuspend` toggles processing.
  - `installation_repositories.added|removed` upserts or disables repositories.
  - `installation.new_permissions_accepted` re-runs the GH-010 permission check.
- **Explicit non-scope:** Billing.
- **Files/modules expected to change:**
  - `apps/api/src/providers/github/normalize.ts`
  - `apps/api/src/webhooks/github-webhook.controller.ts`
- **New files/modules expected:** `apps/api/src/providers/github/installation.service.ts`
- **Dependencies:** GH-002, GH-003, API-003, API-007, SEC-007.
- **Implementation details:**
  - `installations (id, organization_id, provider_installation_id UNIQUE, account_login, account_type, state active|suspended|deleted, permissions jsonb, created_at, updated_at)` (migration if not in DOM-009).
  - Disabling a repository cancels its queued jobs:

    ```sql
    UPDATE jobs SET state='cancelled' WHERE state='queued' AND payload->>'repository_id' = ANY($ids);
    ```

    Running review runs move to `CANCELLED` through a CAS.
  - Token purge: `DEL rg:gh:itok:{installation_id}:*` (SCAN).
- **Data model changes:** `installations` and `repositories.enabled`/`access_state`.
- **API/protocol changes:** None external.
- **Concurrency semantics:** Lifecycle handling runs in the webhook transaction. A job claimed in flight checks `repositories.enabled` at its stage boundaries (SUP-002) and stops.
- **Failure behavior:** Unknown installation on `deleted`: no-op and ignored.
- **Idempotency considerations:** Upserts and state transitions are idempotent.
- **Security considerations:** After `deleted`, the credential broker refuses tokens (409), and the retention purge is scheduled.
- **Observability additions:** Counter `installation_events_total{action}`.
- **Tests required:**
  - `installation_created_creates_org_and_repos`
  - `installation_deleted_cancels_jobs_and_purges_tokens`
  - `suspended_installation_refuses_clone_credentials`
  - `repositories_removed_disables`
  - `new_permissions_rechecked`
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** After a replayed `installation.deleted`, no job for that installation is claimable, and the broker returns 409.
- **Definition of done:** Global DoD.

---

---

### SUP-001 — Supersession on new head (single transaction)
Status: ☑
> **Implementation note:** `SupersessionService.startReview` (`apps/api/src/reviews/supersession.service.ts`) implements the transaction with `lock_timeout = 2s`. The schema forbids inserting the new run before the old one leaves the active set (`review_runs_one_active_per_pr`) and requires `superseded_by` on SUPERSEDED rows, so the new run id is chosen up front and migration `20261008000002_supersession.sql` makes the `superseded_by` FK `DEFERRABLE INITIALLY DEFERRED`; it also adds `superseded_at`, `superseded_by_head`, `idempotency_key UNIQUE` and `depth`. The jobs expression index ships with the PIPE-001 `jobs` table. Job enqueue/cancel go through the `REVIEW_JOBS` port (`review-jobs.port.ts`); its default rejects enqueue (rolling the run back) until the API-007 adapter is bound. A head that returns after its run was superseded gets a retry run keyed `...:rerun:<run>`; a manual re-review uses the trigger suffix `manual:<comment id>` with `retry_of`. Stale events compare against `pull_requests.provider_updated_at`. `ReviewOrchestrator` (webhook sink, reconciler) syncs the repository and PR (GH-005) from authoritative reads and then calls `startReview`; closed PRs and `/review cancel` go through `cancelActiveRuns` with the same lock order.

- **Task ID:** SUP-001
- **Title:** Supersession on new head (single transaction: mark older runs SUPERSEDED, cancel queued jobs, enqueue new)
- **Problem:** Legacy checks the head only at prepare time (`daemon.rs:161`). PRD §77 requires that a new head cancels or supersedes the old run and that no obsolete comments are published.
- **Why it exists:** It addresses risk R12. It is the entry point of every review: webhook, manual trigger and reconciler all call it.
- **Scope:** `SupersessionService.startReview(pullRequestId, headSha, baseSha, trigger, depth)` runs one transaction:
  1. Lock the PR row.
  2. Update the head.
  3. Supersede all non-terminal runs with a different head.
  4. Cancel their queued jobs.
  5. Insert the new run (idempotent).
  6. Enqueue `pr-review`.
- **Explicit non-scope:**
  - Engine stage checks (SUP-002).
  - The publish gate (SUP-003).
- **Files/modules expected to change:** `apps/api/src/reviews/supersession.service.ts` (created in API-009 as a stub).
- **New files/modules expected:** `engine/migrations/{seq}_supersession.sql`
- **Dependencies:** API-007, GH-004, GH-005, DOM-008, PIPE-007.
- **Implementation details:**

  ```sql
  BEGIN;
  SELECT id, head_sha FROM pull_requests WHERE id=$pr FOR UPDATE;
  UPDATE pull_requests SET head_sha=$head, base_sha=$base, updated_at=now() WHERE id=$pr;
  WITH old AS (
    UPDATE review_runs SET state='SUPERSEDED', superseded_at=now(), superseded_by_head=$head
    WHERE pull_request_id=$pr AND head_sha <> $head
      AND state NOT IN ('COMPLETED','SUPERSEDED','CANCELLED','FAILED_INDEXING','FAILED_ANALYSIS','FAILED_REVIEW','FAILED_PUBLISH')
    RETURNING id)
  UPDATE jobs SET state='cancelled', updated_at=now()
    WHERE state='queued' AND (payload->>'review_run_id')::uuid IN (SELECT id FROM old);
  INSERT INTO review_runs (id, organization_id, repository_id, pull_request_id, head_sha, base_sha, trigger, depth, state, idempotency_key)
    VALUES (...,'RECEIVED','pr-review:{provider}:{repo}:{pr}:{head}[:{trigger_suffix}]')
    ON CONFLICT (idempotency_key) DO NOTHING RETURNING id;
  -- if inserted: JobQueue.enqueue(trx, {queue:'pr-review', idempotency_key: same key, payload:{review_run_id}})
  COMMIT;
  ```

  - Running jobs (`state='running'`) are **not** killed. They observe SUPERSEDED at their next stage boundary (SUP-002).
  - The migration adds `review_runs.superseded_at`, `superseded_by_head` and `UNIQUE(idempotency_key)`, plus the jobs expression index on `payload->>'review_run_id'`.
- **Data model changes:** As above.
- **API/protocol changes:** None.
- **Concurrency semantics:**
  - The PR row lock serializes all supersessions and publishes for a PR.
  - Two concurrent heads B and C apply in commit order. If B's transaction commits after C's (late delivery), B supersedes C wrongly. Guard against this: the update applies only if `$head` is not an ancestor-older event. Concretely, compare the event's `pull_request.updated_at` with the stored value; a stale event inserts no run and returns `stale_event`.
- **Failure behavior:** Any error rolls back everything: no partial supersession, and GitHub retries the webhook.
- **Idempotency considerations:** The run idempotency key, plus the job key equal to it.
- **Security considerations:** None beyond tenancy (the transaction sets the org).
- **Observability additions:** Span `supersession`. Counters `review_runs_superseded_total` and `jobs_cancelled_total{reason=superseded}`.
- **Tests required:**
  - `new_head_supersedes_running_and_queued`
  - `same_head_twice_one_run`
  - `stale_event_does_not_supersede_newer`
  - `rollback_leaves_no_partial_state`
  - `completed_runs_untouched`
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** All tests pass against real PG. After two heads, exactly one non-terminal run exists, for the newest head.
- **Definition of done:** Global DoD.

---

### SUP-002 — Engine stage-boundary supersession checks
Status: ☐

- **Task ID:** SUP-002
- **Title:** Engine stage-boundary supersession checks
- **Problem:** A running worker on head A must stop promptly after supersession, so that it does not waste model spend and does not produce outputs for an obsolete head.
- **Why it exists:** PRD §77. This is the cooperative cancellation half of supersession.
- **Scope:**
  - Before and after every stage transition, and before every model call batch, the pipeline runs a CAS that fails if the run is no longer in the expected state.
  - A `CancellationToken` per run is signalled by a 2 s poll of `review_runs.state`, or by the `LISTEN review_run_state` notify fired by a trigger.
  - In-flight model calls are allowed to finish but their output is discarded, or they are aborted when the provider supports it.
  - Candidates of the superseded run are marked `INVALIDATED`.
- **Explicit non-scope:** API-side logic (SUP-001/003).
- **Files/modules expected to change:**
  - `engine/crates/pipeline/src/state.rs`
  - `engine/crates/pipeline/src/run.rs`
  - `engine/crates/model-gateway/src/gateway.rs` (accept a `CancellationToken`)
- **New files/modules expected:**
  - `engine/crates/pipeline/src/cancellation.rs`
  - `engine/migrations/{seq}_review_run_notify.sql` (trigger `pg_notify('review_run_state', id||':'||state)`)
- **Dependencies:** SUP-001, PIPE-007, PIPE-003, GW-001.
- **Implementation details:**
  - `async fn transition(&self, run: RunId, from: State, to: State) -> Result<(), Superseded>` runs `UPDATE review_runs SET state=$to, updated_at=now() WHERE id=$1 AND state=$from RETURNING id`. Zero rows means reading the state: if SUPERSEDED or CANCELLED it returns `Err(Superseded)`, and otherwise `Err(Conflict)`.
  - On `Superseded`, the worker:
    - marks the job `succeeded` with outcome `superseded` (it is not a failure, so there is no retry)
    - marks the candidates `INVALIDATED`
    - stops
  - The model-gateway `select!` combines the request future with `token.cancelled()`, and the partial usage is still accounted.
- **Data model changes:** Notify trigger. `candidate_findings.state` gains `INVALIDATED` if it is missing from DOM-006.
- **API/protocol changes:** None.
- **Concurrency semantics:** The cooperative token, plus a CAS on every transition. No stage output is written after the token fires, because each write checks the token inside its transaction.
- **Failure behavior:** If the notify channel is lost, the 2 s poll still catches supersession.
- **Idempotency considerations:** A superseded job completes and is never retried.
- **Security considerations:** None.
- **Observability additions:**
  - Span event `superseded` on the current stage span.
  - Counter `pipeline_superseded_total{stage}`.
  - Histogram `supersession_stop_latency_seconds`.
- **Tests required:**
  - `cas_transition_fails_after_supersede`
  - `worker_stops_within_3s_of_supersede`
  - `model_call_discarded_on_supersede` (replay provider with an artificial delay)
  - `candidates_invalidated`
  - `superseded_job_not_retried`
- **Benchmarks if applicable:** Stop latency p95 < 3 s (integration).
- **Acceptance criteria:** SUP-004 `superseded_during_model_call` passes, with no `stage_outputs` rows written after supersession for that run.
- **Definition of done:** Global DoD.

---

### SUP-003 — Publish-time gate
Status: ☑
> **Implementation note:** `PublishGate.acquire` (`apps/api/src/publisher/publish-gate.ts`) locks `pull_requests` then `review_runs` explicitly (two statements, so the order is guaranteed), with `lock_timeout = 20s`. A skip moves a still-PUBLISHING run to CANCELLED (SUPERSEDED needs `superseded_by`, which only SUP-001 can set) and records `publications.state='skipped'`. The webhook acknowledgement is detached from orchestration (GH-002 ack under 500 ms), so the 2 s lock timeout surfaces in the orchestrator, which retries with backoff; for synchronous callers `55P03` is classified by `isDbUnavailable` and becomes 503 + `Retry-After` (`webhook_lock_timeout_returns_503` asserts that mapping).

- **Task ID:** SUP-003
- **Title:** Publish-time gate (head current + run not superseded, inside publish txn)
- **Problem:** A run can be superseded between the end of verification and the GitHub POST. A check outside the transaction is a TOCTOU race (R12).
- **Why it exists:** It is the last line of defence for "no obsolete comments" (PRD §77), and it is on the critical path.
- **Scope:** `PublishGate.acquire(trx, runId)`:
  - `SELECT ... FROM review_runs rr JOIN pull_requests pr ON pr.id = rr.pull_request_id WHERE rr.id = $1 FOR UPDATE OF rr, pr`.
  - Assert `rr.state = 'PUBLISHING'` and `pr.head_sha = rr.head_sha` and `pr.state = 'open'` and the repository is enabled.
  - Otherwise return `GateResult::Skip{reason}`.
  - The caller (GH-009) performs the GitHub POST while holding this lock, with a 15 s HTTP timeout and `SET LOCAL lock_timeout = '20s'`.
- **Explicit non-scope:** Rendering and posting (GH-009).
- **Files/modules expected to change:** `apps/api/src/publisher/publisher.service.ts`.
- **New files/modules expected:** `apps/api/src/publisher/publish-gate.ts`
- **Dependencies:** SUP-001, GH-009, PIPE-007.
- **Implementation details:**
  - Lock order is always `pull_requests`, then `review_runs`. SUP-001 uses the same order, so there are no deadlocks.
  - On Skip, the run is CAS'd `PUBLISHING → SUPERSEDED` (if the head moved) or `CANCELLED`, and the job ends `succeeded` with outcome `skipped_superseded`.
  - The trade-off is documented: a webhook for a new head waits at most about 15 s while a publish is in flight. That is acceptable, since GitHub's webhook timeout is 10 s and a 503 makes it retry, and the reconciler covers this.
  - To remove the ack wait, the webhook path uses `SET LOCAL lock_timeout='2s'`. On a timeout it returns 503 and GitHub redelivers.
- **Data model changes:** None.
- **API/protocol changes:** None.
- **Concurrency semantics:** These are the serialization semantics described in Scope. The invariant: a review is posted only if, at the moment of posting, the run is current and not superseded. The lock is held until the posting record commits.
- **Failure behavior:**
  - A lock timeout on the publish side causes a job retry with backoff.
  - An HTTP timeout while holding the lock: commit the `posting` state. The retry then adopts the existing review via the marker (GH-009).
- **Idempotency considerations:** The `publications` PK and marker adoption.
- **Security considerations:** None.
- **Observability additions:** Counter `publish_gate_skips_total{reason}`. Histogram `publish_gate_lock_wait_seconds`.
- **Tests required:**
  - `gate_skips_when_head_moved`
  - `gate_skips_when_superseded`
  - `gate_skips_when_pr_closed`
  - `supersede_waits_for_inflight_publish_then_applies`
  - `webhook_lock_timeout_returns_503`
  - `lock_order_no_deadlock` (stress: 100 interleavings)
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** SUP-004 `superseded_during_publish` passes: no review is posted for the obsolete head in 100 randomized interleavings.
- **Definition of done:** Global DoD.

---

### SUP-004 — Concurrency tests
Status: ◐
> **Implementation note:** API-side cases are in `apps/api/integration/concurrency.int.spec.ts` against real Postgres/Redis and the stateful fake GitHub (`test/helpers/fake-github-api.ts`, with `drop_after_write`, 422, status and delay faults): `two_rapid_updates`, `duplicate_webhook`, `publish_retry`, `superseded_during_publish` (100 randomized interleavings), plus `lock_order_no_deadlock` in `supersession.int.spec.ts`. **Missing:** `retry_after_partial_failure` and `superseded_during_model_call` need the engine worker (SUP-002, PIPE-002/005) and the compose harness under `tests/concurrency/`; the nightly 20-iteration job and the trace-id assertion (OBS-003) are not set up.

- **Task ID:** SUP-004
- **Title:** Concurrency tests (two rapid updates, duplicate webhook, retry after partial failure, superseded during model call, publish retry)
- **Problem:** Race conditions are invisible in unit tests. Production readiness (§17) requires proof.
- **Why it exists:** It verifies R12 mitigations end to end with real PG, Redis, a worker and the fake GitHub API.
- **Scope:** An integration suite in `tests/concurrency/`, running on the compose test profile:
  1. `two_rapid_updates`: heads B and C 50 ms apart. Only C is published, and B's run ends SUPERSEDED.
  2. `duplicate_webhook`: 10 parallel deliveries with the same id give 1 run and 1 job.
  3. `retry_after_partial_failure`: the worker is killed mid-VERIFYING. The reaper requeues, the run resumes from `stage_outputs`, and it publishes once.
  4. `superseded_during_model_call`: the replay provider delays 3 s, and a new head arrives at 1 s. There is no publish for the old head, and the candidates are INVALIDATED.
  5. `publish_retry`: the fake GitHub accepts the POST and then drops the connection. The retry adopts the existing review. There is exactly 1 review.
  6. `superseded_during_publish`: SUP-003's randomized interleavings.
- **Explicit non-scope:** Load testing (PERF-008).
- **Files/modules expected to change:** `infra/compose/docker-compose.test.yml` (fault injection flags for the fake server).
- **New files/modules expected:**
  - `tests/concurrency/{package.json,jest.config.ts,harness.ts,*.spec.ts}`
  - `tests/fake-github/faults.ts`
- **Dependencies:** SUP-001, SUP-002, SUP-003, GH-009, DEV-005, PIPE-002 (reaper), PIPE-005.
- **Implementation details:**
  - The harness starts the api, a worker (engine image) and the fake-github server.
  - It sends webhooks with the signed replay library (DEV-005).
  - It inspects PG, and the fake server's request log through `GET /__log`.
  - Fault injection on the fake server: `POST /__faults { route, mode: drop_after_write|delay_ms|status }`.
  - Worker kill: `docker kill` of the worker container in the harness, or a `SIGKILL` of the process when run locally.
- **Data model changes:** None.
- **API/protocol changes:** Fake-server control endpoints only (test infrastructure).
- **Concurrency semantics:** This task's subject.
- **Failure behavior:** A flaky test is a bug. Each test runs 20 iterations in CI nightly and 3 per PR.
- **Idempotency considerations:** The suite asserts it.
- **Security considerations:** None.
- **Observability additions:** Each test asserts that a single trace id spans all spans of the surviving run (OBS-003).
- **Tests required:**
  - `two_rapid_updates`
  - `duplicate_webhook`
  - `retry_after_partial_failure`
  - `superseded_during_model_call`
  - `publish_retry`
  - `superseded_during_publish`
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** All six are green in CI-004 (3 iterations) and nightly (20 iterations) for 7 consecutive days before M7.
- **Definition of done:** Global DoD, plus the suite marked required in CI.

---

---

### CLI-001 — clap skeleton and output (human/json)
Status: ☐

- **Task ID:** CLI-001
- **Title:** clap skeleton + output (human/json)
- **Problem:** No `review` binary exists. Legacy `pr-review` subcommands are the reference consumer- and agy-specific.
- **Why it exists:** PRD §82 local mode. The CLI is the primary debugging tool (PRD §84).
- **Scope:**
  - A clap 4 derive tree with global flags:
    - `--repo <PATH>` (default: discover upward for `.git`)
    - `--format human|json`
    - `--store auto|local|postgres`
    - `--database-url`
    - `--color auto|always|never`
    - `-v/-q`
  - An `Output` abstraction: human tables (`comfy-table`) and JSON (one document on stdout).
  - Logs go to stderr only.
  - Exit codes.
  - Shell completions (`review completions <shell>`).
- **Explicit non-scope:** The subcommand logic (CLI-002..013).
- **Files/modules expected to change:** `engine/Cargo.toml`.
- **New files/modules expected:**
  - `engine/apps/review-cli/{Cargo.toml,src/main.rs,src/cli.rs,src/output.rs,src/exit.rs,src/context.rs}`
- **Dependencies:** FND-002 (cargo.sh), DOM-001, OBS-001 (telemetry init is optional for the CLI and disabled by default).
- **Implementation details:**
  - Exit codes: `0` ok, `1` findings at or above `--fail-on`, `2` usage, `3` not initialized, `4` graph stale (with `--require-current`), `5` provider or auth error, `10` internal.
  - The JSON envelope is `{ "version": 1, "command": "...", "data": ..., "warnings": [...] }`. Its schema is exported to contracts as `cli-output.v1.json`.
  - `context.rs` resolves the repository root, the `.review/` directory, the store adapter and the config.
- **Data model changes:** None.
- **API/protocol changes:** The CLI JSON output schema (versioned).
- **Concurrency semantics:** Single process, with a Tokio runtime for I/O.
- **Failure behavior:** Errors print `error: <message>` plus a hint, with exit code per the table. With `--format json`, the error goes in `{ "error": { code, message } }`.
- **Idempotency considerations:** N/A.
- **Security considerations:** Flags and env holding secrets (`--github-token`, `ANTHROPIC_API_KEY`) are never echoed. `-v` logs pass through redaction.
- **Observability additions:** `REVIEW_OTEL=1` enables OTLP export for CLI runs (same telemetry crate).
- **Tests required:**
  - `help_snapshot` (insta)
  - `json_envelope_schema_valid`
  - `logs_go_to_stderr`
  - `exit_code_not_initialized_3`
  - `completions_generate`
- **Benchmarks if applicable:** Startup time < 50 ms for `review --version` (hyperfine smoke).
- **Acceptance criteria:**
  - `engine/scripts/cargo.sh run -p review-cli -- --help` matches the snapshot.
  - The Windows MSVC build (CI-009) produces `review.exe` and runs `--version`.
- **Definition of done:** Global DoD.

---

### CLI-002 — `review init`
Status: ☐

- **Task ID:** CLI-002
- **Title:** review init
- **Problem:** PRD §12/§141 require `review init [--repository] [--provider] [--force]` to detect the repository facts, write `.review/` and produce persistent graph state with no manual configuration.
- **Why it exists:** It is the entry point for local mode and the M2 milestone check.
- **Scope:**
  - Run the INIT detectors.
  - Print the detection summary.
  - Write `.review/repository.json` and `.review/config.yaml` (from the POL-001 template, only if absent unless `--force`).
  - Run the full index into the local file store (or PG with `--store postgres`).
  - Print the fingerprint and stats.
  - `--no-index` skips indexing.
- **Explicit non-scope:** Provider onboarding (the API handles it). `--provider github` only records the provider in `repository.json`.
- **Files/modules expected to change:** `engine/apps/review-cli/src/cli.rs`.
- **New files/modules expected:** `engine/apps/review-cli/src/commands/init.rs`
- **Dependencies:** CLI-001, INIT-001..INIT-013, IDX-001, GS-006, POL-001, SEC-003 (secret scan at init).
- **Implementation details:**
  - Output sections: Languages · Package managers · Frameworks · Source roots · Tests · Generated · Workspaces · Entrypoints · Migrations · CI · Rule docs · Secrets warnings (count and paths only) · Index stats (files, symbols, edges, duration) · Fingerprint.
  - `--force` re-detects and overwrites `repository.json`. It never overwrites a user-edited `config.yaml` without `--force-config`.
  - Index progress is shown with `indicatif` on stderr.
- **Data model changes:** None. It writes `.review/` files.
- **API/protocol changes:** None.
- **Concurrency semantics:** An advisory file lock `.review/.lock` (fs2), so two `init` runs cannot interleave.
- **Failure behavior:**
  - Unsupported languages only: exit 0, with a warning "no supported language detected; graph empty".
  - Parse errors are tolerated and counted (`parse_failures`).
- **Idempotency considerations:** A re-run without `--force` reuses the parse cache (IDX-005) and finishes much faster.
- **Security considerations:** Secret findings are listed by path and line only, never by value. `.review/` is added to `.gitignore`, except `config.yaml`, after a prompt (or with `--yes`).
- **Observability additions:** Duration and counters are printed. OTLP export is available with `REVIEW_OTEL=1`.
- **Tests required:**
  - `init_fixture_nestjs_detects_prd141_facts` (source roots, TypeScript, package manager, tests, NestJS)
  - `init_writes_review_dir`
  - `init_rerun_uses_cache`
  - `init_lock_prevents_concurrent`
  - `init_does_not_overwrite_config`
  - `init_json_output_schema`
- **Benchmarks if applicable:** Init on reference-api (1,028 files) < 60 s cold in the container (recorded in PERF-002).
- **Acceptance criteria:** On `fixtures/repositories/nestjs-basic`, all PRD §141 initialization bullets are satisfied (asserted in the JSON output), and `.review/graph/snapshots/*.bin.zst` exists.
- **Definition of done:** Global DoD.

---

### CLI-003 — `review status`
Status: ☐

- **Task ID:** CLI-003
- **Title:** review status
- **Problem:** Users need to know whether the graph is current, which versions produced it, and the fingerprint (ADR-015 consequence).
- **Why it exists:** Reproducibility and debugging.
- **Scope:** It prints:
  - repository root and provider
  - the current HEAD commit vs. the last indexed snapshot commit (current/stale, with the number of commits and files changed since)
  - `graph_schema_version`, analyzer versions, `config_hash`, `profile_version` and the fingerprint
  - snapshot chain length (deltas since the last full snapshot)
  - stats (files, symbols, edges, unresolved refs, parse failures)
  - `--require-current` exits 4 when stale
- **Explicit non-scope:** Updating the graph (`review graph rebuild` or the incremental update run by `diff`).
- **Files/modules expected to change:** `engine/apps/review-cli/src/cli.rs`.
- **New files/modules expected:** `engine/apps/review-cli/src/commands/status.rs`
- **Dependencies:** CLI-001, GS-004, GS-006, INIT-012.
- **Implementation details:** Staleness = `snapshot.commit_sha != HEAD` or a dirty worktree touching indexed files (gix status limited to tracked source roots).
- **Data model changes:** None.
- **API/protocol changes:** None.
- **Concurrency semantics:** Read-only. A shared lock.
- **Failure behavior:** Not initialized: exit 3 with the hint `run review init`.
- **Idempotency considerations:** N/A.
- **Security considerations:** None.
- **Observability additions:** None.
- **Tests required:**
  - `status_current_after_init`
  - `status_stale_after_commit`
  - `status_dirty_worktree_stale`
  - `status_require_current_exit_4`
  - `status_json_contains_fingerprint_and_versions`
- **Benchmarks if applicable:** < 200 ms on reference-api.
- **Acceptance criteria:** The JSON output contains every ADR-015 provenance field.
- **Definition of done:** Global DoD.

---

### CLI-004 — `review doctor` (PRD §83 checks)
Status: ☐

- **Task ID:** CLI-004
- **Title:** review doctor (PRD §83 checks)
- **Problem:** Legacy doctor (`main.rs:245-413`) is the reference consumer- and agy-specific. PRD §83 lists nine generic checks.
- **Why it exists:** It is the first-line diagnosis for local and operator setups.
- **Scope:** These checks, each giving `PASS|WARN|FAIL|NOT_EXECUTED` with detail and a fix hint:
  1. repository initialized
  2. graph current
  3. parser availability (tree-sitter grammars load, analyzer versions)
  4. provider authentication (`GITHUB_TOKEN` scopes via `GET /user` and the `x-oauth-scopes` header, or App config)
  5. model configuration (routing table resolves for each tier; keys present; `replay` recognized)
  6. cache health (parse cache readable, size)
  7. storage health (local store integrity checksum, or a PG connection and the migration version)
  8. language support (detected languages vs. analyzers)
  9. config validity (POL-001 errors with key paths, expired suppressions, knowledge source paths)
- **Explicit non-scope:** Auto-fixing.
- **Files/modules expected to change:** `engine/apps/review-cli/src/cli.rs`.
- **New files/modules expected:** `engine/apps/review-cli/src/commands/doctor/{mod.rs,checks.rs}`
- **Dependencies:** CLI-001, CLI-003, POL-001, GW-001 (router), GS-006, GS-005, PROF-007 (knowledge source diagnostics).
- **Implementation details:**
  - `trait DoctorCheck { fn id(&self) -> &'static str; async fn run(&self, ctx: &Ctx) -> CheckReport }`.
  - Checks run concurrently with a 10 s timeout each.
  - A check that cannot run (for example, no network) reports `NOT_EXECUTED`, never PASS (INV-014).
  - Exit 0 if there is no FAIL, else 1.
- **Data model changes:** None.
- **API/protocol changes:** None.
- **Concurrency semantics:** Concurrent independent checks.
- **Failure behavior:** A timeout gives `NOT_EXECUTED("timeout")`.
- **Idempotency considerations:** N/A.
- **Security considerations:** Tokens are never printed. A warning is shown if `GITHUB_TOKEN` has write scopes beyond what is needed (the legacy PAT lesson).
- **Observability additions:** None.
- **Tests required:**
  - `doctor_all_pass_on_initialized_fixture`
  - `doctor_not_executed_never_pass`
  - `doctor_config_error_key_path`
  - `doctor_token_overprivileged_warns` (wiremock)
  - `doctor_model_none_configured_warns`
  - `doctor_timeout_not_executed`
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** The nine checks appear in the output, and the planted failures produce the expected statuses.
- **Definition of done:** Global DoD.

---

### CLI-005 — `review diff <base>..<head>` and `review branch <a> <b>`
Status: ☐

- **Task ID:** CLI-005
- **Title:** review diff <base>..<head> / review branch <a> <b>
- **Problem:** Local review of arbitrary ranges is the M5 milestone mechanism (`review diff` on the auth-bypass fixture).
- **Why it exists:** PRD §82 (`review diff HEAD~1`, `review branch feature/auth dev`).
- **Scope:**
  - Resolve the range (`A..B`, a single rev meaning `rev..HEAD`'s parent semantics, `--staged`, `--worktree`).
  - Incrementally update the head graph (INC).
  - Run the pipeline stages in-process (PIPE in local mode) with the configured model provider. `--provider replay --replay-dir` is available.
  - Print the change summary, risk, findings (verified) and, with `--all`, suppressed findings with their reasons.
  - `--fail-on <severity>`.
  - `review branch a b` is `diff merge-base(a,b)..a`.
- **Explicit non-scope:** Publishing anywhere.
- **Files/modules expected to change:** `engine/apps/review-cli/src/cli.rs`.
- **New files/modules expected:** `engine/apps/review-cli/src/commands/{diff.rs,branch.rs,render.rs}`
- **Dependencies:** CLI-001, CLI-002, DIFF-001..DIFF-007, PIPE-003, PIPE-011 (local pipeline mode), GW-001, VER-001..VER-012.
- **Implementation details:**
  - With no model provider configured, the deterministic stages run, and reviewers report `NOT_EXECUTED("no model provider configured")` in the output and the exit summary (INV-014). Deterministic findings (POL-005, REV-T-002) still print.
  - The human renderer uses the same §58 structure as GH-007, in terminal form with `path:line` hyperlinks (OSC 8).
  - The JSON output carries findings with full evidence, trace ids and the context package hashes.
- **Data model changes:** None. Local runs write `.review/runs/{run_id}.json`.
- **API/protocol changes:** None.
- **Concurrency semantics:** Reviewers run in parallel in-process.
- **Failure behavior:** Unresolvable revs exit 2. Model errors give degraded completion, reported per reviewer.
- **Idempotency considerations:** The same inputs with the replay provider give identical output (PIPE-009 reproducibility).
- **Security considerations:** Context is redacted before model calls (SEC-004). `privacy.external_models: false` refuses to call external providers.
- **Observability additions:** Per-stage timings are printed with `-v`.
- **Tests required:**
  - `diff_auth_bypass_replay_produces_prd151_finding`
  - `diff_without_provider_not_executed`
  - `branch_uses_merge_base`
  - `fail_on_high_exit_1`
  - `diff_json_contains_evidence`
  - `diff_replay_reproducible`
- **Benchmarks if applicable:** End-to-end local diff on the auth-bypass fixture < 10 s under replay (PERF-008 records it).
- **Acceptance criteria:** The M5 exit criterion: the §151 finding is produced and the planted traps are suppressed (`--all` shows the reasons).
- **Definition of done:** Global DoD.

---

### CLI-006 — `review pr <id>` (local review via GitHub API token)
Status: ☐

- **Task ID:** CLI-006
- **Title:** review pr <id> (local review via GitHub API token)
- **Problem:** Developers want to review a remote PR locally (PRD §82 `review pr 184`) without the hosted service.
- **Why it exists:** Private repositories and debugging hosted results.
- **Scope:**
  - Read the PR metadata (base and head SHAs) through the GitHub REST API with `GITHUB_TOKEN` or `gh auth token`.
  - Fetch `refs/pull/{id}/head` into the local repository.
  - Run CLI-005 on `merge-base(base, head)..head`.
  - Print the results.
  - `--remote origin` selects the repository from the remote URL.
- **Explicit non-scope:** **Posting comments.** The CLI never publishes, so the no-merge and no-approve surface stays in the App only.
- **Files/modules expected to change:** `engine/apps/review-cli/src/cli.rs`.
- **New files/modules expected:**
  - `engine/apps/review-cli/src/commands/pr.rs`
  - `engine/apps/review-cli/src/github_readonly.rs`
- **Dependencies:** CLI-005, DIFF-001.
- **Implementation details:**
  - `github_readonly.rs` uses `reqwest` with rustls, and exposes only `GET /repos/{o}/{r}/pulls/{n}`.
  - The no-merge static test (GH-010) covers this file too.
  - The token comes from `--github-token-env NAME` (default `GITHUB_TOKEN`), falling back to `gh auth token`.
- **Data model changes:** None.
- **API/protocol changes:** None.
- **Concurrency semantics:** N/A.
- **Failure behavior:** 404 or 401 exits 5 with a scope hint.
- **Idempotency considerations:** Fetching refs is idempotent.
- **Security considerations:** The token is held in memory only and only GET requests are made. A test asserts that the client has no method other than GET.
- **Observability additions:** None.
- **Tests required:**
  - `pr_fetches_head_ref` (local bare repo plus wiremock)
  - `pr_client_get_only`
  - `pr_401_exit_5`
  - `pr_uses_merge_base`
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** Against wiremock and a local bare repository with `refs/pull/7/head`, the command produces the same findings as `review diff` on the same range.
- **Definition of done:** Global DoD.

---

### CLI-007 — `review graph symbol` / `inspect`
Status: ☐

- **Task ID:** CLI-007
- **Title:** review graph symbol/inspect
- **Problem:** "Without this, debugging incorrect reviews becomes extremely difficult" (PRD §84).
- **Why it exists:** Graph introspection. `inspect` is an alias (gap analysis §P).
- **Scope:**
  - `review graph symbol <QUERY>`, where QUERY is a `SymbolId`, a `SymbolKey`, a qualified name (`AuthService.authorize`) or `path:name`.
  - It prints kind, id, key, location, signature, visibility, framework facts, lineage, in/out edge counts by kind, and confidence histograms.
  - Ambiguous queries list candidates (≤20).
  - `--snapshot` selects the snapshot.
- **Explicit non-scope:** Traversals (CLI-008/009).
- **Files/modules expected to change:** `engine/apps/review-cli/src/cli.rs`.
- **New files/modules expected:**
  - `engine/apps/review-cli/src/commands/graph/{mod.rs,symbol.rs,resolve.rs}`
- **Dependencies:** CLI-001, CG-007, GS-006, SID-001.
- **Implementation details:** `resolve.rs` tries an exact id, then a key, then a qualified-name index, then a fuzzy match (case-insensitive suffix), with deterministic ordering.
- **Data model changes:** None.
- **API/protocol changes:** None.
- **Concurrency semantics:** Read-only.
- **Failure behavior:** Not found exits 1 with "did you mean" suggestions.
- **Idempotency considerations:** N/A.
- **Security considerations:** None.
- **Observability additions:** None.
- **Tests required:**
  - `symbol_by_qualified_name`
  - `symbol_by_key`
  - `inspect_alias`
  - `ambiguous_lists_candidates`
  - `not_found_suggests`
  - `symbol_json_snapshot`
- **Benchmarks if applicable:** < 300 ms, including the graph load from the local store, on reference-api.
- **Acceptance criteria:** `review graph symbol AuthService.authorize` on the auth fixture prints the expected snapshot.
- **Definition of done:** Global DoD.

---

### CLI-008 — `review graph callers` / `callees` / `tests`
Status: ☐

- **Task ID:** CLI-008
- **Title:** review graph callers/callees/tests
- **Problem:** Impact debugging needs direct neighbor listings with confidence (PRD §84, §141 impact criteria).
- **Why it exists:** It covers the MVP acceptance "direct callers/callees/relevant tests can be retrieved".
- **Scope:**
  - `callers <Q> [--depth N≤3] [--min-confidence F]`
  - `callees <Q> [...]`
  - `tests <Q>`: test cases via `TESTS` edges plus the IMP-005 conventions, showing the mapping reason.
  - Output: a tree with `resolved_by` and confidence per edge, plus `truncated`.
- **Explicit non-scope:** Paths (CLI-009).
- **Files/modules expected to change:** `engine/apps/review-cli/src/commands/graph/mod.rs`.
- **New files/modules expected:** `engine/apps/review-cli/src/commands/graph/{neighbors.rs,tests.rs}`
- **Dependencies:** CLI-007, CG-007, IMP-005.
- **Implementation details:** It uses `bounded_bfs(seeds, dir, kinds=[CALLS], max_depth, max_nodes=500, min_confidence)`. The tree view deduplicates repeated nodes with `(see above)`.
- **Data model changes:** None.
- **API/protocol changes:** None.
- **Concurrency semantics:** Read-only.
- **Failure behavior:** Budget truncation prints a `… truncated at 500 nodes` line, and the JSON output carries `truncated: true`.
- **Idempotency considerations:** N/A.
- **Security considerations:** None.
- **Observability additions:** None.
- **Tests required:**
  - `callers_direct`
  - `callers_depth_2`
  - `callees_min_confidence_filters`
  - `tests_shows_mapping_reason`
  - `truncation_reported`
- **Benchmarks if applicable:** Covered by PERF-004.
- **Acceptance criteria:** On the auth fixture, `callers AuthService.authorize --depth 2` includes `AdminService.updateUser` and `UserController.update`, each with its confidence.
- **Definition of done:** Global DoD.

---

### CLI-009 — `review graph path`
Status: ☐

- **Task ID:** CLI-009
- **Title:** review graph path
- **Problem:** Evidence paths in findings (`UserController.update → … → AuthService.authorize`) must be reproducible by hand.
- **Why it exists:** PRD §84 `review graph path A B`. It lets a user verify a finding's claimed relation.
- **Scope:** `review graph path <FROM> <TO> [--kinds CALLS,HANDLED_BY,...] [--max-depth 6] [--all --max-paths 10]` prints the shortest path(s) with per-hop confidence and the path minimum confidence.
- **Explicit non-scope:** Weighted or probabilistic path ranking beyond minimum confidence.
- **Files/modules expected to change:** `engine/apps/review-cli/src/commands/graph/mod.rs`.
- **New files/modules expected:** `engine/apps/review-cli/src/commands/graph/path.rs`
- **Dependencies:** CLI-007, CG-007 (`shortest_path`).
- **Implementation details:** Bidirectional BFS, with ties broken by higher minimum confidence and then lexicographic ids, for determinism.
- **Data model changes:** None.
- **API/protocol changes:** None.
- **Concurrency semantics:** Read-only.
- **Failure behavior:** No path within the depth exits 1 with "no path within depth N".
- **Idempotency considerations:** N/A.
- **Security considerations:** None.
- **Observability additions:** None.
- **Tests required:**
  - `path_auth_bypass_chain`
  - `path_none_within_depth`
  - `path_all_limited`
  - `path_deterministic_tiebreak`
- **Benchmarks if applicable:** PERF-004 includes shortest path p95.
- **Acceptance criteria:** `review graph path UserController.update AuthService.authorize` prints the three-node chain from PRD §58.
- **Definition of done:** Global DoD.

---

### CLI-010 — `review graph rebuild`
Status: ☐

- **Task ID:** CLI-010
- **Title:** review graph rebuild
- **Problem:** PRD §24 requires an explicit full rebuild command for recovery.
- **Why it exists:** It recovers from inconsistency, and after schema or analyzer bumps.
- **Scope:**
  - `review graph rebuild [--verify]` forces a full index of HEAD, writes a new full snapshot and resets the delta chain.
  - `--verify` also runs the INC consistency validator: it compares the previous incremental snapshot with the new full one and reports the differences.
- **Explicit non-scope:** Hosted rebuild (API-008 enqueues the same job type).
- **Files/modules expected to change:** `engine/apps/review-cli/src/commands/graph/mod.rs`.
- **New files/modules expected:** `engine/apps/review-cli/src/commands/graph/rebuild.rs`
- **Dependencies:** CLI-002, IDX-001, INC-011, INC-012 (validator).
- **Implementation details:** It takes the exclusive `.review/.lock`. The parse cache is kept unless `--no-cache`.
- **Data model changes:** None.
- **API/protocol changes:** None.
- **Concurrency semantics:** An exclusive lock.
- **Failure behavior:** A failure keeps the previous snapshots intact: the new snapshot is written to a temporary file and renamed atomically.
- **Idempotency considerations:** Repeated rebuilds of the same HEAD produce identical snapshot content hashes.
- **Security considerations:** None.
- **Observability additions:** Stats are printed.
- **Tests required:**
  - `rebuild_resets_delta_chain`
  - `rebuild_atomic_on_failure`
  - `rebuild_verify_reports_no_diff_on_consistent`
  - `rebuild_deterministic_hash`
- **Benchmarks if applicable:** PERF-003.
- **Acceptance criteria:** After a planted corrupt delta, `rebuild --verify` reports the mismatch and the new snapshot is consistent.
- **Definition of done:** Global DoD.

---

### CLI-011 — `review impact`
Status: ☐

- **Task ID:** CLI-011
- **Title:** review impact
- **Problem:** The impact graph is the key intermediate artifact, and users need to see it directly (PRD §82 `review impact src/auth/auth.service.ts:authorize`).
- **Why it exists:** It is for debugging context selection and risk.
- **Scope:**
  - `review impact <file:symbol | symbol | --diff A..B>` prints the ImpactGraph:
    - callers (distance ≤2)
    - callees
    - implementations and interfaces
    - tests
    - API entrypoints
    - config, DB and queue relations, each with distance, path and minimum confidence
  - With `--diff`, it prints per changed symbol plus the RiskAssessment and the clusters.
- **Explicit non-scope:** Running reviewers.
- **Files/modules expected to change:** `engine/apps/review-cli/src/cli.rs`.
- **New files/modules expected:** `engine/apps/review-cli/src/commands/impact.rs`
- **Dependencies:** CLI-007, IMP-001..IMP-010, RISK-001..RISK-005.
- **Implementation details:** It reuses `impact::build(change_model, graph_pair, budget)`. The `file:symbol` form resolves the symbol within that file.
- **Data model changes:** None.
- **API/protocol changes:** None.
- **Concurrency semantics:** Read-only.
- **Failure behavior:** Truncation is reported per category.
- **Idempotency considerations:** N/A.
- **Security considerations:** None.
- **Observability additions:** None.
- **Tests required:**
  - `impact_file_symbol_form`
  - `impact_diff_lists_risk_and_clusters`
  - `impact_entrypoints_listed`
  - `impact_json_snapshot`
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** On the auth fixture, the impact output lists `http:PATCH /users/:id` as an entrypoint, plus the mapped tests.
- **Definition of done:** Global DoD.

---

### CLI-012 — `review profile`
Status: ☐

- **Task ID:** CLI-012
- **Title:** review profile
- **Problem:** Users need to see what the system inferred about their repository, so they can correct it or override it with explicit policy (R10 mitigation).
- **Why it exists:** It covers PRD §82 `review profile` and the precedence transparency of PRD §65.
- **Scope:**
  - `review profile [--section architecture|conventions|testing|api|queues|security|docs] [--recompute]` prints the profile.
  - Conventions are listed with samples, violations, consistency, confidence, scope, exceptions and the enforceable flag.
  - The effective policy per topic (POL-004) is shown with its winner and the overridden sources.
- **Explicit non-scope:** Editing.
- **Files/modules expected to change:** `engine/apps/review-cli/src/cli.rs`.
- **New files/modules expected:** `engine/apps/review-cli/src/commands/profile.rs`
- **Dependencies:** CLI-001, PROF-001..PROF-007, POL-004.
- **Implementation details:** `--recompute` bypasses the PROF-006 cache for the current snapshot.
- **Data model changes:** None.
- **API/protocol changes:** None.
- **Concurrency semantics:** Read-only, unless `--recompute` is given (exclusive lock).
- **Failure behavior:** No profile yet: the hint `run review init`.
- **Idempotency considerations:** N/A.
- **Security considerations:** None.
- **Observability additions:** None.
- **Tests required:**
  - `profile_lists_conventions_with_confidence`
  - `profile_effective_policy_shows_winner`
  - `profile_section_filter`
  - `profile_json_matches_contract_schema`
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** On `nestjs-layered`, the output shows the four PROF-004 conventions with the golden values.
- **Definition of done:** Global DoD.

---

### CLI-013 — `review migrate` and `contracts export`
Status: ☐

- **Task ID:** CLI-013
- **Title:** review migrate + contracts export
- **Problem:**
  - ADR-014: migrations run through `review migrate` or `review-worker migrate`. The API type generation (API-002) needs this.
  - The contracts (JSON Schema from Rust types) need an exporter that CI-006 can diff.
- **Why it exists:** It keeps one schema source and one contract source.
- **Scope:**
  - `review migrate --database-url <URL> [--dry-run] [--target <version>]` applies `engine/migrations` via `sqlx::migrate!` and prints the applied versions.
  - `review contracts export --out packages/contracts/schemas` writes every `schemars` schema: domain, job payloads, engine DTOs, config v1, CLI output and the service token. Output is deterministic (sorted keys).
- **Explicit non-scope:** Down migrations. Expand/contract is a process, not tooling.
- **Files/modules expected to change:** `engine/apps/review-cli/src/cli.rs`.
- **New files/modules expected:**
  - `engine/apps/review-cli/src/commands/{migrate.rs,contracts.rs}`
  - `engine/crates/review-core/src/contracts_registry.rs` (the list of exported types)
- **Dependencies:** CLI-001, DOM-009, FND (contracts package).
- **Implementation details:**
  - `migrate` takes a PG advisory lock (sqlx does this), so concurrent deploy jobs serialize. `--dry-run` lists pending migrations.
  - `contracts export` writes `{name}.json`. The TS types are generated afterwards by `pnpm --filter contracts gen` (`json-schema-to-typescript`).
- **Data model changes:** None.
- **API/protocol changes:** The contracts files (generated).
- **Concurrency semantics:** The migration lock.
- **Failure behavior:** A failing migration stops, and sqlx records the dirty state. Exit 10 with the failing version.
- **Idempotency considerations:** Applying migrations is idempotent. The export is deterministic.
- **Security considerations:** `--database-url` is never echoed. It can also be supplied through env `DATABASE_URL`.
- **Observability additions:** None.
- **Tests required:**
  - `migrate_applies_all_on_empty_db`
  - `migrate_idempotent`
  - `migrate_dry_run_lists_pending`
  - `contracts_export_deterministic`
  - `contracts_registry_covers_job_payloads`
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** CI-005 and CI-006 use these commands, and two consecutive exports produce byte-identical output.
- **Definition of done:** Global DoD.
