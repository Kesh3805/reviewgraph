# Phases 20–26 — Specialist reviewers, repository profile, explicit policy

**Parent:** [MASTER_IMPLEMENTATION_PLAN.md](../MASTER_IMPLEMENTATION_PLAN.md) §9 · **Architecture:** [target-architecture.md](../../architecture/target-architecture.md) §3.10, §4.2–4.3 · **ADRs:** 009, 010, 011, 015

Status markers: ☐ todo · ◐ in progress · ☑ done (acceptance criteria executed and passed).

**Phase scope.**
- Phase 20 (security reviewer) is **MVP**. Phases 21–24 (tests, architecture, performance, maintainability) are post-MVP (M7).
- Phases 25–26 (profile, policy) are MVP for POL-001/002/004/006 and PROF-001/006, because `.review/config.yaml` drives routing, thresholds and suppressions. The convention miners (PROF-003..005) and the rule evaluators (POL-003/005) are needed before REV-A.

**Shared conventions for every reviewer task in this file.**
- A reviewer implements `reviewers::Reviewer` (REV-001). It receives a `ContextPackage` (CTX) and a `&dyn ModelGateway` (GW-001). It never sees raw files, never opens a provider client, and never sets confidence (ADR-011).
- Prompts live at `engine/crates/reviewers/prompts/{kind}/v{n}.md`. Output schemas live at `engine/crates/reviewers/prompts/{kind}/v{n}.schema.json`. Both are embedded with `include_str!` and hashed into `prompt_version = "{kind}:v{n}:{blake3[..8]}"`.
- Every candidate must cite structured evidence: `symbol_keys[]`, `ranges[]` and `claimed_relations[] { from, to, kind }`. Verification (VER-001..012) checks every claim.
- Deterministic pre-checks run **before** the model. Their hits become `Evidence::Deterministic` (DOM-007) and seed the model input. A deterministic hit on its own is still a *candidate*: it goes through verification like any other.
- Tests use the replay provider only. No test needs a model key.

---

## Phase 20 — Security reviewer (MVP)

## Task index

| ID | Title |
|---|---|
| REV-S-001 | Security prompt v1 and output schema |
| REV-S-002 | Security reviewer with trust-boundary context selection |
| REV-S-003 | Security-specific verification checks |
| REV-T-001 | Test reviewer |
| REV-T-002 | Deterministic pre-check: changed public symbol with zero TESTS edges |
| REV-A-001 | Architecture reviewer using profile and rules |
| REV-A-002 | Deterministic boundary-violation and cycle detector |
| REV-P-001 | Performance reviewer (concrete execution path required) |
| REV-P-002 | Deterministic N+1 and IO-in-loop detector |
| REV-M-001 | Maintainability reviewer (high threshold ≥ 0.9) |
| PROF-001 | RepositoryProfile model and persistence |
| PROF-002 | Architecture, module boundary and layer inference from the graph |
| PROF-003 | Convention miner framework |
| PROF-004 | Conventions: controllers-not-repositories, transaction wrapper on writes, error hierarchy, guard usage |
| PROF-005 | Testing, API and queue conventions |
| PROF-006 | Profile versioning and cache |
| PROF-007 | Knowledge-source adapter (vault and ADR markdown with frontmatter → DocumentationRule nodes) |
| POL-001 | `.review/config.yaml` schema, parser and validation |
| POL-002 | Config sync at snapshot time and `config_hash` |
| POL-003 | Rule evaluators |
| POL-004 | Precedence resolver (PRD §65) |
| POL-005 | Rule violations as deterministic evidence |
| POL-006 | Suppression mechanisms with audit |
| POL-007 | the reference consumer reference profile and rule pack (outside core) |

---

### REV-S-001 — Security prompt v1 and output schema
Status: ☐

- **Task ID:** REV-S-001
- **Title:** Security prompt v1 and output schema
- **Problem:** No prompt exists that asks a model for PRD §43 security findings under the PRD §89 structured input. Today the only security material is the the reference consumer tenant checklist hard-coded in legacy `passes.rs:106-131`.
- **Why it exists:** The security reviewer is in the MVP (§16). Its prompt and schema are versioned artifacts. Reproducibility (ADR-015) and evaluation (EVAL) both key on `prompt_version`.
- **Scope:**
  - `prompts/security/v1.md`, covering the 12 PRD §43 responsibilities as closed categories.
  - `prompts/security/v1.schema.json`.
  - A category enum in `reviewers::security::SecurityCategory`.
  - A "repository security patterns" input section, filled from profile conventions (PROF-004 guard usage) when present.
- **Explicit non-scope:**
  - Context selection (REV-S-002).
  - Deterministic checks (REV-S-003).
  - Any consumer-specific wording: tenant/organisation rules come only from the profile or the rule pack (POL-007).
- **Files/modules expected to change:** `engine/crates/reviewers/src/lib.rs` (register the module), `engine/crates/reviewers/Cargo.toml` (none expected).
- **New files/modules expected:**
  - `engine/crates/reviewers/prompts/security/v1.md`
  - `engine/crates/reviewers/prompts/security/v1.schema.json`
  - `engine/crates/reviewers/src/security/mod.rs`
  - `engine/crates/reviewers/src/security/categories.rs`
- **Dependencies:** REV-001 (Reviewer trait, §89 input schema), REV-C-001 (prompt file conventions), DOM-006 (CandidateFinding), DOM-007 (Evidence), GW-001.
- **Implementation details:**
  - `SecurityCategory = AuthzBypass | AuthnRegression | InputValidation | Injection | DataExposure | UnsafeDeserialization | TrustBoundary | CredentialHandling | PathTraversal | Ssrf | InsecureConfig | PrivilegeEscalation`. It serializes as `snake_case`.
  - The schema extends the common candidate schema with:
    - `category` (enum)
    - `entry_points[]`: `{ endpoint_node_id, path_symbol_keys[] }`
    - `trust_boundary: { source: "http_body|http_query|http_header|queue_payload|env|db|external_api", sink_symbol_key }`
    - `missing_control`: `"guard|validation_pipe|parameterization|encoding|allowlist|ownership_check"`
  - The prompt rules are:
    - Report only issues introduced or exposed by the change.
    - Every claim must name an execution path that appears in the provided `impact.paths`.
    - Use "cannot determine" instead of speculating.
    - Do not restate the diff.
    - The banned-phrase list from PRD §58 (`potentially might`, `consider whether`, `maybe`) is enforced in the prompt and rejected by schema post-validation (VER stage 7).
  - Model tier: `REVIEW_REASONER`. It escalates to `DEEP_REASONER` only when `RiskAssessment.level == critical` and the budget allows (ADR-010).
- **Data model changes:** None. `reviewer_runs.prompt_version` already exists (REV-001).
- **API/protocol changes:** `packages/contracts` gains `SecurityCandidateExtension`, exported via schemars.
- **Concurrency semantics:** Pure data. The prompt is loaded once with `include_str!`.
- **Failure behavior:**
  - An invalid model output is retried once with a repair instruction (ADR-009). After that it is counted as `structured_output_failure`, and the reviewer returns `ReviewerOutcome::Failed`, which gives degraded completion (PIPE-008).
- **Idempotency considerations:** `prompt_version` is part of the reviewer stage key `reviewer:{run}:{kind}:{input_hash}` (PRD §76). A prompt edit therefore invalidates only model-derived layers.
- **Security considerations:**
  - The prompt contains no secrets.
  - Context arrives already redacted (SEC-004).
  - The prompt instructs the model never to echo secret-looking literals. The output post-filter runs `telemetry::redact` on `description`.
- **Observability additions:** Span attribute `prompt_version` on `reviewer_execution`. Counter `llm_structured_output_failures_total{reviewer="security"}`.
- **Tests required:**
  - `security_schema_is_valid_json_schema`
  - `security_prompt_version_is_stable_hash`
  - `security_category_roundtrip_snake_case`
  - `security_schema_rejects_missing_entry_point_for_authz_bypass`
  - insta snapshot `security_prompt_render_auth_bypass`
- **Benchmarks if applicable:** EVAL run on the security subset of the corpus (`benchmarks/quality/security/*`). It records structured-output success rate, which must be ≥ 0.98 under replay fixtures recorded from live runs.
- **Acceptance criteria:**
  - The rendered prompt for `fixtures/pull-requests/auth-bypass` matches its snapshot.
  - The schema validates the recorded replay output.
  - `grep -ri reference engine/crates/reviewers/prompts` returns nothing.
- **Definition of done:** Global DoD, plus the prompt and schema committed, `prompt_version` visible on the span, and the snapshot reviewed.

---

### REV-S-002 — Security reviewer with trust-boundary context selection
Status: ☐

- **Task ID:** REV-S-002
- **Title:** Security reviewer with trust-boundary context selection
- **Problem:** Generic context ranking (CTX-005) optimizes for correctness. Security reasoning needs a different set of facts:
  - the entry point that reaches the change
  - the guards and pipes on that path
  - where untrusted input enters
  - the sink it reaches
- **Why it exists:** PRD §43 asks for authz and trust-boundary analysis. The golden scenario (§151, auth-bypass) requires the `UserController.update → AdminService.updateUser → AuthService.authorize` path to be in context.
- **Scope:**
  - `SecurityReviewer: Reviewer`, with the `applies()` and `budget()` rules below.
  - A security context-selection profile: CTX signal weights plus candidate generators for API entry points, guards, validation pipes, DTOs and sinks.
- **Explicit non-scope:**
  - New graph edges. It uses the existing `ROUTES_TO`, `HANDLED_BY`, `GUARDED_BY`, `CALLS`, `READS_FROM` and `WRITES_TO` edges from NEST/IMP.
  - Taint analysis beyond a bounded BFS.
- **Files/modules expected to change:**
  - `engine/crates/reviewers/src/security/mod.rs`
  - `engine/crates/context-engine/src/profiles.rs` (add the `security` weight profile)
  - `engine/crates/reviewers/src/registry.rs`
- **New files/modules expected:**
  - `engine/crates/reviewers/src/security/reviewer.rs`
  - `engine/crates/context-engine/src/candidates/trust_boundary.rs`
- **Dependencies:** REV-S-001, REV-002 (routing), CTX-005, CTX-006, IMP-001..IMP-008, NEST-001..NEST-007 (route, guard and pipe facts), RISK-005.
- **Implementation details:**
  - `applies()` is true when any of these hold:
    - risk signals include `auth_path`, `api_contract_changed`, `guard_changed`, `input_parsing_changed`, `query_construction_changed`, `crypto_or_secret`, `file_path_io` or `outbound_http`
    - a changed symbol is reachable from an `APIEndpoint` within depth 4
    - a `.review/config.yaml` risk path maps to `critical`
  - `applies()` is false for docs-only, test-only or generated-only changes.
  - `budget(risk)` defaults to `{ max_symbols: 40, max_tokens: 12_000, max_tests: 4, max_configs: 6 }`, scaled ×1.5 for `critical`.
  - The `trust_boundary` candidate generator works as follows:
    - For each changed symbol S: `bounded_bfs([S], In, [CALLS, HANDLED_BY, ROUTES_TO], max_depth=4, max_nodes=200)`. Collect `APIEndpoint`s and `QueueConsumer`s.
    - For each entry: collect the `GUARDED_BY` targets, the pipe/DTO validation facts, and the path symbols.
    - Forward from S: `bounded_bfs([S], Out, [CALLS, WRITES_TO, READS_FROM], 2, 100)` to reach sinks (DB writes, raw query builders, `fs`, `child_process`, `http` clients).
  - Security weight profile: `graph_distance 0.30`, `entrypoint_reachability 0.25`, `guard_presence 0.15`, `same_module 0.10`, `lexical 0.10`, `semantic 0.10`. No weight exceeds 0.35 (CTX invariant).
  - Every `CandidateFinding` gets `reviewer = "security"`, `reviewer_version = "security@1"`, `prompt_version` from REV-S-001.
