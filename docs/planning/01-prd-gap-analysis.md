# 01 — PRD Gap Analysis

**Date:** 2026-10-02
**Sources:** [ReviewGraph PRD](../product/PRD.md) (§1–§154), [current-state audit](00-current-state-audit.md), [target architecture](../architecture/target-architecture.md).

Every PRD section is accounted for below. "Current" refers to the repository as audited. In practice almost everything is **MISSING**, because the existing product is an agent wrapper (audit §1).

The task-ID prefixes refer to [`MASTER_IMPLEMENTATION_PLAN.md`](MASTER_IMPLEMENTATION_PLAN.md):

| Prefix | Area |
|---|---|
| FND | Foundation |
| DOM | Domain model |
| INIT | `review init` |
| TSA | TypeScript analyzer |
| NEST | NestJS adapters |
| SID | Symbol identity |
| CG | CodeGraph |
| GS | Graph storage |
| IDX | Full index |
| INC | Incremental indexing |
| DIFF | Diff engine |
| CHG | Change model |
| IMP | Impact graph |
| RISK | Risk engine |
| SEM | Semantic / Qdrant |
| CTX | Context engine |
| GW | Model gateway |
| EVAL | Model evaluation |
| REV | Reviewers |
| VER | Verification |
| DED | Dedup and prioritization |
| PROF | Repository profile |
| POL | Policy / rules |
| PIPE | Pipeline and jobs |
| API | NestJS foundation |
| GH | GitHub integration |
| SUP | Supersession |
| CLI | CLI |
| WEB | Web UI |
| GX | Graph explorer |
| OBS | Observability |
| SEC | Security hardening |
| PERF | Performance hardening |
| QB | Quality benchmark |
| CI | CI/CD |
| DEV | Local development |
| HIST | Historical intelligence |
| MP | Multi-provider |
| LEG | Legacy retirement |

Complexity: **S** <1 day · **M** 1–3 days · **L** 3–10 days · **XL** >10 days. Risk: **L/M/H**, meaning the risk of getting it wrong or blowing the schedule.

---

## A. Product framing (§1–§11)

| PRD | Requirement | Current | Gap | Deps | Cx | Risk | Implementation |
|---|---|---|---|---|---|---|---|
| §1 | Pipeline repo → intelligence → change model → impact → context → reviewers → verification → findings; never "diff → giant prompt" | Agent explores repo with shell tools; diff file handed to agent (`daemon.rs:189`, `passes.rs`) | Entire pipeline | — | XL | H | Whole plan; architecture enforces stage boundaries (`pipeline` crate) |
| §2 | Reusable intelligence platform; review logic decoupled from parsing/graph | Single crate, no layering | Layering | FND | M | L | Crate DAG (target-arch §2.1) + `cargo deny` bans (FND-004) |
| §3 | 12 differentiators | none present | all | — | — | — | Mapped individually below |
| §4.1–4.5 | Diff blindness, retrieval waste, rediscovery, FP saturation, generic review | Agent-dependent; FP control = legacy adjudicator only | Graph, incremental, verification, profile | — | — | H | CG, INC, CTX, VER, PROF |
| §5 | 15 product goals | — | all | — | — | — | Covered by phases 3–37 |
| §6 | Non-goals (no merge, no autonomous fix, no SAST duplication) | No-merge enforced by test (`github.rs:356`) | Port the guarantee | GH | S | L | GH-010 (no-merge permission manifest + test) |
| §7 | Personas | — | — | — | — | — | Drives UI (WEB) + CLI design |
| §8–§9 | `review init` then `review pr`/webhook; end-to-end flow | Poller + agy | all | — | — | — | CLI-*, GH-*, PIPE-* |
| §10 | Package layout | One crate | Workspace | FND | M | L | Adapted to Rust crates (target-arch §2); mapping table in master plan §3 |
| §11 | Ports: RepositoryProvider, ReviewPublisher, LanguageAnalyzer, GraphStore, ModelGateway | None | All five ports | DOM | M | M | TSA-001 (LanguageAnalyzer), GS-001 (GraphStore), GW-001 (ModelGateway), API-006 (RepositoryProvider/ReviewPublisher) |

## B. Repository initialization and state (§12–§15)

