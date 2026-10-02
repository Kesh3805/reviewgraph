# ReviewGraph — Target Architecture

**Status:** Accepted baseline (2026-10-02). Changes go through ADRs in [`docs/decisions/`](../decisions/).
**Inputs:** [PRD](../product/PRD.md), [current-state audit](../planning/00-current-state-audit.md), [gap analysis](../planning/01-prd-gap-analysis.md).

The governing principle is: **understand the repository → model the change → measure impact → select context → reason → verify evidence → comment.**

---

## 1. System context

```
                     ┌──────────────────────────────────┐
                     │  GitHub  (GitLab, Bitbucket later)│
                     └───────┬───────────────▲──────────┘
                     webhooks│               │ review comments, check runs
                             ▼               │
┌──────────────┐    ┌────────────────────────┴───────┐      ┌───────────────┐
│ Next.js web  │───▶│ NestJS API  (control plane)     │◀────▶│ Redis         │
│ (dashboard,  │    │ auth · orgs · repos · webhooks  │      │ rate limits,  │
│  graph expl.)│    │ orchestration · publisher       │      │ idempotency,  │
└──────────────┘    └──────┬────────────────▲────────┘      │ locks, cache  │
                           │ jobs (PG queue) │ jobs          └───────────────┘
                           ▼                 │
                    ┌──────────────────────────────────┐
                    │ PostgreSQL                        │
                    │ metadata · graph · findings · jobs│
                    └──────▲─────────────────▲─────────┘
                           │                 │
       ┌───────────────────┴──┐   ┌──────────┴───────────────┐
       │ review-worker (Rust)  │   │ review-engine (Rust/Axum) │
       │ index · analyze ·     │   │ internal graph/impact API │
       │ review · verify       │   │ for API + graph explorer  │
       └──┬───────┬───────┬───┘   └──────────────────────────┘
          │       │       │
          ▼       ▼       ▼
     Qdrant   Object    Model providers (Anthropic, OpenAI)
     vectors  storage   via the in-engine Model Gateway
              (GCS / S3-compatible)

All components → OpenTelemetry (OTLP/HTTP) → OpenObserve (logs, traces, metrics, dashboards, alerts)
```

**Ownership boundary.** TypeScript owns *who, what and when*: tenants, credentials, providers, lifecycle and publication. Rust owns *understanding*: parsing, the graph, diffs, impact, context, model reasoning and verification. They communicate only through **PostgreSQL rows + the PostgreSQL job queue** (asynchronous) and the **review-engine internal HTTP API** (synchronous, read-mostly). Neither side imports the other's code. Shared shapes are defined once in `packages/contracts` (JSON Schema exported from Rust types).

---

## 2. Repository layout