- **Data model changes:** None. It uses `reviewer_runs` and `candidate_findings`.
- **API/protocol changes:** None.
- **Concurrency semantics:**
  - It runs in parallel with the other reviewers inside PIPE-003's `JoinSet`.
  - It reads only the shared `Arc<Graph>`.
  - Its budget is independent of the other reviewers.
- **Failure behavior:**
  - A gateway error or budget exhaustion produces `ReviewerOutcome::Failed { reason }`. The run completes degraded (PIPE-008), and the summary lists "security reviewer did not run" (INV-013).
  - A truncated BFS sets `context.truncated = true` and adds an `omitted[]` entry.
- **Idempotency considerations:** The context package hash and `prompt_version` give a deterministic `input_hash`. A retried stage reuses `stage_outputs`.
- **Security considerations:** The context passes through SEC-004 redaction before the gateway. Guard and decorator names are code facts, not secrets.
- **Observability additions:**
  - Span `reviewer_execution{reviewer_type=security}`, with child spans `context_selection` and `model_request`.
  - Counters `context_symbols_total{reviewer=security}` and `context_tokens_total{reviewer=security}`.
- **Tests required:**
  - `security_applies_on_auth_path_change`
  - `security_skips_docs_only_change`
  - `trust_boundary_collects_endpoint_and_guards` (fixture `nestjs-guards`)
  - `trust_boundary_respects_bfs_budget`
  - `security_reviewer_auth_bypass_replay_produces_expected_candidate`
  - golden `context_package_security_auth_bypass` (insta)
- **Benchmarks if applicable:** Context selection latency for the security profile, added to PERF-006 (p95 < 150 ms on the reference-api snapshot).
- **Acceptance criteria:**
  - `review diff` on `fixtures/pull-requests/auth-bypass` under replay yields exactly one security candidate with category `authz_bypass`. Its `entry_points` include `http:PATCH /users/:id`.
  - The context package contains all three path symbols.
  - Two planted false-positive traps (an upstream guard exists; the endpoint is internal-only) produce no published finding after VER.
- **Definition of done:** Global DoD, plus registration in the reviewer registry, routing table entries, and a golden package snapshot.

---

### REV-S-003 — Security-specific verification checks
Status: ☐

- **Task ID:** REV-S-003
- **Title:** Security-specific verification checks (authorization on all entry paths, validation pipes, injection sinks)
- **Problem:** The generic contradiction stage (VER-007) knows about upstream guards in general. Security candidates need sharper deterministic predicates to confirm or refute claims such as "authorization missing" or "input unvalidated". Without them, a VERIFIER model gets asked "are you sure?" (R4).
- **Why it exists:** It applies ADR-011 to security claims. Deterministic evidence is the main defence against false positives on authz findings, which are the most damaging false positives.
- **Scope:** Three `SecurityCheck`s are registered into verification stages 3, 5 and 6:
  1. `authorization_on_all_paths`
  2. `validation_pipe_present`
  3. `injection_sink_parameterized`

  Each one is evaluated on Graph(head) and, for base/head comparison, on Graph(base).
- **Explicit non-scope:**
  - A general taint engine.
  - Runtime checks.
  - Secrets scanning (SEC-003).
- **Files/modules expected to change:**
  - `engine/crates/verification/src/stages/contradiction.rs`
  - `engine/crates/verification/src/stages/base_head.rs`
  - `engine/crates/verification/src/registry.rs`
- **New files/modules expected:**
  - `engine/crates/verification/src/checks/security/mod.rs`
  - `authz_paths.rs`
  - `validation.rs`
  - `injection.rs`
- **Dependencies:** REV-S-002, VER-004, VER-006, VER-007, NEST-001..NEST-007 (GUARDED_BY, pipe facts), TSA syntax facts (raw query construction).
- **Implementation details:**
  - `trait SecurityCheck { fn id(&self) -> &'static str; fn applies(&self, c: &CandidateFinding) -> bool; fn evaluate(&self, g: &dyn GraphView, c: &CandidateFinding, cfg: &SecurityCheckCfg) -> CheckResult }`.
  - `CheckResult { holds: Tri(True|False|Unknown), evidence: Vec<Evidence>, paths_examined: u32, truncated: bool }`.
  - `authorization_on_all_paths` enumerates all endpoint→S paths with `max_depth=5` and `max_paths=64`. A path counts as protected when any of these is true:
    - The endpoint or its controller class carries a `GUARDED_BY` edge to a guard node.
    - A symbol on the path calls a profile-declared authorization symbol. The set comes from PROF-004 `guard_usage` plus `rules.security.authorization_symbols` in config.
    - A global guard fact (`APP_GUARD`) exists.

    If every path is protected, that contradicts an "authz bypass" claim and reduces confidence by 0.35 (contradiction term). If any path is unprotected, the result is strong supporting evidence and the path is attached.
  - `validation_pipe_present` checks the handler parameters for a `ValidationPipe`/`ParseIntPipe` fact, a global `useGlobalPipes(ValidationPipe)` fact, or a DTO class with class-validator decorators.
  - `injection_sink_parameterized` looks for a syntax fact `RawQuery { template_literal_with_interpolation | string_concat }` reaching `query()`, `createQueryBuilder().where(string)`, `exec`/`spawn` or `fs` path joins. A parameterized form is a contradiction.
  - Base/head: each check is re-evaluated on the base. When the result is identical and exposure is unchanged, the candidate gets `SUPPRESSED_PREEXISTING` (INV-009).
- **Data model changes:** None. Evidence rows (`finding_evidence`) gain `kind = deterministic_check` with `attrs.check_id`, which is already allowed by the DOM-007 enum.
- **API/protocol changes:** None.
- **Concurrency semantics:** The checks are pure functions over immutable graphs and run in parallel per candidate (rayon) inside the VERIFYING stage.
- **Failure behavior:**
  - A path limit or depth truncation gives `Unknown`, which never counts as a contradiction. It records `inference_uncertainty += 0.1`.
  - A check panics never. Errors become `Unknown` with a reason.
- **Idempotency considerations:** Results are cached under `(candidate_fingerprint, verification_version, snapshot pair)` (VER-012). Adding a check bumps `verification_version`.
- **Security considerations:** None beyond the core purpose. The checks read graph facts only, never source text beyond the cited ranges.
- **Observability additions:**
  - Span `finding_verification` gains `check_id` events.
  - Counters `verification_checks_total{check,outcome}` and `verification_check_truncated_total{check}`.
- **Tests required:**
  - `authz_all_paths_guarded_contradicts_bypass`
  - `authz_one_unguarded_path_supports_bypass`
  - `authz_global_app_guard_contradicts`
  - `authz_truncation_is_unknown_not_contradiction`
  - `validation_global_pipe_detected`
  - `injection_template_literal_query_supported`
  - `injection_parameterized_query_contradicts`
  - `security_check_preexisting_in_base_suppressed`
- **Benchmarks if applicable:** `bench_authz_paths_reference` (criterion), p95 < 20 ms per candidate on the reference-api graph.
- **Acceptance criteria:**
  - The auth-bypass fixture is VERIFIED with the unprotected path attached as evidence.
  - The "guard upstream" trap is SUPPRESSED with reason `contradiction:authorization_on_all_paths`.
  - The "pre-existing raw query" trap is `SUPPRESSED_PREEXISTING`.
- **Definition of done:** Global DoD, plus the checks documented in `docs/reviewers/security.md` (new section) and `verification_version` bumped.

---

---

### REV-T-001 — Test reviewer
Status: ☐

- **Task ID:** REV-T-001
- **Title:** Test reviewer (changed behavior without tests, stale mocks, missing negative tests)
- **Problem:** PRD §44 asks the reviewer to find gaps in regression protection. Generic models flag "add tests" on everything, which is pure noise. The reviewer must reason over the actual test mapping (IMP-005).
- **Why it exists:** Test gaps are among the most actionable findings when they are precise: a named behavior, the named test that should cover it, and why that test does not.
- **Scope:**
  - `TestReviewer`, with prompt `tests/v1.md` and its schema.
  - Context from IMP-005 (`TESTS` edges, path conventions, imports) plus mock facts (`jest.mock`, `useValue` providers).
  - Categories:
    - `changed_behavior_untested`
    - `assertion_no_longer_covers`
    - `stale_mock`
    - `missing_negative_test`
    - `missing_concurrency_test`
    - `broken_fixture`
    - `implementation_coupled_test`
- **Explicit non-scope:**
  - Running tests.
  - Coverage measurement.
  - Generating test code (no autofix, PRD non-goal).
- **Files/modules expected to change:**
  - `engine/crates/reviewers/src/registry.rs`
  - `engine/crates/context-engine/src/profiles.rs` (`tests` profile)
- **New files/modules expected:**
  - `engine/crates/reviewers/src/tests_reviewer/{mod.rs,reviewer.rs}`
  - `engine/crates/reviewers/prompts/tests/v1.md`
  - `engine/crates/reviewers/prompts/tests/v1.schema.json`
- **Dependencies:** REV-001, REV-002, REV-T-002, IMP-005, CTX-006, VER-001..VER-012, NEST-001..NEST-007 (Jest adapter facts).
- **Implementation details:**
  - `applies()` is true when a changed symbol has a behavioral change class (CHG-002..005: `condition_changed`, `return_changed`, `exception_changed`, `call_added/removed`, `signature_changed`). It is false when the change is tests-only, rename-only or formatting-only.
  - The context includes:
    - the changed symbol (signature plus changed body)
    - mapped tests (≤4, ranked by `TESTS` edge confidence, then name similarity)
    - their `describe/it` names and assertions (compressed to `expect(...)` lines)
    - the mocks of the changed symbol's dependencies
  - Stale-mock heuristic input: when a mocked method's signature changed (`signature_hash` differs base→head) and the mock's `mockResolvedValue` shape is unchanged, the reviewer receives a `stale_mock_hint`.
  - Every finding must cite `test_case_node_id` (synthetic node `test:{file}#{suite} › {name}`) or state `no_mapped_test: true` with the mapping evidence from REV-T-002.
  - Default threshold override: publish only when confidence ≥ 0.80.
- **Data model changes:** None.
- **API/protocol changes:** `packages/contracts` gains `TestCandidateExtension`.
- **Concurrency semantics:** Same as the other reviewers (parallel, read-only graph).
- **Failure behavior:** Degraded completion. No mapped tests and no REV-T-002 evidence means no finding (silence over speculation).
- **Idempotency considerations:** Standard reviewer stage key.
- **Security considerations:** Test fixtures often contain fake credentials, and SEC-004 redaction applies to them too.
- **Observability additions:** Span `reviewer_execution{reviewer_type=tests}`. Counter `candidate_findings_total{reviewer=tests}`.
- **Tests required:**
  - `tests_reviewer_skips_test_only_change`
  - `tests_reviewer_context_includes_mapped_tests`
  - `stale_mock_hint_on_signature_change`
  - `tests_reviewer_replay_missing_negative_test`
  - `tests_reviewer_no_finding_without_mapping_evidence`
