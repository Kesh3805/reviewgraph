# ReviewGraph — Master Implementation Plan

**Date:** 2026-10-02 · **Owner:** Principal Architect · **Executable by:** Sonnet 5.5 (or any engineer), task by task.

**Companion documents:**
- [00 Current-state audit](00-current-state-audit.md)
- [01 PRD gap analysis](01-prd-gap-analysis.md)
- [Target architecture](../architecture/target-architecture.md)
- [ADRs](../decisions/)

**Detailed tasks** live in [`tasks/`](tasks/), one file per phase group. Each task there has all 20 required fields. This file holds the framing, the dependency graph and the index.

---

## 1. Executive Summary

**Where we start.** This repository starts empty. The prior asset was a separate legacy prototype: an agent wrapper for one repository, abandoned and not carried over (audit §7). It had no parser, no graph, no symbol model, no context engine, and only structural verification. The legacy system has also measured the problem the PRD targets:
- 15–21 minute reviews
- 0.6–1.6M tokens per review
- unreliable self-reported completeness
- near-zero cross-model agreement

**What we build.** ReviewGraph is a new system:
- a **Rust data plane** for repository intelligence, reasoning and verification
- a **NestJS control plane** for tenancy, the GitHub App, lifecycle and publication
- a **Next.js** UI
- **PostgreSQL** (metadata, graph, jobs), **Qdrant** (semantic), **Redis** (ephemeral), **GCS / S3-compatible store** (artifacts; SeaweedFS locally)
- **OpenTelemetry → OpenObserve**

**The critical path is the intelligence core, not the UI or the integrations:**

```
TS analyzer → stable symbol identity → CodeGraph + storage → incremental update
  → diff→symbol mapping → change model → impact → context → correctness reviewer → verification → GitHub publication
```

**MVP exit.** A real GitHub PR flows end to end:

```
webhook → incremental index → change model → impact → risk → context
  → correctness + security reviewers → verification → dedup → published inline comments
```

The whole run is visible as one trace in OpenObserve.

---

## 2. Current-State Assessment (summary of 00)

| Area | State | Plan action |
|---|---|---|
| Graph/parsing/symbols/incremental | **MISSING** (external `codegraph` only used by the agent) | Build (Phases 4–8) |
| Diff | git text + new-side line set | Replace (Phase 9) |
| AI | agy CLI, single model, prose passes | Replace with Model Gateway + reviewers (15, 17, 20–24) |
| Verification | structural adjudicator (`policy.rs`) | Port as stage 1; build stages 2–8 (18) |
| Persistence | psql subprocess, interpolated SQL, no migrations | Replace (sqlx migrations) |
| GitHub | `gh` + PAT + polling | Replace with GitHub App (27) |
| UI | single Vite page | Replace with Next.js (30–31) |
| Observability / CI | none | Build (32, 36) |
| Build environment | host cannot build C or `windows-sys` | Linux-container builds (ADR-001) |

---

## 3. Target Architecture (summary)

See [target-architecture.md](../architecture/target-architecture.md).

The PRD §10/§148 package names map onto the implementation as follows:

| PRD package | Implementation |
|---|---|
| core | `engine/crates/review-core` |
| codegraph (builder, nodes, edges, query, incremental, snapshots, storage) | `codegraph`, `incremental`, `graph-storage` |
| languages | `analysis-ir`, `lang-typescript` (future `lang-python`, …) |
| frameworks | adapters inside each language crate (`lang-typescript::frameworks::{nestjs,typeorm,bullmq,jest}`) |
| diff | `diff-engine` |
| context | `context-engine` |
| analysis (static, graph, heuristics, semantic, risk) | `impact` (risk, clustering), `semantic`, `pipeline::tools` |
| reviewers | `reviewers` |
| verification | `verification` |
| repository-profile | `profile` |
| integrations | `apps/api/src/providers/*` |
| llm | `model-gateway` |
| persistence | `graph-storage` + `engine/migrations` + `apps/api/src/db` |
| observability | `telemetry` + `apps/api/src/telemetry` |
| workers | `engine/apps/review-worker` + `apps/api` publisher consumer |
| apps/api, worker, cli, dashboard | `apps/api`, `engine/apps/review-worker`, `engine/apps/review-cli`, `apps/web` |