```
reviewgraph/
├── engine/                         Rust workspace (data plane)
│   ├── Cargo.toml                  [workspace], shared deps, lints
│   ├── crates/
│   │   ├── review-core/            ids, domain entities, enums, versions, errors. No I/O.
│   │   ├── telemetry/              tracing + OTLP init, metric instruments, redaction
│   │   ├── repository/             discovery, `review init` detectors, fingerprint, git (gix), file walk
│   │   ├── analysis-ir/            LanguageAnalyzer trait, normalized IR, syntax facts
│   │   ├── lang-typescript/        tree-sitter TS/JS analyzer + NestJS/TypeORM/BullMQ/Jest adapters
│   │   ├── codegraph/              graph model (46 node / 35 edge kinds), in-memory graph, linker, queries
│   │   ├── graph-storage/          GraphStore port; Postgres + local-file adapters; SQL migrations
│   │   ├── incremental/            base graph + changed files → head graph; symbol/edge diff; invalidation
│   │   ├── diff-engine/            base/head file diff, hunk model, hunk→symbol map, change classification
│   │   ├── impact/                 impact graph, risk engine, change clustering
│   │   ├── semantic/               EmbeddingProvider port, Qdrant adapter, incremental embedding sync
│   │   ├── context-engine/         candidates, lexical search, ranking, budgets, compression → ContextPackage
│   │   ├── model-gateway/          ModelGateway, tiers, router, Anthropic/OpenAI/replay adapters, accounting
│   │   ├── reviewers/              Reviewer trait, versioned prompts, six reviewers
│   │   ├── verification/           evidence, anchor, graph, base/head, contradiction, confidence, dedup
│   │   ├── profile/                repository profile, convention inference, `.review/config.yaml`, policy
│   │   └── pipeline/               review-run state machine, stage orchestration, PG job queue client
│   ├── apps/
│   │   ├── review-cli/             `review` binary (init, status, doctor, diff, pr, graph, impact, profile)
│   │   ├── review-worker/          job consumer (repository-index, incremental-index, pr-analysis, review)
│   │   └── review-engine/          Axum internal API (graph queries, impact, subgraphs)
│   ├── migrations/                 sqlx migrations — the ONLY schema source for PostgreSQL
│   └── benches/                    criterion benches (per crate `benches/` dirs also allowed)
├── apps/
│   ├── api/                        NestJS control plane
│   └── web/                        Next.js 15 App Router + Tailwind + shadcn/ui
├── packages/
│   ├── contracts/                  JSON Schemas (generated from Rust via schemars) + generated TS types
│   └── config/                     shared eslint/tsconfig/prettier
├── infra/
│   ├── compose/                    docker-compose.yml (pg, redis, qdrant, openobserve, objectstore, api, engine, worker, web)
│   ├── docker/                     Dockerfiles (engine, api, web)
│   └── openobserve/                dashboards + alert definitions (JSON)
├── fixtures/
│   ├── repositories/               small deterministic git repos (built by script from plain files)
│   └── pull-requests/              golden base+patch scenarios with expected outputs
├── benchmarks/
│   ├── quality/                    labelled PR corpus (expected/forbidden/optional findings)
│   └── perf/                       synthetic large-repo generator + perf scenarios
└── docs/                           architecture, decisions, graph-schema, languages, reviewers, operations, security, planning
```

### 2.1 Crate dependency direction (enforced by `cargo deny` bans + a workspace test)

```
review-core ◀── telemetry
     ▲
     ├── repository
     ├── analysis-ir ◀── lang-typescript
     ├── codegraph (◀ analysis-ir)
     │      ▲
     │      ├── graph-storage
     │      ├── incremental (◀ analysis-ir, graph-storage)
     │      ├── diff-engine (◀ repository, analysis-ir)
     │      ├── impact (◀ diff-engine)
     │      ├── semantic
     │      ├── profile (◀ repository)
     │      └── context-engine (◀ impact, semantic, profile)
     ├── model-gateway          (depends on review-core + telemetry only)
     ├── reviewers (◀ context-engine, model-gateway)
     ├── verification (◀ codegraph, diff-engine, impact, model-gateway)
     └── pipeline (◀ everything above; the only composition root besides apps)
```

Rules:
- `review-core` has no I/O dependencies.
- No crate depends on an app.
- `lang-typescript` is the only crate that knows TypeScript.
- Nothing below `pipeline` knows about jobs, tenants or providers.
- Framework-specific (NestJS) and repository-specific (the reference consumer) knowledge never appears in `codegraph`.
- Library crates use typed errors and never depend on `anyhow`; see [error-handling.md](error-handling.md).

---

## 3. Data plane — repository intelligence

### 3.1 Language analysis pipeline

```
source bytes ──▶ tree-sitter parse ──▶ syntax walk ──▶ framework adapters ──▶ IR (ParsedUnit)
                                                  (NestJS, TypeORM, BullMQ, Jest)
ParsedUnit {
  file: path, language, content_hash, analyzer_version,
  symbols:     [IrSymbol { local_id, kind, name, qualified_name, signature, range,
                           body_hash, signature_hash, modifiers, decorators, parent }],
  references:  [IrReference { from, name, kind (call|type|extends|implements|import|new|decorator|...),
                               receiver_hint, import_binding, range }],
  imports:     [IrImport { specifier, bindings, range }],
  exports:     [...],
  framework:   [IrFrameworkFact { kind (route|module|provider|guard|processor|entity|...), attrs, symbol }],
  syntax_facts per symbol: [SyntaxFact] (calls made, conditions, throws/catches, awaits, returns,
                                         db write calls, transaction wrappers, guard decorators)
  diagnostics: [ParseDiagnostic]
}
```

