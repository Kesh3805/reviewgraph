# Phases 30–37 — Frontend, graph explorer, observability, security, performance, quality, CI/CD, local development

**Parent:** [MASTER_IMPLEMENTATION_PLAN.md](../MASTER_IMPLEMENTATION_PLAN.md) §9 · WEB, GX, OBS, SEC, PERF, QB, CI, DEV

Status markers: ☐ todo · ◐ in progress · ☑ done (acceptance criteria executed and passed). The global Definition of Done in master plan §10 applies to every task.

## Task index

| ID | Title |
|---|---|
| WEB-001 | Next.js skeleton (Tailwind, shadcn/ui, session via API, TanStack Query) |
| WEB-002 | Layout, navigation and Dashboard |
| WEB-003 | Repositories list and Repository Overview |
| WEB-004 | Repository Intelligence and Repository Profile pages |
| WEB-005 | Pull Requests list |
| WEB-006 | Review Detail |
| WEB-007 | Finding Detail with React Flow impact path and base/head evidence |
| WEB-008 | Rules, Integrations, Usage and Settings pages |
| WEB-009 | Feedback actions |
| GX-001 | Graph explorer symbol search |
| GX-002 | Cytoscape subgraph view |
| GX-003 | Impact path view |
| OBS-001 | Rust telemetry crate (tracing JSON + OTLP HTTP to OpenObserve) |
| OBS-002 | NestJS OpenTelemetry SDK |
| OBS-003 | `traceparent` propagation through jobs |
| OBS-004 | Span coverage of the review lifecycle |
| OBS-005 | Metrics instruments |
| OBS-006 | Redaction layers and tests |
| OBS-007 | OpenObserve dashboards as code |
| OBS-008 | Alerts and runbooks |
| SEC-001 | Tenant isolation tests (API + RLS) |
| SEC-002 | Qdrant and PostgreSQL tenant-filter audit tests |
| SEC-003 | Secret detection at init and index time |
| SEC-004 | Redaction before model send |
| SEC-005 | Provider token and GitHub App private key handling |
| SEC-006 | Webhook replay protection |
| SEC-007 | Source retention policies |
| SEC-008 | Audit log |
| SEC-009 | Object storage access controls |
| SEC-010 | Threat model document and dependency scanning |
| PERF-001 | Synthetic large-repo generator |
| PERF-002 | Parse throughput benchmark |
| PERF-003 | Graph build and memory benchmark (1M symbols) |
| PERF-004 | Neighbor lookup and bounded BFS benchmarks (memory and SQL) |
| PERF-005 | Incremental update benchmark |
| PERF-006 | Context selection benchmark |
| PERF-007 | Qdrant retrieval benchmark |
| PERF-008 | End-to-end review latency (replay) and k6 webhook load test |
| QB-001 | Quality corpus expansion |
| QB-002 | Acceptance metric pipeline from feedback |
| QB-003 | Regression gate in CI |
| QB-004 | Confidence calibration |
| QB-005 | Cost per useful finding reporting |
| QB-006 | Quality dashboard |
| CI-001 | Rust fmt, clippy and test (Linux, GitHub Actions) |
| CI-002 | cargo deny and cargo audit |
| CI-003 | TypeScript lint, typecheck and test |
| CI-004 | Integration tests with services |
| CI-005 | Migration validation and Kysely type drift |
| CI-006 | Contract drift check |
| CI-007 | Benchmark smoke |
| CI-008 | Docker builds |
| CI-009 | Windows CLI build (MSVC runner) |
| DEV-001 | Full compose including api, engine, worker and web |
| DEV-002 | Seed data and fixtures load |
| DEV-003 | One-command bootstrap |
| DEV-004 | `docs/operations/local-development.md` |
| DEV-005 | Signed webhook replay tool and fake GitHub API server |
| DEV-006 | GitHub App setup document and ngrok tunnel |

---

### WEB-001 — Next.js skeleton (Tailwind, shadcn/ui, session via API, TanStack Query)
Status: ☑
> **Implementation note:** Built against the stubbed API contract: `lib/api/schema.ts` hand-writes the `paths` type for `GET /api/v1/auth/me` and `POST /api/v1/auth/logout` and `pnpm --filter @reviewgraph/web gen:api` is wired to regenerate from `packages/contracts/openapi/api.json` once API-008 publishes it (then swap the import in `lib/api-client.ts`). The session cookie name `rg_session` and the login URL `/api/v1/auth/github/login` are assumptions to confirm in API-004 (the middleware only checks cookie presence; `getSession()` validates through `/auth/me`). Playwright and Lighthouse are deferred (no browsers in this environment): `middleware_redirects_without_session` is a Vitest unit test on the middleware and the app was smoke-tested with `next build`, `next start` and `next dev`. shadcn/ui components are committed by hand in the generator output style (`components.json` is present for the CLI); only button and card exist so far. Server OTel uses `@vercel/otel` via `instrumentation.ts`, active only when `OTEL_EXPORTER_OTLP_ENDPOINT` is set. The nonce CSP forces dynamic rendering (root layout reads the nonce). `/` is a placeholder authenticated page until WEB-002.

- **Task ID:** WEB-001
- **Title:** Next.js skeleton (Tailwind, shadcn/ui, session via API, TanStack Query)
- **Problem:** The legacy UI is a single Vite page (`ui/src/App.tsx`, 441 lines) with no auth, no routing and no data layer.
- **Why it exists:** It is the foundation for every screen in target-arch §6.
- **Scope:**
  - The `apps/web` package and App Router layout.
  - Tailwind 4 and the shadcn/ui init.
  - A typed API client generated from `packages/contracts/openapi/api.json` (`openapi-typescript` + `openapi-fetch`).
  - TanStack Query provider.
  - Session handling: server components call `GET /auth/me` with forwarded cookies. A `middleware.ts` redirects to `/login` when there is no session.
  - The CSRF header is injected for mutations.
  - Error boundary and not-found pages.
  - Dark mode.
- **Explicit non-scope:** Individual screens (WEB-002..009).
- **Files/modules expected to change:** `pnpm-workspace.yaml`, root `package.json`.
- **New files/modules expected:**
  - `apps/web/{package.json,next.config.ts,tsconfig.json,postcss.config.mjs,components.json,middleware.ts}`
  - `apps/web/app/{layout.tsx,login/page.tsx,error.tsx,not-found.tsx}`
  - `apps/web/lib/{api-client.ts,query-client.tsx,session.ts,csrf.ts}`
  - `apps/web/components/ui/*` (shadcn)
- **Dependencies:** API-001, API-004, API-008 (OpenAPI available), FND-001.
- **Implementation details:**
  - `next.config.ts` rewrites `/api/*` to `API_INTERNAL_URL`, so the browser and the API share an origin and SameSite=Lax cookies work.
  - The CSP is strict: `default-src 'self'`, with no inline scripts apart from the Next nonce.
  - `csrf.ts` reads the `rg_csrf` cookie and sets `X-CSRF-Token` on every non-GET request.
  - `api-client.ts` exposes typed `GET`/`POST` and maps problem+json errors into `ApiError`.
  - Lint: `@next/eslint-plugin-next`, plus the shared config from `packages/config`.
- **Data model changes:** None.
- **API/protocol changes:** None. The web app consumes the API.
- **Concurrency semantics:** TanStack Query handles caching: `staleTime` 30 s, refetch on focus.
- **Failure behavior:**
  - A 401 from any query redirects to `/login`.
  - A 5xx shows the error boundary with a retry button.
  - The OpenAPI client is regenerated in CI. Drift fails CI-006.
- **Idempotency considerations:** Mutations disable their controls while pending and rely on API idempotency.
- **Security considerations:**
  - Strict CSP.
  - Cookies are never readable by JS apart from the CSRF token.
  - No tokens are kept in localStorage.
  - Untrusted text is rendered as text, never with `dangerouslySetInnerHTML`. An ESLint rule bans it.
- **Observability additions:** `@vercel/otel` or the Next OTel instrumentation exports server spans to OTLP (OBS-002 config), with W3C traceparent propagated to the API.
- **Tests required:**
  - `middleware_redirects_without_session` (Playwright)
  - `csrf_header_set_on_mutation` (unit)
  - `api_error_maps_problem_json`
  - `no_dangerously_set_inner_html_lint`
  - `build_succeeds` (`next build`)
- **Benchmarks if applicable:** Lighthouse performance ≥ 90 on the login page (smoke).
- **Acceptance criteria:**
  - `pnpm --filter web dev` serves the login page.
  - After fake OAuth login (DEV-005), `/` renders the authenticated shell.
  - Playwright passes.
- **Definition of done:** Global DoD.

---

### WEB-002 — Layout, navigation and Dashboard
Status: ☑
> **Implementation note:** Web side only. The API half (`apps/api/src/dashboard/*`, `GET /api/v1/organizations/:id/dashboard`, the 60 s Redis cache and the `dashboard_endpoint_tenant_scoped` / 150 ms benchmark) is deferred: it needs the database layer and review/finding tables (API-002, API-009, API-012) that are not built yet. The web client codes against the response shape in `lib/dashboard.ts` (sections may be null for a per-card unavailable state) and is tested with MSW; until the endpoint exists the cards show their error state. Active runs come from the same dashboard payload and the query polls every 5 s only while any run is non-terminal (`refetchInterval`). The org switcher stores the preference in a non-credential `rg_org` cookie that is always re-resolved against the session memberships. Nav targets other than `/` (Repositories, Pull Requests, Rules, Integrations, Usage, Settings) render the not-found page until WEB-003..009 add them. The root `app/layout.tsx` needed no change.

- **Task ID:** WEB-002
- **Title:** Layout/nav + Dashboard
- **Problem:** Users need an at-a-glance view of review activity and quality, replacing the legacy `Header`/`Stats` (App.tsx:34-90).
- **Why it exists:** It is the Dashboard screen from target-arch §6.
- **Scope:**
  - App shell: sidebar with Dashboard · Repositories · Pull Requests · Rules · Integrations · Usage · Settings, an organization switcher and a user menu.
  - Dashboard cards:
    - reviews in the last 7 days (completed, degraded, failed)
    - median and p95 review latency
    - published findings by severity
    - acceptance rate and FP rate (API-012 summary)
    - an active-runs table (state, stage, age)
  - Every metric links to its source list.
- **Explicit non-scope:**
  - Usage and cost (WEB-008).
  - Charts beyond simple sparklines.
- **Files/modules expected to change:** `apps/web/app/layout.tsx`.
- **New files/modules expected:**
  - `apps/web/app/(app)/layout.tsx`
  - `apps/web/app/(app)/page.tsx`
  - `apps/web/components/shell/{Sidebar.tsx,OrgSwitcher.tsx,UserMenu.tsx}`
  - `apps/web/components/dashboard/{StatCard.tsx,ActiveRunsTable.tsx}`
  - `apps/api/src/dashboard/{dashboard.controller.ts,dashboard.service.ts}` (`GET /organizations/:id/dashboard`)
- **Dependencies:** WEB-001, API-009, API-012.
- **Implementation details:**
  - The dashboard endpoint aggregates in SQL over `review_runs`, `findings` and `feedback` in a 7-day window. It is cached in Redis for 60 s per organization.
  - The active-runs table polls every 5 s through `refetchInterval` while any run is non-terminal.
  - Severity colours come from a shared token map.
- **Data model changes:** None.
- **API/protocol changes:** `GET /api/v1/organizations/:id/dashboard`.
- **Concurrency semantics:** Read-only.
- **Failure behavior:** Card-level error states. One failed metric does not blank the page.
- **Idempotency considerations:** N/A.
- **Security considerations:** The organization comes from the session membership. The org switcher only lists the user's memberships.
- **Observability additions:** None beyond the auto spans.
- **Tests required:**
  - `dashboard_renders_cards` (RTL with MSW)
  - `active_runs_polls_only_when_nonterminal`
  - `org_switcher_lists_memberships`
  - `dashboard_endpoint_tenant_scoped` (api)
- **Benchmarks if applicable:** The dashboard API responds in under 150 ms for 10k runs (seeded).
- **Acceptance criteria:** With DEV-002 seed data the dashboard shows non-zero values matching SQL spot checks.
- **Definition of done:** Global DoD.

---

### WEB-003 — Repositories list and Repository Overview
Status: ◐
> **Implementation note:** The web client now uses the generated OpenAPI types (`lib/api/generated.ts`, `gen:api` writes it and runs prettier); routes that are not in the document yet are hand-typed in `lib/api/pending.ts`, merged in `lib/api/schema.ts`, and every call lives in `lib/api/endpoints.ts`. The generated document renders top-level nullable strings as `string[]` (`primary_language`, `initialized_at`, `next_cursor`, `profile_computed_at`), so `pending.ts` restates those DTO fields until the API fixes its OpenAPI output. No task specifies the "Add repository" source, so the dialog calls an assumed `GET /installations/repositories?organization_id=` (installation repositories flagged `enabled`). The last-index column comes from the real `GET /repositories/:id/status` per row; last review and open PRs come from an assumed `GET /organizations/:id/repository-activity` (API-009) and show a dash until it exists. Recent reviews use the API-009 pull-request list and top risk areas an assumed `GET /repositories/:id/risk-areas`. The settings form uses react-hook-form with a zod schema that mirrors `UpdateRepositorySettingsSchema` from the API DTO (the contracts package has no settings schema). The session type was aligned with the real `GET /auth/me` (`display_name`, `slug`, roles owner/admin/member/viewer; `member` is the maintainer role). Tests are Vitest + Testing Library + MSW; the Playwright compose flow is deferred with WEB-001's Playwright setup.

- **Task ID:** WEB-003
- **Title:** Repositories list + Repository Overview
- **Problem:** Users have nowhere to see the onboarded repositories, their index state, or how to enable or initialize them.
- **Why it exists:** It provides the Repositories and Repository Overview screens.
- **Scope:**
  - `/repositories`: a table with name, provider, enabled, last index (time and state), last review, open PRs. It also has an "Add repository" dialog that lists installation repositories not yet enabled, calling `POST /repositories`.
  - `/repositories/[repoId]`: an overview with an initialize/rebuild action (maintainer only), the status card (from API-008 status), recent reviews, top risk areas, and settings for target branches, drafts and bots.
- **Explicit non-scope:** Intelligence and profile details (WEB-004).
- **Files/modules expected to change:** None.
- **New files/modules expected:**
  - `apps/web/app/(app)/repositories/page.tsx`
  - `apps/web/app/(app)/repositories/[repoId]/{page.tsx,layout.tsx}`
  - `apps/web/components/repositories/{RepoTable.tsx,AddRepoDialog.tsx,StatusCard.tsx,RepoSettingsForm.tsx}`
- **Dependencies:** WEB-002, API-008.
- **Implementation details:**
  - The repository layout has tabs: Overview · Intelligence · Profile · Pull Requests · Rules · Graph.
  - Actions use `useMutation` and optimistic disable. A 409 shows "index already running (job …)".
  - The settings form uses react-hook-form with a zod resolver built from the contracts schema.
- **Data model changes:** None.
- **API/protocol changes:** None.
- **Concurrency semantics:** The status card polls every 5 s while an index job is active.
- **Failure behavior:** A 403 on an action shows "requires maintainer role", and the button is hidden for viewers.
- **Idempotency considerations:** Relies on the API keys (initialize is idempotent).
- **Security considerations:** Role-based rendering is cosmetic. The API enforces roles.
- **Observability additions:** None.
- **Tests required:**
  - `repo_table_renders`
  - `add_repo_dialog_lists_unenabled`
  - `initialize_disabled_while_running`
  - `viewer_sees_no_actions`
  - `settings_form_validation`
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** A Playwright flow (add repository → initialize → status shows indexing → indexed) passes against compose with the fixture repository.
- **Definition of done:** Global DoD.

---

### WEB-004 — Repository Intelligence and Repository Profile pages
Status: ☐

- **Task ID:** WEB-004
- **Title:** Repository Intelligence + Repository Profile pages
- **Problem:** Graph health and the inferred conventions are invisible in the UI, so users cannot trust or correct them (R10).
- **Why it exists:** It provides the target-arch §6 Repository Intelligence and Repository Profile screens.
- **Scope:**
  - **Intelligence tab:**
    - snapshot list (full and delta, chain length)
    - fingerprint and versions
    - graph stats: nodes and edges by kind, unresolved references, parse failures, edge confidence histogram, `resolved_by` breakdown
    - languages and frameworks
    - the index job history
  - **Profile tab:**
    - layers and the module matrix
    - a conventions table: rule, scope, samples, violations, consistency, confidence, enforceable badge, exceptions
    - documentation rules
    - the effective policy per topic, with winner and overridden sources (POL-004)
- **Explicit non-scope:** Editing the profile. A "how to override" link points to the config reference.
- **Files/modules expected to change:** `apps/api/src/repositories/repositories.controller.ts` (adds `GET /repositories/:id/intelligence`).
- **New files/modules expected:**
  - `apps/web/app/(app)/repositories/[repoId]/intelligence/page.tsx`
  - `apps/web/app/(app)/repositories/[repoId]/profile/page.tsx`
  - `apps/web/components/intelligence/{SnapshotList.tsx,GraphStats.tsx,ConfidenceHistogram.tsx}`
  - `apps/web/components/profile/{ConventionsTable.tsx,LayerMatrix.tsx,EffectivePolicy.tsx}`
- **Dependencies:** WEB-003, API-008, PROF-001..PROF-007, POL-004.
- **Implementation details:**
  - The intelligence endpoint reads `snapshots.stats jsonb` (written by IDX and INC) and never computes over graph rows at request time.
  - The confidence histogram uses 10 buckets, rendered with a small chart component (recharts).
- **Data model changes:** None.
- **API/protocol changes:** `GET /api/v1/repositories/:id/intelligence`.
- **Concurrency semantics:** Read-only.
- **Failure behavior:** No profile shows an empty state with "Profile is computed after the first full index".
- **Idempotency considerations:** N/A.
- **Security considerations:** Tenant-scoped through the API.
- **Observability additions:** None.
- **Tests required:**
  - `graph_stats_render`
  - `conventions_table_marks_enforceable`
  - `effective_policy_shows_overrides`
  - `empty_profile_state`
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** For `nestjs-layered` (seeded), the profile page shows the four PROF-004 conventions with their golden values.
- **Definition of done:** Global DoD.

---

### WEB-005 — Pull Requests list
Status: ◐
> **Implementation note:** Built against the API-009 contract while API-009 is not merged: the routes and `PullRequestSummary` (with `latest_run {state, degraded}` and published `findings_by_severity`) are hand-typed in `apps/web/lib/api/pending.ts`. API-009 only specifies `GET /repositories/:id/pull-requests`; the organization-wide list assumes `GET /pull-requests?organization_id=` with the same filters (`state`, `has_findings`, `severity`, `cursor`, `limit`). Run states are the contracts `ReviewState` set (RECEIVED…FAILED_*, SUPERSEDED, CANCELLED); "Completed (degraded)" is `COMPLETED` with `degraded: true`. Remaining: switch to generated types and run the seeded acceptance check once API-009 lands.

- **Task ID:** WEB-005
- **Title:** Pull Requests list
- **Problem:** Users need to find PRs and their latest review state.
- **Why it exists:** It provides the Pull Requests screen and replaces the legacy reviews table (App.tsx:348+).
- **Scope:**
  - `/pull-requests` (organization-wide) and `/repositories/[repoId]/pulls`: columns are number and title, author, head SHA (short), latest run state badge, findings by severity, updated time.
  - Filters: repository, state, has findings, severity.
  - Cursor pagination.
  - Rows link to Review Detail.
  - No retry button. Users trigger a re-review with a manual review action (maintainer only) that calls `POST /pull-requests/:id/review`.
- **Explicit non-scope:** Review detail.
- **Files/modules expected to change:** None.
- **New files/modules expected:**
  - `apps/web/app/(app)/pull-requests/page.tsx`
  - `apps/web/app/(app)/repositories/[repoId]/pulls/page.tsx`
  - `apps/web/components/pulls/{PullsTable.tsx,RunStateBadge.tsx,Filters.tsx}`
- **Dependencies:** WEB-002, API-009.
- **Implementation details:**
  - `RunStateBadge` maps all PIPE states, including SUPERSEDED, CANCELLED, the FAILED_* states and "Completed (degraded)".
  - Filter state is kept in URL search params, so views are shareable.
- **Data model changes:** None.
- **API/protocol changes:** None.
- **Concurrency semantics:** Rows with non-terminal runs refetch every 5 s.
- **Failure behavior:** Standard error and empty states.
- **Idempotency considerations:** The manual trigger is idempotent per minute (API-009).
- **Security considerations:** None beyond the API.
- **Observability additions:** None.
- **Tests required:**
  - `pulls_table_filters_via_url`
  - `run_state_badge_all_states`
  - `manual_review_maintainer_only`
  - `pagination_cursor`
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** The seeded data shows superseded and degraded runs with the correct badges.
- **Definition of done:** Global DoD.

---

### WEB-006 — Review Detail
Status: ☐

- **Task ID:** WEB-006
- **Title:** Review Detail (PR summary, change summary, risk summary, findings, changed files, evidence)
- **Problem:** The legacy `DetailPane` (App.tsx:237-346) showed findings, payload and summary tabs plus approve buttons. There was no change model, risk or coverage.
- **Why it exists:** It is the main debugging and review screen, and the only place coverage gaps are fully visible (INV-013/014).
- **Scope:** `/reviews/[reviewId]` with these sections:
  - **Summary:** PR metadata, run state and stage timeline with durations, `trace_id` link to OpenObserve, and completeness (planned, succeeded, failed and not-executed reviewers).
  - **Change:** files by status, behavioral symbols, API contracts, dependencies and schemas (from the change model).
  - **Risk:** level, score, signals and effects (reviewers chosen, depth, budgets).
  - **Findings:** published first, then a "Suppressed (N)" collapsible grouped by reason, plus relocated (outside diff) findings.
  - **Files:** the changed-file list with the findings per file.
  - **Evidence:** the per-finding evidence summary, linking to Finding Detail.
  - A review history selector for the same PR (superseded runs).
- **Explicit non-scope:**
  - The finding deep-dive (WEB-007).
  - Approve and publish actions, which are deliberately absent.
- **Files/modules expected to change:** `apps/api/src/reviews/reviews.controller.ts` (expose `GET /reviews/:id/change-model` and `GET /reviews/:id/risk` if not included in the detail).
- **New files/modules expected:**
  - `apps/web/app/(app)/reviews/[reviewId]/page.tsx`
  - `apps/web/components/review/{StageTimeline.tsx,Completeness.tsx,ChangeSummary.tsx,RiskSummary.tsx,FindingsList.tsx,SuppressedGroup.tsx,ChangedFiles.tsx,TraceLink.tsx}`
- **Dependencies:** WEB-005, API-009, API-010, CHG-006..CHG-008, RISK-005.
- **Implementation details:**
  - The change model and risk come from `stage_outputs` (they are JSON contract types), not recomputed.
  - `TraceLink` builds `${OPENOBSERVE_UI_URL}/web/traces?trace_id=...` from public config.
  - Not-executed checks render as a grey "NOT EXECUTED — reason" chip, never green.
- **Data model changes:** None.
- **API/protocol changes:** Possibly two read endpoints (above).
- **Concurrency semantics:** Polls while the run is non-terminal.
- **Failure behavior:** Missing stage outputs (an early-failed run) show the stage at which it failed and its error class (no stack traces).
- **Idempotency considerations:** N/A.
- **Security considerations:**
  - No prompts or raw model output are shown.
  - Code snippets are fetched through the API-011 redacted source endpoint only.
- **Observability additions:** None.
- **Tests required:**
  - `completeness_shows_failed_reviewer`
  - `not_executed_never_green`
  - `suppressed_grouped_by_reason`
  - `relocated_findings_section`
  - `trace_link_rendered`
  - `no_approve_or_publish_controls` (asserts that no such button exists)
- **Benchmarks if applicable:** Time to interactive under 2 s for a run with 200 findings (Playwright trace).
- **Acceptance criteria:** For the E2E-001 run, the page shows the §151 finding, the risk signal `auth_path`, and the security reviewer as succeeded.
- **Definition of done:** Global DoD.

---

### WEB-007 — Finding Detail with React Flow impact path and base/head evidence
Status: ☐

- **Task ID:** WEB-007
- **Title:** Finding Detail with React Flow impact path + base/head evidence
- **Problem:** An operator must be able to reconstruct why a comment exists (PRD §85/§86) without database access.
- **Why it exists:** It is the explainability UI, using the curated path visualization (React Flow) from target-arch §6.
- **Scope:** `/findings/[findingId]` shows:
  - header (severity, title, reviewer@version, computed confidence with its component breakdown bars)
  - anchor with a redacted source excerpt (head)
  - **Impact path**: React Flow with left-to-right nodes from entrypoint to changed symbol, edge labels showing kind and confidence, and a dashed style for confidence < 0.6
  - **Base/head**: side-by-side excerpts of the anchor symbol at base and head, with the verification predicate result on each
  - a verification stages table (stage, outcome, evidence items)
  - dedup merges
  - effective policy
  - publication (provider comment link)
  - the feedback panel (WEB-009)
- **Explicit non-scope:** Free graph exploration (GX).
- **Files/modules expected to change:** None.
- **New files/modules expected:**
  - `apps/web/app/(app)/findings/[findingId]/page.tsx`
  - `apps/web/components/finding/{ConfidenceBreakdown.tsx,ImpactPathFlow.tsx,BaseHeadCompare.tsx,VerificationStages.tsx,SourceExcerpt.tsx}`
- **Dependencies:** WEB-006, API-010 (trace), API-011 (source excerpts), VER-006, VER-009.
- **Implementation details:**
  - `ImpactPathFlow` uses `@xyflow/react` with a dagre layout and at most 30 nodes (curated paths only). Clicking a node opens the GX symbol page.
  - `SourceExcerpt` highlights with `shiki` server-side and renders the text from the API, already redacted.
  - The confidence breakdown shows the ADR-011 terms (anchor, deterministic, graph, repo, reproduction, agreement, contradiction, uncertainty) with their weights from `verification_version`.
- **Data model changes:** None.
- **API/protocol changes:** None.
- **Concurrency semantics:** Read-only.
- **Failure behavior:**
  - A missing base excerpt (file added in the PR) shows "file did not exist at base".
  - An incomplete trace shows the available stages plus a notice.
- **Idempotency considerations:** N/A.
- **Security considerations:**
  - Excerpts are capped at 200 lines and redacted.
  - The page never shows prompts or raw model output.
- **Observability additions:** None.
- **Tests required:**
  - `impact_path_renders_nodes_in_order`
  - `low_confidence_edge_dashed`
  - `base_head_added_file_message`
  - `confidence_components_sum_matches`
  - `no_prompt_text_rendered`
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** For the §151 finding, the flow shows `UserController.update → AdminService.updateUser → AuthService.authorize`, the base/head panels show the removed `PermissionService.check()` call, and the confidence matches the API.
- **Definition of done:** Global DoD.

---

### WEB-008 — Rules, Integrations, Usage and Settings pages
Status: ☐

- **Task ID:** WEB-008
- **Title:** Rules, Integrations, Usage, Settings pages
- **Problem:** Several target-arch §6 screens remain: policy visibility, GitHub App status, model and cost usage, and organization settings.
- **Why it exists:** It completes the screen list. Usage covers PRD §115 cost metrics.
- **Scope:**
  - **Rules** (`/repositories/[repoId]/rules`): the effective `.review/config.yaml` (read-only, syntax-highlighted), validation errors, explicit rules with recent violation counts, and suppressions with create/revoke (maintainer) and audit history.
  - **Integrations** (`/integrations`): GitHub App installation status, permissions check result (GH-010), webhook health (last delivery, signature failures in the last 24 h) and reconciler status.
  - **Usage** (`/usage`): tokens, model calls and cost by day, by reviewer and by model tier; cost per reviewed PR; cost per useful finding (QB-005).
  - **Settings** (`/settings`): members and roles (admin), data retention policy (SEC-007), and model privacy (`external_models`).
- **Explicit non-scope:**
  - Billing.
  - Editing the config file (it is repository-owned).
- **Files/modules expected to change:** `apps/api/src/repositories/*`, `apps/api/src/organizations/*` (usage and settings endpoints).
- **New files/modules expected:**
  - `apps/web/app/(app)/{integrations,usage,settings}/page.tsx`
  - `apps/web/app/(app)/repositories/[repoId]/rules/page.tsx`
  - `apps/web/components/{rules,usage,settings,integrations}/*.tsx`
  - `apps/api/src/usage/{usage.controller.ts,usage.service.ts}`
- **Dependencies:** WEB-003, POL-001..POL-006, GH-010, GH-012, SEC-007, GW (accounting rows in `model_calls`), QB-005.
- **Implementation details:**
  - Usage aggregates `model_calls (review_run_id, tier, provider, model, input_tokens, output_tokens, cached_tokens, cost_usd_micros)` by day in SQL.
  - Members management: `PATCH /organizations/:id/members/:userId { role }` (admin only, audited).
- **Data model changes:** None, beyond using the existing tables.
- **API/protocol changes:**
  - `GET /organizations/:id/usage?from&to&group_by`
  - `GET/PATCH /organizations/:id/settings`
  - `GET /organizations/:id/integrations/github`
  - `PATCH /organizations/:id/members/:userId`
- **Concurrency semantics:** Read-mostly. Settings updates use optimistic concurrency (an `updated_at` precondition, 409 on conflict).
- **Failure behavior:** Standard.
- **Idempotency considerations:** PATCHes are idempotent.
- **Security considerations:**
  - Settings and members require admin. All changes are audited (SEC-008).
  - Disabling `external_models` takes effect for the next runs and is displayed prominently.
- **Observability additions:** None.
- **Tests required:**
  - `rules_page_shows_validation_errors`
  - `suppression_revoke_audited`
  - `integrations_shows_permission_check`
  - `usage_groups_by_tier`
  - `settings_admin_only`
  - `settings_conflict_409`
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** All four pages render with seed data. A suppression created in the UI is matched on the next E2E run.
- **Definition of done:** Global DoD.

---

### WEB-009 — Feedback actions
Status: ☐

- **Task ID:** WEB-009
- **Title:** Feedback actions
- **Problem:** The PRD §70 feedback must be one click away, wherever findings appear.
- **Why it exists:** Feedback feeds the KPIs (§116/§117) and calibration.
- **Scope:**
  - A `FeedbackMenu` on finding cards (WEB-006) and on Finding Detail (WEB-007): Useful · False positive · Already handled · Not relevant · Intentional, with an optional comment.
  - For Intentional or Not relevant, maintainers get a "Suppress future occurrences" checkbox (fingerprint, symbol or path).
  - The current user's verdict is shown, along with aggregate counts.
- **Explicit non-scope:** GitHub reaction ingestion (API-012 handles it on the server side).
- **Files/modules expected to change:**
  - `apps/web/components/review/FindingsList.tsx`
  - `apps/web/app/(app)/findings/[findingId]/page.tsx`
- **New files/modules expected:** `apps/web/components/feedback/{FeedbackMenu.tsx,SuppressOption.tsx}`
- **Dependencies:** WEB-006, WEB-007, API-012, POL-006.
- **Implementation details:**
  - Uses `useMutation` with an optimistic update. On error, it rolls back and shows a toast.
  - Mutations include the CSRF header (WEB-001).
- **Data model changes:** None.
- **API/protocol changes:** None.
- **Concurrency semantics:** Latest wins (API upsert).
- **Failure behavior:** A 403 on suppress shows the role message, and the verdict is still saved.
- **Idempotency considerations:** Repeated clicks upsert.
- **Security considerations:** The comment is plain text. CSRF protection applies.
- **Observability additions:** None. The server counters come from API-012.
- **Tests required:**
  - `feedback_optimistic_update_and_rollback`
  - `suppress_option_maintainer_only`
  - `verdict_persisted_on_reload` (Playwright)
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** Marking the E2E finding "false positive" shows up in the dashboard FP rate after refresh.
- **Definition of done:** Global DoD.

---

---

### GX-001 — Graph explorer symbol search
Status: ☐

- **Task ID:** GX-001
- **Title:** Graph explorer symbol search
- **Problem:** Users need to find symbols before they can explore them. This is the MVP "GX basic search".
- **Why it exists:** It provides the CodeGraph Explorer entry point.
- **Scope:**
  - `/repositories/[repoId]/graph`: a search box with debounced (200 ms) `GET /graph/symbols?q=` and kind filters (class, method, function, endpoint, queue, table, test).
  - Results show the qualified name, kind, path:line and in/out degree.
  - A selected result opens a symbol panel with the fields from CLI-007.
  - A snapshot selector (default branch latest, or a PR head from a review).
- **Explicit non-scope:** The visual graph (GX-002).
- **Files/modules expected to change:** None.
- **New files/modules expected:**
  - `apps/web/app/(app)/repositories/[repoId]/graph/page.tsx`
  - `apps/web/components/graph/{SymbolSearch.tsx,SymbolPanel.tsx,SnapshotSelect.tsx}`
- **Dependencies:** WEB-003, API-011, API-013.
- **Implementation details:**
  - The engine search ranks by exact name, then prefix, then substring over a name index, limit 50 (API-013).
  - The URL state (`?symbol=<key>&snapshot=<id>`) is deep-linkable from Finding Detail.
- **Data model changes:** None.
- **API/protocol changes:** None.
- **Concurrency semantics:** Previous requests are cancelled through AbortController.
- **Failure behavior:** Engine unavailable shows "Graph service unavailable" with a retry.
- **Idempotency considerations:** N/A.
- **Security considerations:** Tenant-scoped through the proxy.
- **Observability additions:** None.
- **Tests required:**
  - `search_debounced_and_cancelled`
  - `kind_filter_applied`
  - `deep_link_opens_symbol`
  - `snapshot_select_changes_query`