---

## 4. Architecture Principles

1. **Understand → model → measure → select → reason → verify → comment.** Each arrow is a crate boundary with a typed artifact: `ParsedUnit`, `Graph`, `ChangeModel`, `ImpactGraph`, `ContextPackage`, `CandidateFinding`, `VerifiedFinding`.
2. **Deterministic before probabilistic.** Anything a parser, graph query or tool can establish never goes to a model.
3. **Structural before semantic retrieval** (Invariant 10).
4. **Every budget is explicit.** Traversals, context, tokens, model calls and latency all have one, and truncation is reported, never silent.
5. **Uncertainty is data.** Edges carry confidence, findings carry computed confidence, and suppressions are persisted.
6. **Ports and adapters.** Providers, models, stores and analyzers sit behind traits or interfaces. Domain crates have no I/O.
7. **Idempotent, resumable stages.** Every job carries an idempotency key and every stage persists its output before transitioning.
8. **Generic core, specific edges.** NestJS lives in an adapter. the reference consumer lives in a config and rule pack outside the engine.
9. **No infrastructure without a measured reason.** No Kubernetes, Kafka, Neo4j, ClickHouse or service mesh until a benchmark demands it.
10. **Legacy safety guarantees are kept as invariants:**
    - never merge
    - failure is never approval
    - completeness is computed, not self-reported
    - NOT_EXECUTED ≠ PASS
    - out-of-diff findings go to the summary, never silently dropped

---

## 5. Tech Stack

| Layer | Choice | Notes |
|---|---|---|
| Rust | 1.97, edition 2021, tokio, axum, serde, thiserror/anyhow, tracing(+opentelemetry), clap 4, gix, imara-diff, tree-sitter 0.25 + tree-sitter-typescript 0.23, sqlx 0.8 (postgres, rustls), reqwest 0.12 (rustls), blake3, rayon, criterion, proptest, insta | Built in `rust:1-bookworm` (ADR-001) |
| Control plane | Node 24, NestJS 11, TypeScript 5.x strict, Kysely + pg, ioredis, Octokit (`@octokit/app`, `@octokit/webhooks`), Jest, OTel SDK | pnpm 10 workspace |
| Frontend | Next.js 15 (App Router), React 19, Tailwind 4, shadcn/ui + Radix, Cytoscape.js, React Flow (`@xyflow/react`), TanStack Query | |
| Data | PostgreSQL 16, Qdrant (latest stable), Redis 7, SeaweedFS S3 (local) / GCS (prod) | |
| Observability | OpenTelemetry OTLP/HTTP → OpenObserve | no collector in MVP |
| Jobs | PostgreSQL `jobs` table + SKIP LOCKED + LISTEN/NOTIFY | NATS JetStream later (ADR-012) |
| Models | Anthropic + OpenAI adapters, replay adapter | tiers and routing (ADR-010) |

---

## 6. Key Risks (risk register)