- **Range convention.** Every `range` in the IR uses `review_core::location::Position`: lines are 1-based, columns are 0-based UTF-8 byte offsets within the line (tree-sitter columns).
- A `LanguageAnalyzer` is pure: `(path, bytes, AnalyzerConfig) → ParsedUnit`. It does no cross-file work, so per-file results can be cached by `(content_hash, path, analyzer_version)`.
- **Cross-file resolution** happens in `codegraph::linker`. It consumes all ParsedUnits of a snapshot plus a `ModuleResolver` (tsconfig `paths`/`baseUrl`, node resolution, workspace packages). It produces edges with `confidence` and `resolved_by`.

  | `resolved_by` | Confidence | How the edge was found |
  |---|---|---|
  | `import` | 0.95 | import binding |
  | `this_member` | 0.95 | `this.x()` call on the same class |
  | `di_constructor` | 0.85 | constructor parameter type |
  | `type_annotation` | 0.8 | type annotation |
  | `name_unique` | 0.6 | unique name in the repository |
  | `name_ambiguous` | 0.3 | ambiguous name |
  | `framework` | 0.9 | framework fact |

  The values live in one table in `codegraph::confidence` and are calibrated by the benchmark.
- **Semantic enrichment** (TypeScript compiler) is an optional out-of-process helper (`engine/tools/ts-semantic`, Node). The linker calls it only for references left ambiguous. Its results upgrade edges to `resolved_by=type_checker` with confidence 1.0. It is behind the `SemanticProvider` port; MVP ships with it disabled by default (ADR-007).

### 3.2 Stable symbol identity (ADR-005)

```
SymbolId (canonical string) = "{lang}:{module_path}#{qualified_name}/{kind}[~{overload_ordinal}]"
  e.g. ts:src/auth/auth.service#AuthService.authorize/method
SymbolKey = blake3(SymbolId) truncated to 128 bits, hex — used as the DB key
```

- Line numbers never participate. The signature is an attribute, so changing the parameters is *modified*, not delete+add. `signature_hash` and `body_hash` (a normalized token hash that ignores whitespace and comments) drive change classification.
- Renames and moves are detected by `incremental::matcher`. Among removed×added symbols of the same kind, it looks for the best match by body hash, then by token-similarity ≥ 0.8. It records `SymbolTransition::Renamed { from, to, similarity }`. Identity changes, but lineage is preserved through `symbol_lineage`.

### 3.3 CodeGraph model

- Node kinds are the PRD §17 list (46) plus `Repository`. Edge kinds are the PRD §18 list.
- Reverse relations (`CALLED_BY`, `DEPENDED_ON_BY`, `TESTED_BY`) are **views** over the reverse index and are never stored. This avoids double writes and drift.
- Every edge carries: `kind, source, target, confidence, resolved_by, provenance (analyzer|framework|linker|type_checker|heuristic|policy), location?`.
- Synthetic nodes have deterministic IDs:

  | Node | ID format |
  |---|---|
  | API endpoint | `http:{METHOD} {normalized_path}` |
  | Queue | `queue:{name}` |
  | Table | `db:{schema}.{table}` |
  | Environment variable | `env:{NAME}` |
  | External package | `pkg:{ecosystem}/{name}` |
  | Test case | `test:{file}#{suite path} › {name}` |

**In-memory representation (`codegraph::Graph`).**
- An interned node table (`Vec<NodeData>`, `HashMap<SymbolKey, NodeIx>`).
- Forward and reverse adjacency as CSR-like `Vec<Vec<EdgeIx>>`, partitioned by edge kind.
- Built in O(V+E). Immutable once built.
- A head graph is a `GraphOverlay { base: Arc<Graph>, added, removed }`, so a PR never copies the base.

