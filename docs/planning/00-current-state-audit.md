# 00 — Current-State Audit

**Date:** 2026-10-02
**Scope:** This repository starts empty. The audit covers the prior assets the product was planned against:
1. the **legacy prototype reviewer**, a separate repository that is abandoned and not carried over;
2. the **prototype CodeGraph** (a third-party indexer) as used on a NestJS **reference repository**;
3. the build environment.

**Method:** Every legacy module was read in full. `pr-review/src/...:line` references point into the legacy prototype repository, which is kept only as historical evidence. No legacy code is copied here. Rules worth keeping are re-implemented from the specification in §8, so this repository is self-contained.

**References:**
- Target PRD: [`docs/product/PRD.md`](../product/PRD.md)

Classification vocabulary: **KEEP** (carry over as-is), **GENERALIZE** (concept right, scope too narrow), **REFACTOR** (right idea, wrong shape), **REPLACE** (re-implement on the target stack), **REMOVE** (exists only as a workaround), **MISSING** (target capability with no implementation).

---

## 1. Executive findings

1. **The repository contains a different product from the one the PRD describes.** `pr-review/` is a single-user, single-repo (`a reference NestJS repository`) Rust daemon. It polls GitHub, checks out a worktree, and hands the review to an external agentic CLI (`agy.exe`, Google Antigravity) that explores the repository with shell tools. It then gates the agent's JSON findings and posts one GitHub review. **It does not parse code, does not build or own a graph, and has no symbol model.** All repository intelligence comes from the model running `codegraph explore` itself (`pr-review/src/passes.rs:141`).
2. **The "existing prototype CodeGraph" is a third-party tool**, `@colbymchenry/codegraph` v1.5.0 (MIT, global npm). It writes a SQLite index to `reference-api/.codegraph/codegraph.db`. the reference consumer adds JS glue (`scripts/lib/codegraph-adapter.js`, `scripts/agent-context.js`) and a hand-curated Obsidian vault (`.agent/knowledge/`, 101 notes). Nothing in this repo reads the graph DB (`pr-review/src/intel.rs` only parses vault frontmatter).
3. **The host toolchain cannot build the target Rust stack.** Verified on 2026-10-02: `x86_64-pc-windows-gnu` with no C compiler. The bundled `dlltool` fails, so `windows-sys` (tokio, sqlx, reqwest) fails, and `cc` finds no `gcc`, so tree-sitter fails (probe build log: "failed to find tool gcc.exe"; "dlltool could not create import library"). This is why the current crate's dependencies are limited to serde/serde_json/toml/anyhow (`pr-review/Cargo.toml:6-21`). **Docker Desktop (engine 28.0.4, 8 CPU / 8 GB) works.** The new engine is built and tested in a Linux container (see ADR-001 and the engine Dockerfile).
4. **Safety ideas worth keeping:**
   - a fail-safe decision choke point, where only `Approved` maps to APPROVE (`core.rs:341-349`);
   - completeness computed from exit status, never from the model's self-report (`agent.rs:82-84`);
   - the claim key `(repo, pr, head_sha)` (`store.rs:17-18`);
   - out-of-diff findings relocated to the summary, never dropped (`policy.rs:128-137`);
   - NOT_EXECUTED is never PASS (`validate.rs:23-43`);
   - a structural no-merge test (`github.rs:356-375`);
   - a privacy whitelist for agent telemetry (`activity.rs:73-74`).
5. **Defects found** (do not migrate them):
   - Dedup runs before the decision, so a blocker that was already posted stops blocking (`daemon.rs:415-449`).
   - `mark_finding_status` is not scoped to a run and corrupts other runs' findings (`store.rs:612-618`).
   - Findings still present but deduped are marked FIXED (`daemon.rs:550-555`).
   - The prompts reference an "ALREADY SAID" list that is never rendered (`passes.rs:215`).
   - No runtime review timeout exists (`config.rs:396`, the only use).
   - The broker concurrency bound does not hold under fan-out (`broker.rs:201` vs `agent.rs:510-541`).
   - `-Live` mode silently stays in dry-run (`compose.yaml:49` + `start.ps1:175`).
   - The dashboard has unauthenticated POST routes that can publish a review under the operator's GitHub identity (a CSRF path) (`web.rs:219-341`).
   - SQL is built by string interpolation (`store.rs:179-181`).
   - The agent runs with `--dangerously-skip-permissions` (`agent.rs:705`). Confinement rests on `--mode plan` plus a check of `git status` after the run.