| PRD | Requirement | Current | Gap | Deps | Cx | Risk | Implementation |
|---|---|---|---|---|---|---|---|
| §12 | `review init [--repository] [--provider] [--force]` | `pr-review init-config` writes a starter TOML only | Command | CLI, INIT | M | L | CLI-002 |
| §13 | Detect 22 facts (languages, frameworks, package managers, build, tests, roots, generated, manifests, workspaces, entrypoints, routes, CLI/worker entrypoints, migrations, infra, auth boundaries, lint, compiler, CI, arch metadata, rule docs) | none (`doctor` checks paths only) | All detectors | FND | L | M | INIT-001..INIT-010 |
| §14 | `.review/` persistent layout | none | Layout + file store | GS | M | L | INIT-011, GS-006 (file adapter) |
| §15 | Fingerprint over repo, commit, analyzer, schema, config, parser, profile versions | none | Fingerprint | DOM | S | L | INIT-012, ADR-015 |

## C. CodeGraph (§16–§24)

| PRD | Requirement | Current | Gap | Deps | Cx | Risk | Implementation |
|---|---|---|---|---|---|---|---|
| §16 | CodeGraph is primary structural model | External codegraph used by the agent only | Own graph | — | XL | H | CG-*, GS-* |
| §17 | 46 node types | external: 13 kinds in use | Full enum + extraction for TS subset | DOM | M | M | CG-001 (enum), TSA/NEST extraction |
| §18 | 35 edge types, versioned schema | external: 8 kinds in use | Enum, versioning, reverse views | DOM | M | M | CG-002, CG-010 (schema version) |
| §19 | Node/edge metadata incl. confidence, visibility, generated, framework metadata | external has confidence in JSON | Typed metadata | CG | M | M | CG-002, CG-003 (confidence table) |
| §20 | Stable symbol identity, renames as transformations | external hashes path (unstable) | Identity + lineage | DOM | L | **H (critical path)** | SID-001..SID-006 |
| §21 | Never full rebuild per PR | external mutates one working index | Snapshots + overlays | GS, SID | XL | **H** | INC-*, ADR-003/004 |
| §22 | 9-step incremental algorithm | none | all | INC | L | **H** | INC-001..INC-010 |
| §23 | Dependency-aware invalidation | none | Invalidation sets | INC | M | H | INC-008 |
| §24 | Full-rebuild conditions + `review graph rebuild` | none | Triggers | INC, CLI | S | L | INC-011, CLI-010 |

## D. Diff and change model (§25–§29)

| PRD | Requirement | Current | Gap | Deps | Cx | Risk | Implementation |
|---|---|---|---|---|---|---|---|
| §25 | Diff engine from base/head + provider metadata | `git diff` text, `DiffMap` = new-side line set (`git.rs:252-323`) | Hunk model, renames, deletions, binary | FND | M | M | DIFF-001..DIFF-005 |
| §26 | PullRequestChangeModel (files, symbols, APIs, deps, schemas, configs, tests, risk) | none | Model | DIFF, CG | M | M | CHG-001, CHG-006..CHG-008 |
| §27 | Line → File → Class → Method + semantic changes | none | Mapping | TSA, DIFF | M | H | DIFF-006, DIFF-007 |
| §28 | 15 AST change classes | none | Classifier over syntax facts | TSA | L | H | CHG-002..CHG-005 |
| §29 | Intent classification (11 classes) | none | Deterministic + CLASSIFIER | CHG, GW | M | M | CHG-009 |

## E. Impact, context, risk (§30–§39)

| PRD | Requirement | Current | Gap | Deps | Cx | Risk | Implementation |
|---|---|---|---|---|---|---|---|
| §30–§31 | Impact graph per changed symbol (callers, callees, impls, tests, APIs, config, DB, queues) | Agent may run `codegraph explore` | All | CG, CHG | L | H | IMP-001..IMP-008 |
| §32 | Changed symbols → expansion → ranking → budget → compressed package | none (agent decides) | Engine | IMP | L | **H** | CTX-001..CTX-010 |
| §33 | 10 relevance signals, none dominating | none | Ranking | CTX | M | H | CTX-005 |
| §34 | Distance heuristic 0/1/2/3/4+ | the reference consumer `agent-context.js` has depth penalty | Port concept | CTX | S | L | CTX-005 |
| §35 | Per-reviewer budgets, stoppable expansion | the reference consumer budgets 1000/2500/6000 tokens | Budgets | CTX | M | M | CTX-006 |
| §36 | Compression preserving source locations | none | Compressor | CTX | M | M | CTX-007 |
| §37 | 18 risk categories pre-review | consumer-only prompt checklists (`passes.rs:106-131`) | Engine | CHG | M | M | RISK-001..RISK-004 |
| §38 | Risk controls depth/budget/reviewers/verification/model/tests/threshold | none | Effects | RISK | M | M | RISK-005 |
| §39 | Low-risk suppression (formatting, comments, pure rename, snapshots) while catching contract changes | none | Classifier | CHG | M | M | RISK-006 |