**Queries.**
- `node(id)`
- `out_edges(id, kinds)`, `in_edges(id, kinds)`
- `neighbors(id, dir, kinds, min_confidence)`
- `bounded_bfs(seeds, dir, kinds, max_depth, max_nodes, min_confidence)`
- `shortest_path(a, b, kinds, max_depth)`
- `subgraph(seeds, depth)` (for the UI)

Every traversal takes an explicit budget and returns `truncated: bool`.

### 3.4 Persistence (ADR-003, ADR-014, ADR-015)

PostgreSQL is the durable store. The graph is persisted **content-addressed per file version, with snapshot overlays**:

```
file_versions   (id, repository_id, path, content_hash, language, analyzer_version, parse_status, ...)
                UNIQUE (repository_id, path, content_hash, analyzer_version)
symbols         (file_version_id, symbol_key, symbol_id, kind, name, qualified_name, signature,
                 start_line, start_col, end_line, end_col, body_hash, signature_hash, visibility,
                 is_exported, is_generated, attrs jsonb)
unresolved_refs (file_version_id, from_symbol_key, name, kind, import_specifier, line, col)
snapshots       (id, repository_id, commit_sha, kind full|delta, base_snapshot_id, status,
                 graph_schema_version, analyzer_versions jsonb, config_hash, fingerprint, stats jsonb)
snapshot_files  (snapshot_id, path, file_version_id NULL=deleted)       -- delta: changed paths only
graph_edges     (snapshot_id, source_key, target_key, kind, confidence, resolved_by, provenance,
                 file_version_id, line, col, removed bool)               -- delta: added + tombstones
synthetic_nodes (snapshot_id, node_key, node_id, kind, attrs jsonb, removed bool)
symbol_lineage  (repository_id, from_snapshot_id, to_snapshot_id, from_key, to_key, transition, similarity)
```

- A **full** snapshot is written on initial index and periodically for the default branch. A **delta** snapshot stores only changed paths and the edges whose validity changed.
- `Graph(head) = load(base full) ⊕ delta chain`. The chain is compacted into a new full snapshot when it exceeds N=20 deltas or 10% of edges.
- Indexes follow PRD §103:
  - `(repository_id, symbol_key)` on symbols via file_versions
  - `(snapshot_id, source_key, kind)`
  - `(snapshot_id, target_key, kind)`
  - `(repository_id, path)`
- The local CLI uses `graph-storage::file` instead: `.review/graph/snapshots/{id}.bin.zst` (bincode + zstd) plus `.review/repository.json`. Both adapters implement the same `GraphStore` trait, and the same conformance test suite runs against both.

### 3.5 Incremental update (ADR-004)

```
inputs: base snapshot S_b (+ its in-memory Graph), head commit H, changed paths P (from git tree diff)
1. for p in P: content_hash(head blob) == base hash? → skip (counter files_skipped_unchanged)
2. parse only changed files (cache lookup by (path, hash, analyzer_version) first)   → files_reparsed_total
3. symbol diff per file: unchanged | modified(signature|body|attrs) | added | removed | renamed(matcher)
4. re-link: references FROM changed files are re-resolved against the head symbol table;
   references FROM unchanged files whose resolved target was removed/renamed, or whose name
   now has new candidates (name index delta), are re-resolved — found via reverse index, not a scan
5. emit delta snapshot: snapshot_files(P), graph_edges(+added, tombstones), lineage
6. invalidation set = changed symbols ∪ 1-hop dependents (by edge kind policy) → cache keys + embeddings
7. counters: files_reparsed, symbols_{added,removed,modified,renamed}, edges_{added,removed},
   invalidations, unchanged_files_skipped — asserted in tests (no reparse of unchanged files)
```

Full-rebuild triggers follow PRD §24: graph schema or analyzer major version change, config hash change in source roots or tsconfig, a failed consistency check, or an explicit `review graph rebuild`.

### 3.6 Diff engine and change model