6. **Missing entirely relative to the PRD:**
   - parsing / AST
   - symbol identity
   - CodeGraph ownership
   - incremental indexing
   - diff→symbol mapping
   - change classification
   - impact graph
   - risk engine
   - context engine
   - Qdrant
   - model gateway / routing
   - specialized reviewers
   - semantic verification (base/head, contradiction)
   - computed confidence
   - repository profile / conventions
   - `.review/config.yaml`
   - GitHub App / webhooks
   - multi-tenancy
   - OpenTelemetry
   - CI/CD
   - evaluation harness with labelled data

---

## 2. Legacy prototype structure

```
└── legacy prototype repository      (abandoned; not carried over)
    ├── README.md                    describes the Antigravity reviewer
    ├── docs/product/                legacy PRD + PLAN (moved from the repo root during this audit)
    ├── bench/compare.py             compares agy NDJSON runs across models (129 lines)
    └── pr-review/                   single Rust crate (~7.5k LOC) + Vite UI
        ├── Cargo.toml               deps: serde, serde_json, toml, anyhow only
        ├── src/ (17 modules)        see §3
        ├── ui/                      Vite 6 + React 18 + Tailwind 4, single page
        ├── Dockerfile, compose.yaml, docker/{config.toml, host-broker.toml, entrypoint.sh}
        └── start.ps1, build-env.sh, .env(.example)
```

There is no workspace, no `packages/`, no `infra/`, no `fixtures/`, no `benchmarks/` beyond `bench/compare.py`, and **no CI configuration** (no `.github/`).

---

## 3. Existing applications and modules

### 3.1 `pr-review` Rust crate

| Module | LOC | Responsibility | Evidence | Class |
|---|---|---|---|---|
| `main.rs` | 502 | Hand-parsed CLI: `daemon, once, review, status, retry, agent-broker, web, doctor, init-config`; doctor; starter config | `main.rs:57-143, 245-413, 429-502` | **REFACTOR** (command set → `review` CLI; doctor → check registry) |
| `core.rs` | 479 | Domain types: Severity P0–P4, Finding, Reachability, Placement, ReviewState (13), Decision (no merge), `decision_for` choke point | `core.rs:16-349` | **KEEP** `decision_for`/Severity; **GENERALIZE** Finding (no symbol/range/fingerprint); **REPLACE** `finding_id` (positional `AR-pr-seq`, unstable) |
| `daemon.rs` | 726 | One ~400-line `review()`: prepare → context → gates → agent → integrity → adjudicate → dedup → decide → post → lifecycle | `daemon.rs:153-559` | **REFACTOR** into explicit, persisted pipeline stages |
| `agent.rs` | 1068 | agy subprocess driver, NDJSON envelopes, bounded sessions + handoff, fan-out, transient/permanent retry | `agent.rs:19-882` | **REPLACE** with the Model Gateway; keep `is_transient` semantics and completeness-from-exit-status |
| `broker.rs` | 435 | Hand-written HTTP/1.1 proxy so a Linux container can reach the Windows-only agy | `broker.rs:105-346` | **REMOVE** (workaround) |
| `passes.rs` | 400 | Six prompt passes plus findings JSON schema; the reference consumer checklists hard-coded | `passes.rs:44-265` | **GENERALIZE** (preamble and schema stay; the reference consumer content moves to repo rules) |
| `git.rs` | 478 | git CLI wrapper: mirror, PR refs, merge-base, worktree, junctions; `DiffMap` = set of new-side lines | `git.rs:9-323` | **REFACTOR** (worktree hygiene); **REPLACE** `DiffMap` with a hunk model |
| `config.rs` | 582 | TOML config, startup validation; agy quirks validated in core config | `config.rs:7-418` | **GENERALIZE** (typed, validated, `deny_unknown_fields`) |
| `store.rs` | 776 | Postgres via a `psql` subprocess per call; DDL on every connect; interpolated SQL; no FKs or migrations | `store.rs:1-271, 55-176` | **REPLACE** mechanism; **KEEP** claim-key and fingerprint ideas |
| `policy.rs` | 504 | Adjudicator (hallucination gates, evidence ≥40 chars, conf ≥0.80 to block, latent never blocks); decision engine | `policy.rs:46-233` | **KEEP** (port into the verification engine as structural gates) |
| `validate.rs` | 498 | Static gates (npm type-check/lint/depcruise/tenant-lint/knip), CI check ingestion, NOT_EXECUTED rules | `validate.rs:23-341` | **KEEP** framework; **GENERALIZE** gate list into repo config |
| `intel.rs` | 405 | Parses `.agent/knowledge` vault frontmatter; matches notes to changed files | `intel.rs:46-250` | **REPLACE** with a generic knowledge-source adapter in the profile/context engine |
| `github.rs` | 488 | `gh api` subprocess; search poller; atomic review POST; RIGHT-side single-line anchors; no pagination | `github.rs:47-375` | **REPLACE** transport (GitHub App + webhooks); **KEEP** atomic review, guards, no-merge test |
| `report.rs` | 279 | Markdown summary and comment bodies | `report.rs:11-140` | **GENERALIZE** |
| `web.rs` | 518 | Hand-written HTTP dashboard API; no auth; 200 status on not-found | `web.rs:38-452` | **REPLACE** (NestJS) |
| `activity.rs` | 417 | Whitelist projection of agent telemetry (no prose stored) | `activity.rs:40-279` | **KEEP** principle → telemetry redaction rules |
| `log.rs` | 75 | `[HH:MM:SS]` logger, no date or structure | `log.rs:16-53` | **REPLACE** (`tracing` + OTel) |