| ID | Risk | Likelihood | Impact | Mitigation | Owner phase | Early signal |
|---|---|---|---|---|---|---|
| R1 | Heuristic call graph too imprecise for verification to rely on | M | H | Confidence-scored edges; NestJS DI resolution; optional TS semantic helper (ADR-007); edge-precision benchmark on labelled fixtures | 4, 6, 34 | Edge precision < 0.85 on fixtures |
| R2 | Stable identity breaks under real refactors | M | H | Lineage matcher + dedicated rename/move suite (SID-006) + reference history replay (IDX-006) | 5 | Lineage miss rate > 5% on replayed history |
| R3 | Incremental graph diverges from full rebuild | M | H | Oracle property test (INC-012), sampled background validator, fail-to-full-rebuild | 8 | Any oracle mismatch |
| R4 | Verification degenerates into "are you sure?" | M | H | LLM-free stages 1–6a; VERIFIER sees only gathered counter-evidence; measured suppression precision | 18 | Verified-but-rejected rate on benchmark |
| R5 | Context ranking misses the decisive caller | M | H | Structural-first candidates; golden context tests; recall@budget metric in EVAL | 14, 16 | Expected finding missed while context lacked its evidence |
| R6 | Latency targets missed (legacy took 15–21 min) | M | M | Budgets, clustering, parallel reviewers, prompt caching, small structured contexts | 15, 34 | Small-PR p95 > 60 s in PERF-008 |
| R7 | Build environment: host can't compile the engine | — (certain) | M | Container builds; CI Windows MSVC build for the native CLI | 1 | — |
| R8 | No API keys / GitHub App in the dev environment | H | M | Replay provider; fake GitHub API server for E2E; documented live setup | 15, 27, 37 | — |
| R9 | Tenant data leakage (Qdrant/PG) | L | Critical | Mandatory `TenantScope` type; RLS; isolation tests in CI | 13, 33 | Isolation test failure |
| R10 | Convention inference enforces accidental patterns | M | M | Sample/consistency thresholds; precedence; maintainability threshold high | 25 | Convention-based FP feedback |
| R11 | Scope explosion (37 phases) delays proof of value | H | H | MVP cut (§16) on the critical path; UI and extra reviewers after core proof | all | Critical-path slip |
| R12 | Supersession races publish obsolete findings | M | H | CAS state transitions + publish-time head check in the same transaction (SUP-003) | 28 | SUP-004 race tests |

---

## 7. Dependency Graph

```
FND (1) ──▶ DOM (2) ──┬──▶ INIT (3) ───────────────────────────────┐
                      ├──▶ TSA/NEST (4) ──▶ SID (5) ──▶ CG/GS (6) ──▶ IDX (7) ──▶ INC (8)
                      │                                       │                      │
                      │                                       └──────▶ DIFF (9) ◀────┘
                      │                                                   │
                      │                                       CHG (10) ◀──┘
                      │                                          │
                      │                          IMP (11) ◀──────┤──▶ RISK (12)
                      │                             │                  │
                      ├──▶ SEM (13) ───────────────▶ CTX (14) ◀────────┘
                      ├──▶ GW (15) ──▶ EVAL (16)       │
                      │                  │             ▼
                      │                  └────────▶ REV-C (17) ──▶ VER (18) ──▶ DED (19) ──▶ PIPE (19A)
                      │                                                                       │
                      ├──▶ API (27A) ──────────────────────────────────────────────▶ GH (27) ◀┘ ──▶ SUP (28)
                      ├──▶ OBS (32) [starts in parallel at phase 2; instrumented per phase]
                      └──▶ CI (36) [starts at phase 1; grows with each phase]

After VER: REV-S (20) ∥ REV-T (21) ∥ REV-A (22) ∥ REV-P (23) ∥ REV-M (24) ; PROF (25) → POL (26) → REV-A
CLI (29) grows alongside: init after INIT, graph after CG, diff after DIFF, impact after IMP, pr after PIPE.
WEB (30) after API; GX (31) after API-013. SEC (33), PERF (34), QB (35) close the loop.
```

**Parallel lanes.** Lanes that can run at the same time, each with one owner:

| Lane | Phases | Starts after |
|---|---|---|
| A — Intelligence core (critical path) | 4 → 5 → 6 → 7 → 8 | DOM |
| B — Diff/change | 9, 10 | 6 is available (IR) |
| C — Model plane | 15 → 16 | DOM |
| D — Semantic | 13 | DOM; integrates in 14 |
| E — Control plane | API-*, then GH-* | DOM (migrations) |
| F — Foundations | OBS, CI, DEV | continuous |
| G — UI | WEB/GX | API-008..011 |

**Critical path.** These have no slack:

```
TSA-001 → TSA-003 → TSA-005 → SID-001 → SID-004 → CG-004 → CG-005 → GS-004 → GS-005 → IDX-001
  → INC-005 → INC-006 → INC-009 → DIFF-006 → CHG-007 → IMP-002 → CTX-006 → REV-C-002
  → VER-004 → VER-006 → VER-009 → PIPE-003 → GH-009 → SUP-003 → E2E-001
```