## F. Review architecture and reviewers (§40–§48)

| PRD | Requirement | Current | Gap | Deps | Cx | Risk | Implementation |
|---|---|---|---|---|---|---|---|
| §40 | Deterministic analysis before LLM | Static gates (typecheck/lint/depcruise) in parallel with agent (`validate.rs`) | Ordering + use as evidence | PROF | M | M | VER-005 (deterministic evidence), PIPE-004 (ported gate runner as `analysis` stage) |
| §41 | Multi-stage, no giant prompt | 6 passes in agent sessions | Reviewer orchestration | CTX, GW | M | M | REV-001, PIPE-003 |
| §42 | Correctness reviewer | Diff pass prompt | Reviewer | REV-001 | L | H | REV-C-001..REV-C-004 |
| §43 | Security reviewer | the reference consumer tenant checklist | Reviewer | REV | L | H | REV-S-001..REV-S-003 |
| §44 | Test reviewer | "tests" pass | Reviewer | IMP test mapping | M | M | REV-T-001..REV-T-002 |
| §45 | Performance reviewer | prompt bullet | Reviewer | REV | M | M | REV-P-001..002 |
| §46 | Architecture reviewer | depcruise gate only | Reviewer + profile | PROF, POL | M | M | REV-A-001..002 |
| §47 | Maintainability reviewer, high threshold | none | Reviewer | REV | M | L | REV-M-001 |
| §48 | Reviewer routing (incl. "database safety") | all passes always | Router | RISK | M | M | REV-002. **PRD gap:** "database safety" reviewer is not among the six. Decision: implemented as a correctness-reviewer *focus profile* (prompt section enabled by `database_write_changed`/migration risk), not a 7th reviewer — recorded in REV-002. |

## G. Findings, verification, comments (§49–§61)

| PRD | Requirement | Current | Gap | Deps | Cx | Risk | Implementation |
|---|---|---|---|---|---|---|---|
| §49 | CandidateFinding model | `Finding` (file, single line, free category, model confidence) `core.rs:73-104` | Symbols, structured evidence, reviewer, artifacts | DOM | S | L | DOM-006 |
| §50 | Verification sequence | Legacy adjudicator: file/line/evidence-length/confidence (`policy.rs:46-142`) | Anchor, graph, repo, base/head, contradiction, actionability | VER | XL | **H (critical)** | VER-001..VER-012 |
| §51 | Base vs head | none | Predicate re-eval on base | INC (base graph), VER | L | H | VER-006 |
| §52 | Evidence model (12 types), ≥1 strong | free text `evidence` | Typed evidence | DOM | M | M | DOM-007, VER-002 |
| §53 | Contradiction pass | none | Deterministic + VERIFIER | VER, GW | L | H | VER-007, VER-008 |
| §54 | Computed confidence formula | model self-confidence | Formula + calibration | VER | M | H | VER-009, QB-004 (calibration) |
| §55 | Thresholds 0.55/0.70/0.85, calibrated | `min_confidence_to_block=0.80` | Thresholds | VER | S | M | VER-010 |
| §56 | Dedup by location/symbol/root cause/similarity/evidence overlap | fingerprint(file,line,first sentence) incl. severity label (`store.rs:672`) | Root-cause dedup + merge record | VER | M | M | DED-001..DED-003 |
| §57 | Prioritization factors, no inflation | severity only | Priority score | DED | S | L | DED-004 |
| §58–§59 | Comment format + 5 questions | `report.rs`/`github.rs:274-303` format | Template with evidence path | DED | S | L | GH-007 |
| §60 | Comment suppression rules | partial (P4 discard) | Policy filters | VER, POL | S | L | VER-011 |
| §61 | Review summary (changed, risk, findings by severity, verified X/Y, suppressed N) | `report.rs` summary (no risk/verified counts) | New summary | DED | S | L | GH-008 |

## H. Repository profile, rules, history (§62–§70)