- Base/head resolution comes from the provider's base/head SHAs and the merge base computed locally (`gix`). A three-dot equivalent is used: diff `merge_base..head`.
- `ChangedFile { path, old_path?, status: added|modified|deleted|renamed|copied, binary, hunks: [Hunk { old_range, new_range, lines }] }`. The diff is computed with `gix` tree diff + `imara-diff`. It does not shell out, so user git config cannot alter it.
- **Hunk→symbol mapping:** the innermost symbol whose range intersects a changed line on the new side (head IR), and on the old side for deletions (base IR).
- **Change classification** compares the `SyntaxFact` sets of the base and head versions of each changed symbol, giving the PRD §28 categories plus `loop_changed`. It is deterministic and has no LLM.
- **Intent classification** (PRD §29) is deterministic signals plus an optional CLASSIFIER model call. It never overrides deterministic risk.
- `PullRequestChangeModel { files, symbols, apis, dependencies, schemas, configs, tests, risk_signals }`.

### 3.7 Impact, risk, clustering

- **ImpactGraph per changed symbol.** Callers (distance ≤2, transitive callers only if budget remains), callees (1), implementations, interfaces, overrides, related types, tests (`TESTS` edges + path conventions + imports), API entrypoints (reverse `HANDLED_BY`/`ROUTES_TO` reachability, bounded), config/DB/queue relations. Each element records `distance`, `path` and `min_confidence`.
- **Risk engine.** A rule table of signal → weight → effects. Inputs are path rules, framework facts (guards, routes), change classes, dependency manifest changes and migration files. Output: `RiskAssessment { score 0..1, level, signals[], effects: { reviewers, depth, context_budget, model_tier, verification_depth } }`.
- **Clustering** groups changed symbols into review units by module plus connected components over call/type edges among the changed set. Clusters are ranked by risk. Budgets are allocated in that order, and unreviewed clusters are reported.

### 3.8 Context engine (ADR: structural before semantic, Invariant 10)

```
seeds (changed symbols of a cluster)
 → structural candidates (impact graph)                         [graph]
 → tests / config / API / rule-doc candidates                   [graph + profile]
 → lexical candidates (identifier index: changed identifiers)   [lexical]
 → semantic candidates (Qdrant, filtered by org/repo/snapshot)  [semantic, only to fill remaining budget]
 → score = Σ wᵢ·signalᵢ  (10 PRD §33 signals; weights per reviewer; no single weight > 0.35)
 → budget (per reviewer: max symbols, tokens, tests, configs) — greedy by score/token
 → compression: signature + changed body + selected related bodies, with source locations
 → ContextPackage { items[], omitted[] with reasons, token_estimate, budget }
```

### 3.9 Semantic layer (Qdrant, ADR-008)

- **One collection per embedding space**, named `rg_{provider}_{model}_{dims}_v{n}` (provider and model lowercased, every character outside `[a-z0-9]` replaced by `_`; see `EmbeddingSpace::collection_name`). Payload indexes cover `organization_id, repository_id, kind, language, symbol_key, file_path, content_hash, snapshot_lineage`.
- **Points.** `point_id = uuid_v5(org, repo, kind, symbol_key | chunk_key, embedding_space)`. Re-embedding the same content is an upsert no-op, detected by a content_hash check *before* calling the embedding provider.
- **Kinds:** `symbol_summary, code_chunk, doc, convention, finding_history`.
- **Every search carries a mandatory tenant filter.** The adapter API makes it impossible to omit: `TenantScope` is a required parameter type.

### 3.10 Repository profile and policy

- `.review/config.yaml` follows PRD §122 + §66: reviewers, confidence thresholds, budgets, generated globs, risk paths, rules (forbidden dependencies, queue job ids, migrations-only, tests required), suppressions, and knowledge sources (e.g. `.agent/knowledge` as a vault adapter).
- Inferred conventions are recorded as `{ rule, scope, samples, violations, consistency, confidence, exceptions[], computed_at, profile_version }`.
- Precedence: explicit policy > documented architecture > inferred (confidence ≥0.9, samples ≥10) > generic.