High-risk nodes on that path:
- SID-005 (rename matcher)
- INC-006 (inbound re-link)
- TSA-009 / CG-005 (resolution precision)
- VER-006 (base/head)
- CTX-005 (ranking)

---

## 8. Milestones

| Milestone | Contents | Exit criteria |
|---|---|---|
| **M0 Baseline** | Phase 0 docs | All five planning deliverables exist and are reviewed ✅ |
| **M1 Foundation** | Phases 1–2 | `engine/scripts/cargo.sh test` green; `pnpm -r test` green; `docker compose up` brings pg/redis/qdrant/openobserve/object-store healthy; migrations apply |
| **M2 Graph** | Phases 3–7 | `review init` + full index on fixture repos and reference-api; golden IR/graph tests; parity report vs external codegraph |
| **M3 Incremental** | Phase 8 | One-file change updates only that region; oracle property test passes 1,000 random edits; counters prove no unchanged reparse |
| **M4 Change & Impact** | Phases 9–12 | Golden diff → changed symbols → change classes → impact graph for all fixture PRs, incl. the auth-bypass scenario |
| **M5 First review offline** | Phases 13–19A | `review diff` on the auth-bypass fixture produces the §151 finding under the replay provider; verification suppresses the planted false-positive traps |
| **M6 MVP** | Phases 20 (security), 25–29, 30 (basic), 32 (basic) | E2E-001 green: signed webhook → … → published comment against a fake GitHub API; trace visible in OpenObserve. E2E-002 run manually against a real repo with a real App + model key |
| **M7 Production readiness** | Phases 21–24, 31, 33–37 | §17 Definition of Production Readiness |

---

## 9. Implementation Phases → task files

Task details (all 20 fields) live in [`tasks/`](tasks/). Status markers are kept up to date: ☐ todo · ◐ in progress · ☑ done (acceptance criteria verified).

| Phase | Title | Task file | Task IDs |
|---|---|---|---|
| 0 | Audit & architecture baseline | (this doc set) | ARCH-001..005 ☑ |
| 1 | Monorepo / build foundation | [tasks/P01-P02-foundation-domain.md](tasks/P01-P02-foundation-domain.md) | FND-001..008 |
| 2 | Core domain model | same | DOM-001..010 |
| 3 | Repository initialization | [tasks/P03-P05-init-parser-identity.md](tasks/P03-P05-init-parser-identity.md) | INIT-001..013 |
| 4 | Parser & language analyzer (TS + NestJS) | same | TSA-001..010, NEST-001..007 |
| 5 | Stable symbol identity | same | SID-001..006 |
| 6 | Persistent CodeGraph | [tasks/P06-P08-graph-index-incremental.md](tasks/P06-P08-graph-index-incremental.md) | CG-001..012, GS-001..008 |
| 7 | Full initial index | same | IDX-001..006 |
| 8 | Incremental indexing | same | INC-001..013 |
| 9 | Git & diff engine | [tasks/P09-P14-diff-change-impact-risk-semantic-context.md](tasks/P09-P14-diff-change-impact-risk-semantic-context.md) | DIFF-001..007 |
| 10 | Semantic change model | same | CHG-001..009 |
| 11 | Impact graph | same | IMP-001..010 |
| 12 | Risk engine | same | RISK-001..006 |
| 13 | Qdrant semantic intelligence | same | SEM-001..009 |
| 14 | Context selection engine | same | CTX-001..010 |
| 15 | Model gateway | [tasks/P15-P19A-models-reviewers-verification-pipeline.md](tasks/P15-P19A-models-reviewers-verification-pipeline.md) | GW-001..010 |
| 16 | Model evaluation harness | same | EVAL-001..006 |
| 17 | Correctness reviewer | same | REV-001..002, REV-C-001..004 |
| 18 | Verification engine | same | VER-001..012 |
| 19 | Finding dedup & prioritization | same | DED-001..004 |
| 19A | Review pipeline & jobs | same | PIPE-001..011 |
| 20–24 | Security, Test, Architecture, Performance, Maintainability reviewers | [tasks/P20-P26-reviewers-profile-policy.md](tasks/P20-P26-reviewers-profile-policy.md) | REV-S-001..003, REV-T-001..002, REV-A-001..002, REV-P-001..002, REV-M-001 |
| 25 | Repository profile | same | PROF-001..007 |
| 26 | Explicit repository rules | same | POL-001..007 |
| 27A | Control-plane foundation (NestJS + engine API) | [tasks/P27-P29-api-github-supersession-cli.md](tasks/P27-P29-api-github-supersession-cli.md) | API-001..013 |
| 27 | GitHub integration | same | GH-001..013 |
| 28 | Review supersession | same | SUP-001..004 |
| 29 | CLI | same | CLI-001..013 |
| 30–31 | Frontend & graph explorer | [tasks/P30-P37-ui-ops-quality.md](tasks/P30-P37-ui-ops-quality.md) | WEB-001..009, GX-001..003 |
| 32 | OpenObserve integration | same | OBS-001..008 |
| 33 | Security hardening | same | SEC-001..010 |
| 34 | Benchmark & performance hardening | same | PERF-001..008 |
| 35 | Quality benchmark | same | QB-001..006 |
| 36 | CI/CD | same | CI-001..009 |
| 37 | Local development | same | DEV-001..006 |
| X | End-to-end, invariants, post-MVP backlog | [tasks/PX-e2e-invariants-post-mvp.md](tasks/PX-e2e-invariants-post-mvp.md) | E2E-001..002, INV-001..015, HIST-001..004, MP-001..002, LANG-*-001 |