### 3.2 UI (`pr-review/ui`)
- Vite 6, React 18.3, TS 5.6 strict, Tailwind 4. One page (`App.tsx`, 441 lines): header, stats, a reviews table with retry, and a detail pane (findings / payload / summary tabs) with Approve/Request/Comment buttons.
- Polls every 5 s. No router, tests or lint.
- The activity endpoint exists but is never called.
- **REPLACE** with Next.js. Keep these patterns: confirm-before-submit, explicit NOT EXECUTED rendering, latent badge, payload preview.

### 3.3 Containers and scripts
- **Dockerfile:** 3 stages, non-root uid 10001.
  - `Cargo.lock` is not copied, so builds are not reproducible (`Dockerfile:17-28`).
  - `.dockerignore` leaks `ui/node_modules`, `ui/dist` and `.env`.
  - Its hardening is worth keeping: `cap_drop ALL`, `no-new-privileges`, loopback ports.
- **compose.yaml:** postgres:16-alpine, reviewer, ui. `start.ps1` runs the host broker and `codegraph init`. **REMOVE/REPLACE.** The new compose lives under `infra/`.

---

## 4. Existing CodeGraph (prototype indexer on the reference repository)

### 4.1 What it is
`@colbymchenry/codegraph` 1.5.0. It uses a native Rust kernel with tree-sitter grammars (~40 languages) and stores the index in SQLite at `reference-api/.codegraph/codegraph.db` (81 MB).

| Table | Purpose |
|---|---|
| `nodes` | symbols |
| `edges` | relationships between nodes |
| `files` | per-file sha256, size, mtime |
| `unresolved_refs` | references that could not be resolved |
| `nodes_fts` | FTS5 full-text index |
| `name_segment_vocab` | name-segment vocabulary |
| `project_metadata` | index metadata |

### 4.2 Graph content (reference-api, read-only inspection)

| Metric | Value |
|---|---|
| Nodes | 14,981 |
| Edges | 53,403 |
| Files | 1,028 |
| Unresolved references (`status=failed`, mostly external libraries) | 75,121 |

**Node kinds:**

| Kind | Count |
|---|---|
| import | 6,740 |
| property | 1,938 |
| method | 1,479 |
| function | 1,170 |
| file | 1,025 |
| constant | 1,006 |
| class | 497 |
| interface | 493 |
| enum_member | 321 |
| route | 120 |
| type_alias | 111 |
| enum | 67 |
| variable | 14 |

**Edge kinds:**

| Kind | Count |
|---|---|
| calls | 21,686 |
| contains | 13,836 |
| imports | 11,446 |
| references | 5,687 |
| instantiates | 359 |
| decorates | 344 |
| extends | 39 |
| implements | 6 |