---

## 4. Review plane

### 4.1 Review-run state machine (persisted, `review_runs.state`)

```
RECEIVED → INDEXING → ANALYZING → REVIEWING → VERIFYING → PUBLISHING → COMPLETED
   └──────────┴──────────┴───────────┴───────────┴────────────┴──▶ SUPERSEDED | CANCELLED
failures: FAILED_INDEXING | FAILED_ANALYSIS | FAILED_REVIEW | FAILED_PUBLISH
```

- Transitions are guarded by `UPDATE ... WHERE id=$1 AND state=$expected AND NOT superseded`, which is an optimistic compare-and-set.
- Every stage writes its outputs before transitioning, so a retried job resumes from the last completed stage (`stage_outputs` keyed by `(review_run_id, stage, input_hash)`).

### 4.2 Reviewers

- Each reviewer is an implementation of `Reviewer { kind, version, applies(&ChangeModel, &Risk) -> bool, budget(&Risk) -> ContextBudget, review(&ContextPackage, &dyn ModelGateway) -> Vec<CandidateFinding> }`.
- Prompts are versioned files (`reviewers/prompts/{kind}/v{n}.md`) with a structured output JSON Schema.
- The model receives the PRD §89 structured input, never raw files.
- Routing follows the PRD §48 table, driven by risk effects.

### 4.3 Verification (ADR-011) — candidate → published

```
CandidateFinding (GENERATED)
 1 schema/structure gate     (ported legacy adjudicator: file exists, line valid, evidence present)
 2 changed-code anchor       location ∈ changed hunk OR affected symbol ∈ impact graph with path to a change
 3 graph evidence            every claimed relation (A calls B, reaches endpoint E) checked against Graph(head)
 4 repository evidence       cited code exists at cited ranges (text match), rule/convention exists
 5 base/head comparison      re-evaluate the predicate on Graph(base)/base source; identical in base → PREEXISTING
 6 contradiction search      deterministic: upstream guard/authorization on all paths, caller-owned transaction,
                             catch wrapper, generated code, type impossibility; then VERIFIER model
                             with *only* the gathered counter-evidence
 7 actionability             concrete corrective direction present; not style; not a diff restatement
 8 confidence                computed (PRD §54 formula, weights in verification::confidence), NOT model-reported
 → EVIDENCE_COLLECTED → VERIFIED → (dedup) DEDUPLICATED → PRIORITIZED → PUBLISHED
   or SUPPRESSED_{LOW_CONFIDENCE|DUPLICATE|PREEXISTING|NOT_ACTIONABLE|POLICY} | INVALIDATED (superseded head)
```

- Publication thresholds: <0.55 suppress; 0.55–0.70 internal; 0.70–0.85 publish if severity ≥ medium; >0.85 publish. Each is overridable per repo (`confidence.minimum_publish`).
- All suppressed findings are persisted with reasons. They are evaluation data.
- The authoritative lifecycle table (allowed `FindingState` edges) is `ALLOWED` in [`engine/crates/review-core/src/finding/state.rs`](../../engine/crates/review-core/src/finding/state.rs).

### 4.4 Model Gateway (ADR-009, ADR-010)

```rust
pub struct ModelRequest {
    task: TaskType, tier: ModelTier, reasoning: ReasoningLevel,
    messages: StructuredInput, output_schema: Option<JsonSchema>,
    max_output_tokens: u32, cache: CachePolicy, privacy: PrivacyPolicy, budget: CallBudget,
    trace: TraceContext,
}
pub struct ModelResponse { output: serde_json::Value | String, usage: Usage{input,output,cached_read,cached_write},
    latency_ms, provider, model, cost_usd_micros, finish_reason, request_hash }
```