- **Benchmarks if applicable:** Search p95 < 150 ms end to end on reference-api.
- **Acceptance criteria:** Searching "authorize" on the auth fixture finds `AuthService.authorize` first.
- **Definition of done:** Global DoD.

---

### GX-002 — Cytoscape subgraph view
Status: ☐

- **Task ID:** GX-002
- **Title:** Cytoscape subgraph view (callers/callees/impls/tests/deps, server-side subgraphs ≤500 nodes)
- **Problem:** Navigating structure visually helps debugging, but a client cannot load whole graphs.
- **Why it exists:** It is the Cytoscape explorer from target-arch §6, with server-side subgraphs only.
- **Scope:**
  - A `CytoscapeGraph` component rendering `POST /graph/subgraph {seeds:[key], depth 1..3, kinds, max_nodes ≤500}`.
  - Toggles: callers, callees, implementations, tests, dependencies.
  - Node styles by kind. Edge width by confidence, dashed below 0.6.
  - Expand a node on double-click (incremental subgraph merge, staying within 500 total).
  - A `truncated` banner.
  - Export as PNG.
- **Explicit non-scope:**
  - Sigma.js and large-graph rendering (deferred per target-arch §9).
  - Editing.
- **Files/modules expected to change:** `apps/web/app/(app)/repositories/[repoId]/graph/page.tsx`.
- **New files/modules expected:**
  - `apps/web/components/graph/{CytoscapeGraph.tsx,GraphToolbar.tsx,styles.ts}`
- **Dependencies:** GX-001, API-011, API-013.
- **Implementation details:**
  - `cytoscape` with the `cytoscape-fcose` layout. The component is loaded with `dynamic(..., { ssr:false })`.
  - The client-side merge dedupes by node key and refuses to expand past 500 nodes, prompting the user to narrow instead.
- **Data model changes:** None.
- **API/protocol changes:** None.
- **Concurrency semantics:** At most one expansion request is in flight at a time.
- **Failure behavior:** A 400 (budget) or `truncated` result shows a banner.
- **Idempotency considerations:** N/A.
- **Security considerations:** None beyond the proxy.
- **Observability additions:** None.
- **Tests required:**
  - `renders_subgraph_nodes_edges`
  - `expand_merges_without_duplicates`
  - `expansion_capped_at_500`
  - `low_confidence_edges_dashed`
  - `kind_toggles_refetch`
- **Benchmarks if applicable:** Rendering 500 nodes stays at ≥30 fps interaction (manual performance check, recorded).
- **Acceptance criteria:** Starting from `AuthService.authorize` with depth 2 callers, the view shows the controller and admin service nodes. Toggling tests adds the test-case nodes.
- **Definition of done:** Global DoD.

---

### GX-003 — Impact path view
Status: ☐

- **Task ID:** GX-003
- **Title:** Impact path view
- **Problem:** Users want to ask "how does A reach B?" interactively (the CLI-009 equivalent).
- **Why it exists:** It is the explorer counterpart of evidence paths, used to verify finding claims.
- **Scope:**
  - A "Path" mode in the explorer: pick FROM and TO (with symbol search), plus kinds and max depth (≤6).
  - Calls `GET /graph/path` and renders the path(s) in React Flow (reusing `ImpactPathFlow`), with per-hop confidence and the path minimum confidence.
  - A "Show impact for review" mode: `GET /reviews/:id/impact/:symbolKey` renders the ImpactGraph categories as grouped lists plus a path flow.
- **Explicit non-scope:** Probabilistic path ranking.
- **Files/modules expected to change:**
  - `apps/web/app/(app)/repositories/[repoId]/graph/page.tsx`
  - `apps/web/components/finding/ImpactPathFlow.tsx` (made reusable)
- **New files/modules expected:** `apps/web/components/graph/{PathFinder.tsx,ImpactGroups.tsx}`
- **Dependencies:** GX-001, WEB-007, API-011, API-013, IMP-001..IMP-008.
- **Implementation details:** With `all=true&max_paths=5`, up to 5 alternative paths are shown as tabs.
- **Data model changes:** None.
- **API/protocol changes:** None.
- **Concurrency semantics:** Read-only.
- **Failure behavior:** No path shows "no path within depth N".
- **Idempotency considerations:** N/A.
- **Security considerations:** None.
- **Observability additions:** None.
- **Tests required:**
  - `path_finder_renders_chain`
  - `no_path_message`
  - `impact_groups_render_categories`
  - `alternative_paths_tabs`
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** The UI path from `UserController.update` to `AuthService.authorize` matches the `review graph path` CLI output for the same snapshot (compared in a test).
- **Definition of done:** Global DoD.

---

---

### OBS-001 — Rust telemetry crate (tracing JSON + OTLP HTTP to OpenObserve)
Status: ☑
> **Implementation note:** Pinned set is opentelemetry/-sdk/-otlp/-appender-tracing 0.33 with tracing-opentelemetry 0.34 (exporters use the blocking reqwest client on their own threads, so `init` needs no runtime; the OTLP HTTP client has no TLS yet, so `https` endpoints need the `reqwest-rustls` feature later). The JSON format is a custom layer (`json_format.rs`) because trace/span ids cannot be read through `Span::current()` inside a subscriber callback; the layer keeps a weak dispatch handle instead. `telemetry_export_failures_total` is implemented (atomic plus OTel counter, stderr line at most once per minute); `telemetry_dropped_spans_total` is not, because the SDK batch processors do not expose drops. The OTel crates' own tracing events are filtered off by default. The redaction slot is `init_with(config, Some(layer))`. The `span_overhead` Criterion bench and an `emit` example (OpenObserve verification) are included; `review-cli doctor` does not exist yet, so the OpenObserve check used `examples/emit.rs` (traces and logs verified in OpenObserve search). Extra test files: `init_twice.rs`, `json_log.rs`, `otlp_shutdown.rs`, `otlp_unreachable.rs`. The "Environment variables" table is in `docs/operations/observability.md`.

- **Task ID:** OBS-001
- **Title:** `telemetry` crate: `tracing` JSON logs, OTLP/HTTP traces, metrics and logs to OpenObserve, single `init()` for every Rust binary
- **Problem:** The `telemetry` crate is an empty shell. Without a shared initializer each binary (`review-cli`, `review-worker`, `review-engine`) would configure logging differently, and nothing reaches OpenObserve.
- **Why it exists:** ADR-013 and target-architecture §8. Master plan §14 says observability starts in Phase 1, and GW-010, IDX-*, CTX-* and the benchmarks all emit through this crate.
- **Scope:**
  - `telemetry::init(TelemetryConfig) -> TelemetryGuard`, which installs the global subscriber and the OTel tracer, meter and logger providers.
  - JSON log output to stdout and a pretty human format for `review-cli`.
  - OTLP HTTP/protobuf export of traces, metrics and logs to `{OTEL_EXPORTER_OTLP_ENDPOINT}/v1/{traces,metrics,logs}`.
  - Resource attributes, env-driven config, a standard attribute constants module, and a clean `shutdown()` flush.
- **Explicit non-scope:**
  - Metric instrument definitions (OBS-005).
  - The redaction layer itself (OBS-006); this task only leaves the hook point in the layer stack.
  - Span coverage of the lifecycle (OBS-004) and `traceparent` through jobs (OBS-003).
  - The NestJS SDK (OBS-002).
- **Files/modules expected to change:** `engine/Cargo.toml` (workspace deps), `engine/crates/telemetry/Cargo.toml`, `engine/deny.toml` only if a new license appears, and the `main` of the three apps (a single `telemetry::init` call each).
- **New files/modules expected:**
  - `engine/crates/telemetry/src/{config.rs,init.rs,otlp.rs,attrs.rs,guard.rs}`
  - `engine/crates/telemetry/tests/{init_noop.rs,otlp_export.rs}`
- **Dependencies (task IDs):** FND-001, FND-002, FND-005 (OpenObserve endpoint), DOM-001 (typed IDs for attribute helpers).
- **Implementation details:**
  - Crates: `tracing`, `tracing-subscriber` (json, env-filter, registry), `tracing-opentelemetry`, `opentelemetry`, `opentelemetry_sdk` (rt-tokio), `opentelemetry-otlp` (`http-proto`, `reqwest-client`), `opentelemetry-appender-tracing`. Pin exact versions in the workspace and confirm the compatible set at implementation time with `cargo tree -d`.
  - `TelemetryConfig::from_env()` reads `OTEL_EXPORTER_OTLP_ENDPOINT` (dev: `http://127.0.0.1:25080/api/default`), `OTEL_EXPORTER_OTLP_HEADERS` (`Authorization=Basic <base64>`), `OTEL_SERVICE_NAME`, `RUST_LOG`, `RG_LOG_FORMAT=json|pretty`, `RG_OTEL_ENABLED=true|false`, `OTEL_TRACES_SAMPLER_ARG` (default 1.0). Inside the dev container the host endpoint is replaced by `http://host.docker.internal:25080/api/default`; `engine/scripts/cargo.sh` already forwards both OTEL variables.
  - Layer stack (order matters): `EnvFilter` -> redaction layer slot (OBS-006) -> JSON fmt layer -> `OpenTelemetryLayer` (spans) -> OTel log appender. If the endpoint is unset or `RG_OTEL_ENABLED=false`, only the stdout layers are installed and `init` still succeeds.
  - Resource: `service.name`, `service.version` (`env!("CARGO_PKG_VERSION")`), `service.instance.id` (hostname + pid), `deployment.environment`, plus `git.sha` when `RG_GIT_SHA` is set.
  - Batch processors: spans (queue 2048, 5 s schedule), metrics (periodic reader, 15 s), logs (batch). Exporter timeout 10 s.
  - `attrs.rs` defines `pub const` names for the standard attributes (`request_id, review_run_id, repository_id, organization_id, pull_request_id, commit_sha, job_id, reviewer_type, candidate_finding_id`) and a `correlation_span!` helper that takes typed IDs.
  - JSON log shape: `{timestamp, level, target, message, span: {...}, spans: [...], trace_id, span_id}` with correlation attributes flattened from the span.
  - `TelemetryGuard::drop` and `shutdown().await` call `force_flush` then `shutdown` with a 5 s bound.
- **Data model changes:** None.
- **API/protocol changes:** OTLP/HTTP protobuf to OpenObserve paths `/api/{org}/v1/traces`, `/v1/metrics`, `/v1/logs`. Public Rust API: `init`, `TelemetryConfig`, `TelemetryGuard`, `attrs::*`.
- **Concurrency semantics:** `init` is callable once per process; a second call returns `TelemetryError::AlreadyInitialized` instead of panicking. Exporters run on their own threads or the Tokio runtime and never block callers; span and log queues are bounded and drop on overflow.
- **Failure behavior:** An unreachable OpenObserve never fails the process. Export errors are counted into an internal `telemetry_export_failures_total` and logged at most once per 60 s to stderr. Invalid config (bad header string) fails `init` with a typed error at startup, not at first export.
- **Idempotency considerations:** `init` with the same config in tests is guarded by a `OnceLock`; the test helper `telemetry::testing::init_for_test()` is safe to call from many tests.
- **Security considerations:** The OTLP auth header is read from env only and is never logged or included in `Debug` output (wrapped in a `Secret` newtype). Telemetry stays local by default (endpoint unset means no network). Redaction (OBS-006) is a required layer: `init` refuses to build a production stack without it once OBS-006 lands.
- **Observability additions:** This task is the foundation; it adds `telemetry_export_failures_total` and `telemetry_dropped_spans_total` self-metrics.
- **Tests required (named):**
  - `init_without_endpoint_is_stdout_only`
  - `init_twice_returns_already_initialized`
  - `json_log_contains_trace_and_span_ids`
  - `correlation_attrs_flattened_into_log_line`
  - `otlp_export_posts_protobuf_to_expected_paths` (wiremock; asserts `/v1/traces` and the auth header)
  - `unreachable_endpoint_does_not_fail_or_block` (call returns under 100 ms)
  - `debug_output_never_prints_auth_header`
  - `shutdown_flushes_pending_spans`
- **Benchmarks if applicable:** Criterion `span_overhead`: an instrumented empty span adds under 2 microseconds with exporter disabled; document the number.
- **Acceptance criteria (verifiable):**
  - `engine/scripts/cargo.sh test -p telemetry` passes.
  - With `pnpm dev:up` running and the endpoint set, `review-cli doctor` produces a trace visible in OpenObserve search under service `review-cli` within 30 s.
  - `engine/scripts/cargo.sh clippy -p telemetry --all-targets -- -D warnings` and `cargo deny check` pass.
- **Definition of done:** Acceptance criteria pass; the three binaries call `init`; `docs/operations/observability.md` gains an "Environment variables" table.

---

### OBS-002 — NestJS OpenTelemetry SDK
Status: ☑
> **Implementation note:** The SDK core lives in `telemetry/sdk.ts` (`instrumentation.ts` is the `--import` side-effect entry), and `start`/`start:dev` load it with `node --import ./dist/telemetry/instrumentation.js`. The http server span is named `HTTP <method> <route>` by a Nest interceptor (route is only known after Express matches). Because Jest bypasses the Node module hooks auto-instrumentation needs, the http/pg/ioredis tests run in a child process (`test/telemetry/run.cjs`) against in-process fake Postgres/Redis servers. The http span query string is overwritten with the path; ioredis spans record only the command name. `otel_export_failures_total` is created lazily on the SDK meter when an export fails (log line limited to once a minute). The `request_id` attribute is set by the request-id middleware. Verified against local OpenObserve (traces and logs ingested). The `ioredis` instrumentation currently emits each command span twice with ioredis 5.11 (cosmetic, upstream). Metric instruments remain OBS-005 and redaction OBS-006 (injection point is the `REDACT_PATHS`/logger factory).

- **Task ID:** OBS-002
- **Title:** `@opentelemetry/sdk-node` bootstrap for `apps/api` with http/pg/ioredis instrumentation, structured logger and graceful flush
- **Problem:** The control plane has no telemetry. The review trace begins at the webhook (`webhook_received`), which is NestJS code, so TypeScript must export spans, metrics and logs to the same OpenObserve backend as Rust.
- **Why it exists:** ADR-013 (NestJS stack), target-architecture §5 (`telemetry` module) and §8.
- **Scope:**
  - `apps/api/src/telemetry/instrumentation.ts`, loaded first through `node --import` (or `--require`) before Nest bootstraps.
  - Auto-instrumentation for http, pg and ioredis; a helper for manual stage spans.
  - A `TelemetryModule` exposing a typed `Tracer` wrapper and the standard attribute names.
  - A structured JSON logger that replaces the Nest default and attaches `trace_id`/`span_id`.
- **Explicit non-scope:**
  - Redaction implementation (OBS-006 supplies the formatter; this task only leaves the injection point).
  - Metric instruments (OBS-005) and job-row `traceparent` (OBS-003).
  - Dashboards (OBS-007).
- **Files/modules expected to change:** `apps/api/src/main.ts` (start script and logger swap), `apps/api/package.json` (scripts: `start` uses `--import ./dist/telemetry/instrumentation.js`), `apps/api/src/app.module.ts` (import `TelemetryModule`).
- **New files/modules expected:**
  - `apps/api/src/telemetry/{instrumentation.ts,telemetry.module.ts,tracer.service.ts,attributes.ts,json-logger.ts,config.ts}`
  - `apps/api/test/telemetry/*.spec.ts`
- **Dependencies (task IDs):** API-001, FND-005, OBS-001 (attribute names must match the Rust constants).
- **Implementation details:**
  - Packages: `@opentelemetry/sdk-node`, `@opentelemetry/exporter-trace-otlp-proto`, `@opentelemetry/exporter-metrics-otlp-proto`, `@opentelemetry/exporter-logs-otlp-proto`, `@opentelemetry/instrumentation-http`, `-pg`, `-ioredis`, `@opentelemetry/resources`, `@opentelemetry/semantic-conventions`. Pin exact versions in `pnpm-lock.yaml`.
  - Config from the same env names as Rust: `OTEL_EXPORTER_OTLP_ENDPOINT` (dev `http://127.0.0.1:25080/api/default`), `OTEL_EXPORTER_OTLP_HEADERS`, `OTEL_SERVICE_NAME=reviewgraph-api`, `RG_OTEL_ENABLED`. Config is parsed by the existing config validation from API-001 (zod); a malformed header fails boot.
  - The instrumentation file must run before any `pg`/`http` import. Use the `--import` flag rather than importing inside `main.ts`.
  - `attributes.ts` mirrors the Rust constants: `request_id, review_run_id, repository_id, organization_id, pull_request_id, commit_sha, job_id, reviewer_type, candidate_finding_id`.
  - `TracerService.withSpan(name, attrs, fn)` starts a child span of the active context, records exceptions, sets status, and always ends the span. Only the stage names from target-architecture §8 are accepted (a union type).
  - Disable noisy instrumentations (`fs`, `dns`, `net`). Ignore `/health/*` and `/metrics` routes in the http instrumentation.
  - Logger: one JSON line per event with `timestamp, level, context, message, trace_id, span_id` plus correlation attributes from the active span; it is also bridged to the OTel Logs API.
  - SIGTERM hook: `sdk.shutdown()` after Nest `app.close()`, bounded to 5 s.
- **Data model changes:** None.
- **API/protocol changes:** Incoming `traceparent` headers on HTTP requests are honored; responses carry `x-request-id` (also the `request_id` attribute).
- **Concurrency semantics:** Context propagation uses AsyncLocalStorage, so concurrent requests keep separate spans. The batch span processor is bounded (queue 2048) and drops on overflow.
- **Failure behavior:** Telemetry failures never change request outcomes. If OpenObserve is down, exporters log once per minute and drop. `RG_OTEL_ENABLED=false` yields a no-op SDK.
- **Idempotency considerations:** SDK start is guarded by a module-level flag; hot-reload in dev does not double-register instrumentations.
- **Security considerations:** The auth header and request/response bodies are never recorded as attributes. The http instrumentation request hook records method, route and status only (no query strings, no `authorization`, `x-hub-signature-256` or cookie headers). pg statements are recorded with parameters stripped (`enhancedDatabaseReporting: false`).
- **Observability additions:** Spans `HTTP <method> <route>`, `pg.query`, `ioredis.*`; self-counter `otel_export_failures_total`.
- **Tests required (named):**
  - `sdk_boots_without_endpoint_as_noop`
  - `http_request_creates_server_span_with_route`
  - `incoming_traceparent_becomes_parent`
  - `pg_query_span_has_no_parameter_values`
  - `ioredis_command_span_created`
  - `health_route_not_traced`
  - `logger_attaches_trace_and_span_ids`
  - `with_span_records_exception_and_ends`
  - `sensitive_headers_never_in_span_attributes`
- **Benchmarks if applicable:** None; the instrumentation overhead is covered by PERF-008 load runs.
- **Acceptance criteria (verifiable):**
  - `pnpm --filter api test` passes.
  - With the stack up, `curl localhost:<api-port>/health/ready` is silent but `POST /api/v1/webhooks/github` appears in OpenObserve as a `reviewgraph-api` trace.
  - A log line in OpenObserve links to its trace through `trace_id`.
- **Definition of done:** Acceptance criteria pass; `docs/operations/observability.md` documents the API start flags; `pnpm lint` and `pnpm typecheck` pass.

---

### OBS-003 — `traceparent` propagation through jobs
Status: ☐

- **Task ID:** OBS-003
- **Title:** Persist W3C `traceparent` on every job row and restore it in Rust and TypeScript consumers so one PR review is one trace
- **Problem:** Asynchronous hops through the PostgreSQL queue break trace context. Without propagation the webhook span, the worker spans and the publisher spans appear as unrelated traces.
- **Why it exists:** ADR-013 ("Trace continuity"), ADR-012 (`jobs.trace_parent`), target-architecture §5 and §8, and MVP exit criterion 7 ("inspectable as one trace in OpenObserve").
- **Scope:**
  - Producers (TS `JobQueue`, Rust `pipeline::jobs`) write the current context as `trace_parent` (and `trace_state` when present) at enqueue time.
  - Consumers (Rust worker, TS publisher consumer) restore the context as the parent of the job span.
  - Job spans use span links back to the enqueuing span as well as parentage for retries.
  - Rust-to-API calls (credential broker, internal endpoints) propagate `traceparent` through HTTP headers.
- **Explicit non-scope:**
  - Choosing the lifecycle spans (OBS-004).
  - The queue implementation itself (API-007, PIPE-*).
  - Sampling policy beyond head sampling at 100 percent in dev.
- **Files/modules expected to change:** `apps/api/src/jobs/*` (enqueue and consume paths from API-007), `engine/crates/pipeline/src/jobs/*` (enqueue, claim, heartbeat), the Rust HTTP client used for API calls (API-005).
- **New files/modules expected:**
  - `engine/crates/telemetry/src/propagation.rs` (`inject_traceparent() -> Option<String>`, `extract_into(span, &str)`)
  - `apps/api/src/telemetry/propagation.ts`
  - `engine/crates/pipeline/tests/trace_propagation.rs`, `apps/api/test/jobs/trace-propagation.spec.ts`, `tests/e2e/trace-continuity.test.ts`
- **Dependencies (task IDs):** OBS-001, OBS-002, API-007, DOM-009 (`jobs.trace_parent` column exists per ADR-012), API-005.
- **Implementation details:**
  - Format: W3C `traceparent` (`00-<32 hex trace id>-<16 hex span id>-<2 hex flags>`) validated with a strict regex on read; invalid values are treated as absent.
  - Enqueue: the producer calls the propagator with the active context, so the value is written in the same INSERT as the job (same transaction as the business row, per ADR-012). The webhook handler's `webhook_received` span is the root, so `pr-review` jobs inherit it directly.
  - Claim: the Rust consumer reads `trace_parent` from the claimed row, builds a remote parent context, and opens the `job` span with `set_parent`. Span attributes: `job_id, queue, attempt, idempotency_key`. A retry attempt creates a new span with the same parent plus a link to the previous attempt's span id when stored in `jobs.last_error` metadata.
  - Chained jobs: a job that enqueues another (`repository-index` to `pr-review`, `pr-review` to `review-publish`) injects its own current span, so the trace is a tree, not a chain of unrelated roots.
  - HTTP: Rust clients add `traceparent` using the OTel text-map propagator; NestJS http instrumentation extracts it, which links the internal credential-broker call into the trace.
  - The W3C `tracestate` is carried only if present and shorter than 512 bytes.
- **Data model changes:** None new (`jobs.trace_parent text` already exists in ADR-012). A migration is required only if DOM-009 omitted `trace_state`; prefer to keep a single column and drop tracestate for the MVP.
- **API/protocol changes:** `traceparent` header on Rust-to-API internal HTTP calls and on the review-engine API calls from NestJS.
- **Concurrency semantics:** Context lives in the span of the job handler only; it is never stored globally. Concurrent jobs in one worker each hold their own parent.
- **Failure behavior:** A missing or malformed `trace_parent` starts a new root span with a `rg.trace_origin="missing"` attribute and a warning counter; the job is never failed because of tracing.
- **Idempotency considerations:** Jobs deduplicated by `idempotency_key` keep the first enqueuer's `trace_parent`; the second enqueuer's context is attached as a span link on the existing job's next attempt, not overwritten.
- **Security considerations:** `traceparent` carries only random ids and flags, never tenant data. It is not trusted for authorization. Inbound `traceparent` on the public webhook is ignored (a new root is started) so external callers cannot inject traces.
- **Observability additions:** Counter `trace_context_missing_total{queue}`; attribute `rg.trace_origin=propagated|missing|invalid`.
- **Tests required (named):**
  - `enqueue_writes_traceparent_from_active_span`
  - `claim_restores_parent_and_shares_trace_id`
  - `retry_attempt_keeps_parent_and_links_previous`
  - `invalid_traceparent_starts_new_root_and_counts`
  - `chained_job_inherits_current_span`
  - `duplicate_idempotency_key_keeps_first_traceparent`
  - `public_webhook_ignores_inbound_traceparent`
  - `e2e_webhook_to_publish_single_trace_id` (fake collector asserts every span of a replayed webhook share one trace id)
- **Benchmarks if applicable:** None.
- **Acceptance criteria (verifiable):**
  - The e2e test passes: spans from `reviewgraph-api` and `review-worker` carry one `trace_id` for a replayed webhook.
  - In OpenObserve, opening that trace shows `webhook_received` as root with worker spans beneath it.
- **Definition of done:** Acceptance criteria pass; a short "trace continuity" section is added to `docs/operations/observability.md`.

---

### OBS-004 — Span coverage of the review lifecycle
Status: ☐

- **Task ID:** OBS-004
- **Title:** Canonical lifecycle spans with standard attributes, and a test that fails when a stage lacks its span
- **Problem:** Each phase adds spans independently, so names drift and attributes go missing. The PRD §114 trace hierarchy and target-architecture §8 names must be one enforced contract.
- **Why it exists:** MVP exit criterion 7. Debugging a slow or failed review needs one trace from webhook to publication with consistent names.
- **Scope:**
  - A registry of span names: `webhook_received, repository_checkout, repository_index, incremental_graph_update, diff_analysis, symbol_mapping, impact_analysis, context_selection, qdrant_search, reviewer_execution, model_request, candidate_generated, finding_verification, deduplication, publication`.
  - Per-span required attributes, helpers that enforce them, and tracing of the parents/children layout.
  - A conformance test that runs the replay pipeline and asserts the expected span tree.
  - Adding any still-missing instrumentation in the owning crates.
- **Explicit non-scope:**
  - Metric instruments (OBS-005).
  - Redefining the span names (fixed by target-architecture §8).
  - Dashboards (OBS-007).
- **Files/modules expected to change:** `engine/crates/{repository,incremental,diff-engine,impact,context-engine,semantic,reviewers,verification,pipeline}/src/**` (add or normalize `#[instrument]`), `apps/api/src/{webhooks,publisher}/**` (use `TracerService.withSpan`).
- **New files/modules expected:**
  - `engine/crates/telemetry/src/spans.rs` (name constants, `SpanName` enum, `required_attrs(name)`)
  - `apps/api/src/telemetry/span-names.ts`
  - `engine/crates/pipeline/tests/span_tree.rs`
  - `docs/operations/span-catalog.md`
- **Dependencies (task IDs):** OBS-001, OBS-002, OBS-003, DOM-008 (review run states), PIPE-003 (pipeline composition root), GW-010 (`model_request`), SEM-005 (`qdrant_search`).
- **Implementation details:**
  - Expected tree for one review: `webhook_received` > `job` (queue `pr-review`) > `repository_checkout`, `repository_index` | `incremental_graph_update`, `diff_analysis`, `symbol_mapping`, `impact_analysis`, then per cluster `context_selection` > `qdrant_search`, then per reviewer `reviewer_execution` > `model_request` > `candidate_generated`, then `finding_verification` (with verifier `model_request` children), `deduplication`, and `publication` under the `review-publish` job.
  - Required attributes by span: all spans carry `organization_id, repository_id, review_run_id`; `webhook_received` adds `event, action, delivery_id, installation_id`; `repository_checkout` adds `commit_sha`; `incremental_graph_update` adds `base_snapshot_id, files_changed`; `context_selection` adds `cluster_id, reviewer_type, token_budget`; `qdrant_search` adds `kinds, limit, hits`; `reviewer_execution` adds `reviewer_type, version`; `model_request` adds the GW-010 attribute set; `candidate_generated` adds `candidate_finding_id, category, severity`; `finding_verification` adds `candidate_finding_id, outcome`; `publication` adds `pull_request_id, findings_published`.
  - Provide `span!`-wrapping macros (`stage_span!(SpanName::ImpactAnalysis, ctx)`) so the name must come from the enum; raw string span names for lifecycle stages are rejected by a `clippy`-style test that greps source for forbidden literals.
  - Event-style stages (`candidate_generated`) use short spans, not log events, so they are searchable as spans.
  - Spans record `otel.status_code` and `error.type` on failure; `rg.outcome=ok|skipped|failed|superseded`.
  - Cardinality: attribute values for ids are allowed on spans; never use them as metric labels.
- **Data model changes:** None.
- **API/protocol changes:** None; the catalog document is the contract.
- **Concurrency semantics:** Cluster and reviewer spans run in parallel under their parents; parentage is set explicitly with `Span::in_scope`/`instrument` on spawned tasks so context is not lost across `tokio::spawn` (tests cover this).
- **Failure behavior:** A stage that errors still ends its span with error status; a superseded run ends open spans with `rg.outcome=superseded`. A missing span never fails the review; the conformance test fails the build instead.
- **Idempotency considerations:** A resumed job (stage output reused) emits the stage span with `rg.outcome=skipped` and `rg.reused=true`, so traces show retries clearly.
- **Security considerations:** Spans never contain source text, diff content, prompts or model output; only ids, counts, hashes, file paths limited to relative repo paths. A test scans exported attributes for forbidden keys (`prompt`, `source`, `diff`, `body`).
- **Observability additions:** This task is the observability contract; adds the catalog doc.
- **Tests required (named):**
  - `replay_review_emits_every_lifecycle_span`
  - `span_tree_parentage_matches_catalog`
  - `all_spans_carry_org_repo_run_ids`
  - `required_attrs_present_per_span_name`
  - `parallel_reviewer_spans_keep_parent_across_spawn`
  - `failed_stage_marks_span_error_and_run_continues_to_failure_state`
  - `no_forbidden_attribute_keys_exported`
  - `ts_publication_and_webhook_spans_use_registry_names`
- **Benchmarks if applicable:** Replay review with telemetry on vs off: overhead under 3 percent of wall time (recorded in PERF-007).
- **Acceptance criteria (verifiable):**
  - `span_tree` test passes in CI.
  - Running the E2E replay locally shows all 15 span names in one OpenObserve trace.
- **Definition of done:** Acceptance criteria pass; `docs/operations/span-catalog.md` lists each span, parent, attributes and owner task.

---

### OBS-005 — Metrics instruments
Status: ☐

- **Task ID:** OBS-005
- **Title:** Central metric registries (Rust and TypeScript) for the full PRD §115 / master-plan metric set, with bounded label cardinality
- **Problem:** Metrics are being defined ad hoc in each crate (GW-010, SEM-005, API-003 each name their own). Names, units, buckets and labels need one definition so dashboards and alerts (OBS-007/008) can rely on them.
- **Why it exists:** ADR-013 (metric names come from the OBS tasks), PRD §115 (quality, performance, cost, graph health), master plan §14.
- **Scope:**
  - `telemetry::metrics` with typed instrument structs created once from the global meter: `review_duration_seconds, incremental_index_duration_seconds, files_reparsed_total, symbols_changed_total, graph_nodes_total, graph_edges_total, graph_invalidations_total, context_tokens_total, context_symbols_total, qdrant_queries_total, qdrant_query_duration_seconds, llm_requests_total, llm_input_tokens_total, llm_output_tokens_total, llm_cached_tokens_total, llm_cost_estimate, candidate_findings_total, verified_findings_total, published_findings_total, suppressed_findings_total, finding_acceptance_rate, finding_false_positive_rate, queue_depth, queue_wait_seconds, worker_duration_seconds`.
  - A TypeScript twin in `apps/api/src/telemetry/metrics.ts` for the metrics TS owns (queue depth, publish counters, feedback rates, webhook counters).
  - Label allow-lists, histogram bucket sets, unit metadata, and a generated `docs/operations/metrics-catalog.md`.
- **Explicit non-scope:**
  - Recording sites inside every phase (each owning task calls the instruments; this task wires the pipeline-level ones: review duration, queue, worker, findings funnel).
  - Dashboards and alerts (OBS-007/008).
  - GW-010's extended LLM metrics (they register through this registry).
- **Files/modules expected to change:** `engine/crates/telemetry/src/lib.rs`, `engine/crates/pipeline/src/{runner,jobs}.rs`, `apps/api/src/{publisher,feedback}/**`, `engine/crates/model-gateway/src/telemetry.rs` (re-register through the registry).
- **New files/modules expected:**
  - `engine/crates/telemetry/src/metrics/{mod.rs,instruments.rs,labels.rs,buckets.rs}`
  - `engine/crates/telemetry/tests/metrics_registry.rs`
  - `apps/api/src/telemetry/metrics.ts`, `apps/api/test/telemetry/metrics.spec.ts`
  - `engine/xtask` subcommand `metrics-catalog` that renders the doc.
- **Dependencies (task IDs):** OBS-001, OBS-002, DOM-006 (finding states), GW-010.
- **Implementation details:**
  - Types: counters for `*_total`; histograms for `*_duration_seconds` and `queue_wait_seconds` (buckets 0.05, 0.1, 0.25, 0.5, 1, 2.5, 5, 10, 30, 60, 120, 300, 900); observable gauges for `graph_nodes_total`, `graph_edges_total` (reported per snapshot), `queue_depth` (polled by a PG query every 15 s from the worker and the API, label `queue`, `state`); `llm_cost_estimate` is a counter in USD micros named with unit `usd_micros`.
  - `finding_acceptance_rate` and `finding_false_positive_rate` are gauges computed from feedback counts over a trailing 7-day window by a periodic job; formulas are written in the catalog: acceptance = `useful + already_handled` over `published with any feedback`; FP rate = `false_positive` over `published with any feedback` (PRD §116/§117; exact definitions come from EVAL-* and must be mirrored here).
  - Label allow-list per instrument, enforced at compile time via typed label structs: `reviewer_type, model, provider, task, tier, queue, outcome, stage, language, severity, category, state`. Forbidden labels: any id (`organization_id`, `repository_id`, `review_run_id`, `commit_sha`, paths). Per-tenant breakdowns come from traces and logs, not metrics.
  - `context_tokens_total{reviewer_type}`, `context_symbols_total{reviewer_type}`; `qdrant_queries_total{kind,outcome}`; `candidate/verified/published/suppressed_findings_total{reviewer_type,category,severity}` plus `reason` on suppressed.
  - Naming: lower_snake_case with unit suffixes; OpenObserve stores them as streams with the same name.