| PRD | Requirement | Current | Gap | Deps | Cx | Risk | Implementation |
|---|---|---|---|---|---|---|---|
| §62 | Auto-generated profile (13 aspects) | the reference consumer vault (hand-written) | Inference | CG, INIT | L | M | PROF-001..PROF-006 |
| §63–§64 | Convention discovery with samples/consistency/exceptions/confidence | none | Inference engine | PROF | L | H (false conventions, R3) | PROF-003..PROF-005 |
| §65 | Precedence policy > docs > convention > generic | none | Resolver | POL | S | L | POL-004 |
| §66 | Rules YAML (forbidden deps, queue ids, migrations-only, tests) | the reference consumer lints in reference repo | Rule evaluators | POL, CG | M | M | POL-001..POL-005 |
| §67–§69 | Historical intelligence + anti-reinforcement | finding OPEN/FIXED by title match | Post-MVP | GH feedback | L | M | HIST-001..HIST-004 (post-MVP, kept in plan) |
| §70 | Feedback loop (useful/FP/handled/not relevant/intentional) | none | API + UI + storage | API, WEB | M | L | API-012, WEB-009 |

## I. Caching, scheduling, idempotency, cancellation (§71–§77)

| PRD | Requirement | Current | Gap | Deps | Cx | Risk | Implementation |
|---|---|---|---|---|---|---|---|
| §71–§73 | 10 caches, explicit keys, review cache reuse | none | Caches per target-arch §7 | many | M | M | IDX-005 (parse cache), CTX-009, GW-008 (response cache), VER-012, PROF-006 |
| §74 | Parallel reviewers and verification | std threads fan-out in agent | Tokio orchestration | PIPE | M | M | PIPE-003 |
| §75 | 7 queues, ID-only payloads | none (serial poller) | PG job queue | FND | M | M | PIPE-001, PIPE-002, API-007 |
| §76 | Idempotency keys per stage | claim key `(repo,pr,head)` | Stage keys | PIPE | M | H | PIPE-005, GH-003 |
| §77 | Supersession; no obsolete comments | head check only at prepare (`daemon.rs:161`) | Supersession at every stage + publish gate | PIPE, GH | M | **H** | SUP-001..SUP-004 |

## J. Providers and CLI (§78–§86)

| PRD | Requirement | Current | Gap | Deps | Cx | Risk | Implementation |
|---|---|---|---|---|---|---|---|
| §78 | Provider adapter responsibilities incl. signature verification, stale comment resolution | gh CLI subprocess | Port + GitHub adapter | API | L | M | API-006, GH-001..GH-012 |
| §79 | GitHub App, events, inline comments, summary check, status | PAT + polling | All | API | L | M (environment: no public ingress, legacy §84) | GH-*; DEV-005 (tunnel/replay webhooks); GH-012 polling reconciler fallback |
| §80–§81 | GitLab, Bitbucket | none | Post-MVP | GH port | L | L | MP-001, MP-002 |
| §82 | Local CLI mode (init, diff, branch, pr, graph inspect, impact, profile, doctor) | `pr-review` subcommands (different) | New CLI | all engine | M | L | CLI-001..CLI-012 |
| §83 | `review doctor` checks | doctor (the reference consumer/agy-specific, `main.rs:245-413`) | Generic checks | CLI | S | L | CLI-004 |
| §84 | Graph debugging commands incl. `path`, `tests` | none | Commands | CG | S | L | CLI-007..CLI-009 |
| §85–§86 | Explainability / internal trace (finding → candidate → verification evidence) | activity events (no prose) | Trace model | VER, OBS | M | M | VER-002 (evidence persisted), WEB-007, OBS-004 |

## K. Models (§87–§92)

| PRD | Requirement | Current | Gap | Deps | Cx | Risk | Implementation |
|---|---|---|---|---|---|---|---|
| §87 | Provider-neutral gateway + router | agy only | Gateway | DOM | L | M | GW-001..GW-010 |
| §88 | Models do not parse/diff/count | agent does everything | Architectural | — | — | — | Enforced by design: reviewers only receive ContextPackage |
| §89 | Structured model input contract | prose prompts | Contract | CTX | S | L | REV-001 (input schema) |
| §90 | Budget manager (symbols, expansion, tokens, candidates, calls, latency) | `print_timeout` only | Budget manager | GW, CTX | M | M | PIPE-006 |
| §91–§92 | Large PR clustering, disclose skipped regions | none | Clustering | IMP | M | M | IMP-009, IMP-010 |

## L. Languages and frameworks (§93–§100)