The LEG tasks (freeze or remove the legacy prototype) were dropped. The prototype lives in its own abandoned repository, so there is nothing to freeze or remove here.

**Ordering note.** The PRD phase list is preserved except in two places:
1. **Observability (OBS-001..004) and CI (CI-001..003) start in Phase 1.** Every later phase adds its spans and metrics as part of its own definition of done. Retrofitting telemetry at Phase 32 would be more expensive and would blind the earlier benchmarks.
2. **The control-plane foundation (27A) and the pipeline (19A) are explicit phases.** The PRD list leaves them implicit.

---

## 10. Task Breakdown

See the task files. Each task has these 20 fields:

1. Task ID
2. Title
3. Problem
4. Why it exists
5. Scope
6. Explicit non-scope
7. Files/modules expected to change
8. New files/modules
9. Dependencies
10. Implementation details
11. Data model changes
12. API/protocol changes
13. Concurrency semantics
14. Failure behavior
15. Idempotency
16. Security
17. Observability
18. Tests
19. Benchmarks
20. Acceptance criteria, and the Definition of done

**Global Definition of Done** (applies to every task, in addition to its own):
- `engine/scripts/cargo.sh fmt --check`, `clippy -D warnings` and `test` are green for affected crates.
- `pnpm -r lint typecheck test` is green for affected packages.
- No `unwrap()`/`expect()` outside tests and `main`-level startup. No panics for normal failures.
- New behaviour has tests at the level the task specifies.
- Spans and metrics named by the task exist.
- Docs touched by the change are updated in the same change.
- The task is marked ☑ in its task file only after its acceptance criteria were executed and passed.

---

## 11. Testing Strategy

| Level | What | Where | Tooling |
|---|---|---|---|
| Unit | symbol IDs, IR extraction, graph mutations, ranking, risk rules, confidence, dedup, router | each crate `#[cfg(test)]` | cargo test, proptest, insta snapshots |
| Golden | IR per fixture file; graph per fixture repo; diff → changed symbols; change classes; impact graph; context packages | `fixtures/` + `insta` snapshots | `cargo insta review` |
| Property | incremental == full rebuild under random edit sequences; BFS budget never exceeded; context budget never exceeded | `incremental`, `codegraph`, `context-engine` | proptest |
| Integration | PostgreSQL GraphStore, jobs, Qdrant, Redis rate limiter, gix on real repos, provider/model adapters (HTTP mocked with `wiremock`) | `engine/crates/*/tests/` + `apps/api/test/` | docker services via `infra/compose/docker-compose.test.yml` |
| Fixture repositories | call graph, inheritance, NestJS routes/guards/DI, TypeORM, BullMQ, tests, renames, moves, cross-module calls, monorepo | `fixtures/repositories/*` built by `fixtures/build.sh` into git repos with scripted history | |
| Review benchmarks | known bad/safe PRs with required, forbidden and optional findings | `benchmarks/quality/` | EVAL runner |
| End-to-end | signed GitHub webhook → fake GitHub API (wiremock-style Node server) → checkout from local bare repo → index → review (replay) → verify → publish → assertions on the received review payload | `tests/e2e/` | docker compose test profile |
| Invariants | INV-001..015 as executable tests | across crates/apps | CI-required |