### 4.3 Properties
- **Edge confidence.** `edges.metadata.confidence` ranges 0.3–0.9, and `resolvedBy` ∈ {import, exact-match, instance-method, framework, qualified-name, fuzzy}. 2,588 calls edges have confidence <0.5. **Uncertainty is recorded, as PRD Invariant 7 requires.**
- **Identity.** Node IDs are `kind:hash(file_path + qualified_name)`, so **identity is not stable across file moves or renames** (PRD §20 requires it).
- **Qualified names.** Formatted `Class::member`.
- **NestJS support.** `route` nodes with `references` edges to handler methods. DI resolution through typed constructor parameters works (326 service→repository `calls` edges, `instance-method`, conf 0.7–0.9). the reference consumer `CLAUDE.md:32` says otherwise; that claim is stale.
- **Incremental indexing.** A per-file sha256 plus (size, mtime) catch-up and a file-watcher daemon. There are no snapshots and no base/head graph: one mutable index for the working tree.
- **Queries.** `query, explore, node, callers, callees, impact (depth 2), affected (files→tests)`, plus MCP tools. There is no two-symbol path query.

### 4.4 the reference consumer glue
- `codegraph-adapter.js:51-106` implements an ABSENT/STALE/CURRENT freshness check (size first; rehash only when mtime is newer).
- `getImpact(depth=2, max=40)` runs a capped inbound BFS and **ignores confidence**.
- `agent-context.js` builds context packs with budgets of 1000/2500/6000 tokens, a depth penalty and verification-status weighting.

### 4.5 Vault (`.agent/knowledge/`)
- 101 notes, each with typed frontmatter (`type, id, verification.status, verified_at, code_refs[], relationships, injects`) and checked by a validator (`verify-agent-layer.js`, 671 lines).
- The **format** is generic. The **content** is entirely the reference consumer: tax, tenancy, decimal money and similar rules.

### 4.6 Dead artifact
`viewer.kuzu` / `viewer-meta.json` hold an earlier merged schema: Controller, Endpoint, Guard, Repository, Processor, DatabaseTable, Queue, Workflow, Risk, plus di-resolver and schema provenance. Nothing regenerates them. They are useful as evidence of which B-layer node types proved valuable.

### 4.7 Classification (PRD §127 categories)

| Concept | Category | Action |
|---|---|---|
| tree-sitter extraction, symbol/edge graph, confidence + resolvedBy, ranges, qualified names, FTS | A | **GENERALIZE** into our own Rust graph core (we do not depend on the external binary) |
| per-file content hash + freshness check | A | **KEEP** the algorithm (re-implemented) |
| callers/callees/impact/affected queries | A | **REPLACE** with engine graph queries |
| NestJS route/DI resolution | B | **GENERALIZE** as the NestJS framework adapter |
| vault format, verification statuses, typed relations | B | **GENERALIZE** as the "knowledge source" adapter feeding repository conventions/rules |
| context-pack budgets/depth penalty | B | **GENERALIZE** into the context engine |
| tenancy, tax, decimal money, EVC no-retry, tenant-isolation lint | C | **KEEP OUTSIDE CORE** — the reference consumer `.review/config.yaml` + rule pack |
| path-hashed node IDs | — | **REPLACE** (ADR-005 stable identity) |

**Decision:** the engine does not shell out to `codegraph`. It is a reference for schema, confidence semantics and benchmark comparison. Its counts on reference-api give a parity target for our TypeScript analyzer (see benchmark plan).

---

## 5. Capability-by-capability inventory

| Area | Current implementation | Evidence | Class |
|---|---|---|---|
| Graph node/edge schema | none in repo; external codegraph schema only | §4 | **MISSING** |
| Indexing | none (external `codegraph init` run on the host by `start.ps1:106-122`) | | **MISSING** |
| Incremental behavior | none in repo; agent queries a stale base-repo index that lacks PR symbols (legacy PRD §84.2 F3) | `passes.rs:136-153` | **MISSING** |
| Parsers / language coverage | none | | **MISSING** |
| Persistence | Postgres via psql subprocess, 5 tables, no migrations | `store.rs:55-176` | **REPLACE** |
| Caching | none (only the `node_modules` junction share) | `daemon.rs:561-579` | **MISSING** |
| AI integration | agy CLI only; one model for every pass; no gateway, routing, token accounting or cost | `agent.rs`, `config.rs:334` | **REPLACE** |
| Queue system | none; a serial poller loop; `max_concurrent_reviews` counts DB rows | `daemon.rs:650-657`, `store.rs:620-628` | **MISSING** |
| Integrations | GitHub via `gh` + PAT, polling only; no App, no webhooks, no GitLab/Bitbucket | `github.rs` | **REPLACE** |
| Tests | 142 unit tests (good policy/validate coverage); zero DB, daemon, route or UI tests | `#[test]` grep | **KEEP** the policy test corpus as a verification-engine regression suite |
| Observability | unstructured stdout logs; activity events in Postgres; no metrics or traces | `log.rs`, `activity.rs` | **REPLACE** |
| Frontend | single-page Vite app | `ui/src/App.tsx` | **REPLACE** |
| CI/CD | none | — | **MISSING** |
| Deployment | local compose + Windows host broker | `compose.yaml`, `start.ps1` | **REPLACE** |
| Docs / ADRs | legacy PRD (with measured-environment §84), PLAN (milestones never marked done beyond M0), README | `docs/product/legacy-*` | **KEEP** as history |