- **Benchmarks if applicable:** EVAL test-gap subset (QB-001 adds ≥8 labelled test-gap PRs). The FP rate on safe PRs must be ≤ 5%.
- **Acceptance criteria:**
  - On `fixtures/pull-requests/condition-changed-untested`, the reviewer produces `changed_behavior_untested` citing REV-T-002 evidence.
  - On `fixtures/pull-requests/refactor-covered`, it produces no finding.
- **Definition of done:** Global DoD, plus `docs/reviewers/tests.md`.

---

### REV-T-002 — Deterministic pre-check: changed public symbol with zero TESTS edges
Status: ☐

- **Task ID:** REV-T-002
- **Title:** Deterministic pre-check (changed public symbol with zero TESTS edges)
- **Problem:** "Is this behavior tested at all?" is a graph question. Asking a model wastes tokens and invites hallucinated test names.
- **Why it exists:** It follows principle 2 (deterministic before probabilistic). It produces strong evidence for REV-T-001 and feeds POL-003 `public_api_changes_require_tests`.
- **Scope:** `tests_precheck(change_model, graph_head, profile) -> Vec<DeterministicFinding>` flags each behaviorally changed symbol that meets all of these:
  - It is exported or public, or it is an endpoint handler.
  - It has zero inbound `TESTS` edges with confidence ≥ 0.6 (direct or via a ≤1-hop wrapper).
  - The PR adds or changes no test file mapped to it.
- **Explicit non-scope:**
  - Assertion quality.
  - Mocks (REV-T-001).
- **Files/modules expected to change:** `engine/crates/pipeline/src/stages/analysis.rs` (invoke the pre-checks).
- **New files/modules expected:** `engine/crates/impact/src/prechecks/tests_gap.rs`
- **Dependencies:** IMP-005, CHG-002..CHG-005, CG-008 (reverse views), PIPE-004.
- **Implementation details:**
  - `DeterministicFinding { rule_id: "tests.public_symbol_untested", symbol_key, anchor_range, evidence: [GraphAbsence { edge_kind: TESTS, min_confidence: 0.6, searched_depth: 1 }], severity_hint: medium|low }`.
  - Severity is medium when the symbol is an endpoint handler or has >5 callers, and low otherwise.
  - Test files touched in the PR count as covering when they import the symbol's module or contain the symbol name in a `describe` title (IMP-005 conventions).
  - Its output enters `ChangeModel.risk_signals` as `untested_public_change` and the candidate pool with `reviewer = "deterministic:tests"`.
- **Data model changes:** None. `candidate_findings.reviewer` accepts the `deterministic:*` prefix (DOM-006).
- **API/protocol changes:** None.
- **Concurrency semantics:** Pure, and runs in the ANALYZING stage.
- **Failure behavior:** If test mapping is unavailable (no test framework detected by INIT), the pre-check is skipped and records `NOT_EXECUTED` with a reason (INV-014). It never passes silently.
- **Idempotency considerations:** Deterministic, with output stored in `stage_outputs`.
- **Security considerations:** None.
- **Observability additions:** Counter `deterministic_findings_total{rule="tests.public_symbol_untested"}`. Span event on `impact_analysis`.
- **Tests required:**
  - `untested_exported_method_flagged`
  - `tested_via_wrapper_not_flagged`
  - `pr_adding_test_suppresses_flag`
  - `private_helper_not_flagged`
  - `no_test_framework_reports_not_executed`
- **Benchmarks if applicable:** None. It is O(changed symbols × in-degree).
- **Acceptance criteria:**
  - On the fixture `untested-public-change`, exactly one deterministic finding is produced with `GraphAbsence` evidence.
  - Status `NOT_EXECUTED` is visible in the summary when there is no test framework.
- **Definition of done:** Global DoD.

---

---

### REV-A-001 — Architecture reviewer using profile and rules
Status: ☐

- **Task ID:** REV-A-001
- **Title:** Architecture reviewer using profile and rules
- **Problem:** Architecture opinions without repository evidence are explicitly suppressed (PRD §60). A useful architecture reviewer must reason from the repository's own declared and inferred structure.
- **Why it exists:** PRD §46. Profile data "is especially important here".
- **Scope:**
  - `ArchitectureReviewer`, with prompt `architecture/v1.md` and its schema.
  - Input:
    - the profile layers and modules (PROF-002)
    - the applicable conventions with their confidence (PROF-003..005)
    - explicit rules and violations (POL-003/005)
    - REV-A-002 detector hits
  - Categories:
    - `boundary_violation`
    - `forbidden_dependency`
    - `cross_layer_coupling`
    - `ownership_violation`
    - `abstraction_bypass`
    - `cycle_risk`
    - `duplicate_architecture`
    - `state_ownership`
- **Explicit non-scope:**
  - Inventing conventions. The prompt may cite only conventions from the provided list, each carrying `convention_id`.
  - Style.
- **Files/modules expected to change:**
  - `engine/crates/reviewers/src/registry.rs`
  - `engine/crates/context-engine/src/profiles.rs` (`architecture` profile)
- **New files/modules expected:**
  - `engine/crates/reviewers/src/architecture/{mod.rs,reviewer.rs}`
  - `engine/crates/reviewers/prompts/architecture/v1.md`
  - `engine/crates/reviewers/prompts/architecture/v1.schema.json`
- **Dependencies:** REV-001, REV-002, REV-A-002, PROF-002, PROF-003, PROF-004, POL-004, POL-005, VER-001..VER-012.
- **Implementation details:**
  - `applies()` is true when any of these hold:
    - The PR adds or changes an `IMPORTS`/`DEPENDS_ON` edge that crosses a module or layer boundary.
    - A new module, provider or repository class appears.
    - REV-A-002 has hits.
    - A migration plus service change happens together (database-safety focus, see REV-002).
  - The schema requires `basis: { kind: explicit_rule|documented|convention, id, confidence }` for every finding. The precedence resolver (POL-004) supplies `kind`.
  - Verification stage 4 rejects any finding whose `basis.id` is not in the profile or ruleset, with `SUPPRESSED_NOT_ACTIONABLE` reason `unknown_basis`.
  - Convention-based findings require convention confidence ≥ 0.9 and samples ≥ 10 (target-arch §3.10). Lower values produce no finding.
- **Data model changes:** None.
- **API/protocol changes:** `ArchitectureCandidateExtension` in contracts.
- **Concurrency semantics:** Parallel reviewer. The profile is read from the cached `Arc<RepositoryProfile>` (PROF-006).
- **Failure behavior:** If there is no profile for the snapshot, the reviewer still runs on explicit rules only and records `profile_unavailable` in reviewer_runs. If there is no profile and no rules, `applies()` is false and the summary records it.
- **Idempotency considerations:** `profile_version` and `config_hash` join the reviewer `input_hash`.
- **Security considerations:** None.
- **Observability additions:** Span `reviewer_execution{reviewer_type=architecture}`. Counter `candidate_findings_total{reviewer=architecture,basis}`.
- **Tests required:**
  - `architecture_requires_basis`
  - `architecture_low_confidence_convention_not_used`
  - `architecture_explicit_rule_outranks_convention`
  - `architecture_replay_controller_uses_repository`
  - `architecture_skips_without_boundary_change`
- **Benchmarks if applicable:** EVAL architecture subset (QB-001: ≥6 PRs), FP ≤ 5%.
- **Acceptance criteria:**
  - The fixture `controller-injects-repository` yields a verified finding with `basis.kind = explicit_rule` when the rule is configured, and `convention` (confidence ≥ 0.9) when it is not.
  - The safe refactor fixture yields nothing.
- **Definition of done:** Global DoD, plus `docs/reviewers/architecture.md`.

---

### REV-A-002 — Deterministic boundary-violation and cycle detector
Status: ☐

- **Task ID:** REV-A-002
- **Title:** Deterministic boundary-violation and cycle detector
- **Problem:** Forbidden dependencies and new import cycles are graph facts. Legacy delegated them to `depcruise` in the reference consumer only.
- **Why it exists:** It provides deterministic evidence for REV-A-001 and POL-003 `forbidden_dependencies`, and it replaces the the reference consumer depcruise gate generically.
- **Scope:**
  1. Layer and module boundary check: every new or changed `IMPORTS`/`DEPENDS_ON`/`CALLS` edge from a changed file is checked against the layer map (PROF-002 plus `rules.layers`).
  2. Cycle detection: a module-level SCC on the head graph, restricted to modules touched by the PR. A cycle is reported only when it is new (absent in base).
- **Explicit non-scope:**
  - Whole-repository cycle reports.
  - Symbol-level cycles (recursion).
- **Files/modules expected to change:** `engine/crates/pipeline/src/stages/analysis.rs`.
- **New files/modules expected:**
  - `engine/crates/impact/src/prechecks/boundaries.rs`
  - `engine/crates/impact/src/prechecks/cycles.rs`
- **Dependencies:** PROF-002, POL-001, CG-007 (queries), INC-007 (edge diff), IMP-001.
- **Implementation details:**
  - The module graph is a projection of `IMPORTS` edges onto `Module` nodes. It is built lazily for the touched module set plus 1-hop neighbors, with `max_nodes = 2_000`.
  - SCCs use Tarjan, on head and base. `new_cycles = scc_head − scc_base`, compared by sorted member set.
  - `DeterministicFinding { rule_id: "architecture.forbidden_dependency" | "architecture.new_cycle", edges: [EdgeRef], layers: (from, to), anchor: import statement range }`.
- **Data model changes:** None.
- **API/protocol changes:** None.
- **Concurrency semantics:** Pure. It runs in the ANALYZING stage alongside REV-T-002.
- **Failure behavior:**
  - No layer map gives `NOT_EXECUTED("no layers defined or inferred")`.
  - Exceeding the projection budget gives a `truncated` finding set plus a summary note.
- **Idempotency considerations:** Deterministic.
- **Security considerations:** None.
- **Observability additions:** Counter `deterministic_findings_total{rule}`. Histogram `cycle_detection_duration_seconds`.
- **Tests required:**
  - `forbidden_controller_to_repository_import_flagged`
  - `allowed_layer_import_not_flagged`
  - `new_cycle_detected`
  - `preexisting_cycle_not_reported`
  - `no_layers_reports_not_executed`
- **Benchmarks if applicable:** `bench_cycles_touched_modules` on a synthetic 100k-file repo (PERF-001), p95 < 50 ms.
- **Acceptance criteria:**
  - On the fixture `cycle-introduced`, exactly one `architecture.new_cycle` finding lists all members.
  - On the base-cycle fixture, nothing is reported.
- **Definition of done:** Global DoD.

---

---

### REV-P-001 — Performance reviewer (concrete execution path required)
Status: ☐

- **Task ID:** REV-P-001
- **Title:** Performance reviewer (concrete execution path required)
- **Problem:** PRD §45 says "It should not speculate about performance without a plausible execution path". Without a structural requirement, models flag every loop.
- **Why it exists:** It catches N+1, unbounded loops, synchronous I/O on hot paths and full-table scans, but only along a proven path from an entry point.
- **Scope:**
  - `PerformanceReviewer`, with prompt `performance/v1.md` and its schema.
  - The schema requires an `execution_path` (symbol keys from an entry point or a queue consumer to the hot spot) and a `cost_driver`: `loop_over_collection|per_item_query|sync_io|unbounded_query|cache_bypass|repeated_computation`.
- **Explicit non-scope:**
  - Micro-optimizations.
  - Benchmark execution.
  - Database EXPLAIN.
- **Files/modules expected to change:**
  - `engine/crates/reviewers/src/registry.rs`
  - `engine/crates/context-engine/src/profiles.rs`