---

## 12. Benchmark Strategy

- **Performance** (`criterion` + `benchmarks/perf`):
  - parse throughput (files/s)
  - full graph build and memory at 10k/100k files
  - neighbor lookup p50/p95 (memory and SQL)
  - bounded BFS
  - incremental update latency vs. changed-file count
  - context selection latency
  - Qdrant filtered search latency
  - end-to-end review latency under replay
- **Synthetic repository generator.** PERF-001 produces a NestJS-shaped repository with 100k files and about 1M symbols, deterministic from a seed.
- **Real repository.** `reference-api`: 1,028 files, about 15k symbols. Parity against the external codegraph counts is reported in IDX-006.
- **Quality.** The EVAL harness reports precision, recall, FP rate, latency, tokens, cost and structured-output success per model configuration.
- **Gates.**
  - CI smoke runs a 5-case subset under replay.
  - The nightly run covers the full corpus.
  - A regression beyond the thresholds (precision −3 pts, FP +3 pts, p95 latency +20%) fails the run.

---

## 13. Security Strategy

1. **Tenant isolation.**
   - Every table carries `organization_id`, with Postgres RLS on tenant tables.
   - The app sets `app.organization_id` per request transaction.
   - Engine queries always include org/repo predicates.
   - Qdrant searches require a `TenantScope`.
   - Object-store keys are prefixed by org/repo, with signed URLs only.
2. **Credentials.**
   - The GitHub App private key comes from env or a secret manager and is never logged.
   - Installation tokens are cached encrypted in Redis with TTL < expiry and are never persisted.
   - Workers receive clone tokens through the internal credential endpoint and keep them in memory only.
3. **Least privilege.** GitHub App permissions:
   - `contents: read`
   - `pull_requests: write`
   - `checks: write`
   - `metadata: read`

   There is **no merge capability**: the code path does not exist, and a test guards it.
4. **Webhooks.** HMAC-SHA256 with constant-time comparison, delivery-id dedup, and a timestamp/replay window.
5. **Model data.**
   - Secret detection runs at index time.
   - The redaction pass runs before any model send.
   - The privacy policy can disable external providers per repository.
   - Prompts are never logged.
6. **Source handling.**
   - Checkouts live in per-job temp dirs and are wiped after the job.
   - Retention policies are enforced (SEC-007).
   - Source is never stored in Redis or in logs.
7. **Supply chain.** `cargo deny` (advisories, licenses, bans), `pnpm audit`, pinned images, and an SBOM per image.
8. **Audit log.** Covers configuration changes, publication and feedback.

---

## 14. Observability Strategy

ADR-013 and target-architecture §8 apply.

**Every phase owns its telemetry.** Each phase contributes its spans and metrics. OBS-007/008 add the dashboards and alerts:

| Dashboards | Alerts |
|---|---|
| review latency | review p95 > 2 min for 15 min |
| index latency | dead jobs > 0 |
| model usage / token / cost | webhook signature failures spike |
| findings funnel (candidate → verified → published → accepted) | model error rate > 5% |
| false-positive feedback | queue wait p95 > 60 s |
| worker health | Qdrant p95 > 500 ms |
| Qdrant latency | PG connection saturation |
| PostgreSQL latency | FP feedback rate > 15% weekly |
| queue depth | |

---

## 15. Deployment Strategy