---

## 6. Facts measured in the legacy system that constrain the new design

These come from legacy PRD §84 and were re-verified where possible.

1. **Toolchain.** The host cannot compile C or link `windows-sys` (re-verified 2026-10-02). → The engine is built in Linux containers. The host CLI binary for Windows is produced by CI on an MSVC runner, or the developer installs MSVC Build Tools. This is documented in `docs/operations/local-development.md`.
2. **Agentic exploration is slow and expensive.** One review took 15–21 min and 0.57–1.64M tokens. One session broke at ~1.58M tokens. → This supports the PRD's structured, budgeted context. The target is < 60 s for a small PR.
3. **The model's self-reported completeness is unreliable.** It claimed completion with 3 of 5 passes failed. → Completeness is computed by the orchestrator (kept as invariant).
4. **Cross-model findings overlap is near zero** (2 of 14 locations). → One run's silence is not proof of a clean PR. Auto-approve stays off. Reviewer agreement is a confidence input, not a requirement.
5. **The environment has no public ingress and the token has no admin scope.** → A GitHub App webhook needs a tunnel (ngrok is installed) or a hosted deployment. The system also keeps a polling reconciler as a fallback trigger.
6. **No model API keys are present** (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY` unset). → The gateway includes a deterministic `replay` provider so tests and benchmarks run offline. Live providers activate when keys are configured.

---

## 7. What happens to the legacy prototype

The legacy prototype stays in its own repository and is abandoned. Nothing is migrated mechanically. The rules worth keeping are listed in §8 and re-implemented, with tests, in the tasks named there.

## 8. Legacy rules to re-implement (self-contained specification)

| Rule | Specification | Re-implemented in |
|---|---|---|
| Fail-safe publication decision | Only an explicit, fully successful review can produce a positive verdict. The MVP never emits APPROVE and has no merge capability. Any error path ends in a failed state that publishes nothing. | DOM-010, INV-011, INV-012 |
| Completeness is computed | A review is complete only if every required stage exited successfully, as recorded by the orchestrator. The model's own claims of completeness are ignored. | PIPE-008, INV-013 |
| Structural gate | Discard a candidate when its file is not in the head tree and not in the diff, when line is 0 or past the end of the file, or when its evidence is empty. Never treat a finding as blocking when its evidence is under 40 characters, its confidence is under the blocking floor, or its reachability is `latent`. | VER-003 |
| Out-of-diff relocation | A verified finding whose anchor is not on a commentable diff line moves to the review summary. It is never silently dropped. | GH-007, INV-015 |
| NOT_EXECUTED ≠ PASS | A deterministic tool that could not run (missing toolchain, missing dependencies, timeout) is recorded as `not_executed` with a reason. It never counts as passing and never blocks on its own. | PIPE-004, INV-014 |
| Claim key | Exactly one review run per `(repository, pull request, head sha)`. A failed run may be re-claimed explicitly. | PIPE-005, SUP-001 |
| Transient vs permanent errors | Retry rate-limit and network-interruption errors with backoff. Never retry quota-exhausted or permission errors. | GW-002 |
| Telemetry privacy | Persist tool names, targets, durations and token counts. Never persist model prose, prompts or tool output. | OBS-006 |
| Atomic publication | All inline comments and the summary of one run go out in a single provider review request. | GH-009 |
| No duplicate comments | Before publishing, load existing PR comments and drop findings whose stable fingerprint was already posted. Dedup happens **after** the verdict is computed, so a blocker that was already posted still counts. | DED-003, GH-011 |