- **Tiers:** `CLASSIFIER, FAST_REASONER, REVIEW_REASONER, VERIFIER, DEEP_REASONER`.
- **Router.** A config table maps (tier, risk band, privacy) → ordered candidate list of `{provider, model}`. It is chosen from evaluation results stored in `model_eval_results`, and the defaults are documented, not hard-coded in reviewers.
- **Adapters:**
  - `anthropic` (Messages API; structured output via tool/JSON schema; prompt caching via `cache_control`)
  - `openai` (Responses API with `json_schema`)
  - `replay` (deterministic fixtures keyed by request hash, used by tests and offline benchmarks)
- **Cross-cutting:**
  - per-call timeout
  - retries with jittered exponential backoff on transient errors only (typed `GatewayError::{Transient, RateLimited{retry_after}, Permanent, SchemaViolation, BudgetExceeded}`)
  - token-bucket rate limit per provider (Redis-backed across workers)
  - budget enforcement
  - token and cost accounting
  - spans
- **Redaction.** Secrets are redacted before anything leaves the process (`telemetry::redact` + init-time secret detection).

---

## 5. Control plane (NestJS, `apps/api`)

| Module | Responsibility |
|---|---|
| `auth` | GitHub OAuth login for humans; session JWT; service tokens for engine→API internal calls |
| `organizations`, `users`, `memberships` | tenancy; every repository/query scoped by `organization_id` (Postgres RLS policies + app guards) |
| `providers/github` | GitHub App: installation webhooks, installation tokens (cached in Redis ≤50 min, never persisted), REST via Octokit, check runs, review comments, pagination, rate-limit handling |
| `webhooks` | signature verification (HMAC-SHA256, constant-time), normalization to `ProviderEvent`, idempotency (`X-GitHub-Delivery` in Redis SETNX + `webhook_deliveries` table), fast 202 ack |
| `repositories` | onboarding, settings, `.review/config.yaml` sync, status |
| `reviews` | creates `review_runs`, supersession (new head ⇒ older runs `SUPERSEDED` + their pending publish jobs cancelled), enqueues jobs |
| `publisher` | consumes `review-publish` jobs; re-checks head currency and run state *inside* the publish transaction; posts one GitHub review (inline + summary) + check run; records `published_findings` with provider comment ids |
| `findings` | finding detail, feedback (`useful, false_positive, already_handled, not_relevant, intentional`) |
| `graph` | authorized proxy to review-engine internal API (adds tenant scope) |
| `internal` | credential broker for workers (`POST /internal/repositories/:id/clone-credentials` → short-lived token), service-auth only |
| `health`, `telemetry` | readiness/liveness, OTel SDK init |

**Provider port** (`RepositoryProvider`, `ReviewPublisher`) is implemented by `providers/github` only in the MVP. Review domain code imports the port, never Octokit.

**Job transport** (ADR-012) is a PostgreSQL `jobs` table:

```
jobs (id uuid, queue text, idempotency_key text UNIQUE, payload jsonb, state queued|running|succeeded|failed|dead|cancelled,
      priority int, attempts int, max_attempts int, run_after timestamptz, locked_by text, locked_until timestamptz,
      last_error text, trace_parent text, created_at, updated_at)
claim: UPDATE jobs SET state='running', locked_by=$w, locked_until=now()+$lease, attempts=attempts+1
       WHERE id = (SELECT id FROM jobs WHERE queue=ANY($q) AND state='queued' AND run_after<=now()
                   ORDER BY priority DESC, created_at FOR UPDATE SKIP LOCKED LIMIT 1) RETURNING *
```

- Workers heartbeat by extending `locked_until`. A reaper requeues expired leases. After `max_attempts` a job goes to `dead`.
- `pg_notify('jobs_<queue>')` wakes idle consumers.
- Queues: `repository-index, incremental-index, pr-review, review-publish, history-ingest`. Payloads carry IDs only.

---

## 6. Frontend (`apps/web`)

- Next.js App Router, TypeScript strict, Tailwind, shadcn/ui (Radix). It calls only the NestJS API (BFF pattern, cookie session).
- **Screens:** Dashboard · Repositories · Repository Overview · Repository Intelligence (index status, graph stats) · Repository Profile · CodeGraph Explorer · Pull Requests · Review Detail (summary / change / risk / findings / files / evidence) · Finding Detail (with impact path) · Repository Rules · Integrations · Usage · Settings.
- **Graph visualization:**
  - Cytoscape.js for the explorer (server-side subgraphs only, ≤500 nodes per view).
  - React Flow for curated impact/evidence paths in Finding Detail.
  - Sigma.js is not adopted (ADR note in target-architecture §9).