| PRD | Requirement | Current | Gap | Deps | Cx | Risk | Implementation |
|---|---|---|---|---|---|---|---|
| §93 | Generated code detection, not LLM-reviewed | none | Detector | INIT | S | L | INIT-008, REV-002 (routing skips generated) |
| §94 | Monorepo workspaces + cross-package deps | none | Workspace detection + resolver | INIT, TSA | M | M | INIT-005, TSA-009 |
| §95 | Common Symbol/Edge IR | none | IR | — | M | M | TSA-001 |
| §96 | Language priority TS → Py → Java → Go → Rust | none | TS in MVP; others post-MVP | TSA | XL | M | LANG-PY-*, LANG-JAVA-*, LANG-GO-*, LANG-RS-* (post-MVP, kept as planned tasks) |
| §97 | TS reference analyzer: ESM/CJS, types, classes, decorators, NestJS, TypeORM, BullMQ, Jest | none | Analyzer + adapters | TSA | XL | **H** | TSA-002..TSA-010, NEST-001..NEST-006 |
| §98 | Modular framework adapters (8 frameworks) | none | Adapter trait; NestJS/TypeORM/BullMQ/Jest in MVP; Express later | TSA | M | L | NEST-*, FW-EXPRESS-001 (post-MVP) |
| §99 | Test mapping (imports, invocation, naming, mocks, path conventions) | the reference consumer `getTests` = callers in test files | Mapper | CG | M | M | IMP-005 |
| §100 | Runtime data (future) | none | Post-MVP | — | — | — | RUNTIME-001 (backlog) |

## M. Storage, API, state machine (§101–§109)

| PRD | Requirement | Current | Gap | Deps | Cx | Risk | Implementation |
|---|---|---|---|---|---|---|---|
| §101–§103 | Metadata DB, graph store, blob store, cache, vector store; relational graph tables + indexes | 5 ad-hoc tables, psql subprocess | Schema + migrations | FND | L | M | GS-002..GS-005, DOM-009 (migrations) |
| §104 | Snapshot model, base + delta | none | Snapshots | GS | L | H | GS-004, INC-009 |
| §105 | Intelligence versioning | none | Version fields | DOM | S | L | DOM-003 |
| §106 | REST API (repositories, initialize, status, profile, rebuild, review, findings, feedback) | dashboard API (`web.rs`) | NestJS API | API | M | L | API-008..API-012 |
| §107 | Webhook gateway: signature, normalize, idempotency, fast ack | none | All | API | M | M | GH-002..GH-004 |
| §108 | State machine RECEIVED…COMPLETED + failure states | 13-state legacy enum (`core.rs:263-310`) | New state machine with CAS transitions | PIPE | M | M | DOM-008, PIPE-007 |
| §109 | Partial reviewer failure → degraded completion recorded | any failed pass ⇒ whole review Failed | Degraded mode | PIPE | S | M | PIPE-008 |

## N. Security, observability, KPIs (§110–§124)

| PRD | Requirement | Current | Gap | Deps | Cx | Risk | Implementation |
|---|---|---|---|---|---|---|---|
| §110 | Encryption, tenancy, least privilege, webhook signatures, redaction, audit, retention, model data controls | loopback-only; no auth; PAT with push scope; CSRF path (`web.rs:219-341`) | All | API | L | **H** | SEC-001..SEC-010 |
| §111 | Secret detection at init; never sent to models; redaction | none | Detector + redactor | INIT, GW | M | H | SEC-003, SEC-004 |
| §112 | Org + repo scoping on every query | single tenant | Tenancy + RLS | API | M | H | API-003, SEC-001, SEC-002 |
| §113 | Source retention policies | none | Policy + jobs | API | M | M | SEC-007 |
| §114 | Trace hierarchy | none | OTel | FND | M | L | OBS-001..OBS-004 |
| §115 | Quality/perf/cost/graph-health metrics | activity events only | Metrics | OBS | M | L | OBS-005, OBS-006 |
| §116–§117 | KPI: accepted/published; FP < 10% then < 5% | none | Measurement | API feedback, QB | M | M | QB-001..QB-006 |
| §118–§120 | Latency targets (<60 s / <2 min / <5 min), incremental in seconds, scale 100k files / 1M symbols | legacy reviews 15–21 min | Perf engineering | all | L | H | PERF-001..PERF-008 |
| §121 | Reproducibility | none | Replay test | GW replay | S | M | PIPE-009 |
| §122–§124 | `.review/config.yaml`, policy file, suppression mechanisms | TOML daemon config | New config | POL | M | L | POL-001, POL-002, POL-006 |