- **Local:** `infra/compose` (DEV-001). One command: `pnpm dev:up` (wraps `docker compose up`).
- **Images:** `engine` (review-worker, review-engine, review-cli in one image with multiple entrypoints), `api`, `web`. All are non-root, read-only root FS and `cap_drop: ALL`.
- **Target environment (GCP):**
  - Cloud Run: api, web, review-engine
  - review-worker on GCE MIG or Cloud Run Jobs, with an ephemeral SSD for checkouts
  - Cloud SQL PG16, Memorystore Redis, GCS
  - Qdrant on a VM or Qdrant Cloud
  - OpenObserve self-hosted
- **Rollout.**
  - Migrations run as a pre-deploy job and are backward-compatible for one release (expand/contract).
  - Workers drain on SIGTERM: they stop claiming, finish or release leases, and exit within 60 s.
- **Backup and recovery.** PG PITR. Qdrant snapshots are rebuildable from PG plus source, so they are not authoritative. A runbook lives in `docs/operations/backup-recovery.md` (SEC/DEV tasks).

---

## 16. Definition of MVP

**Included:**
- TypeScript plus NestJS-aware analysis (with TypeORM/BullMQ/Jest adapters)
- GitHub App
- `review init`
- persistent CodeGraph (PG + local file store)
- incremental indexing
- stable symbols with lineage
- diff → symbol mapping
- change classification
- impact graph
- Qdrant semantic augmentation
- risk classification
- context selection
- correctness and security reviewers
- verification (all 8 stages)
- dedup
- inline comments plus review summary plus check run
- supersession
- Rust CLI
- basic web UI: repositories, PR list, review detail, finding detail, feedback
- OpenObserve telemetry (traces, metrics, logs, core dashboards)

**Not included:**
- GitLab and Bitbucket
- additional languages
- review-history learning
- runtime tracing
- multi-region
- a graph database
- automatic code fixes
- test, architecture, performance and maintainability reviewers (Phase 21–24, after MVP)
- the full graph explorer (GX basic search only)

### MVP exit criteria (verified by E2E-001 plus the E2E-002 manual run)

1. GitHub webhook received and signature-verified, then the PR is normalized.
2. The repository is checked out (bare mirror) and the base graph is loaded.
3. Only the changed files are parsed, the graph is updated (delta snapshot), and the changed symbols are derived.
4. The impact graph is computed, risk is classified, and targeted context is selected within budget.
5. The correctness and security reviewers run, and their candidates are verified.
6. Duplicates are removed, and only high-confidence findings are published inline, with a summary.
7. The entire workflow is inspectable as one trace in OpenObserve.

---

## 17. Definition of Production Readiness

| Requirement | Verified by |
|---|---|
| Tenant isolation proven | SEC-001, SEC-002 in CI |
| Webhook idempotency | GH-003 + SUP-004 duplicate-delivery test |
| Review supersession | SUP-001..004 |
| Retries + dead-letter | PIPE-001/002 tests; dead-job alert |
| Secret redaction | SEC-003/004, OBS-006 |
| Provider-token security | SEC-005 |
| Qdrant isolation/filtering | SEM-005, SEC-002 |
| PostgreSQL migrations | CI-005 |
| Observability + alerts | OBS-001..008 |
| Load tests | PERF-008 + `benchmarks/perf/load` (k6 against the webhook endpoint, 50 PR events/min) |
| Incremental-index benchmarks | PERF-005 within §119 targets |
| Benchmark PR suite | QB-001..003 gate in CI |
| Backup/recovery docs | `docs/operations/backup-recovery.md` |
| Graceful shutdown | PIPE-010 test (SIGTERM releases leases) |
| Dependency scanning | CI-002 |
| CI/CD | CI-001..009 |
| Operational runbooks | `docs/operations/runbooks/*.md` (one per alert) |

---

## 18. Execution protocol

1. Implement in dependency order. Run the parallel lanes (§7) concurrently with separate owners or subagents.
2. Before each task, re-read the affected code. If a plan assumption is stale, fix the plan first (edit the task file, and add an ADR if it is architectural).
3. Run the tests after each coherent change. Run `clippy` and `fmt` continuously.
4. Mark ☑ only after the acceptance criteria are executed.
5. Never defer a hard requirement silently. A deferral must be written into the task file with a reason and a new task ID.