- **Data model changes:** None. Rate gauges read `findings` and feedback tables.
- **API/protocol changes:** OTLP metrics export only. No Prometheus endpoint in the MVP.
- **Concurrency semantics:** Instruments are `Clone + Send + Sync` and lock-free; observable gauge callbacks must not block (they read cached values refreshed by a background task).
- **Failure behavior:** If the gauge refresh query fails, the last value is kept and `metrics_refresh_failures_total` increments. A bad label value is mapped to `other` rather than panicking.
- **Idempotency considerations:** The registry is built once via `OnceLock`; a duplicate registration of the same name with different type fails a startup assertion.
- **Security considerations:** No tenant or source-derived strings in labels (cardinality and leakage). Label values come from closed enums.
- **Observability additions:** This task is the metric contract; `metrics-catalog.md` is generated and drift is checked by a test.
- **Tests required (named):**
  - `all_required_metrics_registered_with_expected_type_and_unit`
  - `label_structs_reject_forbidden_label_names` (trybuild compile-fail)
  - `histogram_buckets_cover_target_ranges`
  - `unknown_label_value_maps_to_other`
  - `queue_depth_gauge_reports_per_queue_and_state`
  - `acceptance_and_fp_rate_formulas_match_catalog_fixture`
  - `metrics_catalog_doc_is_up_to_date` (xtask output equals committed file)
  - `ts_registry_exposes_same_names_as_rust_for_shared_metrics`
- **Benchmarks if applicable:** Criterion: recording a counter with 3 labels under 100 ns.
- **Acceptance criteria (verifiable):**
  - Tests pass.
  - After one replayed review, OpenObserve contains a stream for every metric in the list (`select distinct metric_name`), with non-zero data for the pipeline-level ones.
- **Definition of done:** Acceptance criteria pass; `docs/operations/metrics-catalog.md` committed.

---

### OBS-006 — Redaction layers and tests
Status: ☐

- **Task ID:** OBS-006
- **Title:** Redaction in the Rust `tracing` pipeline and the NestJS log formatter, sharing one pattern set and a shared test corpus
- **Problem:** Secrets (tokens, `Authorization` headers, private keys, `.env` assignments) can leak into logs, span attributes or error messages. Source code and prompts must never be logged.
- **Why it exists:** ADR-013 ("Redaction"), target-architecture §8, master plan §13.5, PRD §111. GW-010 reuses `telemetry::redact::patterns()` for pre-send redaction.
- **Scope:**
  - `telemetry::redact`: the pattern catalog, a `redact_str` function and a `tracing` layer/field formatter that applies it to every event and span field.
  - A `Redacted<T>` wrapper and `Secret<T>` type whose `Debug`/`Display` print `«redacted»`.
  - A NestJS log formatter and OTel span-attribute processor using the same patterns.
  - A shared JSON corpus of positive and negative samples, consumed by both language test suites.
- **Explicit non-scope:**
  - Pre-send model redaction logic (GW-010/SEC-004); this task provides the patterns and the function.
  - Index-time secret detection (SEC-003).
  - Preventing prompt logging by policy: prompts are never passed to log macros (checked by a lint test), redaction is the second line of defense.
- **Files/modules expected to change:** `engine/crates/telemetry/src/init.rs` (layer slot from OBS-001), `apps/api/src/telemetry/{json-logger.ts,instrumentation.ts}`.
- **New files/modules expected:**
  - `engine/crates/telemetry/src/redact/{mod.rs,patterns.rs,layer.rs,secret.rs}`
  - `packages/contracts/redaction-corpus.json` (single source: `{name, input, expected_redacted, pattern}` and `negatives[]`)
  - `apps/api/src/telemetry/redact.ts`
  - `engine/crates/telemetry/tests/redact_corpus.rs`, `apps/api/test/telemetry/redact.spec.ts`, `engine/xtask` check `no-log-of-sensitive-types`
- **Dependencies (task IDs):** OBS-001, OBS-002, DOM-001.
- **Implementation details:**
  - Patterns (`patterns()` returns `&'static [Pattern{name, regex}]`, regexes compiled once with the `regex` crate, which has linear-time matching so no ReDoS): PEM private key blocks (`-----BEGIN [A-Z ]*PRIVATE KEY-----` to `END`), AWS access key ids (`AKIA|ASIA[0-9A-Z]{16}`), GitHub tokens (`gh[pousr]_[A-Za-z0-9]{36,}` and `github_pat_...`), Anthropic and OpenAI style keys (`sk-ant-...`, `sk-...`), JWT-like (`eyJ...\.eyJ...\....`), `Authorization:`/`Bearer`/`Basic` header values, `x-hub-signature-256` values, URL userinfo (`scheme://user:pass@`), and `.env`-style assignments where the key matches `(?i)[A-Z0-9_]*(KEY|SECRET|TOKEN|PASSWORD|PASSWD|PRIVATE)[A-Z0-9_]*\s*[=:]\s*\S+`.
  - Replacement: `«redacted:<pattern>»` for logs. GW-010 appends a blake3 prefix on top; the log layer does not hash, to avoid enabling dictionary attacks on low-entropy secrets.
  - Redaction applies to: event message, all field values rendered as strings, span field values, error `source()` chains, and field keys that match the sensitive-key list (`authorization`, `cookie`, `password`, `token`, `secret`, `private_key`) regardless of value.
  - Hard cap: any single field longer than 4 KiB is truncated with `…[truncated]` so source blobs cannot ride in logs.
  - TypeScript: Nest logger wraps `redactString` and `redactObject` (deep, depth-limited to 6, circular safe). An OTel `SpanProcessor.onEnd` equivalent scrubs string attributes before export.
  - Sensitive types (`InstallationToken`, `PrivateKey`, `WebhookSecret`, `ModelPrompt`) do not implement `Display`, and `Debug` is manual; a workspace test fails if they derive `Debug`.
- **Data model changes:** None.
- **API/protocol changes:** None.
- **Concurrency semantics:** Pure functions over immutable compiled regexes; thread-safe. The layer adds work on the logging path only for events passing the `EnvFilter`.
- **Failure behavior:** If redaction panics or exceeds a 5 ms budget per event, the event is replaced by `«redaction-failed»` with only level and target (fail closed, never emit the original).
- **Idempotency considerations:** Redacting already redacted text is a no-op (`redact(redact(x)) == redact(x)`), covered by a property test.
- **Security considerations:** This is the second line of defense after "never log it". The corpus includes adversarial cases: tokens split by JSON escaping, tokens in query strings, base64 of a Basic header, multi-line PEM, and Unicode lookalike delimiters.
- **Observability additions:** Counter `log_redactions_total{pattern}` (pattern name only, closed set).
- **Tests required (named):**
  - `corpus_positives_are_redacted_rust`, `corpus_negatives_unchanged_rust`
  - `corpus_positives_are_redacted_ts`, `corpus_negatives_unchanged_ts`
  - `rust_and_ts_agree_on_every_corpus_case`
  - `sensitive_keys_redacted_regardless_of_value`
  - `redaction_is_idempotent` (proptest)
  - `long_field_is_truncated`
  - `error_source_chain_is_redacted`
  - `span_attributes_scrubbed_before_export`
  - `redaction_failure_fails_closed`
  - `sensitive_types_do_not_derive_debug`
- **Benchmarks if applicable:** Criterion `redact_typical_log_line` under 5 microseconds; `redact_4kib_field` under 100 microseconds.
- **Acceptance criteria (verifiable):**
  - All tests pass in Rust and TS suites.
  - With a deliberately leaked token logged in a dev run, neither stdout nor OpenObserve contains the raw token (grep of the exported log stream).
- **Definition of done:** Acceptance criteria pass; corpus documented in `docs/security/redaction.md`.

---

### OBS-007 — OpenObserve dashboards as code
Status: ☐

- **Task ID:** OBS-007
- **Title:** Versioned OpenObserve dashboards in `infra/openobserve/dashboards/` and an idempotent apply script
- **Problem:** Dashboards built by clicking in the UI are not reviewable, not reproducible and are lost with the dev volume.
- **Why it exists:** ADR-013 ("Dashboards and alerts are versioned JSON files applied by a script"), master plan §14 dashboard list, MVP scope ("core dashboards").
- **Scope:**
  - One JSON file per dashboard: review latency, index latency, model usage/tokens/cost, findings funnel (candidate to verified to published to accepted), false-positive feedback, worker health, Qdrant latency, PostgreSQL latency, queue depth.
  - `infra/openobserve/apply.mjs` which upserts dashboards (and later alerts from OBS-008) through the OpenObserve HTTP API.
  - A schema check for the JSON and a README of the layout.
- **Explicit non-scope:**
  - Alert rules and runbooks (OBS-008).
  - Product-facing UI in `apps/web` (WEB-*).
  - Defining metrics (OBS-005).
- **Files/modules expected to change:** Root `package.json` (script `obs:apply`), `infra/compose/.env.example` (OpenObserve credentials already present; document use).
- **New files/modules expected:**
  - `infra/openobserve/dashboards/{review-latency,index-latency,model-usage-cost,findings-funnel,false-positive-feedback,worker-health,qdrant-latency,postgres-latency,queue-depth}.json`
  - `infra/openobserve/{apply.mjs,apply.test.mjs,validate.mjs,README.md}`
- **Dependencies (task IDs):** FND-005 (OpenObserve on port 25080), OBS-005 (metric names), OBS-004 (span names), FND-006 (task runner).
- **Implementation details:**
  - Export format: the OpenObserve dashboard JSON (version 5 schema as produced by the UI "export"). Each dashboard has a stable `dashboardId` derived from the file name (for example `rg-review-latency`) so reapplying updates in place.
  - Panels use SQL over metric streams and trace streams. Examples: review p50/p95 from the `review_duration_seconds` histogram; trace-derived stage durations by `span_name` from the traces stream; token usage grouped by `model` and `tier`; funnel as four stacked counts over `candidate_findings_total`, `verified_findings_total`, `published_findings_total` and accepted feedback.
  - Template variables: `reviewer_type`, `model`, `queue`, and a time range; no `organization_id` variable in metric panels (label forbidden by OBS-005); tenant drill-down is a trace/log query panel.
  - `apply.mjs`: reads `OTEL_EXPORTER_OTLP_ENDPOINT` host, `RG_OO_EMAIL`/`RG_OO_PASSWORD`, org `default`; lists existing dashboards (`GET /api/{org}/dashboards`), then `POST` for new and `PUT` (with the current hash) for changed ones; `--dry-run` prints the plan; `--prune` deletes dashboards that carry the `rg-` prefix but have no file.
  - `validate.mjs` checks every file: parseable JSON, unique `dashboardId`, every referenced stream or metric name exists in `docs/operations/metrics-catalog.md` or the span catalog (so renames break CI).
  - Time ranges and refresh: default last 6 h, refresh 30 s.
- **Data model changes:** None.
- **API/protocol changes:** Calls OpenObserve `/api/default/dashboards` with HTTP basic auth.
- **Concurrency semantics:** Apply is sequential; two concurrent runs both converge because writes are upserts keyed by `dashboardId`.
- **Failure behavior:** Any non-2xx response aborts with the dashboard id and status body (credentials never echoed) and a non-zero exit; partial application is safe to rerun.
- **Idempotency considerations:** Applying twice yields no diff (`apply --dry-run` reports "0 changes"); content hashing ignores server-assigned fields (`updatedAt`, `owner`).
- **Security considerations:** Credentials come from env only and are not written to disk or printed. Dashboards contain no secrets and no tenant identifiers. The script refuses non-loopback endpoints unless `--allow-remote` is passed.
- **Observability additions:** The dashboards themselves; also a "telemetry health" panel on worker-health showing `telemetry_export_failures_total`.
- **Tests required (named):**
  - `validate_accepts_all_committed_dashboards`
  - `validate_rejects_unknown_metric_reference`
  - `validate_rejects_duplicate_dashboard_id`
  - `apply_creates_missing_dashboards` (mock HTTP server)
  - `apply_updates_changed_and_skips_identical`
  - `apply_dry_run_makes_no_writes`
  - `prune_only_removes_rg_prefixed_dashboards`
  - `apply_refuses_remote_endpoint_without_flag`
- **Benchmarks if applicable:** None.
- **Acceptance criteria (verifiable):**
  - `node infra/openobserve/validate.mjs` and `node --test infra/openobserve/apply.test.mjs` pass.
  - With the compose stack up, `pnpm obs:apply` exits 0 and a second run reports 0 changes.
  - After a replayed review, the review-latency and model-usage panels render data (manual check recorded with a screenshot in the PR).
- **Definition of done:** Acceptance criteria pass; README lists each dashboard and its source metrics.

---

### OBS-008 — Alerts and runbooks
Status: ☐

- **Task ID:** OBS-008
- **Title:** OpenObserve alert definitions as code plus one runbook per alert
- **Problem:** Dashboards alone do not notify anyone. Production readiness requires alerts and operational runbooks for the failure modes the master plan names.
- **Why it exists:** Master plan §14 (alert list) and §17 ("Operational runbooks: one per alert"; "dead jobs alert").
- **Scope:**
  - Alert JSON files in `infra/openobserve/alerts/` for: review p95 over 2 min for 15 min; dead jobs above 0; webhook signature failure spike; model error rate above 5 percent; queue wait p95 above 60 s; Qdrant p95 above 500 ms; PostgreSQL connection saturation; false-positive feedback rate above 15 percent weekly; plus `qdrant_scope_violation_total` above 0 and `tenancy_denied_total` anomaly (security).
  - Destination definition (webhook template) with a local no-op/log sink for dev.
  - Extension of `infra/openobserve/apply.mjs` (from OBS-007) to apply alerts, destinations and templates.
  - `docs/operations/runbooks/<alert-slug>.md` for each alert.
- **Explicit non-scope:**
  - Choosing a paging vendor; the destination is a generic webhook configured by env in production.
  - Dashboards (OBS-007).
  - Auto-remediation.
- **Files/modules expected to change:** `infra/openobserve/apply.mjs`, `infra/openobserve/validate.mjs`, `infra/openobserve/README.md`.
- **New files/modules expected:**
  - `infra/openobserve/alerts/*.json`, `infra/openobserve/destinations/{default-webhook.json,local-log.json}`
  - `docs/operations/runbooks/{review-latency-high,dead-jobs,webhook-signature-failures,model-error-rate,queue-wait-high,qdrant-latency-high,pg-connection-saturation,false-positive-rate-high,qdrant-scope-violation,tenancy-denied-spike}.md`
  - `infra/openobserve/alerts.test.mjs`
- **Dependencies (task IDs):** OBS-005, OBS-007, GH-002 (`webhook_signature_failures_total`), SEM-005 (`qdrant_scope_violation_total`), API-003 (`tenancy_denied_total`), PIPE-002 (dead-letter state).
- **Implementation details:**
  - Each alert defines: SQL or PromQL-style query, evaluation frequency (1 to 5 min), window, threshold operator, `for` duration, severity (`page|ticket`), labels, destination id, and a `runbook_url` annotation pointing to the repository doc path.
  - Thresholds: review latency p95 greater than 120 s sustained 15 min; `dead` jobs count greater than 0 (query PG-derived `queue_depth{state="dead"}`); signature failures above 10 per 5 min; LLM error ratio above 5 percent over 15 min with minimum volume 20 requests; `queue_wait_seconds` p95 above 60 s for 10 min; `qdrant_query_duration_seconds` p95 above 500 ms for 10 min; PG connections above 80 percent of `max_connections` for 5 min; FP feedback rate above 0.15 over trailing 7 days with minimum 20 feedback items; scope violation above 0 immediately (page); `tenancy_denied_total` increase above 50 per 5 min.
  - Runbook template (fixed headings): Symptom, Impact, Likely causes, Diagnosis (exact queries, dashboard link, `docker compose logs <svc>` commands for local), Mitigation, Escalation, Related alerts. Each links the relevant dashboard from OBS-007.
  - `validate.mjs` additions: every alert has a runbook file, every runbook is referenced by an alert, thresholds parse, referenced metrics exist in the catalog.
  - Silence and maintenance: documented use of OpenObserve alert pause; no suppression logic in code.
- **Data model changes:** None.
- **API/protocol changes:** OpenObserve alert and destination APIs (`/api/{org}/alerts`, `/destinations`, `/templates`).
- **Concurrency semantics:** Alert evaluation is server-side and independent; apply is sequential and upserts by name.
- **Failure behavior:** If the destination is unreachable, OpenObserve retries per its settings; local dev uses the log sink so nothing external is called. Apply failures abort with the alert name.
- **Idempotency considerations:** Alerts are keyed by `name` with an `rg-` prefix; reapply produces no changes; `--prune` removes orphaned `rg-` alerts only.
- **Security considerations:** Destination URLs and tokens come from env or the secret manager, never from committed files; alert payload templates include metric values and runbook links, never log content or tenant data. Security alerts route to the page destination.
- **Observability additions:** The alerts themselves; plus a meta-alert "no telemetry received for 10 min" using `telemetry_export_failures_total` and absence of `review_duration_seconds` while jobs run.
- **Tests required (named):**
  - `every_alert_has_runbook_and_vice_versa`
  - `alert_thresholds_match_master_plan_table`
  - `alert_queries_reference_known_metrics`
  - `apply_alerts_idempotent` (mock server)
  - `destination_secrets_not_in_committed_files` (grep test)
  - `runbooks_have_required_headings`
- **Benchmarks if applicable:** None.
- **Acceptance criteria (verifiable):**
  - `node --test infra/openobserve/alerts.test.mjs` passes.
  - `pnpm obs:apply` creates all alerts locally; forcing a dead job (via a test script) fires the `dead-jobs` alert to the local log sink within 2 evaluation periods.
- **Definition of done:** Acceptance criteria pass; the production-readiness table row "Operational runbooks" is satisfied by the files listed.

---

### SEC-001 — Tenant isolation tests (API + RLS)
Status: ☐

- **Task ID:** SEC-001
- **Title:** Two-organization isolation suite covering every API route and every RLS-protected table
- **Problem:** API-003 builds guards and RLS, but without a systematic suite a new route or table can silently ship without isolation (risk R9, critical).
- **Why it exists:** Production readiness row "Tenant isolation proven"; PRD §112; master plan §13.1. The suite must run in CI (CI-004) and fail when a route or table is added without coverage.
- **Scope:**
  - A shared two-org fixture (org A and org B, users in each, repositories, PRs, review runs, findings, snapshots, graph rows, feedback, suppressions).
  - Route-level tests: every `/api/v1` route called with org A's session against org B's ids returns 404 (never 403) and leaks nothing in headers or body.
  - Database-level tests: every table with `organization_id` is queried as `rg_api` and `rg_engine` with the wrong org setting and without any setting.
  - A coverage guard test that enumerates routes and tables and fails on any not covered or explicitly exempted.
- **Explicit non-scope:**
  - Qdrant filter audit (SEC-002); the object store (SEC-009).
  - Authentication flows (API-004).
  - Implementing the guards and policies (API-003).
- **Files/modules expected to change:** `apps/api/test/helpers/*` (fixture builder export), CI test selection.
- **New files/modules expected:**
  - `apps/api/test/security/{tenancy.fixture.ts,routes-isolation.e2e.spec.ts,rls-tables.spec.ts,coverage-guard.spec.ts}`
  - `engine/crates/graph-storage/tests/rls_engine_role.rs` (engine-side role test)
  - `docs/security/tenant-isolation-test-plan.md`
- **Dependencies (task IDs):** API-003, API-008, API-009, API-010, API-011, API-012, DOM-009, FND-005.
- **Implementation details:**
  - Fixture: created as `rg_ops`, with deterministic uuids. Each org has identical-looking data (same repo names, same paths, same symbol names) so a leak is detectable by id rather than by shape.
  - Route enumeration: the guard test reads Nest's route table (`app.getHttpAdapter().getInstance().router.stack` or the generated OpenAPI) and requires each `METHOD path` to appear in an `ISOLATION_MATRIX` constant with an expected behavior (`foreign_id_404`, `list_filtered`, `public`, `service_auth_only`). Unlisted routes fail the test.
  - List endpoints: org A's list contains zero ids belonging to org B; pagination cursors from org A cannot be replayed to read org B (cursor is signed or scoped).
  - Mutation endpoints (feedback, settings, rerun): attempted with a foreign id leaves the row unchanged (assert by reading as org B).
  - Table enumeration: query `pg_class`/`information_schema` for tables with an `organization_id` column; each must have `relrowsecurity` and `relforcerowsecurity`, and the policy test performs `SELECT/INSERT/UPDATE/DELETE` as `rg_api` with (a) no `app.organization_id`, (b) org A, (c) org B and asserts the results.
  - Engine path: with `rg_engine` and the job's org set, graph-storage reads return only that org's snapshots.
  - Internal service-auth routes are tested to reject user cookies and tenants cannot reach them.
  - Error responses are compared byte-for-byte between "id does not exist" and "id belongs to another org" to prove there is no existence oracle (status, body, relevant headers).
- **Data model changes:** None.
- **API/protocol changes:** None.
- **Concurrency semantics:** Tests run in parallel safely by giving each test file its own pair of organizations; connection pool reuse test confirms a connection used for org A then B shows no carry-over.
- **Failure behavior:** Any leak fails the test with the route or table name. The coverage guard failure message names the missing entry and how to add it.
- **Idempotency considerations:** Fixtures are rebuilt per run in a dedicated schema or database template; no reliance on state from previous runs.
- **Security considerations:** Tests run with the real non-superuser roles, never as a superuser, because superusers bypass RLS and would hide failures (an assertion guards `current_setting('is_superuser') = 'off'`).
- **Observability additions:** Asserts `tenancy_denied_total` increments for denied access.
- **Tests required (named):**
  - `every_route_has_isolation_matrix_entry`
  - `foreign_repo_id_returns_404_on_all_repo_routes`
  - `foreign_review_finding_pr_ids_return_404`
  - `list_endpoints_never_return_other_org_rows`
  - `foreign_cursor_cannot_page_other_org`
  - `mutations_on_foreign_ids_do_not_change_data`
  - `existence_oracle_absent_identical_404_bodies`
  - `every_org_table_has_forced_rls`
  - `rg_api_without_org_setting_sees_zero_rows`
  - `rg_api_cross_org_insert_rejected_with_check`
  - `pooled_connection_does_not_carry_org`
  - `engine_role_scoped_to_job_org`
  - `internal_routes_reject_user_sessions`
- **Benchmarks if applicable:** None.
- **Acceptance criteria (verifiable):**
  - `pnpm --filter api test:security` and the Rust engine-role test pass against `docker-compose.test.yml`.
  - Adding a new route without a matrix entry, or a table with `organization_id` without RLS, makes the suite fail (demonstrated once in the PR).
- **Definition of done:** Acceptance criteria pass; the suite is required in CI-004; `docs/security/tenant-isolation-test-plan.md` explains how to extend the matrix.

---

### SEC-002 — Qdrant and PostgreSQL tenant-filter audit tests
Status: ☐

- **Task ID:** SEC-002
- **Title:** End-to-end cross-tenant tests for Qdrant and engine-side PostgreSQL queries, plus a static audit that every query carries tenant predicates
- **Problem:** SEM-005 proves the Qdrant adapter cannot omit filters, and API-003 enforces RLS, but nothing proves the whole data plane (context engine, graph store, profile, finding history) never returns another tenant's data when two tenants hold identical code.
- **Why it exists:** Production readiness "Qdrant isolation/filtering" (SEM-005 plus SEC-002); master plan §13.1 ("Engine queries always include org/repo predicates").
- **Scope:**
  - A live two-org test where both orgs index the same fixture repository (identical symbols and content hashes) and every search and graph read returns zero cross-org hits.
  - A recording audit layer over the Qdrant HTTP client and a SQL statement audit over engine queries, enabled across the CTX, SEM and GS integration suites.
  - A static scan test that every engine SQL string touching a tenant table includes the org/repo predicate or runs under RLS role.
- **Explicit non-scope:**
  - The `TenantScope` implementation (SEM-005) and RLS policies (API-003).
  - API route isolation (SEC-001); object storage (SEC-009).
- **Files/modules expected to change:** `engine/crates/semantic/tests/*` (reuse audit layer), `engine/crates/graph-storage/tests/*`, `engine/crates/context-engine/tests/*` (enable the audit).
- **New files/modules expected:**
  - `engine/crates/pipeline/tests/security/{cross_tenant_qdrant.rs,cross_tenant_graph.rs,sql_audit.rs}`
  - `engine/crates/graph-storage/src/audit.rs` (test-only statement recorder behind `#[cfg(any(test, feature = "audit"))]`)
  - `engine/xtask` subcommand `audit-sql`
  - `docs/security/tenant-filter-audit.md`
- **Dependencies (task IDs):** SEM-005, API-003, GS-001, CTX-001, DOM-009, FND-005.
- **Implementation details:**
  - Test setup: create orgs A and B with distinct `OrganizationId`/`RepositoryId`, same repository fixture (`fixtures/repositories/*`), same commit sha, same embedding space (hash provider so no network). Run the real index pipeline for both, then query as A.
  - Qdrant assertions: for 25 queries (each fixture symbol name as a text query plus random vectors), every hit payload has `organization_id == A`; running the same as B returns B ids; a deliberately scope-less raw request (made through a test-only raw client) returns both orgs, proving the test has teeth.
  - Qdrant audit layer: records every request body from the whole `semantic`, `context-engine` and `pipeline` test runs; at teardown asserts each search/scroll/count/delete body contains the org condition and a repository condition, and every upsert payload sets tenant keys from scope.
  - Postgres: a statement recorder (wrapping `sqlx` executor in tests) captures SQL text; the audit asserts statements touching tenant tables either bind `organization_id` or run in a connection whose `app.organization_id` is set (checked via `current_setting` at execution). Tables list comes from the same query as SEC-001.
  - Graph reads: `load_snapshot`, `neighbors`, `symbol lookup` and `symbol_lineage` with A's scope never return B's `snapshot_id` rows, even when `symbol_key`s are identical (symbol keys are content-derived, so collisions across tenants are expected and must not leak).
  - Static scan (`xtask audit-sql`): regex plus `sqlx` offline query metadata; allow-list file `engine/security/sql-audit-allow.toml` with a justification per exempt query (for example cross-tenant job claims).
  - Context engine: assert that `ContextPackage` items for A never contain a `file_path`/`symbol_key` from B's snapshot ids.
- **Data model changes:** None.
- **API/protocol changes:** None.
- **Concurrency semantics:** Indexing of both orgs runs concurrently in the test to catch shared caches (in-process graph LRU, model cache keys) leaking across tenants; cache keys must include tenant.
- **Failure behavior:** A cross-tenant hit or an unscoped request fails the test and prints the offending request body or SQL (with values elided).
- **Idempotency considerations:** Re-running the suite on the same database recreates orgs from scratch; point ids are `uuid_v5(org, repo, ...)` so re-indexing is an upsert no-op per tenant.
- **Security considerations:** The raw-client probe exists only in test builds; a `trybuild`/symbol check asserts it is not exported in release builds.
- **Observability additions:** Asserts `qdrant_scope_violation_total` stays 0 across the run.
- **Tests required (named):**
  - `identical_repos_in_two_orgs_never_cross_hit_in_qdrant`
  - `unscoped_raw_probe_returns_both_orgs_proving_test_sensitivity`
  - `all_recorded_qdrant_requests_carry_org_and_repo_filters`
  - `graph_reads_scoped_despite_identical_symbol_keys`
  - `model_and_context_caches_keyed_by_tenant`
  - `sql_audit_every_tenant_table_statement_scoped_or_rls`
  - `sql_audit_allow_list_entries_have_justification`
  - `context_package_contains_only_own_snapshot_items`
  - `raw_probe_not_present_in_release_build`
- **Benchmarks if applicable:** None.
- **Acceptance criteria (verifiable):**
  - The suite passes under `integration` feature against the test compose stack.
  - Removing the tenant filter from any one adapter query makes at least one test fail (mutation check recorded in the PR).
- **Definition of done:** Acceptance criteria pass; CI-004 runs this suite; the doc explains the audit allow-list policy.

---

### SEC-003 — Secret detection at init and index time
Status: ☐

- **Task ID:** SEC-003
- **Title:** Detect likely secrets during `review init` and indexing, store only fingerprints and locations, and flag files as non-transmittable
- **Problem:** Repository source can contain private keys, tokens and `.env` values. Nothing currently detects them, so downstream redaction (SEC-004) cannot rely on a known set, and secrets could be embedded or sent to a model.
- **Why it exists:** PRD §111 ("Repository initialization should detect likely secrets and prevent accidental model transmission"); master plan §13.5; GW-010 consumes secret fingerprints.
- **Scope:**
  - A `SecretScanner` in the `repository` crate: path rules (`.env*`, `*.pem`, `id_rsa*`, `*.p12`, `credentials*`), content rules (reusing `telemetry::redact::patterns()` plus an entropy detector), and an allow marker for documented false positives.
  - Results: `SecretFinding { file, line, pattern, fingerprint (blake3 of the value, 128-bit), confidence }`. The value is never stored.
  - Integration into `review init` and the index pipeline (file_versions get `secret_scan_status`).
  - Files with findings are marked `no_embed` and `redact_required`, which the semantic and gateway layers honor.
- **Explicit non-scope:**
  - Pre-send replacement in model requests (SEC-004).
  - Removing secrets from git history or rotating them; reporting only.
  - Log redaction (OBS-006).
- **Files/modules expected to change:** `engine/crates/repository/src/*` (walker integration), `engine/crates/pipeline/src/index.rs`, `engine/migrations/{seq}_secret_findings.sql`.
- **New files/modules expected:**
  - `engine/crates/repository/src/secrets/{mod.rs,paths.rs,content.rs,entropy.rs,allow.rs}`
  - `engine/crates/repository/tests/secrets.rs`, `fixtures/repositories/secrets-sample/` (synthetic, obviously fake credentials)
  - `docs/security/secret-handling.md`
- **Dependencies (task IDs):** OBS-006, INIT-002, IDX-001, DOM-009, DOM-004.
- **Implementation details:**
  - Table `secret_findings (id, organization_id, repository_id, file_version_id, line, pattern, fingerprint bytea, confidence real, created_at)` with RLS; no value column. Unique `(file_version_id, line, pattern, fingerprint)`.
  - Entropy detector: Shannon entropy of base64/hex-like tokens of length 20 to 200 on assignment right-hand sides and string literals, threshold 4.0 bits per char for base64, 3.0 for hex, suppressed on known benign shapes (uuids, git shas, integrity hashes in lockfiles, `*.lock`, `*.min.js`).
  - Known-secret fingerprints are loaded by the gateway redactor per repository; high-entropy matches also feed fingerprint redaction so a secret that appears in other files is caught even without a pattern.
  - `.review/config.yaml` option `secrets.allow` (path globs and fingerprints) with a mandatory `reason`; allowed items are recorded as `allowed=true` and still listed in the report.
  - `review init` prints a summary (counts by pattern and file, never values) and writes `.review/secrets-report.json` without values.
  - Skips binary files and files above 1 MiB; scan cost is linear per file.
- **Data model changes:** New table `secret_findings`; `file_versions.secret_scan_status` (`clean|findings|skipped`) and `file_versions.no_embed boolean`.
- **API/protocol changes:** `GET /api/v1/repositories/:id/secrets-summary` (counts only; maintainer role); CLI `review doctor` shows the summary.
- **Concurrency semantics:** Scanning is per file and pure; runs inside the existing parallel file pipeline. Writes batch per file version.
- **Failure behavior:** A scanner error on one file marks it `skipped` with `no_embed=true` (fail closed: unscanned means not transmittable) and does not abort indexing.
- **Idempotency considerations:** Findings are keyed by file version, so re-indexing the same content hash reuses results; analyzer version of the scanner is part of the cache key.
- **Security considerations:** Values are never persisted, logged or sent; fingerprints are truncated blake3 and salted per organization to prevent cross-tenant correlation. Test fixtures use clearly synthetic strings (no real credentials).
- **Observability additions:** Counter `secret_findings_total{pattern}`, `secret_scan_skipped_total{reason}`; span attribute `secrets_found` on `repository_index`.
- **Tests required (named):**
  - `detects_each_pattern_in_corpus`
  - `env_file_paths_flagged_without_reading_values`
  - `entropy_flags_random_token_and_ignores_uuid_and_lockfile_hashes`
  - `no_value_persisted_only_fingerprint`
  - `fingerprint_salted_per_org`
  - `allow_list_requires_reason_and_is_reported`
  - `scanner_error_marks_file_no_embed`
  - `flagged_file_excluded_from_embedding_units`
  - `rescan_same_content_hash_is_cache_hit`
- **Benchmarks if applicable:** Criterion scan throughput over a 10 MB synthetic tree; target at least 100 MB/s.
- **Acceptance criteria (verifiable):**
  - `engine/scripts/cargo.sh test -p repository --features integration` passes.
  - Running `review init` on the secrets fixture reports expected counts and `psql` shows no secret value in any column.
- **Definition of done:** Acceptance criteria pass; `docs/security/secret-handling.md` describes detection, fingerprints and allow-listing.

---

### SEC-004 — Redaction before model send
Status: ☐

- **Task ID:** SEC-004
- **Title:** End-to-end guarantee that no secret reaches a model provider: policy, enforcement tests and per-repository privacy controls
- **Problem:** GW-010 adds the gateway redaction hook, but the guarantee spans context selection, summarization, embeddings and the verifier. Without an end-to-end test and policy, a new code path (for example the embedding provider) could bypass it.
- **Why it exists:** PRD §111; master plan §13.5 (redaction before send; privacy policy can disable external providers per repository); target-architecture §4.4.
- **Scope:**
  - Extend redaction to embedding inputs and summarization requests, not only chat requests.
  - Per-repository `privacy` policy in `.review/config.yaml`: `external_providers: allow|deny`, `redact_mode: standard|strict`, `path_denylist` (never sent).
  - Context engine honors `no_embed`/`redact_required` flags from SEC-003 and omits denylisted paths.
  - An enforcement test that records all outbound HTTP bodies in a full replay review and scans them against the secret corpus.