---

## 7. Cache strategy

| Cache | Key | Store | Invalidation |
|---|---|---|---|
| AST/IR (ParsedUnit) | `(path, content_hash, analyzer_version)` | PG `file_versions` + object store blob | analyzer version bump |
| symbol table | file_version_id | PG | immutable |
| graph | snapshot_id | PG + in-process LRU (`Arc<Graph>`) | immutable; LRU eviction |
| dependency/module resolution | `(snapshot_id, config_hash)` | in-process | snapshot change |
| semantic summary | `(symbol_key, body_hash, summarizer_version, model)` | PG `symbol_summaries` | body change |
| embedding | `(point_id, content_hash, embedding_space)` | Qdrant payload | content change / space change |
| context package | `(review_run, cluster, reviewer, input_hash)` | PG `stage_outputs` | input hash |
| model response | `request_hash` (only for `cache: Allowed` tasks, never across tenants) | PG `model_cache` | TTL 7d + model version |
| verification | `(candidate_fingerprint, verification_version, snapshot pair)` | PG | version |
| repository profile | `(snapshot_id, profile_version, config_hash)` | PG | snapshot/config |

**Rule:** every cache key is a pure function of its listed inputs. Redis holds only rate limits, idempotency keys, locks, installation tokens (encrypted, short TTL) and hot query caches. No ASTs, graphs or source.

---

## 8. Observability (ADR-013)

- **Rust:** `tracing` + `tracing-opentelemetry` + `opentelemetry-otlp` (HTTP/protobuf) → OpenObserve `/api/{org}/v1/{traces,metrics,logs}`.
- **NestJS:** `@opentelemetry/sdk-node` with HTTP/pg/ioredis instrumentation.
- **Trace propagation.** `traceparent` is stored on every job row and restored by the consumer, so one PR review is one trace from webhook to publish.
- **Standard attributes:** `request_id, review_run_id, repository_id, organization_id, pull_request_id, commit_sha, job_id, reviewer_type, candidate_finding_id`.
- **Span names:** `webhook_received, repository_checkout, repository_index, incremental_graph_update, diff_analysis, symbol_mapping, impact_analysis, context_selection, qdrant_search, reviewer_execution, model_request, candidate_generated, finding_verification, deduplication, publication`.
- **Metrics:** the full list in the implementation plan (OBS tasks) and PRD §115.
- **Redaction.** Log fields pass through a redaction layer: token patterns, `Authorization` headers and `.env`-style assignments. Source code is never logged. Model prompts are never logged; only hashes and token counts are.

---

## 9. Deployment

- **Local:** `infra/compose/docker-compose.yml` brings up postgres 16, redis 7, qdrant, openobserve, object store (SeaweedFS S3), api, engine, worker and web (`make up` / `pnpm dev:up`). The Rust engine builds inside `rust:1-bookworm` because the Windows host toolchain cannot compile C dependencies (audit §1.3).
- **Target (GCP):**
  - Cloud Run for `api`, `web`, `review-engine`.
  - Cloud Run Jobs or a GCE managed instance group for `review-worker` (needs a local disk for checkouts).
  - Cloud SQL Postgres, Memorystore Redis, GCS (S3-compatible API via HMAC or native).
  - Qdrant on a dedicated VM or Qdrant Cloud.
  - Self-hosted OpenObserve.
  - Kubernetes is **not** adopted until worker autoscaling on queue depth needs it.
- **Images:** distroless/debian-slim, non-root, read-only root FS, `cap_drop: ALL`.
- **Graph visualization libraries.** Cytoscape.js and React Flow are both adopted because their use cases do not overlap (explorer vs. curated path). Sigma.js is deferred until a measured need: >5k rendered nodes.