- **New files/modules expected:**
  - `engine/crates/reviewers/src/performance/{mod.rs,reviewer.rs}`
  - `engine/crates/reviewers/prompts/performance/v1.md`
  - `engine/crates/reviewers/prompts/performance/v1.schema.json`
- **Dependencies:** REV-001, REV-002, REV-P-002, IMP-001..IMP-008, CTX-006, VER-004.
- **Implementation details:**
  - `applies()` is true when any of these hold:
    - change classes include `loop_changed`, `call_added` inside a loop, `query_changed` or `await_added`
    - REV-P-002 has hits
    - risk signal `hot_path` is present (the endpoint is in the profile's high-traffic list or `rules.performance.hot_paths`)
  - Verification stage 3 checks every `execution_path` hop as a real `CALLS`/`HANDLED_BY` edge with confidence ≥ 0.6. Any missing hop gives `SUPPRESSED_NOT_ACTIONABLE("unproven_path")`.
- **Data model changes:** None.
- **API/protocol changes:** `PerformanceCandidateExtension`.
- **Concurrency semantics:** Parallel reviewer.
- **Failure behavior:** Degraded completion.
- **Idempotency considerations:** Standard.
- **Security considerations:** None.
- **Observability additions:** Span `reviewer_execution{reviewer_type=performance}`.
- **Tests required:**
  - `performance_finding_without_path_rejected`
  - `performance_replay_n_plus_one`
  - `performance_skips_without_loop_or_query_change`
  - `performance_path_hop_confidence_checked`
- **Benchmarks if applicable:** EVAL performance subset (≥6 PRs).
- **Acceptance criteria:**
  - The fixture `n-plus-one-in-loop` yields one verified finding with an endpoint→loop path.
  - The fixture `loop-over-constant-array` yields nothing.
- **Definition of done:** Global DoD, plus `docs/reviewers/performance.md`.

---

### REV-P-002 — Deterministic N+1 and IO-in-loop detector
Status: ☐

- **Task ID:** REV-P-002
- **Title:** Deterministic N+1 / IO-in-loop detector
- **Problem:** "A repository or HTTP call inside a loop body over a collection" is a syntax-and-graph fact.
- **Why it exists:** It provides strong evidence for REV-P-001 and stops the model from deciding on its own what counts as IO.
- **Scope:** For each changed symbol, find:
  - `SyntaxFact::Loop { kind: for_of|for|while|map|forEach|reduce, range }` containing an `await` call whose target resolves, within ≤2 hops, to:
    - a TypeORM repository or query-builder method (TypeORM adapter facts)
    - a `PRODUCES_JOB` edge
    - an outbound HTTP client
    - `fs.*Sync`
  - `Promise.all(items.map(async ...))` with a per-item query, flagged as `fan_out_query` (a lower severity hint).
- **Explicit non-scope:**
  - Unchanged code.
  - Loops over literal arrays of ≤10 elements.
- **Files/modules expected to change:** `engine/crates/pipeline/src/stages/analysis.rs`.
- **New files/modules expected:** `engine/crates/impact/src/prechecks/io_in_loop.rs`
- **Dependencies:** TSA (loop/await syntax facts), NEST-001..NEST-007 (TypeORM/BullMQ facts), CHG-002..CHG-005, IMP-002.
- **Implementation details:**
  - `DeterministicFinding { rule_id: "performance.io_in_loop" | "performance.fan_out_query", loop_range, io_call_range, io_kind, path: [SymbolKey] }`.
  - It is reported only if the loop or the IO call is in a changed hunk, or the loop's iterable source changed.
  - IO targets are resolved through `CALLS` edges with confidence ≥ 0.6.
- **Data model changes:** None.
- **API/protocol changes:** None.
- **Concurrency semantics:** Pure.
- **Failure behavior:** If loop facts are missing (analyzer version too old), it gives `NOT_EXECUTED`.
- **Idempotency considerations:** Deterministic.
- **Security considerations:** None.
- **Observability additions:** Counter `deterministic_findings_total{rule="performance.io_in_loop"}`.
- **Tests required:**
  - `await_repository_find_in_for_of_flagged`
  - `batched_find_in_not_flagged`
  - `literal_small_array_not_flagged`
  - `io_two_hops_deep_flagged`
  - `unchanged_loop_not_flagged`
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** The fixture `n-plus-one-in-loop` produces exactly one `io_in_loop` finding with the `io_kind = db_read` path.
- **Definition of done:** Global DoD.

---

---

### REV-M-001 — Maintainability reviewer (high threshold ≥ 0.9)
Status: ☐

- **Task ID:** REV-M-001
- **Title:** Maintainability reviewer (high threshold ≥0.9)
- **Problem:** Style noise is "particularly damaging" (PRD §47). The reviewer must comment almost never, and only on evidence-backed issues.
- **Why it exists:** It covers duplicated logic, unreviewable functions and abstraction leakage, which are useful when they are rare and concrete.
- **Scope:**
  - `MaintainabilityReviewer`, with prompt `maintainability/v1.md` and its schema.
  - Categories:
    - `duplicated_logic` (requires the duplicate's symbol key and a similarity ≥ 0.85 from the semantic layer or a body token Jaccard)
    - `unreviewable_function` (requires measured metrics: cyclomatic > 25 or > 150 lines, newly crossed in this PR)
    - `abstraction_leak`
    - `inconsistent_pattern` (requires a convention with confidence ≥ 0.9)
    - `unreadable_control_flow` (nesting depth > 5, newly introduced)
  - A per-reviewer threshold of `minimum_publish = 0.90`, which config cannot lower below 0.85.
  - Off by default (PRD §122 example).
- **Explicit non-scope:**
  - Naming and formatting.
  - Anything a linter covers. Findings that duplicate configured lint rules are suppressed (VER-011).
- **Files/modules expected to change:**
  - `engine/crates/reviewers/src/registry.rs`
  - `engine/crates/verification/src/thresholds.rs` (per-reviewer floor)
- **New files/modules expected:**
  - `engine/crates/reviewers/src/maintainability/{mod.rs,reviewer.rs,metrics.rs}`
  - `engine/crates/reviewers/prompts/maintainability/v1.md`
  - `engine/crates/reviewers/prompts/maintainability/v1.schema.json`
- **Dependencies:** REV-001, REV-002, SEM-006 (similar-symbol search), PROF-003, VER-010, POL-001.
- **Implementation details:**
  - `metrics.rs` computes cyclomatic complexity, line count and max nesting from syntax facts on base and head. A metric qualifies only if the threshold is crossed base→head.
  - Duplicate candidates are found by a Qdrant `code_chunk` search scoped by `TenantScope` with similarity ≥ 0.85, then confirmed with token Jaccard ≥ 0.7 on normalized bodies.
  - `applies()` requires `review.reviewers.maintainability: true` and at least one qualifying metric or a duplicate candidate. Without a deterministic seed, the model is never called.
- **Data model changes:** None.
- **API/protocol changes:** `MaintainabilityCandidateExtension`.
- **Concurrency semantics:** Parallel reviewer.
- **Failure behavior:** Degraded completion. If Qdrant is unavailable, duplicate detection is skipped and recorded.
- **Idempotency considerations:** Standard.
- **Security considerations:** The Qdrant query carries `TenantScope` (SEM-005).
- **Observability additions:** Span `reviewer_execution{reviewer_type=maintainability}`. Counter `suppressed_findings_total{reviewer=maintainability,reason}`.
- **Tests required:**
  - `maintainability_disabled_by_default`
  - `maintainability_threshold_floor_085`
  - `complexity_crossed_threshold_qualifies`
  - `complexity_preexisting_not_qualify`
  - `duplicate_requires_two_signals`
  - `no_model_call_without_seed`
- **Benchmarks if applicable:** EVAL maintainability subset. On safe PRs the published FP must be 0, and on the whole corpus ≤ 2%.
- **Acceptance criteria:**
  - When enabled on `fixtures/pull-requests/duplicated-validation`, the reviewer yields one verified `duplicated_logic` finding.
  - Across all other corpus PRs it publishes zero maintainability findings.
- **Definition of done:** Global DoD, plus `docs/reviewers/maintainability.md`.

---

---

### PROF-001 — RepositoryProfile model and persistence
Status: ☐

- **Task ID:** PROF-001
- **Title:** RepositoryProfile model & persistence
- **Problem:** PRD §62 lists 13 profile aspects, and nothing stores them. The the reference consumer vault is hand-written and not machine-readable.
- **Why it exists:** Architecture reviewers, context ranking and POL precedence all need a typed, versioned profile keyed to a snapshot.
- **Scope:**
  - Rust types in the `profile` crate.
  - A PostgreSQL schema.
  - A `ProfileStore` port with PG and local-file adapters (`.review/profile.json`).
  - JSON Schema export to contracts.
- **Explicit non-scope:**
  - Inference logic (PROF-002..005).
  - Caching policy (PROF-006).
- **Files/modules expected to change:**
  - `engine/crates/profile/src/lib.rs`
  - `packages/contracts/schemas/` (generated)
- **New files/modules expected:**
  - `engine/crates/profile/src/model.rs`
  - `engine/crates/profile/src/store/{mod.rs,pg.rs,file.rs}`
  - `engine/migrations/{seq}_repository_profiles.sql`
- **Dependencies:** DOM-003 (versions), DOM-009 (migrations), GS-001 (store conventions), INIT-011 (`.review/` layout).
- **Implementation details:**

  ```rust
  pub struct RepositoryProfile { repository_id: RepositoryId, snapshot_id: SnapshotId, profile_version: u32,
    config_hash: ConfigHash, languages: Vec<LanguageShare>, frameworks: Vec<FrameworkFact>,
    architecture: ArchitectureModel, modules: Vec<ModuleBoundary>, conventions: Vec<Convention>,
    testing: TestingProfile, api: ApiProfile, security: SecurityProfile, persistence: PersistenceProfile,
    queues: QueueProfile, naming: NamingProfile, errors: ErrorHandlingProfile, documentation_rules: Vec<DocumentationRuleRef>,
    historical: Option<HistoricalSignalsRef>, computed_at: DateTime<Utc> }
  pub struct Convention { id: ConventionId, rule: String, scope: Glob, samples: u32, violations: u32,
    consistency: f32, confidence: f32, exceptions: Vec<ConventionException>, source: ConventionSource /*inferred|documented|explicit*/,
    computed_at, profile_version }
  ```

  The SQL:

  ```sql
  CREATE TABLE repository_profiles (id uuid PK, organization_id uuid NOT NULL, repository_id uuid NOT NULL,
    snapshot_id uuid NOT NULL REFERENCES snapshots(id), profile_version int NOT NULL, config_hash text NOT NULL,
    profile jsonb NOT NULL, computed_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (repository_id, snapshot_id, profile_version, config_hash));
  CREATE TABLE profile_conventions (profile_id uuid REFERENCES repository_profiles(id) ON DELETE CASCADE,
    convention_id text, rule text, scope text, samples int, violations int, consistency real, confidence real,
    exceptions jsonb, source text CHECK (source IN ('inferred','documented','explicit')), PRIMARY KEY(profile_id, convention_id));
  ```

  Both tables get RLS policies on `organization_id` (API-003 pattern).
- **Data model changes:** The two tables above.
- **API/protocol changes:** The `RepositoryProfile` JSON Schema in `packages/contracts`. It is consumed by API-008 `GET /repositories/:id/profile`.
- **Concurrency semantics:** Profiles are immutable per key. Writes use `INSERT ... ON CONFLICT DO NOTHING`, so concurrent computations of the same key converge.
- **Failure behavior:** A store write failure fails the profile stage. Reviewers then run in `profile_unavailable` mode, and the run itself does not fail.
- **Idempotency considerations:** The natural key `(repository_id, snapshot_id, profile_version, config_hash)`.
- **Security considerations:** The profile contains code facts only, never source bodies. It is tenant-scoped through RLS.
- **Observability additions:** Span `profile_compute` (parent `repository_index`). Histogram `profile_compute_duration_seconds`.
- **Tests required:**
  - `profile_roundtrip_pg` (integration)
  - `profile_roundtrip_file`
  - `profile_insert_conflict_is_noop`
  - `profile_store_conformance_suite` (same suite against both adapters)
  - `profile_schema_exported`
- **Benchmarks if applicable:** None.
- **Acceptance criteria:**
  - The migration applies cleanly.
  - The conformance suite passes against both adapters.
  - The contracts schema is generated, and CI-006 shows no drift.
- **Definition of done:** Global DoD.

---

### PROF-002 — Architecture, module boundary and layer inference from the graph
Status: ☐

- **Task ID:** PROF-002
- **Title:** Architecture/module boundary/layer inference from graph
- **Problem:** Boundary checks (REV-A-002) and the architecture reviewer need a layer map. Most repositories never declare one.
- **Why it exists:** PRD §62 lists architecture and module boundaries. It generalizes the the reference consumer `depcruise` layers into inference over any repository.
- **Scope:** It infers:
  - modules: NestJS `@Module` facts, workspace packages, and top-level directories under the source roots
  - layers: role classification of each class/file as `controller|service|repository|entity|dto|guard|processor|module|config|util|test` from framework facts, then suffix conventions, then directory names
  - the observed layer dependency matrix: counts of `IMPORTS`/`CALLS` between layers
- **Explicit non-scope:**
  - Enforcing anything (POL-003).
  - consumer-specific layer names.
- **Files/modules expected to change:** `engine/crates/profile/src/lib.rs`.
- **New files/modules expected:**
  - `engine/crates/profile/src/infer/architecture.rs`
  - `engine/crates/profile/src/infer/roles.rs`
- **Dependencies:** PROF-001, CG-005, CG-007, NEST-001..NEST-007, INIT-005 (workspaces).
- **Implementation details:**
  - Role precedence:
    1. framework fact (confidence 0.95)
    2. file suffix `*.controller.ts` (0.85)
    3. directory segment `controllers/` (0.7)
    4. unknown
  - `ArchitectureModel { layers: Vec<Layer{name, globs, member_count, role_confidence}>, matrix: HashMap<(Layer,Layer), EdgeCount>, modules: Vec<ModuleBoundary{ id, root, public_exports, internal_dependents }> }`.
  - `ModuleBoundary.public_exports` comes from NestJS `exports: [...]` facts or barrel `index.ts`.
  - The matrix is emitted as conventions, for example `layer_dependency:controller->repository` with `samples = controller count` and `violations = controllers with a repository edge`. These feed PROF-003 confidence.
- **Data model changes:** None (inside the profile jsonb).
- **API/protocol changes:** None.
- **Concurrency semantics:** Pure over `Arc<Graph>`. It runs after the full index and after each default-branch delta.
- **Failure behavior:** A repository with no recognizable roles gets an empty layer list and a profile note `architecture_inference: insufficient_signal`.
- **Idempotency considerations:** Deterministic for a given snapshot (sorted iteration).
- **Security considerations:** None.
- **Observability additions:** Gauges `profile_layers_total` and `profile_modules_total` as span attributes on `profile_compute`.
- **Tests required:**
  - `roles_from_nest_decorators`
  - `roles_from_suffix_fallback`
  - `layer_matrix_counts_fixture`
  - `module_public_exports_from_nest_module`
  - `inference_deterministic_order`
- **Benchmarks if applicable:** `bench_architecture_inference` on the 100k-file synthetic repo (PERF-001), < 5 s.
- **Acceptance criteria:** On `fixtures/repositories/nestjs-layered`, the inferred layers equal the golden JSON, and the matrix shows 0 controller→repository edges except for the planted one.
- **Definition of done:** Global DoD, plus the golden snapshot.

---

### PROF-003 — Convention miner framework
Status: ☐

- **Task ID:** PROF-003
- **Title:** Convention miner framework (samples, violations, consistency, exceptions, confidence, scope)
- **Problem:** PRD §63 says inference must separate intentional convention from incidental repetition. R10 is that accidental patterns get enforced.
- **Why it exists:** It is one framework that every convention (PROF-004/005) plugs into, so the confidence semantics are uniform and calibratable.
- **Scope:**
  - The `ConventionMiner` trait.
  - Sample collection.
  - Scope narrowing (choosing the tightest glob with high consistency).
  - Exception detection.
  - A confidence formula.
  - A registry.
- **Explicit non-scope:**
  - Specific conventions (PROF-004/005).
  - Historical signals (HIST).
- **Files/modules expected to change:** `engine/crates/profile/src/lib.rs`.
- **New files/modules expected:**
  - `engine/crates/profile/src/conventions/{mod.rs,miner.rs,scope.rs,confidence.rs,registry.rs}`
- **Dependencies:** PROF-001, PROF-002.
- **Implementation details:**

  ```rust
  trait ConventionMiner { fn id(&self) -> &'static str; fn version(&self) -> u32;
    fn population(&self, g: &dyn GraphView, p: &PartialProfile) -> Vec<Sample>;   // e.g. all service methods that write DB
    fn conforms(&self, s: &Sample, g: &dyn GraphView) -> Conformance; }           // Conforms | Violates{evidence} | NotApplicable
  ```

  - Consistency is `conforming / (conforming + violating)`.
  - Confidence is `consistency × sample_factor × spread_factor`, where:
    - `sample_factor = min(1, ln(1+n)/ln(1+30))`
    - `spread_factor = min(1, distinct_modules/3)` (a pattern seen in only one module is weak)
  - Scope narrowing evaluates candidate globs (source root, then `src/modules/**`, then per-module) and picks the broadest scope with consistency ≥ 0.9.
  - Exceptions are violations annotated with `// review-ignore: <convention-id>`, files under the generated globs, and paths listed in config `conventions.exceptions`.
  - A convention is **enforceable** only if confidence ≥ 0.9 and samples ≥ 10 (target-arch §3.10). Everything else is stored as `informational`.
- **Data model changes:** None (`profile_conventions` from PROF-001).
- **API/protocol changes:** None.
- **Concurrency semantics:** Miners run in parallel (rayon) over an immutable graph.
- **Failure behavior:** A miner error is logged and that convention is omitted. The other miners proceed.
- **Idempotency considerations:** Deterministic. The miner version enters `profile_version`, so a change to any miner bumps it.
- **Security considerations:** None.
- **Observability additions:** Counter `conventions_mined_total{miner,enforceable}`. Histogram `convention_miner_duration_seconds{miner}`.
- **Tests required:**
  - `confidence_formula_table` (property test: monotonic in samples and consistency)
  - `single_module_pattern_low_confidence`
  - `scope_narrowing_picks_broadest_consistent`
  - `review_ignore_annotation_is_exception`
  - `below_threshold_is_informational`
- **Benchmarks if applicable:** None.
- **Acceptance criteria:**
  - The PRD §63 example (38 methods, 37 conform) yields confidence ≥ 0.9 when spread across ≥3 modules.
  - The same counts concentrated in one module yield < 0.9.
- **Definition of done:** Global DoD, plus `docs/reviewers/conventions.md` documenting the formula.

---

### PROF-004 — Conventions: controllers-not-repositories, transaction wrapper on writes, error hierarchy, guard usage
Status: ☐

- **Task ID:** PROF-004
- **Title:** Conventions: controllers-not-repositories, transaction wrapper on writes, error hierarchy, guard usage
- **Problem:** These four are the highest-value structural conventions for NestJS services. The legacy the reference consumer prompts carried them as hard-coded prose.
- **Why it exists:** It provides concrete, generic miners that the reference consumer and any other NestJS repository both benefit from (A/B extraction, PRD §125–127).
- **Scope:** Four miners:
  1. `controllers_do_not_access_repositories`
  2. `db_writes_use_transaction_wrapper`
  3. `errors_extend_domain_hierarchy`
  4. `endpoints_are_guarded`
- **Explicit non-scope:**
  - the reference consumer tenant-specific rules. These live in POL-007.
- **Files/modules expected to change:** `engine/crates/profile/src/conventions/registry.rs`.
- **New files/modules expected:**
  - `engine/crates/profile/src/conventions/builtin/{controller_repository.rs,transaction_wrapper.rs,error_hierarchy.rs,guard_usage.rs}`
- **Dependencies:** PROF-003, NEST-001..NEST-007 (guards, TypeORM repository, transaction facts), TSA syntax facts (db write calls, transaction wrappers, throws).
- **Implementation details:**
  - **Controller/repository.** The population is controller classes. A controller violates when any `DEPENDS_ON`/`CALLS` edge goes to a `repository`-role symbol or to a TypeORM `Repository<T>` injection.
  - **Transaction wrapper.** The population is service methods with a `SyntaxFact::DbWrite`. A method conforms when the write is lexically inside a `SyntaxFact::TransactionWrapper` (`dataSource.transaction`, `queryRunner.startTransaction`, `@Transactional`, or a profile-declared wrapper symbol), or when every caller wraps it. That is the "caller-owned transaction" case, checked through `CALLS` in-edges at depth 1.
  - **Error hierarchy.** The population is `throw new X(...)` in services. A throw conforms when X extends a common base class. The miner discovers the base as the most frequent ancestor of thrown classes, as long as that ancestor is not a built-in `Error`/`HttpException`.
  - **Guard usage.** The population is endpoints. An endpoint conforms when it is `GUARDED_BY` (method, class or global). The miner also records `authorization_symbols`: the symbols most frequently called first in guarded handlers. REV-S-003 consumes them.
- **Data model changes:** None.
- **API/protocol changes:** None.
- **Concurrency semantics:** Same as PROF-003.
- **Failure behavior:** Missing framework facts make the population empty, and no convention is emitted.
- **Idempotency considerations:** Deterministic.
- **Security considerations:** None.
- **Observability additions:** Covered by the PROF-003 counters with the miner label.
- **Tests required:**
  - `controller_repository_violation_detected`
  - `transaction_caller_owned_conforms`
  - `transaction_missing_violates`
  - `error_base_discovered`
  - `guard_global_counts_as_conforming`
  - `authorization_symbols_extracted`
  - golden `reference_api_conventions` (expected JSON, runs in CI when the fixture snapshot is present)
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** On `fixtures/repositories/nestjs-layered`, all four conventions are produced with the golden samples, violations and confidence values. The planted missing-transaction method is listed as a violation with evidence.
- **Definition of done:** Global DoD.

---

### PROF-005 — Testing, API and queue conventions
Status: ☐

- **Task ID:** PROF-005
- **Title:** Testing/API/queue conventions
- **Problem:** The test reviewer, the API contract checks and POL-003 `queue_jobs.require_deterministic_id` need profile facts that nothing gathers yet:
  - where tests live
  - how endpoints are versioned
  - how jobs are identified
- **Why it exists:** It covers the PRD §62 testing, API and queue aspects.
- **Scope:** Miners and profile sections:
  - `TestingProfile { frameworks, file_patterns, colocated|separate, mock_style: jest.mock|provider_override, e2e_roots }`
  - convention `public_services_have_unit_tests`
  - `ApiProfile { versioning: path|header|none, dto_validation: global|per_route|none, response_envelope? }`
  - convention `endpoints_use_dto_validation`
  - `QueueProfile { queues: [queue:{name}], producers, consumers }`
  - convention `jobs_have_deterministic_id` (a `queue.add(name, data, { jobId })` fact with a non-random jobId expression)
- **Explicit non-scope:**
  - The rule evaluators themselves (POL-003).
- **Files/modules expected to change:**
  - `engine/crates/profile/src/conventions/registry.rs`
  - `engine/crates/profile/src/model.rs`
- **New files/modules expected:**
  - `engine/crates/profile/src/conventions/builtin/{testing.rs,api.rs,queues.rs}`
- **Dependencies:** PROF-003, IMP-005, NEST-001..NEST-007 (BullMQ, route facts), INIT-006 (test detection).
- **Implementation details:**
  - A deterministic jobId means a `jobId` property whose expression contains no `uuid()`, `Date.now()`, `Math.random()` or `nanoid()`, built from template literals over identifiers.
  - API versioning is the majority prefix of routes (`/v\d+/`), recorded with a consistency value.
  - The test layout is the ratio of colocated (`*.spec.ts` next to source) to `test/` directory files.
- **Data model changes:** None.
- **API/protocol changes:** None.
- **Concurrency semantics:** Same as PROF-003.
- **Failure behavior:** Without BullMQ, `QueueProfile.queues` is empty and the convention is not emitted.
- **Idempotency considerations:** Deterministic.
- **Security considerations:** None.
- **Observability additions:** Covered by the PROF-003 counters.
- **Tests required:**
  - `deterministic_job_id_detected`
  - `random_job_id_violates`
  - `api_versioning_majority`
  - `test_layout_colocated`
  - `dto_validation_global_pipe`
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** On `fixtures/repositories/nestjs-bullmq`, `jobs_have_deterministic_id` lists the planted `uuid()` violation, and `QueueProfile` lists both queues.
- **Definition of done:** Global DoD.

---

### PROF-006 — Profile versioning and cache
Status: ☐

- **Task ID:** PROF-006
- **Title:** Profile versioning & cache
- **Problem:** Recomputing the profile on every PR repeats the full-repository work that Invariant 1 forbids.
- **Why it exists:** Target-arch §7 defines the cache key `(snapshot_id, profile_version, config_hash)`.
- **Scope:**
  - `profile_version` = hash of all miner versions plus the inference code version.
  - Reuse rule: for a PR head (delta snapshot), use the base snapshot's profile. A PR never recomputes conventions. It computes only "violations introduced by this PR" (POL-005, REV-A).
  - Recompute on a default-branch full snapshot, or when `config_hash` or `profile_version` changes.
  - An in-process LRU of `Arc<RepositoryProfile>`.
- **Explicit non-scope:** Distributed cache (Redis must not hold profiles, per target-arch §7).
- **Files/modules expected to change:**
  - `engine/crates/profile/src/lib.rs`
  - `engine/crates/pipeline/src/stages/indexing.rs`
- **New files/modules expected:**
  - `engine/crates/profile/src/version.rs`
  - `engine/crates/profile/src/cache.rs`
- **Dependencies:** PROF-001..PROF-005, INC-009, INIT-012 (fingerprint includes profile_version), POL-002.
- **Implementation details:**
  - `fn profile_for(snapshot: &Snapshot) -> ProfileRef`. It resolves `snapshot.kind == delta` to the nearest full ancestor on the default branch (`base_snapshot_id` chain).
  - The LRU holds 32 entries.
  - The compute guard is a PG advisory lock `pg_advisory_xact_lock(hashtext('profile:'||repository_id||':'||snapshot_id))`, so concurrent workers do not duplicate work.
- **Data model changes:** None.
- **API/protocol changes:** `review status` and `GET /repositories/:id/status` expose `profile_version` and `profile_computed_at`.
- **Concurrency semantics:** The advisory lock serializes computation per key. Losers wait, then read the inserted row.
- **Failure behavior:** A compute failure leaves the previous profile in use, marked `stale: true` in reviewer inputs.
- **Idempotency considerations:** Keyed insert (PROF-001).
- **Security considerations:** None.
- **Observability additions:** Counters `profile_cache_hits_total` and `profile_cache_misses_total`. Span attribute `profile_reused=true|false`.
- **Tests required:**
  - `pr_head_reuses_base_profile`
  - `config_hash_change_recomputes`
  - `miner_version_bump_changes_profile_version`
  - `concurrent_compute_single_row` (integration, two tasks)
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** Over a PR review, `profile_compute` never runs (asserted by the span count in an integration test), and the profile is reused from the base.
- **Definition of done:** Global DoD.

---

### PROF-007 — Knowledge-source adapter (vault and ADR markdown with frontmatter → DocumentationRule nodes)
Status: ☐

- **Task ID:** PROF-007
- **Title:** Knowledge-source adapter (vault/ADR markdown with frontmatter → DocumentationRule nodes; generalizes the reference consumer .agent vault)
- **Problem:** the reference consumer encodes architecture rules in `.agent/knowledge/*.md`, and other teams keep ADRs. Precedence level 2 ("architecture documentation", PRD §65) has no machine representation.
- **Why it exists:** It turns documented rules into `DocumentationRule` graph nodes linked to the scopes they govern. Context (CTX "rule-doc candidates") and POL-004 can then use them.
- **Scope:**
  - `KnowledgeSourceAdapter` trait.
  - A `markdown_vault` adapter: frontmatter keys `id`, `applies_to` (globs or symbol ids), `kind: rule|decision|guide`, `severity`, `status: accepted|superseded`.
  - An `adr` adapter (`docs/decisions/ADR-*.md`, which reads `Status:` lines when there is no frontmatter).
  - Configured by `knowledge_sources:` in `.review/config.yaml`.
- **Explicit non-scope:**
  - Natural-language rule extraction by a model (later; it would go through VER).
  - The the reference consumer content itself (POL-007 fixture).
- **Files/modules expected to change:**
  - `engine/crates/profile/src/lib.rs`
  - `engine/crates/codegraph/src/linker.rs` (accept `DocumentationRule` synthetic nodes and `GOVERNS` edges with provenance `policy`)
- **New files/modules expected:**
  - `engine/crates/profile/src/knowledge/{mod.rs,markdown_vault.rs,adr.rs,frontmatter.rs}`
- **Dependencies:** PROF-001, POL-001 (knowledge_sources schema), CG-001/CG-002 (DocumentationRule node kind and an edge kind present in the §17/§18 lists), INIT-010 (rule docs detection).
- **Implementation details:**
  - Node ID: `doc:{source_id}/{rule_id}`.
  - Node attributes: `title`, `kind`, `severity`, `status`, `body_hash`, `path`, `line_range`.
  - Bodies are **not** stored in the graph. They are referenced by path and range and loaded into context with redaction.
  - Edges: `DocumentationRule -[DOCUMENTS/GOVERNS]-> Module|File|Symbol`, resolved from the `applies_to` globs against the snapshot file list. Edges are capped at 500 per rule; above that, the rule attaches to the module.
  - Superseded documents are ignored.
  - When frontmatter is missing, the document is indexed as a `guide` with no `applies_to`. It is retrievable lexically and semantically but has no structural edges.
- **Data model changes:** None new. Synthetic nodes go in `synthetic_nodes` with `kind = 'DocumentationRule'`.
- **API/protocol changes:** None.
- **Concurrency semantics:** Runs during indexing. It is incremental: only changed markdown files are re-read (they are part of the snapshot file diff).
- **Failure behavior:**
  - Malformed frontmatter skips that document with a diagnostic in `review doctor`.
  - A missing configured path gives a doctor warning, not an index failure.
- **Idempotency considerations:** Deterministic IDs. The documents are content-addressed like other files.
- **Security considerations:** Vault documents may contain internal URLs or credentials. They pass SEC-003 secret detection like source.
- **Observability additions:** Counter `documentation_rules_total{source}`. Span `knowledge_source_ingest`.
- **Tests required:**
  - `vault_frontmatter_parsed`
  - `adr_status_line_parsed`
  - `superseded_doc_ignored`
  - `applies_to_glob_creates_governs_edges`
  - `edge_cap_falls_back_to_module`
  - `malformed_frontmatter_diagnostic`
- **Benchmarks if applicable:** None.
- **Acceptance criteria:**
  - `fixtures/reference-profile/vault/` (POL-007) produces the expected DocumentationRule nodes and edges, shown by the golden graph snippet.
  - `review graph symbol doc:reference/tenant-scope` resolves.
  - No the reference consumer string appears in `engine/crates/profile/src` (INV-005 grep).
- **Definition of done:** Global DoD, plus a `docs/reviewers/knowledge-sources.md` format spec.

---

---

### POL-001 — `.review/config.yaml` schema, parser and validation
Status: ☐

- **Task ID:** POL-001
- **Title:** .review/config.yaml schema (PRD §122 + §66) + parser + validation
- **Problem:** Legacy configuration is a daemon TOML. PRD §122/§123 require a version-controlled, repository-owned YAML file with reviewers, confidence, budgets, generated globs, risk paths, rules, suppressions and knowledge sources.
- **Why it exists:** It is the single source of explicit policy, which is the highest precedence level in PRD §65.
- **Scope:**
  - Rust types with `serde` and `deny_unknown_fields`.
  - A JSON Schema generated with schemars and published in contracts. It also provides editor autocompletion via a `# yaml-language-server: $schema=` hint.
  - Semantic validation.
  - A normalized form for hashing.
  - The `review init` template.
- **Explicit non-scope:**
  - Syncing from the provider (POL-002).
  - Rule semantics (POL-003).
- **Files/modules expected to change:**
  - `engine/crates/profile/src/lib.rs`
  - `engine/crates/repository/src/init/template.rs` (writes the starter file)
- **New files/modules expected:**
  - `engine/crates/profile/src/config/{mod.rs,schema.rs,validate.rs,normalize.rs}`
  - `packages/contracts/schemas/review-config.v1.json`
  - `docs/reviewers/config-reference.md`
- **Dependencies:** DOM-003, INIT-011, FND (contracts export).
- **Implementation details:** Schema v1:

  ```yaml
  version: 1
  review:
    reviewers: { correctness: true, security: true, tests: true, performance: true, architecture: true, maintainability: false }
    confidence: { minimum_publish: 0.72, per_reviewer: { maintainability: 0.90 } }   # floor 0.55 (gap §P), maint floor 0.85
    budgets: { max_symbols: 100, max_context_tokens: 40000, max_model_calls: 30, max_review_seconds: 300 }
    generated: { ignore: ["**/*.generated.ts"] }
    risk: { paths: { "src/auth/**": critical, "migrations/**": high } }          # critical|high|medium|low
    privacy: { external_models: true }                                            # false => NoEligibleProvider (ADR-010)
    publish: { inline_cap: 25, summary: true, check_run: true }
  architecture:
    layers: { controllers: ["src/**/*.controller.ts"], repositories: ["src/**/*.repository.ts"] }
  rules:
    forbidden_dependencies: [ { id: no-ctrl-repo, from: controllers, to: repositories, severity: high, reason: "..." } ]
    queue_jobs: { require_deterministic_id: true }
    database: { migrations_only: true, migration_paths: ["migrations/**", "src/migrations/**"] }
    tests: { public_api_changes_require_tests: true }
    security: { authorization_symbols: ["PermissionService.check"] }
  conventions: { exceptions: ["src/legacy/**"] }
  suppressions: [ { id: s1, type: path, value: "src/legacy/**", reason: "...", owner: "@team", expires: 2027-01-01 } ]
  knowledge_sources: [ { id: reference, kind: markdown_vault, path: .agent/knowledge }, { id: adr, kind: adr, path: docs/decisions } ]
  ```

  Semantic validation rules:
  - Layer names referenced in rules must exist (declared or inferable names).
  - Globs must compile (`globset`).
  - `minimum_publish` must be in [0.55, 1].
  - Suppression `expires` must be in the future or produce a warning.
  - Unknown keys are an error with the path (`rules.queue_job` → "did you mean queue_jobs").

  Normalization:
  - Sort map keys.
  - Expand defaults.
  - `config_hash = blake3(canonical_json(normalized))`.
- **Data model changes:** None (POL-002 stores it).
- **API/protocol changes:** `ReviewConfigV1` in contracts.
- **Concurrency semantics:** Pure parsing.
- **Failure behavior:**
  - An invalid config fails closed to **defaults plus the explicit validation error surfaced in the summary and doctor**. Reviews still run. Explicit suppressions in an invalid file are **not** applied, because the system never silently suppresses on a broken config.
- **Idempotency considerations:** Normalization makes the hash independent of key order and whitespace.
- **Security considerations:**
  - YAML parsing with `serde_yaml`, safe: no anchors bombs. Input is capped at 256 KiB and alias expansion at depth 32.
  - Paths are confined to the repository root (no `..`).
- **Observability additions:** Counter `config_validation_errors_total{kind}`.
- **Tests required:**
  - `prd_122_example_parses`
  - `prd_66_rules_parse`
  - `unknown_key_error_with_suggestion`
  - `minimum_publish_below_floor_rejected`
  - `undefined_layer_rejected`
  - `config_hash_order_independent`
  - `yaml_bomb_rejected`
  - `path_escape_rejected`
  - `invalid_config_ignores_suppressions`
- **Benchmarks if applicable:** None.
- **Acceptance criteria:**
  - `review init` writes a file that validates.
  - The schema is published in contracts.
  - All tests listed above pass.
  - `review doctor` reports a planted error with its key path.
- **Definition of done:** Global DoD, plus the config reference doc.

---

### POL-002 — Config sync at snapshot time and `config_hash`
Status: ☐

- **Task ID:** POL-002
- **Title:** Config sync at snapshot + config_hash
- **Problem:** The policy must be the one **at the reviewed commit**, and it must be reproducible. Reading the default branch's config while reviewing a PR would let a PR change its own rules unnoticed, or stop it from changing them at all.
- **Why it exists:** ADR-015 puts `config_hash` in the fingerprint. Reproducibility (PRD §121) requires the config to be bound to the snapshot.
- **Scope:**
  - During indexing (full and delta), read `.review/config.yaml` from the snapshot tree, parse and validate it (POL-001), and store it in `repository_configs`. Set `snapshots.config_hash`.
  - Policy for PR heads: **the rules come from the base** (merge-base) config. The head config is parsed and diffed, and changes to `rules`/`suppressions` are reported as a risk signal `review_policy_changed` (a PR cannot silently weaken its own review).
- **Explicit non-scope:** UI editing (WEB-008 is read-only for config in v1).
- **Files/modules expected to change:**
  - `engine/crates/pipeline/src/stages/indexing.rs`
  - `engine/crates/impact/src/risk/signals.rs`
- **New files/modules expected:**
  - `engine/crates/profile/src/config/sync.rs`
  - `engine/migrations/{seq}_repository_configs.sql`
- **Dependencies:** POL-001, GS-004 (snapshots), INC-009, INIT-012, RISK-001.
- **Implementation details:**

  ```sql
  CREATE TABLE repository_configs (organization_id uuid NOT NULL, repository_id uuid NOT NULL, config_hash text NOT NULL,
    normalized jsonb NOT NULL, raw_blob_sha text, validation jsonb NOT NULL DEFAULT '[]', created_at timestamptz DEFAULT now(),
    PRIMARY KEY (repository_id, config_hash));
  ALTER TABLE snapshots ADD COLUMN config_source_path text;
  ```

  - With no file, the defaults are used and `config_hash = hash(defaults)`.
  - Full rebuild trigger (INC-011): a `config_hash` change counts only when `generated.ignore`, source roots or tsconfig-affecting keys changed. Rule-only changes do not rebuild the graph.
- **Data model changes:** `repository_configs` (with RLS) and `snapshots.config_source_path`.
- **API/protocol changes:** `GET /repositories/:id/status` includes `config_hash` and validation errors.
- **Concurrency semantics:** `INSERT ... ON CONFLICT DO NOTHING` on `(repository_id, config_hash)`.
- **Failure behavior:** A parse failure stores the validation errors with `normalized = defaults` and falls back per POL-001.
- **Idempotency considerations:** Content-addressed by hash.
- **Security considerations:** A PR that edits `.review/config.yaml` to add suppressions does not affect its own review (base policy), and it is flagged.
- **Observability additions:** Span attribute `config_hash` on `repository_index`. Counter `review_policy_changed_total`.
- **Tests required:**
  - `config_read_from_snapshot_tree`
  - `pr_uses_base_policy`
  - `pr_adding_suppression_flagged_not_applied`
  - `rule_only_change_does_not_rebuild_graph`
  - `generated_glob_change_triggers_rebuild`
  - `missing_config_uses_defaults_hash`
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** The fixture PR `weaken-own-policy` is reviewed under the base rules, and its summary contains "This PR changes review policy".
- **Definition of done:** Global DoD.

---

### POL-003 — Rule evaluators
Status: ☐

- **Task ID:** POL-003
- **Title:** Rule evaluators (forbidden_dependencies, migrations_only, public_api_changes_require_tests, queue require_deterministic_id)
- **Problem:** Explicit rules (PRD §66) need deterministic evaluators. Otherwise "policy" is just prompt text.
- **Why it exists:** Explicit policy is the strongest evidence class (PRD §65). The evaluators produce violations (POL-005) that reviewers and verification consume.
- **Scope:** A `RuleEvaluator` trait and four evaluators, all evaluated **only over the PR's change** (new edges and changed symbols):
  1. `forbidden_dependencies`: delegates to REV-A-002 boundaries, using config layers.
  2. `database.migrations_only`: a schema-affecting change outside `migration_paths` is a violation. This covers changed TypeORM entity column decorators without a migration in the PR, `synchronize: true` in config, and raw DDL strings (`CREATE TABLE`, `ALTER TABLE`) in non-migration code.
  3. `tests.public_api_changes_require_tests`: a changed `APIEndpoint` contract (route, DTO, or response type change, CHG `api_contract_changed`) with no mapped test changed or added in the PR. It uses REV-T-002.
  4. `queue_jobs.require_deterministic_id`: a new or changed `PRODUCES_JOB` call site with a missing or random `jobId` (the PROF-005 predicate).
- **Explicit non-scope:**
  - Custom user-defined rule DSL (later).
  - Repository-wide audits.
- **Files/modules expected to change:** `engine/crates/pipeline/src/stages/analysis.rs`.
- **New files/modules expected:**
  - `engine/crates/profile/src/policy/{mod.rs,evaluator.rs,forbidden_deps.rs,migrations_only.rs,api_tests.rs,queue_ids.rs}`
- **Dependencies:** POL-001, POL-002, REV-A-002, REV-T-002, PROF-005, CHG-006..CHG-008, NEST-001..NEST-007.
- **Implementation details:**
  - `trait RuleEvaluator { fn rule_key(&self) -> &'static str; fn evaluate(&self, cm: &ChangeModel, g: &GraphPair, cfg: &ReviewConfigV1, prof: &RepositoryProfile) -> RuleOutcome }`.
  - `RuleOutcome { status: Executed|NotExecuted{reason}, violations: Vec<RuleViolation> }`.
  - `RuleViolation { rule_id, rule_key, anchor: FileRange, symbol_key?, evidence: Vec<Evidence>, severity, message_template }`.
  - Each evaluator is pure. "Base already violated" is checked through `GraphPair.base`, and pre-existing violations are not reported (INV-009).
- **Data model changes:** None (POL-005 persists).
- **API/protocol changes:** None.
- **Concurrency semantics:** The evaluators run in parallel in ANALYZING.
- **Failure behavior:** Missing inputs (for example, no migration paths and none detected) give `NotExecuted` with a reason, shown in the summary. They never give a silent pass (INV-014).
- **Idempotency considerations:** Deterministic.
- **Security considerations:** None.
- **Observability additions:** Counter `rule_evaluations_total{rule_key,status}`. Counter `rule_violations_total{rule_key}`.
- **Tests required:**
  - `forbidden_dep_new_edge_violation`
  - `forbidden_dep_preexisting_ignored`
  - `entity_column_change_without_migration_violation`
  - `entity_change_with_migration_ok`
  - `raw_ddl_outside_migrations_violation`
  - `api_contract_change_without_test_violation`
  - `queue_add_with_uuid_job_id_violation`
  - `queue_add_with_template_job_id_ok`
  - `not_executed_reason_reported`
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** Each of the four fixture PRs under `fixtures/pull-requests/policy-*` yields exactly the expected violation, and the safe twin of each yields none.
- **Definition of done:** Global DoD, plus each rule documented in the config reference.

---

### POL-004 — Precedence resolver (PRD §65)
Status: ☐

- **Task ID:** POL-004
- **Title:** Precedence resolver (PRD §65)
- **Problem:** Explicit rules, documentation rules, inferred conventions and generic guidance can disagree. One example: a convention says "always use a transaction wrapper", while the config lists `src/reports/**` as an exception.
- **Why it exists:** PRD §65 sets the order: explicit policy > architecture docs > established convention > generic. Without a resolver, the reviewers get contradictory context and verification cannot rank evidence.
- **Scope:**
  - `PolicyResolver::effective(scope: &FileOrSymbol, topic: PolicyTopic) -> EffectivePolicy` with full provenance.
  - It is applied when building reviewer inputs (only the effective rules are sent) and when verification evaluates `basis`.
- **Explicit non-scope:** Historical signals (HIST-004 adds a fifth, lowest, signals-only level).
- **Files/modules expected to change:**
  - `engine/crates/context-engine/src/candidates/rules.rs`
  - `engine/crates/verification/src/stages/repo_evidence.rs`
- **New files/modules expected:** `engine/crates/profile/src/policy/precedence.rs`
- **Dependencies:** POL-001, PROF-003, PROF-007.
- **Implementation details:**
  - `enum PolicySource { Explicit{rule_id}, Documented{doc_id}, Convention{convention_id, confidence, samples}, Generic }`, ordered by rank 4 to 1.
  - `PolicyTopic` is a closed enum: `LayerDependency{from,to}`, `TransactionOnWrite`, `GuardOnEndpoint`, `JobIdDeterminism`, `TestsForApiChange`, `ErrorHierarchy`, `MigrationsOnly`.
  - Each source maps to topics.
  - The resolution rules:
    - The highest-ranked applicable source wins.
    - An explicit exception (config `conventions.exceptions` or a suppression with `type: rule`) **negates** lower-ranked sources for that scope.
    - A convention is applicable only if enforceable (PROF-003 thresholds).
  - `EffectivePolicy { topic, decision: Required|Forbidden|Allowed|NoPolicy, winner: PolicySource, overridden: Vec<PolicySource> }`. `overridden` is kept for the explainability trace (PRD §85).
- **Data model changes:** None.
- **API/protocol changes:** The finding trace (API-010 `GET /findings/:id/trace`) includes `effective_policy`.
- **Concurrency semantics:** Pure. The resolver is built once per run.
- **Failure behavior:** Conflicting sources at the same rank (two docs) resolve to the stricter decision and record `conflict: true` on the effective policy, which is visible in doctor.
- **Idempotency considerations:** Deterministic.
- **Security considerations:** None.
- **Observability additions:** Counter `policy_conflicts_total`.
- **Tests required:**
  - `explicit_beats_convention`
  - `documented_beats_convention`
  - `explicit_exception_negates_convention`
  - `nonenforceable_convention_ignored`
  - `same_rank_conflict_strictest_and_flagged`
  - `overridden_sources_recorded`
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** A fixture table test covers all 4×4 source combinations, and the finding trace shows the winner and the overridden sources.
- **Definition of done:** Global DoD.

---

### POL-005 — Rule violations as deterministic evidence
Status: ☐

- **Task ID:** POL-005
- **Title:** Rule violations as deterministic evidence
- **Problem:** POL-003 output must enter the finding pipeline as both:
  - candidate findings in their own right (deterministic reviewer)
  - strong evidence attached to model candidates that make the same claim
- **Why it exists:** ADR-011 gives deterministic evidence weight 0.20 in the confidence formula. Explicit-rule violations are also the clearest actionable comments.
- **Scope:**
  - Persist the violations.
  - Convert each one to a `CandidateFinding { reviewer: "deterministic:policy", rule_id }`.
  - Attach `Evidence::RuleViolation` to overlapping model candidates (same anchor ±3 lines, or the same symbol, plus the same topic), so DED merges them.
- **Explicit non-scope:** Rule evaluation itself (POL-003).
- **Files/modules expected to change:**
  - `engine/crates/verification/src/stages/deterministic.rs`
  - `engine/crates/pipeline/src/stages/reviewing.rs`
- **New files/modules expected:**
  - `engine/crates/profile/src/policy/to_candidates.rs`
  - `engine/migrations/{seq}_rule_violations.sql`
- **Dependencies:** POL-003, POL-004, VER-005, DED-001, DOM-006, DOM-007.
- **Implementation details:**

  ```sql
  CREATE TABLE rule_violations (id uuid PK, organization_id uuid NOT NULL, repository_id uuid NOT NULL, review_run_id uuid NOT NULL
    REFERENCES review_runs(id), rule_id text NOT NULL, rule_key text NOT NULL, source text NOT NULL, path text, start_line int, end_line int,
    symbol_key text, evidence jsonb NOT NULL, severity text NOT NULL, candidate_finding_id uuid NULL,
    UNIQUE (review_run_id, rule_id, path, start_line, symbol_key));
  ```

  - Comment text comes from templates per `rule_key`. For example, `forbidden_dependency`: "`{from}` now depends on `{to}` (`{import}`), which `{rule_id}` forbids: {reason}." The template provides PRD §59 answers 1–3 deterministically, and the corrective direction comes from the rule's `reason`/`fix` field.
  - Confidence comes from the VER formula, with deterministic = 1 and anchor = 1. An explicit rule violation typically scores ≥ 0.85, so it is published.
- **Data model changes:** `rule_violations` (with RLS).
- **API/protocol changes:** The finding trace includes the rule violation reference.
- **Concurrency semantics:** Written in the ANALYZING stage, in the same transaction as the stage output row.
- **Failure behavior:** A persist failure fails the stage, which is retried (PIPE-005).
- **Idempotency considerations:** The unique key makes a retry a no-op.
- **Security considerations:** None.
- **Observability additions:** Counter `candidate_findings_total{reviewer="deterministic:policy"}`.
- **Tests required:**
  - `violation_becomes_candidate`
  - `violation_attaches_to_overlapping_model_candidate`
  - `rule_violation_template_renders_reason`
  - `violation_retry_noop`
  - `deterministic_candidate_still_verified` (passes through all VER stages: INV-002)
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** On `policy-forbidden-dep`, one published finding has `basis = explicit_rule`, and the model candidate making the same claim is merged into it (DED merge record exists).
- **Definition of done:** Global DoD.

---

### POL-006 — Suppression mechanisms with audit
Status: ☐

- **Task ID:** POL-006
- **Title:** Suppression mechanisms (type/path/symbol/rule/fingerprint) with audit
- **Problem:** PRD §124: teams must be able to suppress by finding type, path, symbol, rule or comment fingerprint, explicitly and auditably. Legacy only had a P4 discard.
- **Why it exists:** Without suppressions, recurring intentional patterns erode trust. Without an audit trail, suppressions hide real issues.
- **Scope:** Two sources:
  1. Config `suppressions:` (POL-001), taken from the base policy (POL-002).
  2. API/UI-created suppressions (the "intentional" feedback action, API-012 → `suppressions` table).

  Matching runs as the verification stage `policy_suppression` (state `SUPPRESSED_POLICY`). Every match is persisted with the suppression id. Expiry is honored.
- **Explicit non-scope:**
  - Inline code-comment suppressions beyond `review-ignore` (PROF-003 handles conventions only). Inline `// reviewgraph-ignore[rule]` is listed as a follow-up.
- **Files/modules expected to change:**
  - `engine/crates/verification/src/stages/mod.rs`
  - `apps/api/src/findings/feedback.service.ts` (creates a suppression on "intentional" when the user opts in)
- **New files/modules expected:**
  - `engine/crates/verification/src/stages/policy_suppression.rs`
  - `engine/migrations/{seq}_suppressions.sql`
- **Dependencies:** POL-001, POL-002, VER-011, DED-002 (fingerprint), API-012, SEC-008 (audit log).
- **Implementation details:**

  ```sql
  CREATE TABLE suppressions (id uuid PK, organization_id uuid NOT NULL, repository_id uuid NOT NULL,
    kind text CHECK (kind IN ('type','path','symbol','rule','fingerprint')), value text NOT NULL,
    source text CHECK (source IN ('config','api')), config_hash text NULL, reason text NOT NULL, created_by uuid NULL,
    expires_at timestamptz NULL, revoked_at timestamptz NULL, created_at timestamptz DEFAULT now());
  CREATE TABLE suppression_matches (suppression_id uuid, candidate_finding_id uuid, review_run_id uuid, matched_at timestamptz,
    PRIMARY KEY (suppression_id, candidate_finding_id));
  ```

  Matching semantics:
  - `type` is the reviewer category (for example `security.input_validation`).
  - `path` is a glob on the anchor path.
  - `symbol` is a `SymbolId` prefix, and it follows lineage on rename (ADR-005).
  - `rule` is a `rule_id`.
  - `fingerprint` is the DED root-cause fingerprint.

  Guardrails:
  - `type` and `path` suppressions cannot match `critical` severity findings unless `allow_critical: true` is set explicitly.
  - Every suppression requires a `reason`.
  - Expired suppressions do not match, and the doctor lists them.
- **Data model changes:** `suppressions` and `suppression_matches` (with RLS).
- **API/protocol changes:**
  - `GET/POST/DELETE /repositories/:id/suppressions`. DELETE sets `revoked_at` (soft delete, audit).
  - The summary line "Suppressed: N by policy" (GH-008).
- **Concurrency semantics:** Suppressions are read once at the start of VERIFYING (a snapshot of the active set), so mid-run changes apply to the next run.
- **Failure behavior:** An invalid config gives no config suppressions (POL-001). API suppressions still apply.
- **Idempotency considerations:** The match insert uses `ON CONFLICT DO NOTHING`.
- **Security considerations:**
  - Creating or revoking a suppression requires the repository `maintainer` role (API-003).
  - Every create and revoke writes `audit_log` (SEC-008).
- **Observability additions:** Counter `suppressed_findings_total{reason="policy",kind}`.
- **Tests required:**
  - `suppress_by_type`
  - `suppress_by_path_glob`
  - `suppress_by_symbol_follows_rename`
  - `suppress_by_rule`
  - `suppress_by_fingerprint`
  - `expired_suppression_ignored`
  - `critical_not_suppressed_by_path_without_flag`
  - `match_persisted_with_id`
  - `api_suppression_requires_maintainer` (apps/api)
  - `suppression_create_writes_audit` (apps/api)
- **Benchmarks if applicable:** None.
- **Acceptance criteria:**
  - A fingerprint suppression created via the API suppresses the same finding on the next run, and `suppression_matches` has a row.
  - The summary shows the count.
  - The audit log has the create entry.
- **Definition of done:** Global DoD.

---

### POL-007 — the reference consumer reference profile and rule pack (outside core)
Status: ☐

- **Task ID:** POL-007
- **Title:** the reference consumer reference profile & rule pack in fixtures/reference-profile (outside core)
- **Problem:** Legacy hard-codes the reference consumer knowledge throughout (audit §4/§7). PRD §125–127 require the reference consumer to be a *consumer* configured through generic mechanisms, and Invariant 5 forbids the reference consumer semantics in core.
- **Why it exists:** It proves the extraction (A/B/C classes): everything consumer-specific can be expressed as config, a rule pack and knowledge sources.
- **Scope:**
  - `fixtures/reference-profile/.review/config.yaml`: layers, forbidden deps, `authorization_symbols`, risk paths (tenant and auth modules critical), migrations-only, queue ids.
  - `fixtures/reference-profile/vault/`: a sanitized sample of `.agent/knowledge`-style docs with frontmatter.
  - `fixtures/reference-profile/expected/`: the expected profile conventions and doc nodes.
  - A README mapping each legacy hard-coded rule (the `passes.rs` checklist items) to its generic mechanism.
- **Explicit non-scope:**
  - Copying proprietary reference-api source. Use only sanitized and synthetic examples, plus references to the real repo path for the optional local golden run.
- **Files/modules expected to change:** None in `engine/` (that is the point).
- **New files/modules expected:**
  - `fixtures/reference-profile/{README.md,.review/config.yaml,vault/*.md,expected/profile.json,expected/doc_nodes.json}`
  - `engine/crates/profile/tests/reference_profile.rs`
- **Dependencies:** POL-001, POL-003, PROF-004, PROF-005, PROF-007.
- **Implementation details:**
  - Each legacy checklist item is mapped to one of:
    - `rules.*`
    - `rules.security.authorization_symbols`
    - a DocumentationRule
    - a convention expectation
    - "dropped (not evidence-based)", with a reason
  - The integration test loads the config and vault against `fixtures/repositories/nestjs-layered` (synthetic, the reference consumer-shaped) and asserts the expected outputs.
  - An optional env `RG_REFERENCE_REPO_PATH` runs the same config against the real repository and prints a diff report without asserting (local only).
- **Data model changes:** None.
- **API/protocol changes:** None.
- **Concurrency semantics:** N/A (fixture).
- **Failure behavior:** If `RG_REFERENCE_REPO_PATH` is unset, the optional real-repo run is skipped with a message.
- **Idempotency considerations:** N/A.
- **Security considerations:** The fixture must contain no real secrets, customer data or internal hostnames. It is scanned by SEC-003 in CI.
- **Observability additions:** None.
- **Tests required:**
  - `reference_config_validates`
  - `reference_vault_produces_doc_nodes`
  - `reference_rules_fire_on_fixture_prs`
  - `invariant5_no_reference_in_engine_crates` (grep test over `engine/crates/**`, excluding tests and fixtures; shared with INV-005)
- **Benchmarks if applicable:** QB-001 uses this profile for the labelled reference-api PRs.
- **Acceptance criteria:**
  - All tests pass.
  - The README mapping table covers 100% of the `passes.rs:106-131` checklist items.
  - `rg -i reference engine/crates --glob '!**/tests/**'` is empty.
- **Definition of done:** Global DoD, plus the mapping table reviewed.