- **Explicit non-scope:**
  - Pattern definitions (OBS-006), detection (SEC-003), gateway hook mechanics (GW-010).
  - Provider-side data retention contracts (documented only).
- **Files/modules expected to change:** `engine/crates/semantic/src/provider/*` (embedding providers call the redactor), `engine/crates/context-engine/src/*` (filters), `engine/crates/profile/src/policy.rs` (privacy section), `engine/crates/model-gateway/src/redact.rs`.
- **New files/modules expected:**
  - `engine/crates/pipeline/tests/security/no_secret_egress.rs`
  - `engine/crates/model-gateway/tests/egress_recorder.rs` (a recording `HttpTransport` wrapper, test-only)
  - `fixtures/pull-requests/secret-in-diff/` (synthetic key in base, head and an `.env.example`)
  - `docs/security/model-data-handling.md`
- **Dependencies (task IDs):** GW-010, SEC-003, OBS-006, SEM-002, CTX-001, POL-001, EVAL-001.
- **Implementation details:**
  - Policy schema: `privacy: { external_providers: allow|deny, redact_mode: standard|strict, path_denylist: [glob] }`. `deny` makes the router select only a local or replay provider; with none configured the review fails with `GatewayError::Permanent(PolicyDenied)` and a clear finding-free summary.
  - `strict` mode additionally redacts every high-entropy token of 20 or more characters and any string assigned to identifiers matching the sensitive-name list, even without a pattern match.
  - The redactor runs in the embedding path before text is hashed for the content_hash check, so the stored hash corresponds to the redacted text (consistent re-embeds).
  - Redaction placeholders keep structure: `KEY="«redacted:<pattern>:<hash8>»"` so reasoning about code shape still works (PRD §111).
  - Enforcement test: run the secret-in-diff scenario through the full replay pipeline with the recording transport and a fake embedding HTTP provider; collect every outbound request body; assert none contains any raw value from the secret corpus and every body that contained a secret location carries a placeholder; assert `path_denylist` files appear nowhere.
  - Runtime tripwire: after redaction the gateway re-scans the outgoing serialized body with the same patterns; a hit means a bug, so the request fails closed and increments `llm_redaction_tripwire_total`.
- **Data model changes:** None; `repository_configs` JSON gains the `privacy` section (schema in `packages/contracts`).
- **API/protocol changes:** Config schema addition; the repository settings API exposes `privacy` read-only for viewers and writable for admins (audited by SEC-008).
- **Concurrency semantics:** Redaction is per request on the caller task; the repository's known-fingerprint set is an `Arc` snapshot loaded per run, so concurrent updates cannot tear.
- **Failure behavior:** Redactor panic, tripwire hit or policy denial fails the model call closed. No fallback to unredacted send exists. The review reports `FAILED_REVIEW` with a reason and never silently skips redaction.
- **Idempotency considerations:** Redaction is deterministic, so `request_hash` stays stable across retries and the response cache keys remain valid.
- **Security considerations:** This task closes the data-exfiltration path for secrets; it also verifies no prompt text is logged. Test corpora use synthetic credentials.
- **Observability additions:** `llm_redactions_total{pattern}`, `llm_redaction_tripwire_total` (alert at greater than 0 via OBS-008 follow-up), `llm_blocked_requests_total{reason=policy|tripwire}`.
- **Tests required (named):**
  - `full_review_replay_sends_no_raw_secret_to_any_provider`
  - `embedding_inputs_are_redacted_before_hashing`
  - `path_denylist_files_never_leave_process`
  - `external_providers_deny_blocks_remote_routes`
  - `strict_mode_redacts_unmatched_high_entropy_tokens`
  - `tripwire_fails_closed_on_unredacted_body`
  - `placeholder_preserves_code_shape`
  - `privacy_policy_schema_validation`
- **Benchmarks if applicable:** Redaction plus tripwire overhead under 2 ms for a 100 KiB request (Criterion).
- **Acceptance criteria (verifiable):**
  - The egress test passes in CI-004.
  - Disabling the redactor in a scratch branch makes `full_review_replay_sends_no_raw_secret_to_any_provider` fail.
- **Definition of done:** Acceptance criteria pass; `docs/security/model-data-handling.md` documents policy options and guarantees.

---

### SEC-005 — Provider token and GitHub App private key handling
Status: ☐

- **Task ID:** SEC-005
- **Title:** Secure handling of the GitHub App private key, installation tokens and short-lived clone credentials, with leak tests
- **Problem:** The App private key can mint tokens for every installation, and installation tokens read customer source. Handling is currently specified across GH-001 and GH-006 but there is no single verification that these never touch disk, logs, DB rows or error messages.
- **Why it exists:** Production readiness "Provider-token security"; master plan §13.2 (key from env or secret manager, tokens cached encrypted in Redis with TTL below expiry and never persisted, workers get clone tokens in memory only).
- **Scope:**
  - A `SecretSource` abstraction for the private key (env var content, env var path, secret-manager adapter stub) with startup validation.
  - Redis token cache encryption (AES-256-GCM, key from `TOKEN_CACHE_KEY`, key id for rotation), TTL at most 50 minutes and strictly below token expiry.
  - Clone credential broker constraints: per-job token, single-repository scope via `repository_ids`, minimal permissions, expiry at most 10 minutes, audited issuance.
  - Worker-side handling: token passed to git through an in-memory credential helper or `http.extraHeader` env, never in the URL, never written to `.git/config`.
  - Leak tests.
- **Explicit non-scope:**
  - The GitHub App auth flow itself (GH-001) and broker endpoint (GH-006); this task hardens and verifies them.
  - OAuth login secrets (API-004).
  - A real KMS integration (adapter interface and docs only).
- **Files/modules expected to change:** `apps/api/src/providers/github/auth/*`, `apps/api/src/internal/*`, `engine/crates/repository/src/git/*` (credential injection).
- **New files/modules expected:**
  - `apps/api/src/security/{secret-source.ts,token-cipher.ts}`
  - `apps/api/test/security/{token-leak.spec.ts,token-cache-encryption.spec.ts}`
  - `engine/crates/repository/tests/clone_credential_hygiene.rs`
  - `docs/security/credential-handling.md`
- **Dependencies (task IDs):** GH-001, GH-006, API-005, OBS-006, SEC-008.
- **Implementation details:**
  - Private key loaded once at boot into a `KeyObject` (`crypto.createPrivateKey`), original string zeroed where the runtime allows; format validated (RS256 PEM, at least 2048 bits). Never present in `process.env` dumps: config module exposes it only through `SecretSource`.
  - Token cache: Redis value is `{kid, iv, ciphertext, tag}`; the Redis key includes installation id only. AAD binds installation id, so a swapped value fails to decrypt. Cache TTL = `min(50 min, expires_at - now - 60 s)`.
  - Broker: `POST /internal/repositories/:id/clone-credentials` requests an installation token restricted to one repository with `contents: read`; response carries `expires_at`; broker refuses if the job is not `running` for that repository, with the job id from service-auth claims (API-005).
  - Git invocation: workers use `GIT_ASKPASS`-style in-process helper or `-c http.extraHeader="Authorization: Basic ..."` through environment of the child process (`GIT_CONFIG_COUNT` variables), never on the command line (visible in process listings). After checkout, a post-step asserts `.git/config` contains no credentials.
  - Debug/Display of token types are redacted (`Secret<T>`); error mapping strips headers from Octokit and git errors.
  - Rotation: documented procedure for private key (GitHub supports multiple keys) and `TOKEN_CACHE_KEY` (decrypt with previous kid, write with new).
- **Data model changes:** None. Explicit non-change: no token or key column exists; a schema test asserts no column named like `token|secret|private_key` stores credentials (allow-list for hashed session ids).
- **API/protocol changes:** Broker response contract adds `expires_at` and `scope` fields.
- **Concurrency semantics:** Concurrent token requests for one installation are single-flight (one mint, others await); the cache write uses `SET NX PX`-style semantics to avoid thundering herd.
- **Failure behavior:** Missing or invalid key fails API startup (fail fast). Cache decrypt failure is treated as a miss and re-mints. Broker denial returns 404 for unknown job/repository (no oracle) and 403 for wrong state.
- **Idempotency considerations:** Re-requesting credentials for the same job within TTL may return the cached short-lived token; a new job gets a new token.
- **Security considerations:** The core of the task. Tokens appear in no log, span attribute, job payload, DB row, object-store object or error message; the leak tests assert it by scanning all sinks after a full webhook-to-clone run with a sentinel token value.
- **Observability additions:** `github_token_mint_total{outcome}`, `clone_credentials_issued_total`, `token_cache_decrypt_failures_total`; audit events (SEC-008) for issuance.
- **Tests required (named):**
  - `private_key_missing_or_weak_fails_startup`
  - `token_cache_value_is_ciphertext_in_redis`
  - `token_cache_ttl_below_expiry`
  - `swapped_cache_value_fails_aad_check`
  - `single_flight_mint_per_installation`
  - `sentinel_token_absent_from_logs_spans_db_and_redis_plaintext`
  - `git_config_contains_no_credentials_after_checkout`
  - `token_not_in_process_arguments`
  - `broker_rejects_non_running_job_and_foreign_repo`
- **Benchmarks if applicable:** None.
- **Acceptance criteria (verifiable):**
  - All tests pass in CI-004; the sentinel leak test runs with the fake GitHub server (DEV-005).
  - `docs/security/credential-handling.md` includes rotation steps verified once in staging.
- **Definition of done:** Acceptance criteria pass; GH-010 no-merge test still green; rotation doc reviewed.

---

### SEC-006 — Webhook replay protection
Status: ☐

- **Task ID:** SEC-006
- **Title:** Replay window and delivery-age checks on top of HMAC verification and delivery-id deduplication
- **Problem:** An attacker who captures a valid signed webhook body can resend it later. HMAC alone proves origin, not freshness; GitHub webhook payloads carry no signed timestamp, so freshness must be derived and the replay surface bounded.
- **Why it exists:** Master plan §13.4 ("HMAC-SHA256 with constant-time comparison, delivery-id dedup, and a timestamp/replay window"); GH-002 explicitly defers the replay window here; GH-003 owns idempotency.
- **Scope:**
  - Delivery-id replay detection that outlives the idempotency TTL for security purposes (`webhook_deliveries` retention), with a distinction between a legitimate GitHub redelivery and a replay.
  - A freshness check using the event's own timestamps (`pull_request.updated_at`, `head_commit.timestamp`, `check_suite.updated_at`) against a configurable maximum age, plus state-based staleness (head sha no longer current).
  - Optional per-installation rate limit on webhook intake.
  - Clear metrics and audit entries for rejected replays.
- **Explicit non-scope:**
  - HMAC verification (GH-002) and event normalization (GH-004).
  - IP allow-listing of GitHub hook ranges (documented as optional infra control).
  - Webhooks from non-GitHub providers.
- **Files/modules expected to change:** `apps/api/src/webhooks/github-webhook.controller.ts`, `apps/api/src/webhooks/delivery.service.ts` (GH-003).
- **New files/modules expected:**
  - `apps/api/src/webhooks/replay-guard.ts`
  - `apps/api/test/webhooks/replay-guard.spec.ts`
  - `engine/migrations/{seq}_webhook_delivery_security.sql` (only if columns are missing)
  - `docs/security/webhook-replay.md`
- **Dependencies (task IDs):** GH-002, GH-003, GH-004, SEC-008, DOM-009.
- **Implementation details:**
  - Order in the controller: verify HMAC, then replay guard, then idempotency record, then normalize. The guard never runs before signature verification, so unauthenticated callers cannot probe it.
  - Delivery id: `X-GitHub-Delivery` seen before with the same body hash and `X-GitHub-Hook-ID` is an idempotent duplicate (202 `accepted:false, reason:"duplicate"`). Seen before with a different body hash is a conflict, since GitHub redelivery reuses the same payload; respond 202 with `reason:"delivery_id_reuse"` and write an audit event with severity `warning`.
  - Retention: `webhook_deliveries` rows keep `delivery_id, body_sha256, received_at, outcome` for at least 30 days (configurable `WEBHOOK_REPLAY_RETENTION_DAYS`), longer than the Redis SETNX TTL, so a replay after Redis expiry is still detected via the table.
  - Freshness: configurable `WEBHOOK_MAX_EVENT_AGE_SECONDS` (default 900 for events with a timestamp). Older events are accepted-and-ignored (`reason:"stale_event"`) because GitHub itself may redeliver manually; the actual processing guard remains the head-sha check in review creation (events for a superseded head do nothing).
  - GitHub manual redelivery by an operator is allowed through an explicit flag in `webhook_deliveries` (`redelivery_of`) when the delivery id differs; same-id redelivery is recognized by `X-GitHub-Hook-Installation-Target-ID` plus identical body hash and processed only if the prior outcome was `failed` or `received`.
  - Per-installation intake limit: Redis sliding window, default 120 events per minute, excess returns 429 so GitHub retries later.
- **Data model changes:** `webhook_deliveries` gains `body_sha256 bytea`, `outcome text`, `redelivery_of uuid NULL` if absent; index on `(received_at)` for retention sweeps.
- **API/protocol changes:** `POST /api/v1/webhooks/github` response `reason` values `duplicate | delivery_id_reuse | stale_event | rate_limited`.
- **Concurrency semantics:** Two simultaneous identical deliveries race on `INSERT ... ON CONFLICT (delivery_id) DO NOTHING`; exactly one proceeds.
- **Failure behavior:** Redis unavailable falls back to the table check (slower but correct). Database unavailable returns 503 so GitHub retries. The guard never turns a valid first delivery into a rejection because of its own errors.
- **Idempotency considerations:** The guard is part of the idempotency story; all outcomes are deterministic for a given (delivery id, body hash) pair.
- **Security considerations:** Rejection responses are empty-bodied or minimal and leak no state; the body hash is stored, not the body. Rejected replays are audited (SEC-008) with delivery id and installation id only.
- **Observability additions:** `webhook_replays_rejected_total{reason}`, `webhook_stale_events_total`; the existing signature failure alert is complemented by a replay spike alert definition (OBS-008 follow-up).
- **Tests required (named):**
  - `duplicate_delivery_same_body_is_noop_202`
  - `same_delivery_id_different_body_flagged_and_audited`
  - `replay_after_redis_ttl_still_detected_via_table`
  - `stale_event_accepted_but_ignored`
  - `fresh_event_processed_once_under_concurrent_duplicates`
  - `operator_redelivery_of_failed_delivery_is_processed`
  - `guard_runs_only_after_valid_signature`
  - `rate_limit_returns_429_per_installation`
  - `redis_down_falls_back_to_table`
- **Benchmarks if applicable:** Guard adds under 5 ms p95 to the ack path (k6 in PERF-008).
- **Acceptance criteria (verifiable):**
  - Tests pass; replaying the same captured request twice with the replay tool (DEV-005) yields one review and one `duplicate` response.
  - A body tampered after signing still returns 401, unchanged from GH-002.
- **Definition of done:** Acceptance criteria pass; `docs/security/webhook-replay.md` describes windows and tuning.

---

### SEC-007 — Source retention policies
Status: ☐

- **Task ID:** SEC-007
- **Title:** Configurable source retention (indefinite, N days, graph-only, ephemeral) enforced by a sweeper, with checkout wipe guarantees
- **Problem:** The system stores sensitive source (object-store file blobs, bare mirrors, snippets in findings evidence). Nothing limits how long it stays or lets an organization choose a stricter policy.
- **Why it exists:** PRD §113 (retain indefinitely, N days, graph but not raw source, ephemeral processing); master plan §13.6 (checkouts wiped after the job, retention enforced).
- **Scope:**
  - Policy model at organization level with per-repository override: `retain_indefinitely | retain_days(N) | graph_only | ephemeral`.
  - What each policy removes: raw file content blobs in object storage, bare mirror clones on worker disks, evidence snippets in `finding_evidence`, model cache entries, embeddings payload text; graph rows stay except under `ephemeral`.
  - A scheduled retention sweeper job (`retention-sweep` queue) and a per-job checkout wiper.
  - Settings API and audit entries.
- **Explicit non-scope:**
  - Deleting an entire organization or repository on request (GDPR-style erasure is a follow-up; the sweeper design allows it).
  - Backups and PITR retention (operations doc).
  - Web UI for the setting (WEB task).
- **Files/modules expected to change:** `engine/crates/pipeline/src/*` (checkout lifecycle), `engine/crates/graph-storage/src/*` (blob store usage), `apps/api/src/repositories/*` (settings), `engine/crates/semantic/src/*` (payload text removal).
- **New files/modules expected:**
  - `engine/migrations/{seq}_retention_policies.sql`
  - `engine/crates/pipeline/src/retention/{mod.rs,policy.rs,sweeper.rs,checkout_guard.rs}`
  - `engine/crates/pipeline/tests/retention.rs`, `apps/api/test/repositories/retention-settings.spec.ts`
  - `docs/security/source-retention.md`
- **Dependencies (task IDs):** GS-001, IDX-001, SEC-009, SEC-008, API-008, DOM-009, PIPE-001.
- **Implementation details:**
  - Columns: `organizations.retention_policy jsonb`, `repositories.retention_override jsonb NULL`; resolved policy = repository override else organization else default `retain_days(90)`.
  - `checkout_guard`: an RAII type owning the per-job temp directory (`/work/{job_id}`); on drop (and in a panic-safe finalizer and on SIGTERM drain) the directory is removed recursively; bare mirrors are kept only under `retain_*` policies, in a cache dir with a last-used timestamp and a max total size.
  - `retain_days(N)`: sweeper deletes object-store blobs whose `last_referenced_at` is older than N days, nulls `finding_evidence.snippet` and any stored context text for runs older than N days (keeping hashes, ranges and ids), prunes `model_cache`.
  - `graph_only`: raw content blobs are deleted right after the index stage completes; symbol rows, edges and signatures remain (signatures are code; the doc states this explicitly); evidence snippets are stored as ranges only and re-fetched from the provider on demand.
  - `ephemeral`: after the review is published, the sweeper removes snapshots, file versions, embeddings and blobs for that review's head; only findings metadata without snippets remains.
  - Sweeper: runs daily per organization in batches (1000 rows or 100 objects), keyed by `(organization_id, policy_version)`, resumable; every deletion batch writes an audit entry with counts.
  - Policy change is forward-looking at sweep time; tightening applies on the next sweep, loosening never resurrects deleted data.