## O. the reference consumer, phasing, MVP, quality (§125–§154)

| PRD | Requirement | Current | Gap | Deps | Cx | Risk | Implementation |
|---|---|---|---|---|---|---|---|
| §125–§127 | the reference consumer as reference consumer; A/B/C extraction | the reference consumer hard-coded throughout (audit §7 of core report) | Extraction | PROF, POL | M | M | `fixtures/reference-profile/` config + rule pack (POL-007); vault adapter (PROF-007). Extraction report = audit §4. |
| §128 | Phase 0 audit + CodeGraph Extraction Report | — | Done | — | — | — | This document set (00, 01, target-arch, ADRs) |
| §129–§139 | PRD phases 1–11 | — | — | — | — | — | Re-sequenced into 37 plan phases (master plan §8); PRD phase order preserved: graph → diff → impact → context → correctness → verification → GitHub → more reviewers → profile → history → multi-provider |
| §140–§141 | MVP definition + acceptance criteria | — | — | — | — | — | Master plan §16 (MVP) and the E2E acceptance test E2E-001 |
| §142–§144 | Benchmark corpus; regression harness; precision bias | `bench/compare.py` (agy logs, no labels) | Harness + corpus | EVAL | L | M | EVAL-001..EVAL-006, QB-001..QB-006 |
| §145, §152–§154 | Moat / differentiation / principle | — | — | — | — | — | Architectural principles (master plan §4) |
| §146 | Technical risks R1–R6 | — | — | — | — | — | Risk register (master plan §6) |
| §147 | 10 critical invariants | partially (no merge, fail≠approve) | All enforceable ones become tests | — | M | H | Invariant test suite INV-001..INV-010 (master plan §9.Z) |
| §148 | Repository structure | — | Adapted | FND | — | — | Target-arch §2 (Rust crates + TS apps) |
| §149 | 20 domain objects | partial | All | DOM | M | L | DOM-004..DOM-007 |
| §150 | Finding lifecycle + persisted suppression | OPEN/FIXED/DISMISSED | New lifecycle | DOM | S | L | DOM-006, VER-001 |
| §151 | Golden scenario (AuthService.authorize) | — | Acceptance fixture | — | M | M | `fixtures/pull-requests/auth-bypass/` used by E2E-001 and QB |

## P. PRD inconsistencies resolved here

| Issue | Resolution |
|---|---|
| `TESTED_BY` (§31) is not in the §18 edge list | `TESTED_BY`, `CALLED_BY` and `DEPENDED_ON_BY` are reverse **views** of `TESTS`, `CALLS` and `DEPENDS_ON`. They are not stored. |
| "database safety" reviewer (§48) is not among the six | It is a correctness-reviewer focus profile (REV-002). |
| `review graph inspect` (§82) vs `review graph symbol` (§84) | `inspect` is an alias of `symbol`. |
| §55 thresholds vs §122 `minimum_publish: 0.72` | §55 sets the defaults. `minimum_publish` overrides the publish floor per repository and cannot go below 0.55. |
| Severity enum undefined | `critical, high, medium, low, info`. The legacy `P0..P4` maps 1:1 for the ported tests. |
| No approve/request-changes policy | The MVP publishes a **COMMENT** review plus a check run. It never approves; this carries over the legacy decision (auto-approve off, legacy §85 O2). The check-run conclusion is `neutral` (findings) or `success` (none), and blocking is configurable later. |
| Legacy safety rules absent from the new PRD (never merge, failure ≠ approval, completeness from exit status, NOT_EXECUTED ≠ PASS, out-of-diff → summary) | All of them are **kept** as invariants INV-011..INV-015. |

## Q. Environment gaps that block the plan

| Blocker | Impact | Plan response |
|---|---|---|
| Host Rust toolchain cannot compile C or `windows-sys` | No tree-sitter/tokio/sqlx on the host | All engine builds run in a Linux container (ADR-001, FND-002) |
| No model API keys configured | Live reviewers cannot run | A `replay` provider runs all tests and benchmarks offline. Live E2E requires `ANTHROPIC_API_KEY` or `OPENAI_API_KEY` (documented in `docs/operations/local-development.md`). |
| No GitHub App, no public ingress, no repo admin | Real webhook delivery is impossible from GitHub | Signed webhook replay for E2E (DEV-005). A real App needs the user to create it (`docs/operations/github-app-setup.md`) and to run ngrok for ingress. A polling reconciler is the fallback (GH-012). |