- **Data model changes:** The columns above; `file_versions.blob_key`, `blob_deleted_at`; `finding_evidence.snippet_purged_at`.
- **API/protocol changes:** `PUT /api/v1/organizations/:id/retention` and `PUT /api/v1/repositories/:id/retention` (admin role).
- **Concurrency semantics:** The sweeper takes an advisory lock per organization; it skips blobs referenced by running jobs (`last_referenced_at` bumped by readers; the job's snapshot id is excluded).
- **Failure behavior:** Object-store delete errors are retried next sweep and reported; the sweep never marks a row purged until the delete succeeds. A failed checkout wipe logs an error and raises `checkout_wipe_failures_total`; the worker refuses to start another job when free disk is below a threshold.
- **Idempotency considerations:** Deleting an already-absent object is success; purge markers make the sweep re-runnable.
- **Security considerations:** Stricter policy wins on conflicts; defaults are conservative. Checkout directories are created with mode 0700 and under a non-shared path. Sweeper uses `rg_ops` only inside the sweeper process.
- **Observability additions:** `retention_objects_deleted_total{policy}`, `retention_sweep_duration_seconds`, `checkout_wipe_failures_total`; span `retention_sweep`.
- **Tests required (named):**
  - `policy_resolution_override_beats_org_default`
  - `checkout_dir_removed_on_success_failure_and_panic`
  - `retain_days_deletes_only_expired_blobs`
  - `graph_only_deletes_blobs_keeps_symbols`
  - `ephemeral_purges_snapshots_after_publish`
  - `sweeper_skips_blobs_of_running_jobs`
  - `sweep_is_resumable_and_idempotent`
  - `tightening_applies_loosening_does_not_restore`
  - `retention_changes_are_audited`
- **Benchmarks if applicable:** Sweep throughput of at least 1000 objects per minute on local SeaweedFS.
- **Acceptance criteria (verifiable):**
  - Tests pass against the test stack; after an `ephemeral` review no blob keys remain under the org prefix (listing check).
  - `docs/security/source-retention.md` states what each policy retains.
- **Definition of done:** Acceptance criteria pass; sweeper registered as a scheduled job; audit entries visible.

---

### SEC-008 — Audit log
Status: ☐

- **Task ID:** SEC-008
- **Title:** Append-only audit log for configuration changes, publication, feedback and security-relevant events
- **Problem:** There is no durable record of who changed repository configuration, what was published, which feedback was given or which security events occurred. This is needed for enterprise review and incident analysis.
- **Why it exists:** PRD §110 ("audit logging"); master plan §13.8 ("covers configuration changes, publication and feedback"); API-003 already includes `audit_log` in the RLS table list.
- **Scope:**
  - `audit_log` table (append-only), an `AuditService` in NestJS and a Rust `audit` helper that writes through the same table.
  - Events: config/policy/retention/privacy changes, membership and role changes, repository enable/disable, review publication, finding feedback, credential issuance (SEC-005), replay rejections (SEC-006), RLS/guard denials above a threshold, and login/logout.
  - Read API for admins with filtering and pagination.
  - Tamper evidence: per-organization hash chain.
- **Explicit non-scope:**
  - A SIEM export (documented hook only).
  - UI (WEB task).
  - Application logs (OBS tasks); audit is a business record, not a debug log.
- **Files/modules expected to change:** Services that perform audited actions (`repositories`, `publisher`, `findings`, `auth`, `internal`), `apps/api/src/app.module.ts`.
- **New files/modules expected:**
  - `engine/migrations/{seq}_audit_log.sql` (if DOM-009 did not already define it, extend it)
  - `apps/api/src/audit/{audit.module.ts,audit.service.ts,audit.controller.ts,audit.types.ts}`
  - `engine/crates/review-core/src/audit.rs` (event enum shared via contracts)
  - `apps/api/test/audit/*.spec.ts`
  - `docs/security/audit-log.md`
- **Dependencies (task IDs):** API-003, API-008, API-010, API-012, GH-009, DOM-009.
- **Implementation details:**
  - Table: `audit_log (id uuid, organization_id, repository_id NULL, occurred_at timestamptz, actor_type user|service|system, actor_id text, action text, target_type text, target_id text, outcome success|denied|failure, metadata jsonb, request_id, trace_id, prev_hash bytea, hash bytea)`.
  - Append-only enforcement: `rg_api` and `rg_engine` have `INSERT, SELECT` only; a trigger rejects `UPDATE` and `DELETE` for all roles except a retention role used by the documented purge procedure. RLS applies for reads.
  - Hash chain: `hash = sha256(prev_hash || canonical_json(row without hash))`, computed in an `BEFORE INSERT` trigger under an advisory lock per organization so the chain is gap-free; a verification function and `GET /api/v1/organizations/:id/audit/verify` check integrity.
  - Metadata policy: only ids, enum values, before/after of non-secret config fields (config diffs go through the redactor from OBS-006); no source, tokens or prompts. A typed `AuditEvent` union restricts what each action may carry.
  - Writing in the same transaction as the audited change wherever possible (publication, config change, feedback), so an action cannot succeed without its audit row; for read-only events (login) best-effort async insert is acceptable.
  - Read API: `GET /api/v1/organizations/:id/audit?action=&actor=&from=&to=&cursor=` (admin role), cursor-based pagination, max page 200.
- **Data model changes:** `audit_log` as above with indexes `(organization_id, occurred_at DESC)`, `(organization_id, action, occurred_at)`.
- **API/protocol changes:** The two read endpoints; event type catalog in `packages/contracts`.
- **Concurrency semantics:** Per-organization advisory lock serializes chain writes; throughput is expected in the low hundreds of events per minute, well within limits. Different organizations proceed in parallel.
- **Failure behavior:** Failure to write an audit row for a mutating action rolls back that action (fail closed). Denial events use a separate short transaction so rollbacks of the request do not erase them.
- **Idempotency considerations:** Events carry a deterministic `dedupe_key` (for example `publish:{review_run_id}:{head_sha}`) with a unique index, so retried jobs do not double-audit.
- **Security considerations:** Immutable by roles and triggers; readable only by admins of that organization (RLS plus guard); service accounts cannot read. Hash chain detects tampering by a privileged DB user.
- **Observability additions:** `audit_events_total{action,outcome}`, `audit_write_failures_total`, `audit_chain_verify_failures_total` (alert candidate).
- **Tests required (named):**
  - `config_change_writes_audit_row_in_same_transaction`
  - `audit_failure_rolls_back_mutation`
  - `update_and_delete_on_audit_log_rejected_for_api_role`
  - `hash_chain_detects_row_tampering`
  - `concurrent_inserts_keep_chain_gapless`
  - `publication_and_feedback_are_audited_once_under_retry`
  - `audit_read_requires_admin_and_is_tenant_scoped`
  - `metadata_never_contains_secret_patterns`
  - `denied_access_event_survives_request_rollback`
- **Benchmarks if applicable:** Insert latency p95 under 10 ms with 10 concurrent writers per organization.
- **Acceptance criteria (verifiable):**
  - Tests pass; manually changing a repository retention setting yields one row visible through the audit API and verification passes.
  - SEC-001 matrix includes the audit routes.
- **Definition of done:** Acceptance criteria pass; `docs/security/audit-log.md` lists every action name and its metadata fields.

---

### SEC-009 — Object storage access controls
Status: ☐

- **Task ID:** SEC-009
- **Title:** Tenant-prefixed, private object storage with signed URLs only, least-privilege credentials and a conformance test on SeaweedFS (local) and GCS (target)
- **Problem:** Repository artifacts (file blobs, parse caches, snapshots) live in S3-compatible storage. Without enforced key structure and access rules a bug or a leaked URL could expose source across tenants.
- **Why it exists:** Master plan §13.1 ("Object-store keys are prefixed by org/repo, with signed URLs only"); PRD §110 ("access-controlled repository artifacts").
- **Scope:**
  - An `ArtifactStore` port (Rust) with a `TenantKey` type that forces the key to be `org/{organization_id}/repo/{repository_id}/{kind}/{...}`; no raw string keys in the public API.
  - Adapters for S3-compatible (local SeaweedFS at port 29000, bucket `reviewgraph-artifacts`) and GCS (S3 interoperability); the TS side uses the same prefix helper for signed URLs.
  - Signed URL generation with short expiry (at most 5 minutes) and method restriction; no public ACLs.
  - Credential separation: worker read/write, API signing-only, ops admin; bucket private by default.
  - A conformance test and a bucket-policy check script.
- **Explicit non-scope:**
  - Retention sweeping (SEC-007) though it uses this port.
  - Encryption key management beyond enabling server-side encryption in production config.
  - CDN or public asset hosting.
- **Files/modules expected to change:** `infra/compose/objectstore/s3.json` (separate identities), `infra/compose/docker-compose.yml` (init script only if needed), `engine/crates/graph-storage/src/blob.rs` (use the port).
- **New files/modules expected:**
  - `engine/crates/graph-storage/src/artifact_store/{mod.rs,key.rs,s3.rs,signed_url.rs}`
  - `engine/crates/graph-storage/tests/artifact_store_conformance.rs`
  - `apps/api/src/storage/{artifact-url.service.ts,tenant-key.ts}`, `apps/api/test/storage/*.spec.ts`
  - `infra/scripts/check-bucket-policy.sh`, `docs/security/object-storage.md`
- **Dependencies (task IDs):** FND-005, GS-001, API-003, SEC-001.
- **Implementation details:**
  - `TenantKey::new(org, repo, kind, rest)` validates `rest` (no `..`, no leading `/`, no control chars, max 512 bytes) and returns the fully qualified key. `ArtifactStore::{put,get,head,delete,list_prefix}` accept only `TenantKey` or a `TenantScope` prefix. The list operation is always bounded by the scope prefix.
  - Kinds: `file-blob`, `parse-cache`, `snapshot`, `export`. Object content for `file-blob` is addressed by content hash inside the prefix (`.../file-blob/{sha256}`), deduplicated per repository, not across tenants.
  - Signed URLs: only for `export` and UI-driven downloads, generated by the API after the tenancy guard; query contains the signature and expiry, no tenant identifiers beyond the key; method fixed to GET; max 5 minutes. Workers use authenticated SDK access, not URLs.
  - Local config: `s3.json` defines identities `rg-worker` (Read, Write, List, Tagging on the bucket), `rg-api-signer` (Read), `rg-admin` (Admin, used only by init). The dev defaults stay loopback only; the production doc uses IAM or HMAC keys scoped by bucket and prefix conditions.
  - Server-side encryption flag in production config (`RG_S3_SSE=AES256|aws:kms`), TLS required for non-loopback endpoints (config validation rejects `http://` for non-loopback hosts).
  - `check-bucket-policy.sh`: asserts the bucket denies anonymous list and get (unauthenticated `curl` returns 403) and that versioning/lifecycle settings match the doc.
- **Data model changes:** `file_versions.blob_key` stores the relative key; the org/repo prefix is derived, never stored from user input.
- **API/protocol changes:** `GET /api/v1/repositories/:id/artifacts/:artifactId/download-url` returns `{url, expires_at}` after guard and role checks.
- **Concurrency semantics:** Puts are idempotent by content hash (`If-None-Match: *` where supported, otherwise HEAD-then-PUT); concurrent identical puts converge.
- **Failure behavior:** Store errors map to typed retryable (`Unavailable`, `Throttled`) and permanent (`Denied`, `NotFound`) errors; a foreign key attempt returns `NotFound` rather than `Denied` to avoid an oracle.
- **Idempotency considerations:** `put` of existing content is a no-op returning the same key; `delete` of a missing key succeeds.
- **Security considerations:** No bucket is public; credentials per role; signed URL expiry capped; path traversal rejected; object listing never crosses prefixes; all access paths covered by SEC-001/002-style tests.
- **Observability additions:** `object_store_requests_total{op,outcome}`, `object_store_denied_total`, span `object_store.{op}` with `kind` and size, never key contents beyond kind.
- **Tests required (named):**
  - `tenant_key_rejects_traversal_and_long_keys`
  - `put_get_roundtrip_and_content_hash_dedup`
  - `list_prefix_never_returns_other_tenant_keys`
  - `foreign_key_get_returns_not_found`
  - `anonymous_get_and_list_denied` (live SeaweedFS)
  - `signed_url_expires_and_rejects_other_method`
  - `api_signer_identity_cannot_write`
  - `non_loopback_http_endpoint_rejected_by_config`
  - `gcs_adapter_conformance` (ignored unless credentials present)
- **Benchmarks if applicable:** Put/get throughput of 1 MiB objects against local SeaweedFS recorded in PERF notes.
- **Acceptance criteria (verifiable):**
  - Tests pass with the dev compose stack; `bash infra/scripts/check-bucket-policy.sh` exits 0.
  - Code review check: no use of raw string keys in `ArtifactStore` callers (a grep test enforces it).
- **Definition of done:** Acceptance criteria pass; `docs/security/object-storage.md` documents roles, prefixes and the production IAM template.

---

### SEC-010 — Threat model document and dependency scanning
Status: ☐

- **Task ID:** SEC-010
- **Title:** STRIDE threat model for ReviewGraph and an SBOM plus dependency/image scanning policy
- **Problem:** Security controls exist as scattered tasks with no consolidated threat model, and the supply chain (crates, npm packages, container images) has no SBOM or vulnerability triage process beyond the base `cargo deny` config.
- **Why it exists:** Master plan §13.7 ("`cargo deny`, `pnpm audit`, pinned images, SBOM per image"); production readiness row "Dependency scanning"; PRD §110.
- **Scope:**
  - `docs/security/threat-model.md`: assets, trust boundaries, data-flow diagram, STRIDE table per boundary, mitigations mapped to task IDs, residual risks and owners.
  - SBOM generation (CycloneDX) for Rust, Node and each image; image vulnerability scan; a vulnerability triage policy with SLAs and an exceptions file.
  - Scripts and a CI-facing entry point (`pnpm security:scan`) that CI-002 and CI-008 call.
- **Explicit non-scope:**
  - Implementing the controls (other SEC tasks) or CI workflow wiring (CI-002, CI-008).
  - Penetration testing or a third-party audit.
  - Runtime threat detection.
- **Files/modules expected to change:** Root `package.json` (scripts), `engine/deny.toml` (advisory settings referenced by the doc), `docs/security/README.md` index.
- **New files/modules expected:**
  - `docs/security/{threat-model.md,vulnerability-management.md}`
  - `security/exceptions.toml` (accepted advisories with id, reason, expiry date, owner)
  - `scripts/security/{sbom.sh,scan-images.sh,check-exceptions.mjs}`, `scripts/security/check-exceptions.test.mjs`
- **Dependencies (task IDs):** SEC-001 to SEC-009 (mapping targets), FND-004, CI-002, CI-008.
- **Implementation details:**
  - Threat model structure: assets (source code, installation tokens, App private key, tenant data, model prompts and outputs, audit log); actors (external attacker, malicious tenant, compromised dependency, curious provider, insider); boundaries (GitHub to webhook, browser to API, API to PG/Redis, worker to GitHub/model providers/Qdrant/object store, analyzed repository code to the parser).
  - Must include repository-specific threats: malicious repository content (parser DoS, huge files, deeply nested code; mitigated by limits), prompt injection from PR text or code comments steering the reviewer (mitigations: verification stages, no tool execution, structured output, no merge capability, evidence requirements), path traversal in checkout, symlink escapes in worktrees, token exfiltration via crafted logs, cross-tenant graph or vector leakage, supply-chain compromise, SSRF through provider base URLs, and webhook replay.
  - Each threat row: id (`T-xxx`), STRIDE class, boundary, mitigation task ID, test name where one exists, residual risk (L/M/H). A script `check-exceptions.mjs` also validates that every mitigation task ID referenced exists in the plan index and that every `T-` id is unique.
  - SBOM: `cargo cyclonedx` for the workspace, `@cyclonedx/cyclonedx-npm` for Node, and `syft` for images, output to `artifacts/sbom/*.json` (not committed).
  - Scanning: `cargo deny check advisories` (CI-002), `pnpm audit --audit-level=high --prod`, and `grype`/`trivy` for images with fail threshold HIGH or CRITICAL unless listed in `security/exceptions.toml`.
  - Policy: critical advisories fixed or mitigated within 7 days, high within 30, exceptions expire after at most 90 days; expired exceptions fail the scan.
  - Image hygiene checks: non-root user, no secrets in layers (`docker history` grep for patterns from OBS-006), pinned base image digests.
- **Data model changes:** None.
- **API/protocol changes:** None.
- **Concurrency semantics:** Scripts are stateless and safe to run in parallel in CI matrix jobs.
- **Failure behavior:** Scanner tool missing fails with a clear install hint locally and fails CI; transient registry errors retry twice. An expired or malformed exception fails the run.
- **Idempotency considerations:** Reruns overwrite SBOM outputs; scan results are deterministic for a fixed advisory database date (recorded in the output).
- **Security considerations:** Exceptions must have an owner and a reason; scan outputs never include secrets; the threat model is reviewed on each ADR touching a boundary.
- **Observability additions:** None at runtime; CI publishes the SBOM and scan report as artifacts.
- **Tests required (named):**
  - `exceptions_file_parses_and_rejects_expired_entries`
  - `exceptions_require_owner_and_reason`
  - `threat_model_task_references_resolve`
  - `threat_ids_unique`
  - `sbom_script_produces_cyclonedx_for_workspace` (smoke)
  - `scan_images_fails_on_unlisted_high_vulnerability` (using a fixture report)
- **Benchmarks if applicable:** None.
- **Acceptance criteria (verifiable):**
  - `node --test scripts/security/check-exceptions.test.mjs` passes; `pnpm security:scan` runs locally (with tools installed) and exits 0 on a clean tree.
  - The threat model contains at least the threats listed above and a mapped mitigation for each.
- **Definition of done:** Acceptance criteria pass; documents reviewed and linked from `docs/security/README.md`; CI-002 and CI-008 reference the scripts.

---

### PERF-001 — Synthetic large-repo generator
Status: ☐

- **Task ID:** PERF-001
- **Title:** Deterministic synthetic NestJS-shaped repository generator (100k files, ~1M symbols)
- **Problem:** PRD §120 sets a production scale target of 100,000 source files, 1,000,000 symbols and a multi-million-edge graph. No real repository is that large (the reference repository is ~1k files, ~15k symbols), so the scale claims and the ADR-014 graph-database trigger thresholds cannot be measured without a generated repo.
- **Why it exists:** Master plan §12 (synthetic generator, seeded), ADR-014 (benchmarks on the 1M-symbol synthetic repository decide whether a graph DB returns), PRD §120. Every PERF-00x task and SEM-009 consume its output.
- **Scope:**
  - A generator that writes a NestJS-shaped TypeScript tree (modules, controllers, services, repositories, entities, DTOs, queue processors, specs) to disk, deterministically from `(seed, profile)`.
  - Profiles: `s` (1k files / ~15k symbols, mirrors the reference repository's shape), `m` (10k files), `l` (100k files / ~1M symbols).
  - A generated `manifest.json` with exact expected counts (files, symbols by kind, planted edges, routes, entities) so benches can assert parse/graph correctness.
  - An optional `--git` mode that commits the tree and a `--churn N` mode producing a head commit with N modified files (consumed by PERF-005).
- **Explicit non-scope:** Any benchmark measurement (PERF-002..008); embedding vectors (SEM-009 derives them from the manifest); realistic business logic.
- **Files/modules expected to change:** `engine/Cargo.toml` (workspace member for the generator tool).
- **New files/modules expected:** `benchmarks/perf/synth-repo/{Cargo.toml, src/main.rs, src/profile.rs, src/templates.rs, src/rng.rs, src/manifest.rs}`, `benchmarks/perf/README.md`, `benchmarks/perf/profiles/{s,m,l}.yaml`.
- **Dependencies:** FND-001 (workspace), TSA-001 (to validate that emitted symbols match IR kinds in manifest tests).
- **Implementation details:**
  - RNG: `rand_chacha::ChaCha8Rng` seeded from `blake3(seed || module_index)` so each module is generated independently (parallelisable, order-independent output).
  - Shape per module (default l): ~40 modules x ~25 sub-features; each feature has `*.module.ts`, `*.controller.ts` (4-8 routes with `@UseGuards`), `*.service.ts` (8-20 methods), `*.repository.ts`, `*.entity.ts`, `dto/*.ts`, optional `*.processor.ts` (BullMQ), and `*.spec.ts`. Average ~10 symbols per file gives ~1M symbols at 100k files.
  - Cross-file edges: constructor DI to services in the same module (80%), imports from shared modules (15%), long-range cross-module calls (5%) following a Zipf distribution so a few hub symbols have fan-in > 10k (stress for BFS budgets). A fixed fraction (2%) of call names are deliberately ambiguous to exercise `name_ambiguous`.
  - Output is written with a bounded rayon pool; each file is written via temp + rename.
  - `manifest.json` records `seed`, `profile`, `generator_version`, counts, and `blake3` of the sorted `(path, blake3(bytes))` list (`tree_hash`).
  - Generated files carry a `// @generated-by synth-repo` header only in the `generated/` folder (3% of files) to exercise `is_generated` handling.
- **Data model changes:** None.
- **API/protocol changes:** CLI `synth-repo --profile <s|m|l> --seed <u64> --out <dir> [--git] [--churn <n>]`.
- **Concurrency semantics:** Parallel by module; no shared mutable state; output independent of thread count.
- **Failure behavior:** Refuses to write into a non-empty `--out` unless it holds a matching manifest (then it verifies and exits 0). Disk-space precheck (estimated bytes x 1.2) fails fast.
- **Idempotency considerations:** Same `(seed, profile, generator_version)` yields byte-identical trees and identical `tree_hash`; `--git` uses fixed author/date like EVAL-001 so SHAs are stable.
- **Security considerations:** Output contains no real secrets or identifiers; names come from an embedded word list. No network access.
- **Observability additions:** None (tooling). Prints generation throughput and counts to stderr.
- **Tests required:**
  - `same_seed_same_tree_hash`
  - `different_seed_different_tree_hash`
  - `thread_count_does_not_change_output`
  - `profile_s_counts_within_5pct_of_target`
  - `manifest_counts_match_parsed_symbols_profile_s` (parse with lang-typescript, compare)
  - `hub_fanin_follows_zipf_bounds`
  - `refuses_dirty_output_dir`
  - `churn_modifies_exactly_n_files`
- **Benchmarks if applicable:** Generation itself: profile `l` completes in < 10 min and < 8 GB disk on the reference VM (reported, not gated).
- **Acceptance criteria:** Profile `l` yields 100,000 files (+-2%) and 1,000,000 symbols (+-10%) per PRD §120, with a manifest edge estimate >= 2,000,000. Profile `s` is within 10% of ~1k files / ~15k symbols. Two generations with the same seed produce an identical `tree_hash`.
- **Definition of done:** Global DoD, plus `benchmarks/perf/README.md` documents profiles, the reference VM spec and how generated trees are cached (never committed; `.gitignore`d).

---

---

### PERF-002 — Parse throughput benchmark
Status: ☐

- **Task ID:** PERF-002
- **Title:** Parse throughput benchmark (files/s, MB/s) for the TypeScript analyzer
- **Problem:** Indexing 100k files (PRD §120) is dominated by parsing. Without a measured files/s figure and a regression guard, parser changes (adapters, syntax facts) can silently make a full index take hours.
- **Why it exists:** Master plan §12 ("parse throughput (files/s)"), PRD §120.
- **Scope:**
  - A criterion benchmark of `LanguageAnalyzer::analyze` on a fixed sample, single-threaded and rayon-parallel.
  - A throughput script over a whole synthetic tree (profile `m` and `l`) reporting files/s, MB/s, p50/p95 per-file latency, and peak RSS.
  - A JSON report and a baseline-comparison step.
- **Explicit non-scope:** Linking and graph build (PERF-003); parse cache hit-rate behaviour (IDX-005 tests); I/O (file reading is excluded from the criterion bench, included in the tree script).
- **Files/modules expected to change:** None.
- **New files/modules expected:** `engine/crates/lang-typescript/benches/parse.rs`, `benchmarks/perf/parse/{run.sh, compare.py, baseline.json, README.md}`.
- **Dependencies:** PERF-001, TSA-002, TSA-003, TSA-004, TSA-005.
- **Implementation details:**
  - Criterion groups: `parse_only` (tree-sitter only), `analyze_no_facts`, `analyze_full` (facts and adapters on), using 200 files sampled by seed from profile `s` (mixed controllers/services/entities/specs).
  - Tree script: `review-cli index --parse-only` is not required; the bench links `lang-typescript` directly, reads files with a bounded reader and parses on a rayon pool of `N = {1, 4, 8, num_cpus}` threads, printing a scaling table.
  - Reports `files_per_sec`, `mb_per_sec`, `ParseStatus` distribution (must be 100% Ok on synthetic input), and bytes of IR produced per file.
  - Baseline file stores the last accepted numbers per `(host_class, profile)`; `compare.py` fails when files/s drops > 15% (noise-tolerant) versus the baseline of the same host class.
- **Data model changes:** None.
- **API/protocol changes:** None.
- **Concurrency semantics:** Thread-count sweep verifies near-linear scaling to 8 threads (>= 5x). Analyzers are shared via `Arc` (TSA-001 Send+Sync).
- **Failure behavior:** If any file parses `Partial/Failed`, the run aborts with the path (generator or analyzer bug) rather than reporting skewed throughput.
- **Idempotency considerations:** Seeded input; each criterion iteration is pure.
- **Security considerations:** None; synthetic input only.
- **Observability additions:** Exercises the `files_parsed_total` and `parse_duration_ms` metrics from the analyzer; the bench asserts the counters equal the number of files parsed.
- **Tests required:**
  - `bench_sample_selection_deterministic`
  - `compare_script_flags_regression` (unit test of `compare.py` with fixtures)
  - `bench_aborts_on_partial_parse`
- **Benchmarks if applicable:** This task. Reported on the reference VM (8 vCPU) and a developer laptop class.
- **Acceptance criteria:**
  - Full-tree parse of profile `l` (100k files) finishes in < 10 min on 8 threads (>= 170 files/s sustained), so a cold full index fits the §120 scale story together with PERF-003.
  - Single-thread analyzer throughput >= 150 files/s on the profile `s` sample (median file < 7 ms).
  - Scaling efficiency >= 5x at 8 threads.
  - Baseline committed; CI nightly job runs the criterion subset and posts the comparison.
- **Definition of done:** Global DoD, plus README lists how to run on the reference VM and the baseline refresh procedure.

---

---

### PERF-003 — Graph build and memory benchmark (1M symbols)
Status: ☐

- **Task ID:** PERF-003
- **Title:** Full graph build time, PostgreSQL load time and in-memory footprint at 10k, 100k and 1M symbols
- **Problem:** ADR-014 keeps PostgreSQL plus an in-memory graph only while load time stays below 30 s and memory below 50% of worker RAM. Those numbers are unmeasured, and the CSR layout in target-architecture §3.3 is an untested assumption at multi-million-edge scale (PRD §120).
- **Why it exists:** ADR-014 (graph-database trigger thresholds), master plan §12 ("full graph build and memory at 10k/100k files"), PRD §120.
- **Scope:**
  - Bench of `codegraph` builder + linker on pre-parsed IR (no parsing time included).
  - Bench of `GraphStore::write_full` and `load_graph` on PostgreSQL for profiles `m` (10k files) and `l` (100k files / ~1M symbols).
  - Memory measurement: heap bytes of `Graph`, bytes per node and per edge, process peak RSS.
  - Report against the three ADR-014 thresholds.
- **Explicit non-scope:** Traversal latency (PERF-004); delta snapshots (PERF-005); graph compaction (GS-007).
- **Files/modules expected to change:** None.
- **New files/modules expected:** `engine/crates/codegraph/benches/build.rs`, `engine/crates/graph-storage/benches/pg_load.rs` (custom harness, needs live PG), `benchmarks/perf/graph/{run.sh, README.md, baseline.json}`.
- **Dependencies:** PERF-001, PERF-002 (IR cache reuse), CG-004, CG-005, GS-004, GS-005, GS-008, IDX-001.
- **Implementation details:**
  - Phase 1: parse the tree once, serialize `ParsedUnit`s to a bincode cache (reused by later PERF tasks, keyed by manifest `tree_hash`).
  - Phase 2 (criterion): build `Graph` from cached IR; measure wall time, `edges_total`, `unresolved_refs`, resolved-by distribution.
  - Phase 3: `write_full` into a fresh schema of a throwaway database; measure rows/s and total bytes (`pg_total_relation_size`); then cold `load_graph` into memory (cache cleared) and warm LRU hit.
  - Memory: use `dhat`/`jemalloc_ctl` for allocated bytes and read `/proc/self/status` `VmHWM`; report bytes/node and bytes/edge. Peak RSS during load is also recorded (load must stream, not hold both the rows and the graph).
  - Worker RAM reference: 16 GB; threshold 50% = 8 GB.
- **Data model changes:** None.
- **API/protocol changes:** None.
- **Concurrency semantics:** Load uses the production code path (parallel COPY readers if implemented by GS-005); the bench runs 1 loader and then 4 concurrent loaders of distinct snapshots to measure LRU-bounded memory.
- **Failure behavior:** Refuses to run unless `BENCH_DATABASE_URL` points at a database whose name starts with `rg_bench_`; drops it at the end.
- **Idempotency considerations:** Seeded input, deterministic graph; `tree_hash` and node/edge counts are asserted equal to the previous run's cache before timing.
- **Security considerations:** Synthetic data only; benchmark DB is isolated and contains no tenant data.
- **Observability additions:** Reads existing metrics `graph_load_duration_ms` and `graph_memory_bytes`; fails if they are not emitted.
- **Tests required:**
  - `bench_refuses_non_bench_database`
  - `graph_counts_match_manifest_profile_s`
  - `bytes_per_edge_reported_nonzero`
  - `report_flags_threshold_breach` (unit test of the report generator)
- **Benchmarks if applicable:** This task.
- **Acceptance criteria (ADR-014 thresholds, PRD §120):**
  - At 1M symbols: in-memory load from PostgreSQL < 30 s cold; resident graph memory < 8 GB (50% of a 16 GB worker); `write_full` completes < 15 min.
  - Edge count >= 2M is built and loaded without OOM.
  - If any threshold is breached, a follow-up ADR note is filed with measurements (graph-database reconsideration per ADR-014) before this task closes.
- **Definition of done:** Global DoD, plus the first report is committed under `benchmarks/perf/graph/reports/` and baseline refreshed.

---

---

### PERF-004 — Neighbor lookup and bounded BFS benchmarks (memory and SQL)
Status: ☐

- **Task ID:** PERF-004
- **Title:** Neighbor p50/p95 and bounded BFS latency in memory and via PostgreSQL SQL at 1M symbols
- **Problem:** ADR-014 states neighbor lookups from SQL must stay below 20 ms p95 and that traversals run in memory with depth <= 3. Both claims are unmeasured, and a hub symbol with huge fan-in (Zipf tail) could break budgets silently.
- **Why it exists:** ADR-014, master plan §12 ("neighbor lookup p50/p95 (memory and SQL); bounded BFS"), target-architecture §3.3 (every traversal has an explicit budget and `truncated`).
- **Scope:**
  - Criterion benches for `neighbors`, `bounded_bfs` (depth 1/2/3, max_nodes 100/500/5000, with and without `min_confidence`), `shortest_path`.
  - A custom harness for the SQL single-hop neighbor query (`(snapshot_id, source_key, kind)` and `(snapshot_id, target_key, kind)` indexes) with a warm and a cold buffer cache.
  - Seed sampling: uniform random symbols, plus the top-100 hub symbols (worst case).
- **Explicit non-scope:** Graph load (PERF-003); incremental overlay queries (PERF-005); recursive SQL (explicitly not used per ADR-014).
- **Files/modules expected to change:** None.
- **New files/modules expected:** `engine/crates/codegraph/benches/traverse.rs`, `engine/crates/graph-storage/benches/sql_neighbors.rs`, `benchmarks/perf/traverse/{README.md, baseline.json}`.
- **Dependencies:** PERF-001, PERF-003, CG-007, CG-008, CG-009, CG-010 (overlay), GS-005.
- **Implementation details:**
  - 10,000 sampled seeds (seeded) per scenario; report p50/p95/p99 and `truncated` rate.
  - Overlay variant: the same queries over `GraphOverlay{base, added, removed}` with a 50-file delta, to confirm overlay overhead < 25% versus the base graph.
  - SQL variant: `SELECT ... WHERE snapshot_id=$1 AND source_key=$2 AND kind=ANY($3)` (production statement from GS-005, prepared); also reports `EXPLAIN (ANALYZE, BUFFERS)` plans for one hub and one leaf into the report to prove index use.
  - Assertions on every iteration: result size <= budget, `truncated=true` whenever the budget was hit, deterministic ordering (second run returns identical node order).
- **Data model changes:** None.
- **API/protocol changes:** None.
- **Concurrency semantics:** Memory graph is `Arc<Graph>` shared across 1/8/32 reader threads; throughput (queries/s) reported to show no lock contention. SQL variant runs 1 and 16 pooled connections.
- **Failure behavior:** A BFS that exceeds its budget is a bench failure (invariant violation), not a slow result. Refuses to run on non-`rg_bench_` databases.
- **Idempotency considerations:** Seeded samples; read-only queries.
- **Security considerations:** Bench queries go through the same tenant-scoped statements as production (organization/snapshot predicates present); no cross-tenant data exists in the bench DB.
- **Observability additions:** Verifies `graph_query_duration_ms` histogram records; none added.
- **Tests required:**
  - `bfs_never_exceeds_budget_on_hub_seeds`
  - `bfs_ordering_deterministic_across_runs`
  - `sql_neighbor_uses_index_scan` (asserts plan contains Index Scan/Index Only Scan, no Seq Scan)
  - `overlay_overhead_within_25pct`
- **Benchmarks if applicable:** This task.
- **Acceptance criteria (ADR-014, PRD §120):**
  - SQL direct-neighbor p95 < 20 ms warm at 1M symbols (cold-cache figure reported).
  - In-memory `neighbors` p95 < 100 microseconds; `bounded_bfs` depth 3 / max_nodes 500 p95 < 5 ms on the reference VM, including hub seeds.
  - `shortest_path` depth <= 6 p95 < 20 ms.
  - Zero budget violations over 10k seeds.
  - Breach of the SQL 20 ms threshold files the ADR-014 revisit note.
- **Definition of done:** Global DoD, plus reports committed and the OBS graph-query latency alert thresholds confirmed against measured p95.

---

---

### PERF-005 — Incremental update benchmark
Status: ☐

- **Task ID:** PERF-005
- **Title:** Incremental graph update latency versus changed-file count on a 100k-file repository
- **Problem:** PRD §119 requires that fewer than 10 changed files update the graph "in seconds" and that work is not proportional to repository size. Only a benchmark on a large base graph can prove no hidden O(N) step (e.g. a full name-index rebuild or full edge scan) remains.
- **Why it exists:** PRD §119, master plan §17 (PERF-005 within §119 targets), ADR-004, target-architecture §3.5.
- **Scope:**
  - A benchmark running `incremental` (base graph + changed paths -> head overlay + delta snapshot) for changed-file counts N in {1, 5, 9, 50, 200, 1000} against the profile `l` base.
  - Three edit classes per N: body-only edit, signature change of a hub symbol (large re-link fan-out), file rename/move.
  - Stage timings: diff, parse, symbol diff, re-link, delta write, invalidation set.
  - A scaling check: latency at N=9 on profile `m` vs `l` (must not grow with repo size beyond noise).
- **Explicit non-scope:** Review pipeline latency (PERF-008); embedding sync (covered by SEM-007 and PERF-007); compaction (GS-007).
- **Files/modules expected to change:** None.
- **New files/modules expected:** `engine/crates/incremental/benches/update.rs`, `benchmarks/perf/incremental/{run.sh, README.md, baseline.json}`.
- **Dependencies:** PERF-001 (`--churn`), PERF-003, INC-001..INC-008 (incremental engine), GS-004, CG-010.
- **Implementation details:**
  - Base: `Graph` loaded from PostgreSQL (PERF-003 snapshot). Head commit produced by `synth-repo --churn N` using the deterministic edit classes; hub-signature edits pick top-Zipf symbols.
  - Records the counters mandated by target-architecture §3.5: `files_reparsed`, `files_skipped_unchanged`, `symbols_{added,removed,modified,renamed}`, `edges_{added,removed}`, `invalidations`, `files_reparsed_for_relink`.
  - Correctness guard in the bench: for N <= 50 the resulting head graph is compared with a full rebuild using the CG-012 graph-compare (must be equal), proving speed is not bought with incorrectness.
  - Reports wall time, peak RSS delta, and delta snapshot row count.
- **Data model changes:** None.
- **API/protocol changes:** None.
- **Concurrency semantics:** Sequential single update (the production unit); an additional run issues 4 concurrent updates on distinct heads sharing one `Arc<Graph>` base to confirm no base copies (RSS delta < 5% of base).
- **Failure behavior:** If `files_reparsed` > N + relink allowance or any unchanged file is reparsed without relink justification, the bench fails (matches the "no reparse of unchanged files" assertion).
- **Idempotency considerations:** Running the same update twice yields an identical delta snapshot content hash.
- **Security considerations:** Synthetic data; benchmark DB isolation (`rg_bench_`).
- **Observability additions:** Asserts the incremental metrics and span `incremental_graph_update` are emitted with the counters above.
- **Tests required:**
  - `update_equals_full_rebuild_for_small_n` (sampled)
  - `unchanged_files_never_reparsed`
  - `latency_independent_of_repo_size_n9` (m vs l ratio < 1.5)
  - `concurrent_updates_share_base_graph`
  - `delta_snapshot_hash_stable`
- **Benchmarks if applicable:** This task.
- **Acceptance criteria (PRD §119):**
  - N < 10 changed files: p95 end-to-end update < 5 s (target "seconds") on the reference VM against the 100k-file base; body-only edits < 2 s.
  - N = 50: < 15 s; N = 1000: < 3 min and < 5x slower per-file than N = 9.
  - Hub-signature edit (worst-case fan-out) with N = 1 stays < 10 s.
  - Update latency at N = 9 does not differ more than 1.5x between 10k-file and 100k-file bases.
- **Definition of done:** Global DoD, plus the report committed and PRD §119 compliance noted in `benchmarks/perf/README.md`.

---

---

### PERF-006 — Context selection benchmark
Status: ☐

- **Task ID:** PERF-006
- **Title:** Context selection latency and budget adherence on large repositories
- **Problem:** Context selection (structural, lexical, semantic candidates, ranking, budgeting, compression) must meet the PR latency targets in PRD §118 even when the graph has 1M symbols and the identifier index is large. A slow stage here eats the < 60 s small-PR budget before any model call.
- **Why it exists:** Master plan §12 ("context selection latency"), PRD §118/§120, Invariant 10 (structural before semantic).
- **Scope:**
  - Criterion benches per stage and end-to-end `build_context` for synthetic PRs touching 1, 10, 50 and 200 changed symbols.
  - Lexical index build time and size on 100k files (CTX-004), incremental update of the index.
  - Compression throughput (bytes of source touched per package).
  - Budget property checks while benchmarking.
- **Explicit non-scope:** Retrieval quality (CTX-010, EVAL); Qdrant network latency (PERF-007, stubbed here with a fixed-latency fake).
- **Files/modules expected to change:** None.
- **New files/modules expected:** `engine/crates/context-engine/benches/context.rs`, `benchmarks/perf/context/{README.md, baseline.json}`.
- **Dependencies:** PERF-001, PERF-003, PERF-005 (head overlay), CTX-001..CTX-009, IMP-001..IMP-010, RISK-005.
- **Implementation details:**
  - Inputs: base `Graph` + head overlay from `--churn` commits; changed symbols from the diff engine; cluster/reviewer budgets from RISK-005 defaults (correctness and security).
  - Semantic candidates use a deterministic fake `Semantic` port returning k=50 hits after a configurable latency (0 ms for CPU measurement, 50 ms to confirm overlap with other stages).
  - Per-stage histograms: structural, tests/config/API, lexical, ranking, budgeting, compression; and total.
  - Hub scenario: changed symbol with fan-in > 10k, verifying structural expansion stops at the impact budget (IMP-007) and reports `omitted` reasons.
  - Memory: peak heap during `build_context` for the 200-symbol case.
- **Data model changes:** None.
- **API/protocol changes:** None.
- **Concurrency semantics:** Clusters are built in parallel (as in the pipeline); the bench runs 1 and 8 clusters concurrently to detect shared-state contention in the lexical index.
- **Failure behavior:** Any package exceeding its token/symbol budget fails the bench (CTX-010 property reused). A missing lexical index falls back with a counted warning; the bench treats the fallback as failure.
- **Idempotency considerations:** The package hash (CTX-009) must be identical across repeated runs of the same inputs; asserted every iteration.
- **Security considerations:** Synthetic data only; no tenant filters bypassed (semantic fake requires `TenantScope`).
- **Observability additions:** Verifies span `context_selection` and the `context_build_duration_ms` histogram record per stage; none added.
- **Tests required:**
  - `context_package_hash_stable_across_runs`
  - `budget_never_exceeded_under_hub_changes`
  - `fallback_without_lexical_index_is_flagged`
  - `parallel_clusters_produce_same_packages_as_serial`
- **Benchmarks if applicable:** This task.
- **Acceptance criteria (PRD §118):**
  - Small PR (<10 changed files, <= 30 symbols): `build_context` p95 < 2 s per cluster on the 100k-file base (leaves the < 60 s PR budget dominated by model calls).
  - Medium (50 files): p95 < 6 s total; large (200 files, risk-budgeted): p95 < 20 s total.
  - Lexical index full build on 100k files < 5 min; incremental update for 9 files < 1 s.
  - Zero budget violations; peak heap for the 200-symbol case < 2 GB above the loaded graph.
- **Definition of done:** Global DoD, baseline committed, nightly CI runs the 10-symbol and 50-symbol cases.

---

---

### PERF-007 — Qdrant retrieval benchmark
Status: ☐

- **Task ID:** PERF-007
- **Title:** Qdrant filtered-search latency and throughput under the production adapter at scale
- **Problem:** SEM-009 validates filtered recall on a synthetic multi-tenant corpus. It does not exercise the production `semantic` adapter path (TenantScope enforcement, query building, retry, batching) under concurrent review load, nor the upsert throughput needed to embed a 1M-symbol repository.
- **Why it exists:** Master plan §12 ("Qdrant filtered search latency"), alert threshold Qdrant p95 > 500 ms, ADR-008, PRD §120.
- **Scope:**
  - Retrieval latency benchmark through `semantic::QdrantSearch` (not raw REST) at 1M points for concurrency 1, 8, 32.
  - Bulk upsert throughput (points/s) and index build time for a 1M-symbol repository, and incremental upsert of 200 changed symbols (SEM-007).
  - Memory/disk footprint of the collection and payload indexes.
- **Explicit non-scope:** Recall under selective filters and ground truth (SEM-009, whose corpus and brute-force harness are reused); embedding model quality (EVAL); real embedding provider latency (the `hash` provider is used).
- **Files/modules expected to change:** `engine/crates/semantic/benches/qdrant_filtered.rs` (shared harness helpers extracted for reuse; no behavior change).
- **New files/modules expected:** `engine/crates/semantic/benches/qdrant_adapter.rs`, `benchmarks/perf/qdrant/{README.md, reports/.gitkeep, baseline.json}`.
- **Dependencies:** PERF-001, SEM-002 (hash provider), SEM-003, SEM-005, SEM-007, SEM-009.
- **Implementation details:**
  - Corpus from the synthetic manifest: symbol summaries, code chunks and docs for the profile `l` repository plus 19 small tenant repos (reuse SEM-009 shape).
  - Query mix replicates the context engine: top-k=20 with `kind` filters, `snapshot_lineage` filter, and mandatory `TenantScope` (organization_id + repository_id).
  - Metrics: p50/p95/p99 latency, QPS at saturation, error rate, retry count, per-request payload bytes; upsert batch sizes 128/512/1024 sweep.
  - Records `hnsw_ef`, collection config and Qdrant version in the report header.
- **Data model changes:** None.
- **API/protocol changes:** None.
- **Concurrency semantics:** Closed-loop clients (1/8/32) for 120 s each after a 30 s warm-up; upload phase uses parallel batch upserts bounded by the adapter's own concurrency limit.
- **Failure behavior:** Refuses to run without `QDRANT_URL` and an empty `rg_bench_` prefix; transient errors are counted, and an error rate above 0.1% fails the run.
- **Idempotency considerations:** Seeded corpus; re-running upload is an upsert no-op (point ids are uuid_v5); the bench asserts the point count is unchanged after a second upload.
- **Security considerations:** Dedicated `rg_bench_` collections, deleted at the end; test asserts a query without `TenantScope` does not compile (compile-fail test already in SEM-005, referenced).
- **Observability additions:** Verifies `qdrant_search_duration_ms` and `embedding_upserts_total` metrics.
- **Tests required:**
  - `bench_collection_prefix_enforced`
  - `second_upload_is_noop`
  - `error_rate_gate_fails_run`
  - `latency_percentile_math` (unit)
- **Benchmarks if applicable:** This task.
- **Acceptance criteria (SEM-009 targets, alert threshold):**
  - Filtered top-20 search p95 < 100 ms at 1M points and concurrency 8 on the reference VM; p99 < 250 ms; never above the 500 ms alert threshold at concurrency 32.
  - Full 1M-point upload < 30 min; 200-symbol incremental upsert < 3 s.
  - Error rate < 0.1%.
  - Breach opens an ADR-008 follow-up with measurements.
- **Definition of done:** Global DoD, first report committed under `benchmarks/perf/qdrant/reports/`, OBS Qdrant alert threshold confirmed against measured p95.

---

---

### PERF-008 — End-to-end review latency (replay) and k6 webhook load test
Status: ☐

- **Task ID:** PERF-008
- **Title:** End-to-end review latency benchmark under replay plus k6 load test of the webhook endpoint (50 PR events/min)
- **Problem:** PRD §118 sets review-completion targets (small < 60 s, medium < 2 min, large < 5 min) for indexed repositories, and production readiness (master plan §17) requires a load test of the webhook endpoint at 50 PR events/min. Neither has a measurement, and live model latency would make runs non-reproducible.
- **Why it exists:** PRD §118, master plan §12 and §17 ("PERF-008 + `benchmarks/perf/load`").
- **Scope:**
  - Replay-mode E2E latency harness: webhook (signed) -> fake GitHub -> checkout -> incremental index -> review (replay gateway with a latency model) -> verify -> publish, for small/medium/large synthetic PRs against an indexed profile `l` repository.
  - A latency model for the replay adapter (configurable per-call delay drawn from recorded distributions) so model time is realistic and explicit.
  - A k6 script posting signed `pull_request` events at 50/min sustained, with bursts to 150/min, plus 10% duplicate deliveries and 10% superseding pushes.
  - Per-stage timing breakdown from OpenTelemetry spans.
- **Explicit non-scope:** Live-provider latency or cost (QB-005); model quality (EVAL); GitHub real-API rate limits.
- **Files/modules expected to change:** `infra/compose/docker-compose.test.yml` (load-test profile with resource limits).
- **New files/modules expected:** `benchmarks/perf/e2e/{run.sh, latency_model.yaml, README.md, baseline.json}`, `benchmarks/perf/load/{webhook.k6.js, README.md, thresholds.json}`, `tests/e2e/perf_harness.ts`.
- **Dependencies:** PERF-001, PERF-003, PERF-005, PERF-006, GH-002, GH-003, GH-009, GW-005 (replay), PIPE-001, SUP-004, E2E-001, DEV-005 (fake GitHub).
- **Implementation details:**
  - PR sizes: small (5 files), medium (30), large (150 files, risk-budgeted); 30 PRs per size, seeded; time measured from webhook receipt to check-run completed, split per state of the review-run machine.
  - Latency model: REVIEW_REASONER ~ lognormal(median 8 s, p95 20 s), VERIFIER ~ (median 4 s), concurrency per gateway limits; the model is documented as an assumption and versioned.
  - k6: `ramping-arrival-rate` executor, HMAC computed in-script, unique `X-GitHub-Delivery`; thresholds in `thresholds.json`.
  - Report: p50/p95 per size, state breakdown, queue wait, worker utilisation, publish success, duplicate suppression counts.
- **Data model changes:** None.
- **API/protocol changes:** None.
- **Concurrency semantics:** Load phase runs 4 workers and 2 API replicas under compose limits; duplicate deliveries must create one `review_run`; superseded runs must never publish (SUP invariants asserted).
- **Failure behavior:** Any lost event (delivery acked but no run or terminal state within 10 min), double publication, or dead job fails the run; results written even on failure.
- **Idempotency considerations:** Seeded PRs; replay fixtures keyed by request hash; the load run is repeatable against a reset database.
- **Security considerations:** Test webhook secret and fake GitHub only; no real tokens; k6 script reads the secret from env and never prints it.
- **Observability additions:** None; consumes existing spans (`webhook_received` .. `publication`) and queue-depth metrics; fails if required spans are missing.
- **Tests required:**
  - `e2e_perf_small_pr_under_budget_replay`
  - `load_duplicate_deliveries_single_run`
  - `load_superseded_runs_do_not_publish`
  - `latency_report_state_breakdown_sums_to_total`
  - `k6_thresholds_file_valid`
- **Benchmarks if applicable:** This task.
- **Acceptance criteria (PRD §118, master plan §17):**
  - Replay E2E on an indexed repo: small PR p95 < 60 s, medium p95 < 120 s, large p95 < 300 s.
  - Non-model overhead (everything except `model_request` spans) for a small PR p95 < 15 s.
  - k6 at 50 events/min for 30 min: webhook ack p95 < 500 ms, 0 errors above 0.1%, no job in `dead`, queue wait p95 < 10 s, zero double publications.
  - Burst to 150/min drains within 10 min without lost events.
- **Definition of done:** Global DoD, reports committed, nightly CI runs the small-PR case and a 5-minute k6 smoke; runbook for the queue-depth alert references measured capacity.

---

---

### QB-001 — Quality corpus expansion
Status: ☐

- **Task ID:** QB-001
- **Title:** Expand the labelled quality corpus to at least 50 PRs, including real reference-repository PRs
- **Problem:** The initial corpus (EVAL-006) has 22-23 small synthetic cases. That is enough for the M5 gate but too small for statistically meaningful precision and recall (Wilson intervals on ~20 cases are wide), and it contains no real-world code, so overfitting to synthetic shapes is undetected.
- **Why it exists:** PRD §142 (known bugs, safe changes, security regressions, test gaps, architecture violations, performance regressions, false-positive traps), master plan §12/§17 ("Benchmark PR suite: QB-001..003 gate in CI").
- **Scope:**
  - Grow the corpus to >= 50 cases with minimum class quotas: correctness_regression 10, safe_change 10, security 6, architecture 4, fp_trap 10, test_gap 4, concurrency 4, transaction 3, api_contract 4, performance 3 (new `performance_regression` class).
  - Add >= 10 cases derived from real PRs of a mid-size NestJS reference repository (~1k files, ~15k symbols): labelled by two human reviewers, reconciled, stored as `base/` snapshot subsets plus `head.patch`.
  - Add a case-provenance field (`origin: synthetic | real_sanitized`), labeller ids, and a labelling guideline.
  - Replay fixtures for all new cases (synthetic for synthetic cases; recorded live fixtures for real cases via GW-005 record mode).
- **Explicit non-scope:** Metric definitions (EVAL-004), gates (QB-003), calibration (QB-004), ingestion of past PRs for runtime history (HIST-001).
- **Files/modules expected to change:** `benchmarks/quality/schema/case.v1.schema.json` (add `performance_regression` class, `origin`, `labelers`; schema v1 -> v1.1 backward compatible), `benchmarks/quality/smoke.txt`, `benchmarks/quality/README.md`.
- **New files/modules expected:** `benchmarks/quality/cases/<new ids>/**`, `benchmarks/quality/LABELING.md`, `benchmarks/quality/tools/sanitize.rs` (or script) for real-PR sanitisation, `fixtures/model-replay/**` additions, `benchmarks/quality/cases/real-*/provenance.yaml`.
- **Dependencies:** EVAL-001..EVAL-006, GW-005, REV-C-004, VER-008.
- **Implementation details:**
  - Real-PR cases: select merged PRs spanning feature, bugfix, refactor and dependency changes; include >= 4 known regression PRs (later reverted or hot-fixed) and >= 3 safe refactors; subset the base tree to the files within impact depth 2 (keeps cases <= 300 files) and pass the sanitiser (rename org/customer identifiers, strip secrets, normalise names deterministically, hash-stable).
  - Labelling: each case labelled independently by two reviewers; disagreements resolved by a third; inter-labeller agreement (Cohen's kappa) computed and reported, target >= 0.7. Ambiguous findings become `optional_findings`, never `expected`.
  - Held-out split: 20% of cases tagged `split: holdout`, excluded from prompt/threshold tuning (QB-004 uses train only); holdout used only in the nightly gate.
  - Corpus freeze policy: adding cases is allowed; editing labels requires a changelog entry in `benchmarks/quality/CHANGELOG.md` and bumps `corpus_hash`.
- **Data model changes:** None (files).
- **API/protocol changes:** `review eval validate` accepts schema v1.1; `review eval stats` prints class counts and split sizes.
- **Concurrency semantics:** Case builds remain parallel (EVAL-001).
- **Failure behavior:** Validation fails when a class quota is unmet, a real case lacks provenance or sanitisation attestation, or any case lacks required replay fixtures.
- **Idempotency considerations:** Deterministic builds; sanitiser output is stable (same input -> same bytes), asserted by hash in `provenance.yaml`.
- **Security considerations:** Real-PR cases must contain no secrets, personal data or customer-identifying strings: the sanitiser output is scanned by the SEC secret detector and a deny-list test; originals never enter the repository. Cases are reviewed by a human before merge.
- **Observability additions:** None.
- **Tests required:**
  - `corpus_has_at_least_50_cases`
  - `corpus_meets_class_quotas`
  - `real_cases_have_provenance_and_sanitized_attestation`
  - `no_secret_patterns_in_corpus` (secret scanner over all cases, canary allow-list only)
  - `holdout_split_is_20pct_and_stable`
  - `every_case_has_replay_fixtures_for_enabled_reviewers`
  - `sanitizer_is_deterministic`
- **Benchmarks if applicable:** The corpus is the benchmark; a new baseline `replay-default.json` is generated from the first green run.
- **Acceptance criteria:**
  - >= 50 valid cases with >= 10 real_sanitized cases; kappa >= 0.7.
  - Under `replay-default`: `trap_fp_rate == 0`, `safe_change_fp_rate == 0`, and precision Wilson lower bound >= 0.80 (PRD §117 trajectory: FP rate < 10%).
  - Full-corpus replay run completes in < 10 minutes on CI.
- **Definition of done:** Global DoD, plus `LABELING.md` and the class/quota table are in `benchmarks/quality/README.md`.

---

---

### QB-002 — Acceptance metric pipeline from feedback
Status: ☐

- **Task ID:** QB-002
- **Title:** Production quality metrics from developer feedback and finding outcomes (accepted findings / published findings)
- **Problem:** The primary product KPI is actionable accepted findings over published findings, not comments per PR (PRD §116), with a false-positive rate under 10% for medium/high-confidence comments (PRD §117). Feedback is stored (API-012) but nothing computes these KPIs, so there is no production-side quality signal.
- **Why it exists:** PRD §116, §117, §70 (feedback contributes to calibration), master plan §17 (benchmark suite and quality gates).
- **Scope:**
  - Standard, written KPI definitions ("exact definitions must be standardized", PRD §117).
  - A metrics job computing per-repository, per-reviewer, per-category and per-confidence-band metrics from `findings`, `published_findings`, `feedback`, and resolution signals.
  - A `quality_metrics_daily` table and an internal API for QB-004/005/006.
- **Explicit non-scope:** Calibration curves (QB-004), cost metrics (QB-005), dashboard UI (QB-006), history-based learning (HIST-*).
- **Files/modules expected to change:** `apps/api/src/findings/feedback.service.ts` (emit events consumed by the aggregator, no behavior change).
- **New files/modules expected:** `engine/migrations/{seq}_quality_metrics.sql`, `engine/crates/pipeline/src/quality/{mod.rs, kpi.rs, aggregate.rs}`, `docs/operations/quality-kpis.md`, `engine/apps/review-worker/src/jobs/quality_aggregate.rs`.
- **Dependencies:** API-012, GH-009 (published_findings), GH-011 (stale-comment resolution), VER-010, EVAL-004 (shared formulas), OBS-001.
- **Implementation details:**
  - Definitions (documented in `quality-kpis.md`):
    - `accepted` = feedback `useful`, or the inline comment thread resolved by a commit touching the cited range, or the provider "suggestion applied" event.
    - `rejected` = feedback `false_positive` | `not_relevant` | `intentional`; `already_handled` is neutral (excluded from numerator and denominator, reported separately).
    - `acceptance_rate = accepted / (accepted + rejected)` over findings with an outcome; `coverage = findings_with_outcome / published`.
    - `fp_rate_production = false_positive / (published with outcome)` for confidence >= 0.70 (PRD §117 scope).
    - `kpi_primary = accepted / published` (lower bound reported alongside because many findings have no outcome).
  - Aggregation: daily job upserts `quality_metrics_daily(date, organization_id, repository_id, reviewer, category, confidence_band, severity, published, accepted, rejected, neutral, unlabeled)`; Wilson intervals computed at read time.
  - Outcome sources are recorded with provenance so explicit feedback is weighted above inferred resolution.
- **Data model changes:** New table `quality_metrics_daily` (RLS by `organization_id`), index `(repository_id, date)`; no change to `feedback`.
- **API/protocol changes:** `GET /internal/quality/metrics?repository_id&from&to&group_by=` (service-auth only; tenant scope enforced); public repository-level summary reuses API-012 summary.
- **Concurrency semantics:** The aggregation job holds an advisory lock per (organization, day); safe to run concurrently for different tenants; late feedback re-aggregates the last 14 days.
- **Failure behavior:** A failed aggregation leaves prior rows untouched (transactional upsert); zero denominators yield null, never 0 or 1.
- **Idempotency considerations:** Recomputing a day is a full replace of that day's rows; results are a pure function of source tables.
- **Security considerations:** Tenant-scoped queries with RLS; no finding text in the aggregate table (counts only); internal endpoint requires service token.
- **Observability additions:** Gauges `quality_acceptance_rate{repository,reviewer}`, `quality_fp_rate_production`, `quality_outcome_coverage`; span `quality_aggregate`; counter `quality_aggregate_failures_total`.
- **Tests required:**
  - `acceptance_rate_definition_table`
  - `already_handled_is_neutral`
  - `zero_denominator_is_null`
  - `late_feedback_reaggregates_window`
  - `aggregation_idempotent_replace`
  - `tenant_isolation_in_quality_endpoint`
  - `resolved_thread_counts_as_accepted_with_provenance`
- **Benchmarks if applicable:** Aggregation over 1M findings completes < 60 s.
- **Acceptance criteria:**
  - On a seeded dataset (10k findings, 40% with outcomes) metrics equal the hand computation in `tests/data/quality_expected.json` exactly.
  - Dashboard-grade latency: the metrics endpoint answers < 300 ms p95 for a 90-day range.
  - `fp_rate_production` computed for confidence >= 0.70 per PRD §117 and is alertable at > 10%.
- **Definition of done:** Global DoD, plus KPI definitions doc reviewed and linked from the PRD gap analysis.

---

---

### QB-003 — Regression gate in CI
Status: ☐

- **Task ID:** QB-003
- **Title:** CI regression gate over the quality corpus (smoke on PRs, full on nightly, baseline governance)
- **Problem:** PRD §143 says every product change must run against benchmark PRs and that more findings is not an improvement. Without an automated gate, prompt, router, ranking or threshold changes can regress precision unnoticed.
- **Why it exists:** PRD §143, master plan §12 gates (precision -3 pts, FP +3 pts, p95 latency +20% fail the run), ADR-010 (default changes require an evaluation report), master plan §17.
- **Scope:**
  - CI job `quality-smoke` (5 smoke cases, replay) on every PR that touches `engine/`, `fixtures/`, `benchmarks/quality/` or prompts.
  - Nightly job `quality-full` (full corpus incl. holdout, replay) with trend upload.
  - Baseline governance: committed `baselines/replay-default.json`, an update procedure with required reviewer approval and a CODEOWNERS rule.
  - A PR comment bot summarising metric deltas (CI-side only).
- **Explicit non-scope:** Metric definitions (EVAL-004), live-model runs (manual `eval-live` workflow documented only), production KPIs (QB-002).
- **Files/modules expected to change:** `.github/workflows/ci.yml` (add jobs), `CODEOWNERS`, `benchmarks/quality/README.md`.
- **New files/modules expected:** `.github/workflows/quality-nightly.yml`, `benchmarks/quality/baselines/replay-default.json` (from EVAL-006/QB-001), `benchmarks/quality/tools/delta_comment.py`, `benchmarks/quality/BASELINE_POLICY.md`.
- **Dependencies:** EVAL-001..EVAL-006, QB-001, CI-001, CI-004 (artifact upload).
- **Implementation details:**
  - Gate command: `review eval run --suite smoke --gateway replay --out results.jsonl` then `review eval metrics --results results.jsonl --baseline baselines/replay-default.json` (EVAL-004 exit code 2 = regression -> job fails).
  - Thresholds (master plan §12 + EVAL-004): precision drop > 0.03, fp_rate rise > 0.03, `latency_p95_ms` rise > 20%, `structured_output_success` drop > 0.05; plus hard invariants: `trap_fp_rate == 0` and `safe_change_fp_rate == 0` on replay.
  - Corpus integrity check first: `review eval validate` and `corpus_hash` in the report must equal the committed hash unless the PR also changes cases with a changelog entry.
  - Baseline refresh: only via a dedicated PR changing `baselines/*.json` with the evaluation report attached; CI refuses baseline edits mixed with engine code changes.
  - Flake policy: replay is deterministic, so no retries; a failure is a real regression.
  - Smoke must finish in < 5 minutes; nightly in < 30 minutes.
- **Data model changes:** None (nightly results optionally inserted into `model_eval_results` via EVAL-005 when a DB is available).
- **API/protocol changes:** None.
- **Concurrency semantics:** Smoke cases run in parallel (rayon); one nightly run at a time (`concurrency` group).
- **Failure behavior:** Missing baseline, mismatched `corpus_hash` without changelog, or malformed JSONL fails closed. Infrastructure errors (exit 1) are distinguished from regressions (exit 2) in the job summary.
- **Idempotency considerations:** Replay output is deterministic; rerunning the job on the same commit yields identical metrics (asserted by a determinism test).
- **Security considerations:** No provider API keys in these workflows (replay only); the PR-comment bot uses a least-privilege token and never runs on forks' secrets.
- **Observability additions:** Nightly metrics pushed to the quality dashboard source (QB-006) as a JSON artifact; none in product runtime.
- **Tests required:**
  - `gate_fails_on_precision_drop_over_3pts`
  - `gate_fails_on_trap_fp_nonzero`
  - `gate_passes_on_identical_run`
  - `baseline_edit_mixed_with_code_is_rejected` (workflow script test)
  - `corpus_hash_mismatch_requires_changelog`
  - `replay_run_is_deterministic`
- **Benchmarks if applicable:** Runtime budgets above.
- **Acceptance criteria:**
  - A deliberately regressed branch (a prompt change that publishes a planted trap) fails the smoke gate; the committed demo PR proves it.
  - Main-branch smoke stays green; smoke < 5 min and nightly < 30 min.
  - Branch protection requires `quality-smoke`.
- **Definition of done:** Global DoD, plus `BASELINE_POLICY.md` and the CI-005/CI docs list the required checks.

---

---

### QB-004 — Confidence calibration
Status: ☐

- **Task ID:** QB-004
- **Title:** Calibrate computed finding confidence against corpus labels and production feedback
- **Problem:** Confidence is computed from a weighted formula (VER-009, PRD §54) and gated by thresholds 0.55/0.70/0.85 (VER-010), but the weights and thresholds are provisional. If 0.85 does not actually mean ~85% of such findings are accepted, publication gating is miscalibrated, and the FP target (PRD §117: < 10% for medium/high, long-term < 5% for high) cannot be met.
- **Why it exists:** PRD §70 ("feedback should contribute to calibration"), PRD §117, §144 (precision bias), VER-009 weight tuning, ADR-010 (changes need an evaluation report).
- **Scope:**
  - Offline calibration tooling: reliability curves (binned predicted confidence vs observed precision), Expected Calibration Error (ECE), Brier score, per-reviewer and per-category breakdowns.
  - Fitting of the confidence weights and publication thresholds on the training split of the corpus (QB-001) with a monotone calibration map (isotonic regression) kept as an explicit, versioned artifact.
  - Production recalibration check using QB-002 outcomes; produces a proposed (never auto-applied) config diff.
- **Explicit non-scope:** Changing the confidence formula's structure (VER-009); online learning; per-user thresholds; history signals (HIST-*).
- **Files/modules expected to change:** `engine/crates/verification/src/confidence.rs` (load calibration table `calibration_version`; default identity map), `engine/crates/verification/src/thresholds.rs`.
- **New files/modules expected:** `engine/apps/review-cli/src/eval/calibrate.rs`, `engine/crates/verification/calibration/v1.json`, `benchmarks/quality/calibration/{README.md, reports/.gitkeep}`, `docs/operations/confidence-calibration.md`.
- **Dependencies:** QB-001, QB-002, EVAL-004, VER-009, VER-010.
- **Implementation details:**
  - Inputs: eval results JSONL with computed confidence per candidate and match label (TP/FP); production rows from `quality_metrics_daily` plus per-finding outcomes.
  - Procedure: fit on `split: train`, validate on `holdout`; require >= 200 labelled candidates per fit (else report "insufficient data" and keep identity map); isotonic map stored as breakpoints with `fitted_on_corpus_hash` and `fitted_at`.
  - Threshold search: choose `minimum_publish` as the smallest confidence at which holdout precision >= 0.90 (PRD §117) subject to recall loss <= 5 points versus the current thresholds; high-risk/security profile may use a recall-biased alternative (PRD §144) recorded separately.
  - Calibrated confidence is stored beside raw confidence (`confidence_raw`, `confidence`, `calibration_version`) so ranking and publication use the calibrated value and the raw value stays auditable.
  - Report: curves, ECE (target <= 0.05), Brier, threshold table, delta versus current config; committed to `benchmarks/quality/calibration/reports/` as the required evaluation report.
- **Data model changes:** `findings` gains `confidence_raw real`, `calibration_version text` (migration), backfilled with `confidence_raw = confidence`.
- **API/protocol changes:** `review eval calibrate --results <jsonl> [--production <csv>] --out <json>`; finding payload exposes `calibration_version`.
- **Concurrency semantics:** The calibration map is immutable and loaded at start; a version change takes effect for new runs only (reproducibility, PRD §121).
- **Failure behavior:** Insufficient or degenerate data (all one class) falls back to identity and states why; a fitted map that is non-monotone fails validation and is rejected.
- **Idempotency considerations:** Fitting is deterministic (fixed ordering, no randomness); same inputs yield an identical artifact hash.
- **Security considerations:** Production data used only in aggregate (counts and confidences, no finding text); proposals require human review before config change.
- **Observability additions:** Gauge `confidence_calibration_ece{reviewer}` (from the nightly), span attribute `calibration_version`, histogram `finding_confidence_bucket` (raw and calibrated).
- **Tests required:**
  - `isotonic_map_is_monotone`
  - `ece_known_values`
  - `insufficient_data_falls_back_to_identity`
  - `threshold_search_respects_recall_loss_cap`
  - `calibration_artifact_deterministic`
  - `publication_uses_calibrated_confidence_and_stores_raw`
- **Benchmarks if applicable:** Fit on 10k candidates < 5 s.
- **Acceptance criteria:**
  - On holdout after calibration: ECE <= 0.05; precision of findings with calibrated confidence >= 0.85 is >= 0.90 and >= 0.70 band is >= 0.85 (PRD §117 trajectory), with recall loss <= 5 points versus pre-calibration.
  - Report committed; `calibration_version` visible in finding payloads.
- **Definition of done:** Global DoD, plus the operations doc describes the recalibration cadence (quarterly or after >= 500 new outcomes).

---

---

### QB-005 — Cost per useful finding reporting
Status: ☐

- **Task ID:** QB-005
- **Title:** Cost per useful finding: token and cost attribution from model call to accepted finding
- **Problem:** Cost is accounted per model call (GW-008) and quality is tracked separately (QB-002). Nobody can answer "what does one accepted finding cost?" or whether a routing change that is cheaper per review is more expensive per useful finding (PRD §143 tracks tokens and cost beside precision).
- **Why it exists:** PRD §143 (tokens, cost), PRD §116 (optimize accepted findings, not volume), ADR-010 (routing changes need an evaluation report including cost).
- **Scope:**
  - Attribution of `cost_usd_micros` and tokens per review run to reviewer, tier, stage (generation, verification, summary, embeddings) and to outcome class.
  - Metrics: `cost_per_review`, `cost_per_published_finding`, `cost_per_accepted_finding`, `cost_per_kloc_changed`, cache savings.
  - Reports for both the benchmark corpus (replay with recorded costs) and production.
- **Explicit non-scope:** Billing and invoicing; changing the router (ADR-010 process); budget enforcement (GW-007).
- **Files/modules expected to change:** `engine/apps/review-cli/src/eval/metrics.rs` (add cost-efficiency fields), `engine/migrations/{seq}_quality_metrics.sql` (extend with cost columns).
- **New files/modules expected:** `engine/crates/pipeline/src/quality/cost.rs`, `benchmarks/quality/reports/cost-template.md`, `docs/operations/cost-reporting.md`.
- **Dependencies:** GW-008, QB-002, EVAL-004, EVAL-005, REV-001.
- **Implementation details:**
  - Joins: `reviewer_runs` (usage per call) -> candidates -> findings -> `published_findings` -> outcomes (QB-002). Shared costs (summary, classification, embeddings) are allocated across the run's published findings proportionally; shared allocation rules are documented and unit-tested; unallocated residue is reported, never hidden.
  - Cost of suppressed candidates (generation + verification) is reported as `waste_cost`: the price of candidates that were never published, which measures verification economics.
  - Eval side: `review eval metrics` gains `cost_per_true_positive` (corpus) next to `cost_usd_micros` using replay-recorded usage; models without recorded cost use the pricing table from GW-008 (versioned, with `pricing_version` in the report).
  - Daily rollup columns added to `quality_metrics_daily`: `cost_usd_micros`, `input_tokens`, `output_tokens`, `cached_tokens`.
  - Report template compares configurations side by side (tier routing candidates), highlighting precision, recall and cost per accepted finding together, so a cheaper but less precise config is not presented as a win.
- **Data model changes:** Add cost/token columns to `quality_metrics_daily`; no changes to `reviewer_runs`.
- **API/protocol changes:** `GET /internal/quality/cost?repository_id&from&to` (service-auth); `review eval metrics` output extended (backward compatible JSON fields).
- **Concurrency semantics:** Computed by the same daily job as QB-002 under the same advisory lock.
- **Failure behavior:** Missing usage for a call (provider omitted it) is counted in `usage_missing_total` and excluded with a visible flag; costs never default silently to zero.
- **Idempotency considerations:** Recompute is a replace per day; allocation is deterministic.
- **Security considerations:** Aggregates only; tenant-scoped with RLS; no prompts or code in cost tables.
- **Observability additions:** Gauges `cost_per_accepted_finding_usd{repository,reviewer}`, `waste_cost_ratio`; counter `usage_missing_total`.
- **Tests required:**
  - `shared_cost_allocation_sums_to_total`
  - `waste_cost_counts_suppressed_candidates`
  - `cost_per_accepted_zero_denominator_null`
  - `missing_usage_flagged_not_zeroed`
  - `pricing_version_recorded_in_report`
- **Benchmarks if applicable:** None beyond QB-002 aggregation budget.
- **Acceptance criteria:**
  - For a seeded replay corpus run, sum of attributed costs equals total gateway cost to within 0.1% (residue reported).
  - Production report available per repository and per reviewer with 95% confidence intervals where outcomes exist.
  - A routing evaluation report (ADR-010) includes cost per true positive and cost per accepted finding.
- **Definition of done:** Global DoD, plus the cost definitions doc is linked from ADR-010 evaluation instructions.

---

---

### QB-006 — Quality dashboard
Status: ☐

- **Task ID:** QB-006
- **Title:** Quality dashboard: acceptance, false positives, calibration, cost and benchmark trends
- **Problem:** QB-002..QB-005 produce KPIs but nobody can see them. Operators need one place to answer whether review quality is improving, which reviewer or category regresses, and whether the nightly benchmark is drifting.
- **Why it exists:** PRD §116/§117 (KPIs), PRD §143 (track precision, recall, FP, latency, tokens, cost), master plan §14 observability, WEB usage and settings screens.
- **Scope:**
  - A web "Quality" screen per repository and per organization, reading the internal quality API through the NestJS BFF.
  - An OpenObserve dashboard JSON for operators (nightly benchmark trends, production gauges) under `infra/openobserve/`.
  - Alerts: production FP rate, acceptance-rate drop, nightly regression, calibration drift.
- **Explicit non-scope:** Computing metrics (QB-002/004/005); editing thresholds from the UI; per-developer scoring (explicitly excluded; see Security).
- **Files/modules expected to change:** `apps/web/src/app/(app)/repositories/[id]/layout.tsx` (nav entry), `apps/api/src/graph/` is untouched; `apps/api/src/findings/` (BFF endpoint).
- **New files/modules expected:** `apps/api/src/quality/{quality.controller.ts, quality.service.ts}`, `apps/web/src/app/(app)/repositories/[id]/quality/page.tsx`, `apps/web/src/components/quality/{KpiTiles.tsx, ReliabilityChart.tsx, TrendChart.tsx, CostTable.tsx}`, `infra/openobserve/dashboards/quality.json`, `infra/openobserve/alerts/quality-*.json`, `docs/operations/runbooks/quality-*.md`.
- **Dependencies:** QB-002, QB-003 (nightly artifact), QB-004, QB-005, WEB-001, API-003, OBS-007 (alert definitions).
- **Implementation details:**
  - Screen sections: KPI tiles (accepted/published with Wilson interval, FP rate for confidence >= 0.70 with target line at 10% per PRD §117, outcome coverage); trend charts (30/90 days); per-reviewer and per-category tables; reliability diagram from calibration reports; cost per accepted finding; latest nightly benchmark metrics with link to the report.
  - Every number shows its denominator and "n" so low-sample values are visibly flagged (< 30 outcomes shows "insufficient data").
  - Aggregation windows and group-by controls map directly to QB-002 API parameters; no client-side recomputation of rates.
  - OpenObserve alerts: `fp_rate_production > 0.10` over 7 days with n >= 30; `acceptance_rate` drop > 10 points week over week; nightly gate failure; ECE > 0.08. Each alert has a runbook.
  - Accessibility and empty states follow the web design system; charts use accessible palettes.
- **Data model changes:** None.
- **API/protocol changes:** `GET /repositories/:id/quality?from&to&group_by` and `GET /organizations/:id/quality/summary` on the NestJS API, proxying the internal engine/aggregate API with tenant scope.
- **Concurrency semantics:** Read-only; responses cached for 5 minutes per (repository, range) in Redis keyed with the organization id.
- **Failure behavior:** If the aggregate is stale or missing, the screen shows "no data yet / last computed at" instead of zeros; API errors render a retryable error state.
- **Idempotency considerations:** Pure reads.
- **Security considerations:** Tenant isolation through API-003 guards and RLS; aggregates are per repository/reviewer/category only. No per-developer metrics or rankings are exposed, to avoid punitive use of feedback; finding text is not shown here.
- **Observability additions:** Web page-load and API latency spans; dashboard and alert JSON checked in and validated in CI (schema lint).
- **Tests required:**
  - `quality_api_enforces_tenant_scope`
  - `quality_api_returns_null_not_zero_for_empty`
  - `quality_page_flags_low_sample`
  - `quality_page_renders_empty_state` (Playwright)
  - `alert_definitions_valid_json_schema`
  - `quality_cache_key_includes_org`
- **Benchmarks if applicable:** Page data load < 1 s p95 for a 90-day range.
- **Acceptance criteria:**
  - A seeded environment shows KPI tiles equal to QB-002 expected values.
  - The FP-rate alert fires in a test using injected metrics above 10% with n >= 30 and does not fire below n = 30.
  - Dashboard JSON imports into OpenObserve without errors.
- **Definition of done:** Global DoD, plus runbooks linked to each alert and a screenshot test baseline stored.

---

---

### CI-001 — Rust fmt, clippy and test (Linux, GitHub Actions)
Status: ☐

- **Task ID:** CI-001
- **Title:** `rust.yml` workflow: rustfmt check, clippy with warnings denied, unit tests and doc tests on Linux
- **Problem:** No CI exists. Local Rust work goes through a Linux container because the Windows host cannot compile the C dependencies, so CI must reproduce that Linux environment and block merges on format, lint and test failures.
- **Why it exists:** Master plan §258 (CI-001..003 start in Phase 1), production readiness row "CI/CD", and the global definition of done (fmt, clippy `-D warnings`, tests).
- **Scope:**
  - `.github/workflows/rust.yml` triggered on pull requests and pushes to the default branch, filtered to `engine/**`, `packages/contracts/**`, the workflow itself and `rust-toolchain.toml`.
  - Jobs: `fmt`, `clippy`, `test` (workspace unit and doc tests), `msrv-parity` (toolchain equals the dev image).
  - Caching, concurrency control, minimal permissions, and JUnit-style test summaries.
- **Explicit non-scope:**
  - `cargo deny`/audit (CI-002); integration tests needing services (CI-004); benchmarks (CI-007); Windows build (CI-009); Docker image builds (CI-008).
- **Files/modules expected to change:** `engine/rust-toolchain.toml` only if the pinned channel changes (currently 1.97); root `README.md` badge section.
- **New files/modules expected:**
  - `.github/workflows/rust.yml`
  - `scripts/ci/check-toolchain-parity.sh`
  - `docs/operations/ci.md` (CI overview created here, extended by later CI tasks)
- **Dependencies (task IDs):** FND-001, FND-002, FND-006.
- **Implementation details:**
  - Runner `ubuntu-24.04`. Toolchain from `engine/rust-toolchain.toml` using `dtolnay/rust-toolchain` with the channel read from the file; components `rustfmt, clippy`. Install `git` and `postgresql-client` via apt to match `engine/docker/dev.Dockerfile` (tree-sitter needs a C compiler already present on the runner).
  - Commands run exactly as local ones without Docker: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked` and `cargo test --workspace --doc --locked`, with `working-directory: engine`. `--locked` guarantees `Cargo.lock` is current.
  - `Swatinem/rust-cache` keyed by `engine/Cargo.lock` and the job name; `CARGO_TERM_COLOR=always`, `RUSTFLAGS=-D warnings` only on clippy job, `CARGO_INCREMENTAL=0`.
  - `msrv-parity`: script compares `rustc --version` in CI against `rustup show` pin and the `toolchain install 1.97` line in the dev Dockerfile; mismatch fails, so local and CI never drift.
  - Test run uses `cargo nextest` if adopted later; initial version uses `cargo test` with `-- --test-threads` default. Snapshot tests run with `INSTA_UPDATE=no` so unreviewed snapshot changes fail.
  - `concurrency: group: rust-${{ github.ref }}`, `cancel-in-progress: true` for PRs.
  - `permissions: contents: read` at workflow level; third-party actions pinned by commit SHA.
- **Data model changes:** None.
- **API/protocol changes:** None. Required status check names: `rust / fmt`, `rust / clippy`, `rust / test`.
- **Concurrency semantics:** Jobs run in parallel; newer pushes cancel older runs on the same ref; main-branch runs are never cancelled.
- **Failure behavior:** Any non-zero step fails the job. Cache restore failures fall back to a cold build. Flaky tests are not retried automatically (policy: fix or quarantine with an issue).
- **Idempotency considerations:** Workflow reruns produce identical results for the same commit because of `--locked` and pinned toolchain.
- **Security considerations:** Read-only token, no secrets exposed to pull requests from forks, actions pinned by SHA, no `pull_request_target`.
- **Observability additions:** Job summary lists test counts and slowest 10 tests; cold and warm build times are written to the summary for tracking.
- **Tests required (named):**
  - `ci_toolchain_matches_dev_image` (the parity script, run in CI and locally)
  - `workflow_yaml_lints_clean` (actionlint in a pre-step)
  - `workflow_paths_filter_covers_engine_and_contracts`
- **Benchmarks if applicable:** Record warm-cache workflow duration; target under 12 minutes.
- **Acceptance criteria (verifiable):**
  - A PR touching `engine/` shows the three required checks green; introducing a format error, a clippy warning and a failing test each fail the corresponding job (demonstrated on a scratch branch).
  - `actionlint` passes locally.
- **Definition of done:** Acceptance criteria pass; checks marked required in branch protection; `docs/operations/ci.md` explains how to reproduce locally via `pnpm test:rust` and `pnpm lint`.

---

### CI-002 — cargo deny and cargo audit
Status: ☐

- **Task ID:** CI-002
- **Title:** Supply-chain gate for Rust (advisories, licenses, bans, sources) and Node (`pnpm audit`), with an expiring exceptions mechanism
- **Problem:** `engine/deny.toml` exists, but nothing runs it automatically, so a vulnerable or disallowed-license dependency or a forbidden crate (for example `neo4rs`) could merge unnoticed.
- **Why it exists:** Master plan §13.7 and production readiness "Dependency scanning: CI-002"; target-architecture §2.1 (dependency direction enforced by `cargo deny` bans).
- **Scope:**
  - `.github/workflows/supply-chain.yml` running `cargo deny check` (advisories, bans, licenses, sources) and `cargo audit` as a second advisory source, plus `pnpm audit --prod --audit-level=high`.
  - A scheduled nightly run so new advisories fail on `main` without a code change.
  - Integration with the exceptions policy from SEC-010.
- **Explicit non-scope:**
  - SBOM and image scanning (SEC-010/CI-008); dependency updates (a Dependabot or Renovate config is a separate chore).
  - Choosing license policy (already in `deny.toml`).
- **Files/modules expected to change:** `engine/deny.toml` (add `[advisories] ignore` entries only through the exceptions file process), root `package.json` (`audit` script).
- **New files/modules expected:**
  - `.github/workflows/supply-chain.yml`
  - `scripts/ci/audit-node.mjs`, `scripts/ci/audit-node.test.mjs`
  - `scripts/ci/deny-exceptions-sync.mjs` (checks `deny.toml` ignores against `security/exceptions.toml`)
- **Dependencies (task IDs):** FND-004, FND-006, CI-001, SEC-010 (exceptions file; can start with an empty one).
- **Implementation details:**
  - `cargo deny` via `EmbarkStudios/cargo-deny-action` pinned by SHA, `command: check`, manifest `engine/Cargo.toml`, version matching the dev image (`^0.18`). All four checks run; `bans.multiple-versions = warn` shows as annotations but does not fail; `wildcards = deny` does.
  - `cargo audit` (`rustsec/audit-check` or `cargo install cargo-audit --locked` cached) against `engine/Cargo.lock`; `--deny warnings` for unmaintained and yanked once the tree is clean, otherwise warnings reported only. The RustSec database fetch is cached per day.
  - Node audit: `pnpm audit --prod --audit-level=high --json`, processed by `audit-node.mjs` to drop advisories listed and unexpired in `security/exceptions.toml` and to print a compact table; non-zero exit on remaining high/critical findings.
  - Exceptions sync: every `ignore` entry in `deny.toml` must have a matching unexpired record (id, reason, owner, expiry at most 90 days) in `security/exceptions.toml`; orphan or expired entries fail the job.
  - Triggers: pull requests touching `engine/Cargo.*`, `pnpm-lock.yaml`, `**/package.json`, `deny.toml`; `schedule: cron "17 3 * * *"` on the default branch; `workflow_dispatch`.
  - Scheduled failures open or update a single GitHub issue labeled `supply-chain` (using `gh issue` with the built-in token) instead of only failing silently.
- **Data model changes:** None.
- **API/protocol changes:** Required check names `supply-chain / deny`, `supply-chain / audit-rust`, `supply-chain / audit-node`.
- **Concurrency semantics:** Independent jobs in parallel; scheduled and PR runs do not share cancel groups.
- **Failure behavior:** A new advisory on an existing dependency fails nightly (not PRs unrelated to it, since PR runs only trigger on dependency-file changes). Network failure to the advisory DB retries 3 times, then fails with a clear message rather than passing silently.
- **Idempotency considerations:** The issue creation step searches for an open issue with the label first, so repeated failures update one issue.
- **Security considerations:** Token permissions `contents: read`, `issues: write` only on the scheduled job; no secrets involved; actions pinned by SHA.
- **Observability additions:** Job summary with counts by severity; scheduled-run result visible on the README badge.
- **Tests required (named):**
  - `deny_check_passes_on_clean_tree`
  - `forbidden_crate_neo4rs_is_rejected` (fixture manifest in a temp workspace)
  - `deny_ignore_without_exception_record_fails` (sync script)
  - `expired_exception_fails_audit_node`
  - `audit_node_filters_only_listed_unexpired_advisories`
- **Benchmarks if applicable:** None.
- **Acceptance criteria (verifiable):**
  - `engine/scripts/cargo.sh deny check` exits 0 locally and the workflow is green on a clean PR.
  - Adding a banned crate on a scratch branch turns `supply-chain / deny` red.
- **Definition of done:** Acceptance criteria pass; checks required; `docs/operations/ci.md` explains the exception process.

---

### CI-003 — TypeScript lint, typecheck and test
Status: ☐

- **Task ID:** CI-003
- **Title:** `ts.yml` workflow: pnpm install with frozen lockfile, ESLint, `tsc --noEmit`, Prettier check, unit tests for `apps/api`, `apps/web` and `packages/*`
- **Problem:** The TypeScript workspace has lint, typecheck and test scripts but nothing enforces them on pull requests.
- **Why it exists:** Master plan §258 (CI-001..003 start in Phase 1), global definition of done; the contracts package is generated code consumed by both apps.
- **Scope:**
  - `.github/workflows/ts.yml` with jobs `lint`, `typecheck`, `unit`, `format`.
  - Node 24 and pnpm 10.15 per `package.json` `packageManager`/`engines`, store caching, affected-package optimization via `pnpm --filter ...[origin/main]` on pull requests (full run on `main`).
  - Unit tests only; tests needing Postgres or Redis are CI-004.
- **Explicit non-scope:**
  - Integration tests (CI-004); contract drift (CI-006); Docker build of apps (CI-008); web end-to-end tests with a browser (separate WEB task).
- **Files/modules expected to change:** Root `package.json` (ensure `lint`, `typecheck`, `test`, `format:check` exist; they do), `packages/config` (shared ESLint config adjustments only if CI exposes gaps).
- **New files/modules expected:**
  - `.github/workflows/ts.yml`
  - `scripts/ci/affected-filter.mjs`, `scripts/ci/affected-filter.test.mjs`
- **Dependencies (task IDs):** FND-003, FND-006, FND-007.
- **Implementation details:**
  - Setup: `actions/checkout`, `pnpm/action-setup` (version from `packageManager`), `actions/setup-node` with `node-version-file` and `cache: pnpm`. Install with `pnpm install --frozen-lockfile --ignore-scripts` then run required postinstall steps explicitly (a documented allow-list), reducing supply-chain exposure.
  - Commands: `pnpm -r --if-present lint`, `pnpm -r --if-present typecheck`, `pnpm -r --if-present test -- --reporter=junit`, `pnpm format:check`. Lint uses `--max-warnings 0`.
  - Typecheck is strict (`tsc --noEmit -p tsconfig.json` per package via project references). Test jobs set `CI=true` and `TZ=UTC` for determinism.
  - `affected-filter.mjs` maps the changed files (from `git diff --name-only origin/main...HEAD`) to workspace packages plus dependents, outputting a filter string; changes to root config, lockfile or `packages/config` select everything.
  - JUnit results and coverage summaries are uploaded as artifacts; coverage thresholds are advisory initially and become enforced once baseline is recorded (tracked in a follow-up note in the workflow comment).
  - `concurrency` cancels superseded PR runs; `permissions: contents: read`; actions pinned by SHA.
  - A matrix of `ubuntu-24.04` only; Windows behavior of scripts is covered by `scripts/rg.test.mjs` which runs in the `unit` job.
- **Data model changes:** None.
- **API/protocol changes:** Required check names `ts / lint`, `ts / typecheck`, `ts / unit`, `ts / format`.
- **Concurrency semantics:** Jobs run in parallel and share the pnpm store cache; cache key is the lockfile hash.
- **Failure behavior:** Any failing step fails the job; a lockfile out of sync fails at install (`--frozen-lockfile`) with the standard pnpm message; cache miss only slows the run.
- **Idempotency considerations:** Rerunning a workflow on the same commit yields the same result; tests must not depend on wall-clock beyond fake timers.
- **Security considerations:** `--ignore-scripts` on install; no secrets exposed; dependency lifecycle scripts are allow-listed explicitly.
- **Observability additions:** Job summary with slowest tests and per-package durations.
- **Tests required (named):**
  - `affected_filter_selects_dependents_of_contracts`
  - `affected_filter_selects_all_on_lockfile_change`
  - `affected_filter_empty_for_docs_only_change`
  - `lockfile_frozen_install_fails_when_out_of_sync` (verified once manually and recorded)
- **Benchmarks if applicable:** Warm-cache workflow under 6 minutes.
- **Acceptance criteria (verifiable):**
  - A clean PR is green on all four checks; each of: a lint error, a type error, a failing unit test and an unformatted file fails exactly the corresponding job.
  - `node --test scripts/ci/affected-filter.test.mjs` passes.
- **Definition of done:** Acceptance criteria pass; checks required in branch protection; `docs/operations/ci.md` has a TypeScript section.

---

### CI-004 — Integration tests with services
Status: ☐

- **Task ID:** CI-004
- **Title:** `integration.yml`: Rust and TypeScript integration suites against PostgreSQL, Redis, Qdrant and the object store, including the tenant-isolation suites
- **Problem:** Many invariants (jobs queue, RLS, Qdrant filters, graph storage conformance) only hold against real services. Unit CI does not cover them.
- **Why it exists:** Master plan §11 (integration level uses `docker-compose.test.yml`), production readiness rows (tenant isolation, migrations, retries and dead-letter), SEC-001/002 must run in CI.
- **Scope:**
  - `.github/workflows/integration.yml` bringing up `infra/compose/docker-compose.test.yml` (postgres on 35432, redis 36379, qdrant 36333) and running `cargo test --workspace --features integration` plus `pnpm --filter api test:integration` and `test:security`.
  - Object-store service added to the test compose for SEC-009 and SEC-007 tests (SeaweedFS, because MinIO images are not pullable).
  - Migrations applied before the suites; service logs collected on failure.
- **Explicit non-scope:**
  - Full end-to-end tests with the real engine, API and web containers (E2E-001 uses DEV-001 compose); benchmarks (CI-007).
- **Files/modules expected to change:** `infra/compose/docker-compose.test.yml` (add `objectstore` with tmpfs and port 39000; no other service changes), `infra/compose/check-ports.sh` is unchanged and must still pass.
- **New files/modules expected:**
  - `.github/workflows/integration.yml`
  - `scripts/ci/wait-for-stack.sh`, `scripts/ci/collect-logs.sh`
  - `infra/compose/objectstore/s3.test.json`
- **Dependencies (task IDs):** FND-005, FND-006, DOM-009, API-003, SEC-001, SEC-002, CI-001, CI-003.
- **Implementation details:**
  - Steps: checkout; setup Rust and Node as in CI-001/CI-003; `docker compose -f infra/compose/docker-compose.test.yml up -d --wait`; run `sqlx migrate run` via the engine test harness (`DATABASE_URL=postgres://reviewgraph:reviewgraph-test@127.0.0.1:35432/reviewgraph`); run suites; always `down -v` in an `if: always()` step.
  - Runner uses native `cargo` against `127.0.0.1` ports (no dev container), so `REVIEWGRAPH_DOCKER_NETWORK` is not needed; env: `DATABASE_URL`, `REDIS_URL=redis://127.0.0.1:36379`, `QDRANT_URL=http://127.0.0.1:36333`, `S3_ENDPOINT=http://127.0.0.1:39000`.
  - Parallel jobs: `rust-integration`, `api-integration`, `security-isolation`; each starts its own compose project name (`COMPOSE_PROJECT_NAME=rg-it-${{ github.job }}-${{ github.run_id }}`) with distinct host ports via `RG_TEST_*_PORT` overrides so they do not collide on a runner.
  - Tests are gated by the `integration` Cargo feature (FND-006); a guard test fails if a test file touching `DATABASE_URL` lacks the gate.
  - Quarantine: tests tagged `#[ignore = "quarantine:ISSUE-n"]` are listed in the summary; the count must not grow without a linked issue (script check).
  - Timeouts: job 25 minutes; stack wait 3 minutes; on failure upload `docker compose logs --no-color` and Postgres slow query log as artifacts.
- **Data model changes:** None; but the workflow proves migrations apply from empty.
- **API/protocol changes:** Required checks `integration / rust`, `integration / api`, `integration / security`.
- **Concurrency semantics:** Distinct project names and ports allow parallel jobs on one runner; cross-run concurrency cancels older PR runs only.
- **Failure behavior:** Compose unhealthy within the wait window fails with the failing service name and logs. Teardown always runs. Retry of the entire job is manual; no automatic test retries.
- **Idempotency considerations:** Each run uses tmpfs storage, so there is no state carried over; tests create their own organizations and schemas.
- **Security considerations:** Test credentials are the published dev defaults and the ports bind to loopback only; no repository secrets are injected; model providers are never contacted (replay adapter, hash embeddings).
- **Observability additions:** Summary with per-suite durations and the quarantined test list.
- **Tests required (named):**
  - `compose_test_stack_includes_objectstore_and_stays_loopback_only` (runs `check-ports.sh`)
  - `integration_feature_gate_guard`
  - `quarantine_count_does_not_increase`
  - `wait_for_stack_times_out_with_service_name` (script test with a stub)
- **Benchmarks if applicable:** Record stack-up time (target under 90 s) and suite durations.
- **Acceptance criteria (verifiable):**
  - The workflow is green on `main`; disabling RLS on one table in a scratch branch makes `integration / security` fail.
  - `docker volume ls` on a self-hosted rerun shows no leftover `reviewgraph-test` volumes.
- **Definition of done:** Acceptance criteria pass; required checks configured; `docs/operations/ci.md` documents local reproduction with `pnpm test:integration`.

---

### CI-005 — Migration validation and Kysely type drift
Status: ☐

- **Task ID:** CI-005
- **Title:** Validate sqlx migrations (apply from empty, checksum immutability, expand/contract safety) and fail when generated Kysely types drift from the migrated schema
- **Problem:** `engine/migrations` is the only schema source, yet nothing verifies that migrations apply cleanly from empty, that merged migrations are not edited, or that `apps/api` Kysely types match the real schema (API-002).
- **Why it exists:** Production readiness row "PostgreSQL migrations: CI-005"; master plan §15 (migrations backward compatible for one release, expand/contract); API-002 types are generated from the migrated database.
- **Scope:**
  - A `migrations.yml` workflow (or job in `integration.yml`) that: applies all migrations to an empty Postgres 16, re-runs to prove idempotence, applies the previous release's migrations then the new ones (upgrade path), regenerates Kysely types and diffs them against the committed file.
  - Static lints over new migration files: immutability of already-merged files, naming and ordering, destructive-change detection, `CREATE INDEX` without `CONCURRENTLY` on large tables flagged, RLS present on tenant tables.
- **Explicit non-scope:**
  - Writing migrations (DOM-009 and later); the production migration runner; data backfills.
- **Files/modules expected to change:** `apps/api/package.json` (script `db:types`), `scripts/` task runner entries.
- **New files/modules expected:**
  - `.github/workflows/migrations.yml`
  - `scripts/ci/migration-lint.mjs`, `scripts/ci/migration-lint.test.mjs`
  - `scripts/ci/check-kysely-drift.sh`
  - `engine/migrations/.checksums.json` (generated list of merged migration checksums; updated only by a release-tagged script)
  - `docs/operations/migrations.md`
- **Dependencies (task IDs):** DOM-009, API-002, FND-005, FND-006, CI-004.
- **Implementation details:**
  - Apply check: fresh `postgres:16` service; `sqlx migrate run --source engine/migrations`; then `sqlx migrate info` must show all applied; second `run` is a no-op.
  - Upgrade path: check out the base branch's migrations into a temp dir, apply, then apply the PR's; a failing upgrade (for example a column renamed in a single step) fails the job. Data seeding between steps uses `fixtures/sql/minimal-seed.sql` so non-empty tables are exercised.
  - Immutability: `migration-lint.mjs` compares sha256 of every migration present on the base branch against the PR; any modified or deleted file fails (new files only). Out-of-order timestamps fail.
  - Destructive lint: `DROP TABLE|COLUMN`, `ALTER COLUMN ... TYPE`, `RENAME`, `SET NOT NULL` without default are flagged unless the file contains `-- rg:contract-phase issue#N` comment, enforcing expand/contract; `CREATE INDEX` on tables above a size hint list requires `CONCURRENTLY` (sqlx supports it with `-- no-transaction`).
  - Tenant check: every created table with an `organization_id` column must also enable and force RLS in the same or a later migration (parsed from SQL); exceptions in an allow-list file with reasons (`jobs`, `webhook_deliveries` per API-003).
  - Kysely drift: run `pnpm --filter api db:types` (kysely-codegen against the migrated database on port 35432) and `git diff --exit-code apps/api/src/db/types.generated.ts`; the diff is printed on failure with the fix command.
  - Rust side: `cargo sqlx prepare --check` for crates using `query!` macros so offline metadata is not stale.
- **Data model changes:** None.
- **API/protocol changes:** Required check `migrations / validate`.
- **Concurrency semantics:** One Postgres service per job; lint is a pure function over files.
- **Failure behavior:** Any failure blocks merge and prints which migration and rule failed. If the base branch is unavailable (first commit) the upgrade-path step is skipped with a notice.
- **Idempotency considerations:** The re-run step asserts idempotence explicitly; the generated types file is byte-stable (sorted output).
- **Security considerations:** Tenant RLS lint prevents shipping a tenant table without isolation (links to SEC-001); no credentials beyond the throwaway test database.
- **Observability additions:** Summary lists migrations applied and apply time per migration (flags any above 5 s on the seed dataset).
- **Tests required (named):**
  - `lint_rejects_modified_merged_migration`
  - `lint_flags_drop_column_without_contract_marker`
  - `lint_allows_contract_marker_with_issue`
  - `lint_requires_rls_for_org_tables_except_allow_list`
  - `lint_rejects_non_concurrent_index_on_listed_tables`
  - `kysely_drift_detected_when_column_added_without_regenerate`
  - `upgrade_path_applies_with_seed_data`
- **Benchmarks if applicable:** Total apply time of all migrations on an empty database recorded; target under 20 s.
- **Acceptance criteria (verifiable):**
  - Workflow green on a clean tree; editing an applied migration, or adding a column without regenerating types, fails the workflow.
  - `node --test scripts/ci/migration-lint.test.mjs` passes.
- **Definition of done:** Acceptance criteria pass; `docs/operations/migrations.md` documents expand/contract rules and the contract-phase marker.

---

### CI-006 — Contract drift check
Status: ☐

- **Task ID:** CI-006
- **Title:** CI gate that regenerates JSON Schemas and TypeScript types from the Rust contract types and fails on any diff, plus API-contract compatibility checks
- **Problem:** `packages/contracts` is generated from Rust (schemars) and consumed by NestJS and Next.js. If someone changes a Rust type without regenerating, or hand-edits generated files, TypeScript and Rust silently disagree on payload shapes (job payloads, internal API, config schema).
- **Why it exists:** Target-architecture §1 ("Shared shapes are defined once in `packages/contracts`"); FND-007 builds the pipeline and a local drift check; this task enforces it in CI and adds a breaking-change guard.
- **Scope:**
  - A `contracts.yml` workflow running the FND-007 generation and `git diff --exit-code packages/contracts`.
  - A breaking-change detector comparing the generated schemas against the base branch (removed fields, narrowed types, new required fields) with an explicit override label.
  - Validation that example payloads in `fixtures/` still validate against the new schemas.
  - A check that no hand-written file is in the generated directories.
- **Explicit non-scope:**
  - The generator itself (FND-007); OpenAPI generation for the REST API (the API contract is checked only if API-001 exports one, via the same job).
- **Files/modules expected to change:** `packages/contracts/package.json` (script `check`), root scripts.
- **New files/modules expected:**
  - `.github/workflows/contracts.yml`
  - `scripts/ci/schema-compat.mjs`, `scripts/ci/schema-compat.test.mjs`
  - `scripts/ci/validate-fixture-payloads.mjs`
  - `packages/contracts/.generated-manifest.json` (hash list of generated files)
- **Dependencies (task IDs):** FND-007, DOM-001, CI-001, CI-003.
- **Implementation details:**
  - Steps: build the generator in Rust (`cargo run -p xtask -- contracts`), run TS type generation (`json-schema-to-typescript` or the FND-007 choice), then `git diff --exit-code -- packages/contracts`; the failure message prints the diff stat and the local command `pnpm contracts:generate`.
  - Determinism requirement: generation output must be stable (sorted keys, no timestamps, LF endings); a test runs the generator twice and compares.
  - Manifest: generated files carry a header `// @generated by xtask contracts, do not edit`; the manifest lists sha256 per file; a check fails if a file in the generated directory is missing the header or not in the manifest.
  - Compatibility: `schema-compat.mjs` loads schemas at the base commit (via `git show origin/main:path`) and the PR, then applies rules: removed property, type change, enum value removed, `required` addition, `additionalProperties` tightening are breaking. Breaking changes are allowed only when the PR carries the label `contract-break` and the schema's `$comment` contains a version bump marker; for job payloads (queued data may still be in flight) breaking changes additionally require a `payload_version` increment (ADR-012 payload schemas).
  - Fixture validation: every JSON example under `fixtures/contracts/**` is validated with `ajv` against its schema; invalid examples fail.
  - Cross-language round trip: a Rust test serializes sample values of key types (JobPayload, ContextPackage metadata, CandidateFinding) to JSON that the TS validator accepts, and the reverse using TS-produced samples.
- **Data model changes:** None.
- **API/protocol changes:** Required check `contracts / drift`; label `contract-break` is introduced in repository settings docs.
- **Concurrency semantics:** Stateless; runs in parallel with other workflows; path-filtered to `engine/crates/**`, `packages/contracts/**`, `fixtures/contracts/**`.
- **Failure behavior:** Drift fails with the regenerate hint; schema-compat failure lists each breaking change with JSON pointer and rule.
- **Idempotency considerations:** Regeneration on a clean tree is a no-op; the double-run determinism test guards this.
- **Security considerations:** Schemas for secret-bearing fields must keep `writeOnly`/redaction annotations; a rule fails if a removed `writeOnly` annotation is detected.
- **Observability additions:** Summary lists changed schemas and compatibility verdicts.
- **Tests required (named):**
  - `generation_is_deterministic_across_two_runs`
  - `drift_detected_when_rust_field_added_without_regeneration`
  - `hand_edited_generated_file_rejected`
  - `compat_flags_removed_property_and_new_required_field`
  - `compat_allows_optional_field_addition`
  - `payload_break_requires_payload_version_bump`
  - `fixture_payloads_validate_against_current_schemas`
  - `rust_ts_roundtrip_samples_validate_both_ways`
- **Benchmarks if applicable:** None.
- **Acceptance criteria (verifiable):**
  - Clean tree green; adding a field to a Rust contract type without regenerating fails `contracts / drift`.
  - `node --test scripts/ci/schema-compat.test.mjs` passes.
- **Definition of done:** Acceptance criteria pass; required check configured; `docs/operations/ci.md` documents the `contract-break` process.

---

### CI-007 — Benchmark smoke
Status: ☐

- **Task ID:** CI-007
- **Title:** Fast benchmark smoke on pull requests (criterion compile and quick run, 5-case quality subset under replay) and a nightly full benchmark with regression thresholds
- **Problem:** Performance and review-quality regressions are only visible if benchmarks run automatically, but the full suites are too slow and too noisy for every pull request.
- **Why it exists:** Master plan §12 gates: "CI smoke runs a 5-case subset under replay; the nightly run covers the full corpus; regression beyond thresholds (precision −3 pts, FP +3 pts, p95 latency +20%) fails the run".
- **Scope:**
  - `bench.yml` with a PR job `bench-smoke` and a scheduled job `bench-nightly`.
  - Smoke: `cargo bench --no-run` for all crates (benches must compile), run a tiny criterion profile on selected benches, and run the EVAL harness on 5 labeled PR cases with the replay model adapter.
  - Nightly: full criterion set on a fixed synthetic repository (PERF-001), full quality corpus under replay, compare with the stored baseline, upload results.
  - Baseline storage and comparison script.
- **Explicit non-scope:**
  - Writing the benchmarks (PERF-*, EVAL-*, QB-*); live-model evaluation runs (manual, costs money); load tests with k6 (PERF-008).
- **Files/modules expected to change:** `benchmarks/quality/README.md` (declare the smoke subset list), `engine/Cargo.toml` (bench profile).
- **New files/modules expected:**
  - `.github/workflows/bench.yml`
  - `benchmarks/quality/smoke-set.txt` (5 case ids)
  - `scripts/ci/bench-compare.mjs`, `scripts/ci/bench-compare.test.mjs`
  - `benchmarks/baselines/{perf.json,quality.json}` (committed baselines; updated by a reviewed PR only)
- **Dependencies (task IDs):** CI-001, EVAL-001, EVAL-004, GW-005 (replay adapter), PERF-001, PERF-002.
- **Implementation details:**
  - PR smoke: `cargo bench --workspace --no-run --locked`; `cargo bench -p codegraph -p incremental -- --warm-up-time 1 --measurement-time 2 --sample-size 10` on 3 named benches with a tiny fixture; results are informational (no failure on timing, since shared runners are noisy) except a crash or panic. Quality smoke: `review-eval run --set benchmarks/quality/smoke-set.txt --adapter replay --output eval-smoke.json`; fails if precision or recall for expected findings falls below the absolute floor stored in `quality.json` (floor, not baseline) so smoke is deterministic.
  - Nightly: `ubuntu-24.04` larger runner if available; full criterion run with `--save-baseline nightly`; `bench-compare.mjs` compares medians against `perf.json` with thresholds: p95 latency regression greater than 20 percent fails, memory regression greater than 15 percent fails; quality compare: precision drop more than 3 points or FP rate rise more than 3 points fails.
  - Noise control: nightly repeats failing comparisons once; documents CPU model in output; a regression opens an issue labeled `perf-regression` with the table (using the built-in token).
  - Results artifacts: criterion HTML, `eval-*.json`, comparison table in the job summary. Baselines are updated only by a PR that edits `benchmarks/baselines/*` with the justification in the description.
  - Replay mode means zero provider network calls and no API keys in these workflows.
- **Data model changes:** None.
- **API/protocol changes:** Required check on PRs: `bench / smoke`. Nightly status is advisory.
- **Concurrency semantics:** Nightly runs are serialized (`concurrency: bench-nightly`, no cancel) to avoid noisy neighbors; PR smoke cancels superseded runs.
- **Failure behavior:** Benchmark compile failure fails PRs. Missing baseline file fails with a bootstrap instruction. Replay fixture missing for a smoke case fails with the case id.
- **Idempotency considerations:** Replay and fixed seeds make quality results deterministic; perf results are compared with tolerances, not equality.
- **Security considerations:** No secrets, no model keys; fixtures contain synthetic code only; artifacts hold no tokens.
- **Observability additions:** Trend table of key metrics per night in the issue/summary (parse files per second, incremental update latency, review latency under replay, precision, FP rate).
- **Tests required (named):**
  - `compare_fails_on_p95_regression_over_20_percent`
  - `compare_passes_within_noise_tolerance`
  - `compare_fails_on_precision_drop_over_3_points`
  - `compare_fails_on_fp_rate_rise_over_3_points`
  - `smoke_set_has_exactly_five_cases_that_exist`
  - `missing_baseline_gives_bootstrap_message`
  - `bench_targets_all_compile` (the `--no-run` step itself)
- **Benchmarks if applicable:** This task consumes them; records smoke job wall time (target under 8 minutes).
- **Acceptance criteria (verifiable):**
  - PR smoke job green; nightly dry-run (`workflow_dispatch`) produces comparison output and a green or intentionally red result with an injected slowdown fixture.
  - `node --test scripts/ci/bench-compare.test.mjs` passes.
- **Definition of done:** Acceptance criteria pass; `docs/operations/ci.md` explains smoke vs nightly and how to update baselines.

---

### CI-008 — Docker builds
Status: ☐

- **Task ID:** CI-008
- **Title:** Build the `engine`, `api` and `web` images in CI with layer caching, image hardening checks and optional registry push on tagged releases
- **Problem:** Dockerfiles for `infra/docker/` (engine, api, web) can break unnoticed, and the target-architecture requires non-root, read-only-root-FS-compatible images with pinned bases.
- **Why it exists:** Target-architecture §9 (images: distroless/debian-slim, non-root, read-only root FS, `cap_drop: ALL`), master plan §15 (one engine image with multiple entrypoints), SEC-010 (SBOM and image scan).
- **Scope:**
  - `docker.yml` building three images with Buildx and GitHub Actions cache, on pull requests (build only, no push) and on tags/`main` (push to the configured registry).
  - Hardening verification: runs as non-root UID, no shell where distroless, healthcheck or entrypoint present, read-only root filesystem smoke start with `--read-only --cap-drop ALL --security-opt no-new-privileges`.
  - SBOM and vulnerability scan integration through SEC-010 scripts.
- **Explicit non-scope:**
  - Writing the Dockerfiles (DEV-001 and image tasks own content) beyond minimal fixes found by CI; deployment to GCP; signing (noted as follow-up).
- **Files/modules expected to change:** `infra/docker/*.Dockerfile` (fixes only), `.dockerignore`.
- **New files/modules expected:**
  - `.github/workflows/docker.yml`
  - `scripts/ci/image-hardening-check.sh`, `scripts/ci/image-smoke.sh`
  - `infra/docker/README.md`
- **Dependencies (task IDs):** DEV-001, SEC-010, CI-001, CI-003.
- **Implementation details:**
  - Matrix `image: [engine, api, web]`; context is repo root, Dockerfile `infra/docker/<image>.Dockerfile`. `docker/build-push-action` with `cache-from/to: type=gha,scope=<image>`; PR builds use `load: true` to run local checks, release builds `push: true` with tags `sha-<short>` and semver on `v*` tags. The registry and credentials come from repository secrets, used only in the push job on non-fork refs.
  - Engine image: multi-stage on `rust:1-bookworm` builder with `cargo build --release --locked`; runtime `debian:bookworm-slim` (or distroless cc) containing `review-worker`, `review-engine`, `review-cli`; entrypoint selected by argument. Base images pinned by digest through build args updated by a reviewed PR.
  - Hardening script asserts via `docker inspect`: `Config.User` is non-root numeric; no `latest` base; `docker history` has no secret-pattern matches (patterns from OBS-006 corpus); image size below documented limits (engine 400 MB, api 300 MB, web 250 MB, tunable).
  - Smoke: `image-smoke.sh` starts each image with `--read-only --tmpfs /tmp --cap-drop ALL --security-opt no-new-privileges:true` and checks the health endpoint or `--version` within 30 s; engine smoke runs `review-cli --version` and `review-worker --help`.
  - Scan: `trivy image --severity HIGH,CRITICAL --exit-code 1 --ignore-unfixed` honoring `security/exceptions.toml` (SEC-010); SBOM via `syft` uploaded as an artifact.
  - Build args include `GIT_SHA` and `BUILD_TIME` labeled as OCI image labels.
- **Data model changes:** None.
- **API/protocol changes:** Required checks `docker / engine`, `docker / api`, `docker / web`.
- **Concurrency semantics:** Matrix jobs in parallel; registry push job needs all three to pass; tag builds are not cancelled.
- **Failure behavior:** Build failure, hardening failure, smoke failure or unwaived HIGH/CRITICAL vulnerability fails the job. Registry push failures retry once. Fork PRs build without push or secrets.
- **Idempotency considerations:** Tags are immutable `sha-<short>`; re-pushing the same commit produces the same digest where reproducible, and the push step skips if the tag exists.
- **Security considerations:** Non-root, minimal bases, no build secrets in layers (secrets only through BuildKit `--secret` if ever needed), pinned digests, least-privilege `GITHUB_TOKEN` (`packages: write` only on push job).
- **Observability additions:** Summary with image sizes, build times and cache hit ratio.
- **Tests required (named):**
  - `images_run_as_non_root`
  - `images_start_with_read_only_rootfs_and_dropped_caps`
  - `no_secret_patterns_in_image_history`
  - `engine_image_contains_three_binaries`
  - `base_images_pinned_not_latest`
  - `image_size_within_limits`
- **Benchmarks if applicable:** Warm-cache build time per image recorded (target under 6 minutes for engine).
- **Acceptance criteria (verifiable):**
  - All three jobs green on a PR; adding `USER root` to a Dockerfile fails the hardening check.
  - A release tag run pushes three images and uploads SBOMs.
- **Definition of done:** Acceptance criteria pass; required checks configured; `infra/docker/README.md` describes entrypoints and build arguments.

---

### CI-009 — Windows CLI build (MSVC runner)
Status: ☐

- **Task ID:** CI-009
- **Title:** Build and smoke-test the `review` CLI natively on a Windows MSVC runner and publish a release artifact
- **Problem:** Developer machines are Windows, but the Windows host toolchain here cannot compile the C dependencies (tree-sitter) or link some crates, so local Windows builds go through Linux containers. The shipped CLI must still build and run natively on Windows (MSVC toolchain with a full C compiler).
- **Why it exists:** Master plan scope for the Rust CLI as a product deliverable (CLI-* tasks, local file graph store); ADR-001 notes the Linux container is a development workaround, not a product limitation. CI on a Windows runner proves the CLI is distributable.
- **Scope:**
  - `windows-cli.yml` on `windows-2022`/`windows-latest` using `x86_64-pc-windows-msvc`.
  - Builds `review-cli` (release, locked), runs a Windows-specific test subset, and smoke runs `review.exe init` and `review.exe status` against a fixture repository.
  - Uploads `review-x86_64-pc-windows-msvc.zip` with SHA-256 checksum as an artifact; attaches to GitHub Releases on tags.
- **Explicit non-scope:**
  - Building the worker or engine on Windows (Linux only); code signing (documented follow-up); macOS builds; installer packaging.
- **Files/modules expected to change:** `engine/apps/review-cli/Cargo.toml` (feature flags to exclude Postgres-only deps on Windows if needed, never silently), crates with path handling that fail on Windows (fixes found by this job).
- **New files/modules expected:**
  - `.github/workflows/windows-cli.yml`
  - `scripts/ci/windows-smoke.ps1`
  - `engine/apps/review-cli/tests/windows_paths.rs`
  - `docs/operations/cli-distribution.md`
- **Dependencies (task IDs):** CLI-001, INIT-001, FND-008 (fixture repo builder), CI-001.
- **Implementation details:**
  - Toolchain: `dtolnay/rust-toolchain` pinned to the channel in `engine/rust-toolchain.toml`, target `x86_64-pc-windows-msvc`; MSVC build tools are preinstalled on the runner; `git` available. Cache via `Swatinem/rust-cache`.
  - Commands: `cargo build --release --locked -p review-cli`; `cargo test -p review-cli -p repository --locked` (the CLI and repository crates are expected to be Windows-clean); other crates are not tested on Windows initially, with the list of excluded crates recorded in `docs/operations/cli-distribution.md`.
  - Fixture repo creation: Windows has no bash fixtures builder by default, so the job uses Git Bash (`shell: bash`) to run `fixtures/build.sh` (FND-008) before the PowerShell smoke; line endings are pinned with `.gitattributes` (`* text=auto eol=lf` for fixtures and golden files) to avoid CRLF diffs.
  - Smoke (`windows-smoke.ps1`): `review.exe --version`; `review.exe init --repository <fixture>` produces `.review/repository.json` and a local graph snapshot; `review.exe status` reports index state; exit codes asserted; paths containing spaces and a drive-letter path are exercised; a repository path longer than 260 characters uses long-path support (`\\?\` handling is verified or the limit documented).
  - Windows-specific tests: path normalization to forward slashes in symbol ids (ADR-005 `module_path` is POSIX style regardless of OS), case-insensitive filesystem collisions detected, `gix` opens a repo with CRLF content, file locking on the `.review/graph` store.
  - Artifact: zip containing `review.exe`, `LICENSE`, and `README`; checksum file; attached on `v*` tags using `gh release upload` with `contents: write` only on that job.
- **Data model changes:** None.
- **API/protocol changes:** CLI command contract unchanged; the artifact naming convention is new.
- **Concurrency semantics:** Single job; cancels superseded PR runs; tag runs not cancelled.
- **Failure behavior:** Build failure, test failure or smoke failure fails the job; missing MSVC tooling fails with the installer hint. Windows-only regressions are therefore caught before release.
- **Idempotency considerations:** Reruns on the same commit overwrite the same artifact name; release upload uses `--clobber` only for the same tag.
- **Security considerations:** Read-only token on PRs, no secrets; release job permissions scoped to contents write; artifact checksum published; the binary is not signed yet (documented limitation).
- **Observability additions:** Summary with binary size, build time and smoke timings.
- **Tests required (named):**
  - `symbol_ids_use_posix_module_paths_on_windows`
  - `path_with_spaces_and_drive_letter_roundtrips`
  - `case_insensitive_collision_is_reported`
  - `crlf_files_hash_consistently_with_lf_fixture` (documented policy: hash raw bytes; fixtures forced LF)
  - `smoke_init_and_status_exit_zero`
- **Benchmarks if applicable:** Binary size and cold start time of `review.exe --version` recorded; target under 100 ms.
- **Acceptance criteria (verifiable):**
  - The workflow is green; the artifact downloaded on a Windows machine runs `review.exe init` on a fixture repository.
  - A deliberate backslash path in an id test fails `symbol_ids_use_posix_module_paths_on_windows`.
- **Definition of done:** Acceptance criteria pass; `docs/operations/cli-distribution.md` documents install, limitations and unsigned status.

---

### DEV-001 — Full compose including api, engine, worker and web
Status: ☐

- **Task ID:** DEV-001
- **Title:** Extend `infra/compose/docker-compose.yml` with the application services (`api`, `engine`, `worker`, `web`) behind a profile, with hardened containers and healthchecks
- **Problem:** FND-005 provides only the backing services (postgres 25432, redis 26379, qdrant 26333, openobserve 25080, SeaweedFS S3 29000). Developers and the E2E test need the whole system running with one command.
- **Why it exists:** Master plan §15 ("Local: `infra/compose` (DEV-001). One command: `pnpm dev:up`"); target-architecture §9 (images non-root, read-only root FS, `cap_drop: ALL`); E2E-001 uses this stack.
- **Scope:**
  - Services: `migrate` (one-shot), `api` (NestJS), `engine` (review-engine Axum internal API), `worker` (review-worker), `web` (Next.js), all on the default network with the backing services.
  - A `docker-compose.override.example.yml` for bind-mounted hot reload dev (optional).
  - Compose profiles: default `infra` services only; `app` profile adds application services; `pnpm dev:up:app` selects it.
  - Environment wiring through `infra/compose/.env` defaults.
- **Explicit non-scope:**
  - Building and publishing images (CI-008); seed data (DEV-002); the bootstrap script (DEV-003); production deployment manifests.
- **Files/modules expected to change:** `infra/compose/docker-compose.yml`, `infra/compose/.env.example`, `infra/compose/check-ports.sh` (extend to the new ports), root `package.json` (`dev:up:app`, `dev:logs`), `docker-compose.test.yml` (unchanged).
- **New files/modules expected:**
  - `infra/docker/{engine,api,web}.Dockerfile` if not yet present (images from CI-008), `infra/compose/docker-compose.override.example.yml`
  - `infra/compose/app.env.example` (service-specific variables)
- **Dependencies (task IDs):** FND-005, FND-006, API-001, API-005, DOM-009, OBS-001, OBS-002.
- **Implementation details:**
  - Host ports (loopback only, overridable): `api` 127.0.0.1:23000, `engine` 127.0.0.1:23100 (internal API, also exposed for debugging), `web` 127.0.0.1:23001; `worker` has no ports. Container ports are fixed (3000, 3100, 3001).
  - `migrate`: runs `review-worker migrate` (DOM-009; sqlx migrations) once, `restart: "no"`, `depends_on` postgres healthy; `api`, `engine`, `worker` depend on `migrate` with `condition: service_completed_successfully`.
  - Dependencies and health: `api` healthcheck `GET /health/ready` (API-001); `engine` `GET /healthz`; `web` `GET /api/health`; `worker` uses a file or a tiny HTTP probe on a loopback admin port (9102) exposing `/healthz` and metrics-self for readiness.
  - Hardening per service: `read_only: true`, `tmpfs: [/tmp, /work]` (worker checkout dir, 2 GB size limit), `cap_drop: [ALL]`, `security_opt: [no-new-privileges:true]`, non-root `user`, `pids_limit`, resource limits (`mem_limit` api 512m, worker 2g, engine 1g, web 512m).
  - Environment inside the network: `DATABASE_URL=postgres://...@postgres:5432/reviewgraph`, `REDIS_URL=redis://redis:6379`, `QDRANT_URL=http://qdrant:6333`, `S3_ENDPOINT=http://objectstore:8333` (SeaweedFS), `OTEL_EXPORTER_OTLP_ENDPOINT=http://openobserve:5080/api/default`, `GITHUB_API_BASE_URL` default `https://api.github.com` overridable to the fake server (DEV-005). Secrets (`GITHUB_APP_PRIVATE_KEY`, `ANTHROPIC_API_KEY`, webhook secret) come from the uncommitted `.env` or Compose `secrets:` files, never baked into images.
  - Worker needs `git`; the engine image includes it. The worker mounts a named volume `rg-worker-work` for the mirror cache (ephemeral per SEC-007 policy).
  - Replay mode switch: `RG_MODEL_ADAPTER=replay|anthropic|openai` default `replay` so `up` never spends money.
- **Data model changes:** None.
- **API/protocol changes:** None; documents port and URL map in `.env.example`.
- **Concurrency semantics:** Start order enforced via `depends_on` conditions; scaling the worker with `--scale worker=N` is supported (SKIP LOCKED queue) and documented.
- **Failure behavior:** A failed `migrate` blocks app services and shows the migration error; unhealthy services make `up --wait` fail naming the service. Port collisions fail fast.
- **Idempotency considerations:** `up` re-runnable; migrations idempotent; `down -v` resets everything including the SeaweedFS bucket via `objectstore-init`.
- **Security considerations:** Loopback-only ports (`check-ports.sh` extended and still passing), read-only root FS, dropped capabilities, no secrets in the compose file, dev credentials only, no docker socket mounts.
- **Observability additions:** All services export OTLP to local OpenObserve; container logs are JSON on stdout.
- **Tests required (named):**
  - `compose_config_valid_with_app_profile`
  - `ports_loopback_only_including_app_services`
  - `app_stack_up_all_healthy` (`up --wait` with profile `app`)
  - `containers_run_non_root_read_only_cap_dropped` (inspect)
  - `worker_scales_to_two_without_duplicate_job_claims` (smoke through the queue)
  - `migrate_failure_blocks_dependents`
- **Benchmarks if applicable:** Record cold and warm `up --wait` time (target under 120 s warm).
- **Acceptance criteria (verifiable):**
  - `docker compose --profile app -f infra/compose/docker-compose.yml up -d --wait` exits 0; `curl http://127.0.0.1:23000/health/ready` and the web page respond.
  - `bash infra/compose/check-ports.sh` exits 0.
- **Definition of done:** Acceptance criteria pass; ports table in `docs/operations/local-development.md` (DEV-004) matches the file.

---

### DEV-002 — Seed data and fixtures load
Status: ☐

- **Task ID:** DEV-002
- **Title:** Deterministic dev seed (organizations, users, repositories, indexed fixture graph, sample review runs and findings) and a fixture loader command
- **Problem:** A fresh stack is empty, so developers cannot see the UI, the graph explorer or the review flow without manually onboarding a real GitHub App. Integration tests also need a shared, deterministic dataset.
- **Why it exists:** Master plan §11 (fixture repositories, golden PR scenarios), M1/M6 manual verification, WEB-* screens need data, SEC-001 uses a two-org fixture built from the same helpers.
- **Scope:**
  - A `review-worker seed` (or `xtask seed`) subcommand that creates two organizations, users and memberships, repositories, an indexed snapshot built from `fixtures/repositories/*` by the real index pipeline, a handful of PRs with review runs, candidate and published findings in different states, and feedback.
  - A fixture loader that builds the git fixture repositories (FND-008) into `fixtures/.built/` and registers them as local bare mirrors.
  - Idempotent `seed` and a `seed --reset` that truncates seeded rows only.
- **Explicit non-scope:**
  - The bootstrap orchestration (DEV-003); production data; GitHub App installation records pointing to real installations; synthetic 100k-file repos (PERF-001).
- **Files/modules expected to change:** `engine/apps/review-worker/src/main.rs` (subcommand), root `package.json` (`dev:seed`, `dev:seed:reset`).
- **New files/modules expected:**
  - `engine/crates/pipeline/src/seed/{mod.rs,orgs.rs,repos.rs,reviews.rs,findings.rs}`
  - `fixtures/seed/{seed.yaml,README.md}` (declarative description of the dataset)
  - `engine/crates/pipeline/tests/seed.rs`
  - `apps/api/test/helpers/seed-fixture.ts` (TS wrapper calling the same SQL through the API test harness)
- **Dependencies (task IDs):** DOM-009, IDX-001, FND-008, API-003, DEV-001, GW-005.
- **Implementation details:**
  - Dataset (`seed.yaml`): orgs `acme-dev` and `globex-dev` (clearly fictional); users `dev-admin`, `dev-maintainer`, `dev-viewer` in acme, `other-admin` in globex; repos from fixtures (`nest-basic`, `renames-moves`, etc.) registered with provider `github` and fake ids; each indexed once into a full snapshot and one delta snapshot; 3 PRs with states `COMPLETED`, `SUPERSEDED`, `REVIEWING`; findings in states `PUBLISHED`, `SUPPRESSED_LOW_CONFIDENCE`, `SUPPRESSED_DUPLICATE`; feedback records for each feedback kind.
  - UUIDs are `uuid_v5` of stable names so reseeding never changes ids and e2e tests can reference them.
  - Findings come from the replay model adapter fixtures (GW-005), not hand-written rows, so the data matches real pipeline output; where the pipeline is not yet available a SQL fallback `fixtures/seed/minimal.sql` loads equivalent rows (feature flag `--minimal`).
  - The seed runs as the `rg_ops` role, sets the org per insert batch for RLS tables, and writes an audit row (`seed.applied`).
  - Dev login: a session cookie helper `pnpm dev:login` prints a dev-only session for `dev-admin` when `RG_DEV_AUTH=1` (disabled by config validation in any non-local environment).
  - Safety: refuses to run if `NODE_ENV=production` or the database host is not loopback/compose-internal; seed rows carry `attrs.seed=true` so `--reset` deletes only those.
- **Data model changes:** None; optionally an `is_seed boolean` convention in `attrs` jsonb, not a column.
- **API/protocol changes:** CLI only (`seed`, `seed --reset`, `seed --minimal`).
- **Concurrency semantics:** One seed at a time (advisory lock `seed`); concurrent runs wait or exit with a message.
- **Failure behavior:** Any step failure rolls back that organization's seed transaction; the command prints which step failed and exits non-zero; partially built fixture repos are rebuilt on next run.
- **Idempotency considerations:** Running twice yields identical row counts and ids (asserted); `--reset` followed by seed restores the same state.
- **Security considerations:** Dev-only guard; fictional data and credentials; no real tokens; the dev login helper is compiled out or config-gated for production builds.
- **Observability additions:** The seed emits a trace and logs counts per entity; the seeded review runs also exercise dashboards.
- **Tests required (named):**
  - `seed_creates_expected_entities_and_counts`
  - `seed_twice_is_idempotent_with_same_ids`
  - `reset_removes_only_seed_rows`
  - `seed_refuses_production_environment`
  - `seed_respects_rls_org_scoping_for_inserts`
  - `seeded_findings_cover_all_states_listed_in_seed_yaml`
  - `dev_login_disabled_unless_flag_set`
- **Benchmarks if applicable:** Seed time recorded (target under 30 s with fixtures cached).
- **Acceptance criteria (verifiable):**
  - After `pnpm dev:up && pnpm migrate && pnpm dev:seed`, `psql` shows 2 organizations and the web dashboard lists the seeded repositories and reviews.
  - Second `pnpm dev:seed` changes no row counts.
- **Definition of done:** Acceptance criteria pass; `fixtures/seed/README.md` documents the dataset and ids.

---

### DEV-003 — One-command bootstrap
Status: ☐

- **Task ID:** DEV-003
- **Title:** `pnpm bootstrap`: preflight checks, dependency install, engine image build, stack up, migrations, seed and a final health report
- **Problem:** A new developer needs Docker, Node 24, pnpm, a Linux engine image, the compose stack, migrations and seed data in the correct order. Today that is a manual sequence across two shells (Git Bash and PowerShell) with Windows-specific pitfalls (WSL `bash`, path conversion).
- **Why it exists:** Master plan §15 ("One command") and M1 exit criteria; reduces onboarding time and gives CI/E2E a single entry point.
- **Scope:**
  - `scripts/bootstrap.mjs`, a dependency-free Node ESM script invoked by `pnpm bootstrap`.
  - Preflight: Docker daemon reachable, Docker Compose v2, free host ports (25432, 26379, 26333, 25080, 29000, 23000, 23001, 23100), Node 24, pnpm version, disk space, and the `host.docker.internal` mapping.
  - Steps in order: `pnpm install --frozen-lockfile`, build `reviewgraph-engine-dev` image, `dev:up`, `migrate`, `obs:apply` (dashboards), `dev:seed`, optional `--app` to include application services, final report with URLs and health.
  - Flags: `--skip-install`, `--no-seed`, `--app`, `--reset` (down -v first), `--yes`.
- **Explicit non-scope:**
  - Installing Docker or Node (prints instructions); GitHub App setup (DEV-006); production provisioning.
- **Files/modules expected to change:** Root `package.json` (`bootstrap`), `scripts/rg.mjs` (export helpers reused: process spawn without `shell: true`, platform dispatch).
- **New files/modules expected:**
  - `scripts/bootstrap.mjs`, `scripts/bootstrap/{preflight.mjs,steps.mjs,report.mjs}`
  - `scripts/bootstrap.test.mjs`
- **Dependencies (task IDs):** FND-002, FND-005, FND-006, DEV-001, DEV-002, OBS-007.
- **Implementation details:**
  - Steps are an ordered array of `{name, run, skipIf, verify}`; every step prints a one-line status with elapsed time, and `verify` re-checks the postcondition (for example `pg_isready` through `docker compose exec`).
  - Preflight port check uses a TCP bind probe on 127.0.0.1; a busy port reports which process if detectable (`netstat -ano` on Windows, `lsof` elsewhere) and suggests the `RG_*_PORT` override.
  - Windows specifics: uses `engine/scripts/cargo.ps1`/`cargo.sh` dispatch from `rg.mjs`; sets `MSYS_NO_PATHCONV=1` for Git Bash invocations; detects WSL `bash` shadowing and warns; avoids symlinks.
  - Engine image: if `reviewgraph-engine-dev:1` is missing it builds from `engine/docker/dev.Dockerfile` (as `cargo.sh` does) and reports the expected duration.
  - Creates `infra/compose/.env` from `.env.example` if absent (never overwrites), reporting which values are placeholders (GitHub App settings, model keys) and that `RG_MODEL_ADAPTER=replay` is the default.
  - Final report: table of service name, URL, health state; next steps (open the web app, run the replay tool DEV-005, set up the GitHub App DEV-006); exit code 0 only if all verifications pass.
  - Re-run behavior: each step is idempotent; completed steps are detected and shown as `ok (already done)`.
- **Data model changes:** None.
- **API/protocol changes:** CLI flags as above.
- **Concurrency semantics:** Steps run sequentially; the script takes a lock file `.bootstrap.lock` to prevent two concurrent runs and removes it in `finally`.
- **Failure behavior:** The first failed step stops the run, prints the failing command, last 40 lines of relevant logs and the exact command to retry that step; exit code non-zero. Ctrl-C cleans up the lock and leaves the stack as is.
- **Idempotency considerations:** Safe to run repeatedly; `--reset` is the only destructive option and asks for confirmation unless `--yes`.
- **Security considerations:** No `shell: true`; no secrets printed (the `.env` summary shows only key names set or unset); dev credentials only; refuses `--reset` if `DATABASE_URL` points to a non-loopback host.
- **Observability additions:** Writes `.bootstrap/last-run.json` (step timings, versions) for support, without environment values.
- **Tests required (named):**
  - `preflight_reports_busy_port_with_override_hint`
  - `preflight_fails_without_docker_daemon` (stubbed spawn)
  - `steps_run_in_order_and_stop_on_first_failure`
  - `env_file_created_from_example_but_never_overwritten`
  - `reset_requires_confirmation_or_yes_flag`
  - `windows_dispatch_uses_ps1_wrapper`
  - `lock_file_prevents_concurrent_runs_and_is_cleaned`
  - `report_exit_code_reflects_health`
- **Benchmarks if applicable:** Time from clean clone to green report (target under 10 minutes cold including the engine image build, under 3 minutes warm).
- **Acceptance criteria (verifiable):**
  - On a clean Windows machine with Docker Desktop and Node 24, `pnpm bootstrap` finishes with all services reported healthy and seed data present; a second run completes quickly with every step `already done`.
  - `node --test scripts/bootstrap.test.mjs` passes.
- **Definition of done:** Acceptance criteria pass; DEV-004 documents the command and flags.

---

### DEV-004 — `docs/operations/local-development.md`
Status: ☐

- **Task ID:** DEV-004
- **Title:** Local development guide covering prerequisites, ports, the Windows container toolchain, daily commands and troubleshooting
- **Problem:** Environment knowledge is scattered across task specs and scripts: why Rust builds happen in a Linux container, which ports are used (pg 25432, redis 26379, qdrant 26333, openobserve 25080, S3 29000), why SeaweedFS replaces MinIO, and how to run tests.
- **Why it exists:** Master plan §15 and M1 exit criteria; onboarding and incident recovery both need a single trusted document, and docs must stay in sync with the compose files and scripts.
- **Scope:**
  - A complete guide: prerequisites, quick start (`pnpm bootstrap`), architecture of the local stack, port and URL table, credentials (dev defaults), the Rust container workflow, running tests (unit, integration, e2e), observability in OpenObserve, working with fixtures and seed data, and troubleshooting.
  - A doc test that verifies the document's ports, scripts and commands match reality.
- **Explicit non-scope:**
  - Production runbooks (OBS-008), GitHub App creation steps (DEV-006), CI documentation (`docs/operations/ci.md`).
- **Files/modules expected to change:** Root `README.md` (link and short quick start), `docs/README.md` index.
- **New files/modules expected:**
  - `docs/operations/local-development.md`
  - `scripts/docs/check-local-dev-doc.mjs`, `scripts/docs/check-local-dev-doc.test.mjs`
- **Dependencies (task IDs):** FND-005, FND-006, DEV-001, DEV-003, OBS-001.
- **Implementation details:**
  - Required sections: Prerequisites (Docker Desktop with Compose v2, Node 24, pnpm 10.15, Git, a PowerShell 7 or Git Bash shell; no Rust toolchain needed on the host); Quick start; Stack map (service, image variable, host port, container port, purpose); Everyday commands (`pnpm dev:up`, `dev:down`, `dev:reset`, `migrate`, `dev:seed`, `test`, `test:integration`, `lint`, `fmt`, `obs:apply`); Rust in a container (how `engine/scripts/cargo.sh` and `.ps1` work, named volumes `rg-cargo-registry`, `rg-cargo-git`, `rg-engine-target`, the `host.docker.internal` mapping, env variables forwarded); Object storage (why SeaweedFS S3 on port 29000 and the bucket `reviewgraph-artifacts`, dev credentials, how to browse); Observability (OpenObserve login at `http://127.0.0.1:25080`, finding a review trace, dashboards); Model adapter modes (`replay` default, how to use a real key safely); Troubleshooting table.
  - Troubleshooting must cover: port already in use (and the `RG_*_PORT` overrides), Docker Desktop not running, WSL `bash` shadowing Git Bash, `MSYS_NO_PATHCONV` path mangling, line-ending issues, slow first build and volume reuse, `sqlx` offline metadata, qdrant or SeaweedFS unhealthy, resetting everything with `pnpm dev:reset`, and out-of-disk for named volumes.
  - Credentials appear only as the documented dev defaults, flagged "local only".
  - `check-local-dev-doc.mjs` parses the doc and asserts: every port in the stack table matches `docker compose config --format json`; every `pnpm` script mentioned exists in `package.json`; every relative link resolves; no occurrence of real-looking tokens (OBS-006 patterns); required headings present.
- **Data model changes:** None.
- **API/protocol changes:** None.
- **Concurrency semantics:** Not applicable (documentation); the doc check is a pure script.
- **Failure behavior:** The doc check fails CI with a precise list of mismatches (port, script, link).
- **Idempotency considerations:** Not applicable; the doc is regenerated only by hand, the check is deterministic.
- **Security considerations:** No real secrets; clear statement that dev credentials are not for any shared environment; instructions never ask users to paste real tokens into files tracked by git; mentions `.env` is ignored.
- **Observability additions:** None.
- **Tests required (named):**
  - `doc_ports_match_compose_config`
  - `doc_scripts_exist_in_package_json`
  - `doc_links_resolve`
  - `doc_has_required_headings`
  - `doc_contains_no_secret_patterns`
- **Benchmarks if applicable:** None.
- **Acceptance criteria (verifiable):**
  - `node --test scripts/docs/check-local-dev-doc.test.mjs` passes and the checker passes on the real document.
  - A new teammate completes bootstrap and first test run following only this document (recorded as a short dry-run note in the PR).
- **Definition of done:** Acceptance criteria pass; the doc check is wired into CI-003; root README links to the guide.

---

### DEV-005 — Signed webhook replay tool and fake GitHub API server
Status: ☐

- **Task ID:** DEV-005
- **Title:** `tools/dev` utilities: a CLI that sends correctly signed GitHub webhook payloads, and a fake GitHub REST/Git server that records the review the system publishes
- **Problem:** End-to-end testing and local development must not depend on a real GitHub App. Developers need to inject realistic signed webhooks, and the platform needs a GitHub stand-in that serves PR data, installation tokens and a local bare repository for checkout, and captures posted reviews and check runs.
- **Why it exists:** Master plan §11 (end-to-end: signed GitHub webhook to fake GitHub API to checkout from local bare repo to publish), E2E-001, GH-002 acceptance criteria (replay tool), SEC-005/SEC-006 tests.
- **Scope:**
  - `tools/dev/replay-webhook` (Node): reads a payload file or a named scenario, computes `X-Hub-Signature-256`, sets `X-GitHub-Event`, `X-GitHub-Delivery`, `X-GitHub-Hook-ID`, `User-Agent`, posts to the API webhook URL; flags for tampering, repeat and redelivery to exercise SEC-006.
  - `tools/dev/fake-github` (Node, no dependencies beyond a small HTTP lib): implements the GitHub endpoints the provider adapter uses (app installation token, installation repositories, pull request, files, compare, contents as needed, create review, create check run, list review comments, reactions) and serves git over smart HTTP (or a read-only dumb-HTTP mirror) from fixture bare repositories.
  - Scenario packs under `fixtures/pull-requests/*` (base+patch) mapped to webhook payloads and fake API state.
  - An admin/inspection API on the fake server: `GET /__recorded` returns posted reviews, comments, check runs for assertions.
- **Explicit non-scope:**
  - Emulating all of GitHub; rate-limit headers beyond basics; OAuth login flows; GraphQL.
- **Files/modules expected to change:** Root `package.json` (scripts `dev:fake-github`, `dev:replay`), `infra/compose/docker-compose.yml` (profile `fake` adds a `fake-github` service on 127.0.0.1:23400), `.env.example` (`GITHUB_API_BASE_URL`).
- **New files/modules expected:**
  - `tools/dev/replay-webhook/{index.mjs,sign.mjs,scenarios.mjs}`
  - `tools/dev/fake-github/{server.mjs,routes/*.mjs,state.mjs,git-http.mjs,tokens.mjs}`
  - `tools/dev/test/*.test.mjs`, `fixtures/pull-requests/*/{scenario.json,payloads/*.json}`
- **Dependencies (task IDs):** GH-002, GH-001, GH-005, GH-009, FND-008, DEV-001.
- **Implementation details:**
  - Signing: `sha256=` + HMAC-SHA256 hex of the exact raw body bytes using `GITHUB_WEBHOOK_SECRET` from env; the tool never re-serializes after signing. Options: `--event pull_request`, `--scenario <name>`, `--secret-env NAME`, `--tamper` (flip one byte after signing to get 401), `--delivery-id <uuid>` (default random), `--repeat N` (same delivery id to test dedup), `--redeliver` (same body, new id).
  - Fake server auth: accepts App JWTs signed by a dev-only keypair generated at startup (public key exposed at `/__dev/public-key`; the API in dev config points to the same private key via `.env` documented in DEV-006); returns installation tokens with an `expires_at`, validates `Authorization: Bearer` on every endpoint, and records token use to assert least privilege.
  - Git access: serves bare repos from `fixtures/.built/*.git` at `/{owner}/{repo}.git` via smart HTTP using `git http-backend` in the engine/fake container (git installed), requiring the installation token (so credential handling from SEC-005 is exercised).
  - Pull request data: scenario JSON defines PR number, base and head SHA (from the fixture history), changed files with patches, comments; endpoints return GitHub-shaped JSON with pagination `Link` headers to exercise the paginator.
  - Recording: `POST /repos/:o/:r/pulls/:n/reviews` and check-run endpoints store the body and return realistic ids; supports simulated failures (`X-Fake-Fail: 502:once`) and 422 for invalid line anchors so publisher error paths are testable.
  - Dev-only guards: both tools bind to 127.0.0.1 and refuse to run if `GITHUB_API_BASE_URL` points to `api.github.com`.
- **Data model changes:** None.
- **API/protocol changes:** The fake server's `/__recorded`, `/__reset` and `/__dev/public-key` endpoints (dev only).
- **Concurrency semantics:** The fake state is in memory keyed per installation; requests are processed concurrently; `/__reset` clears state atomically between e2e tests.
- **Failure behavior:** The replay tool exits non-zero on non-2xx (except when `--expect-status` is set) and prints status plus response body; the fake server returns GitHub-style error JSON for unknown routes (404) and invalid tokens (401).
- **Idempotency considerations:** The same scenario can be replayed repeatedly; fake review creation is not deduplicated (like GitHub) so duplicate publishing bugs become visible.
- **Security considerations:** Secrets only from env; payloads contain synthetic data; no real tokens accepted or emitted; servers loopback-bound; the signing code is test tooling and is not shipped in images.
- **Observability additions:** The fake server logs one JSON line per request (method, path, status, token id hash); the replay tool prints the `traceparent`-free delivery id so the trace can be found by `delivery_id` attribute in OpenObserve.
- **Tests required (named):**
  - `signature_matches_reference_vector`
  - `tamper_flag_yields_401_from_api` (against a stub verifying server)
  - `repeat_flag_sends_same_delivery_id`
  - `fake_github_rejects_invalid_or_missing_token`
  - `fake_github_paginates_files_with_link_headers`
  - `fake_github_records_review_and_check_run`
  - `fake_github_serves_clone_with_token_only`
  - `fake_github_simulated_422_for_bad_anchor`
  - `tools_refuse_real_github_base_url`
- **Benchmarks if applicable:** Fake server handles 100 requests per second on a laptop without errors (smoke).
- **Acceptance criteria (verifiable):**
  - `node --test tools/dev/test` passes; `pnpm dev:replay -- --scenario basic-pr` returns 202 from the running API and `GET /__recorded` shows the review after the pipeline completes.
  - Tampered payload returns 401.
- **Definition of done:** Acceptance criteria pass; scenarios documented in `fixtures/pull-requests/README.md`; E2E-001 uses both tools.

---

### DEV-006 — GitHub App setup document and ngrok tunnel
Status: ☐

- **Task ID:** DEV-006
- **Title:** Step-by-step GitHub App registration for local development with an ngrok tunnel, including the permission manifest and a verification script
- **Problem:** To receive real webhooks locally a developer must register a GitHub App with the exact least-privilege permissions, expose the local webhook endpoint over HTTPS, and configure secrets without leaking them. Doing this ad hoc risks over-broad permissions (violating the no-merge guarantee) or committed secrets.
- **Why it exists:** Master plan §13.3 (App permissions: contents read, pull_requests write, checks write, metadata read; no merge capability), GH-010 (permission manifest + static test), E2E-002 (manual run against a real repository with a real App).
- **Scope:**
  - `docs/operations/github-app-setup.md`: register the App (screenshots-free textual steps), permissions, subscribed events, webhook URL via ngrok, generate and store the private key, install on a test repository, configure `.env`, verify with a script.
  - An `app-manifest.json` for GitHub's manifest flow (same permissions as GH-010's manifest).
  - ngrok integration: a script to start a tunnel to the API port (23000) and print the webhook URL; optional compose profile.
  - `scripts/dev/verify-github-app.mjs` that checks configuration end to end without printing secrets.
- **Explicit non-scope:**
  - Production App registration and secret-manager setup (documented only as a pointer); the fake GitHub flow (DEV-005); creating the App on the user's behalf.
- **Files/modules expected to change:** `infra/compose/.env.example` (variables `GITHUB_APP_ID`, `GITHUB_APP_PRIVATE_KEY_PATH`, `GITHUB_WEBHOOK_SECRET`, `GITHUB_CLIENT_ID`, `GITHUB_CLIENT_SECRET`), `.gitignore` (`*.pem`, `.env`, `.secrets/`), root `package.json` (`dev:tunnel`, `dev:verify-app`).
- **New files/modules expected:**
  - `docs/operations/github-app-setup.md`, `infra/github-app/app-manifest.json`
  - `scripts/dev/{tunnel.mjs,verify-github-app.mjs}`, `scripts/dev/verify-github-app.test.mjs`
- **Dependencies (task IDs):** GH-001, GH-002, GH-010, SEC-005, DEV-001, DEV-004.
- **Implementation details:**
  - Permissions and events in the manifest: repository permissions `contents: read`, `pull_requests: write`, `checks: write`, `metadata: read`; subscribed events `pull_request`, `installation`, `installation_repositories`, optionally `pull_request_review` and `issue_comment` if the feedback flow requires them; no `administration`, no `contents: write`, no merge-related access. A test asserts the manifest equals the GH-010 permission set and contains none of the forbidden permissions.
  - Webhook URL form: `https://<subdomain>.ngrok-free.app/api/v1/webhooks/github`; webhook secret generated locally (`node -e "crypto.randomBytes(32).toString('hex')"` documented), stored only in `.env` (gitignored); the private key is saved under `.secrets/github-app.pem` (gitignored, mode 600 where supported) and referenced by path.
  - `tunnel.mjs`: wraps the `ngrok` CLI (`ngrok http 23000`), reads the public URL from the local ngrok API (127.0.0.1:4040), prints the webhook URL and reminds the user to update the App settings; supports a reserved domain via `NGROK_DOMAIN`. The auth token is configured by `ngrok config add-authtoken` (documented) and never read or printed by the script.
  - `verify-github-app.mjs`: loads config, validates the PEM format, signs an App JWT, calls `GET /app` (shows App name, permissions, events) and compares permissions with the manifest; lists installations; flags extra permissions; prints a PASS/FAIL table; never prints the key, secret or tokens. With `--fake` it targets the fake server instead (DEV-005).
  - Doc safety section: rotate the secret if exposed, never paste keys in issues or chats, ngrok free-tier URLs change per run (update the App webhook URL), ngrok inspection UI at 127.0.0.1:4040 shows payloads so only synthetic or test-repository data should be used.
- **Data model changes:** None.
- **API/protocol changes:** None (documents the webhook URL contract).
- **Concurrency semantics:** The tunnel script runs in the foreground; one tunnel per port; the verify script is read-only.
- **Failure behavior:** Missing ngrok binary prints install instructions; unreachable local API prints a hint to run `pnpm dev:up:app`; verification mismatches exit non-zero with a clear diff of permissions.
- **Idempotency considerations:** Safe to re-run; no state is created in GitHub by the scripts (read-only calls).
- **Security considerations:** Least-privilege manifest guarded by tests; secrets stay out of git, logs and argv; the doc instructs using a dedicated test repository and organization; webhook signature verification stays mandatory even over the tunnel; guidance to stop the tunnel when not in use.
- **Observability additions:** After setup, a real delivery appears as a `webhook_received` trace; the doc shows how to find it in OpenObserve by `delivery_id`.
- **Tests required (named):**
  - `manifest_permissions_match_gh010_manifest`
  - `manifest_contains_no_write_contents_or_admin_permissions`
  - `verify_flags_extra_permission`
  - `verify_never_prints_key_secret_or_token` (captured output scan)
  - `tunnel_script_parses_ngrok_local_api_response`
  - `gitignore_covers_pem_env_and_secrets_dir`
  - `doc_links_and_commands_exist` (reuses the DEV-004 checker)
- **Benchmarks if applicable:** None.
- **Acceptance criteria (verifiable):**
  - `node --test scripts/dev/verify-github-app.test.mjs` passes; with a real test App, `pnpm dev:verify-app` shows PASS for permissions and installation.
  - Opening a PR on the test repository produces a 202 response in ngrok's inspector and a trace in OpenObserve.
- **Definition of done:** Acceptance criteria pass; doc reviewed by following it once on a clean setup; link added from `docs/operations/local-development.md`.
