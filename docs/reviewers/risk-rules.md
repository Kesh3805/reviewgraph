# Risk rules

Generated from `engine/crates/impact/src/risk/rules.rs` by
`impact::risk::rules::render_markdown()`; do not edit by hand. The
`risk_rules_doc_up_to_date` test fails when this file drifts.

Rules version: 1

## Categories (PRD §37)

| Category | Base weight |
|---|---|
| authentication | 0.60 |
| authorization | 0.70 |
| cryptography | 0.60 |
| permissions | 0.60 |
| payments | 0.65 |
| database_migrations | 0.55 |
| schema_changes | 0.50 |
| transaction_boundaries | 0.50 |
| concurrency | 0.45 |
| queues | 0.40 |
| background_jobs | 0.35 |
| public_api_contracts | 0.50 |
| serialization | 0.35 |
| validation | 0.50 |
| external_network_calls | 0.35 |
| filesystem_access | 0.35 |
| secrets | 0.70 |
| dependency_updates | 0.30 |

## Rules

| Rule | Category | Source | Weight | Floor | Description |
|---|---|---|---|---|---|
| `path.configured` | authentication | path | 0.80 | — | File matches a configured risk path; weight by level (critical 0.8, high 0.6, medium 0.4, low 0.1), category inferred from the glob, floor = configured level |
| `path.auth` | authentication | path | 0.50 | — | File under an auth/ or security/ directory (default rule) |
| `path.migrations` | database_migrations | path | 0.55 | — | File under a migrations/ directory (default rule) |
| `path.payments` | payments | path | 0.65 | — | File under a payment or billing directory (default rule) |
| `path.infra` | dependency_updates | path | 0.30 | — | Dockerfile or CI workflow changed (default rule) |
| `dep.added` | dependency_updates | manifest | 0.30 | — | Dependency added |
| `dep.major_bump` | dependency_updates | manifest | 0.45 | — | Dependency bumped across a major version |
| `dep.prerelease` | dependency_updates | manifest | 0.40 | — | Dependency moved to a prerelease version |
| `dep.removed` | dependency_updates | manifest | 0.25 | — | Dependency removed |
| `dep.lock_drift` | dependency_updates | manifest | 0.20 | — | Lockfile changed without a manifest change |
| `migration.added` | database_migrations | manifest | 0.55 | — | Migration added |
| `migration.destructive` | database_migrations | manifest | 0.75 | high | Destructive migration (drop, narrowing type change, NOT NULL without default, rename) |
| `migration.edited_existing` | database_migrations | manifest | 0.70 | — | An existing migration was edited |
| `schema.entity_changed` | schema_changes | manifest | 0.50 | — | ORM entity columns or relations changed |
| `secrets.literal_added` | secrets | secrets | 0.90 | critical | Secret-like literal added (key, token, private key, high-entropy string) |
| `secrets.env_file` | secrets | secrets | 0.80 | — | Environment file committed |
| `authz.call_removed` | authorization | change | 0.85 | high | An authorization call was removed |
| `authz.direct_role_check` | authorization | change | 0.60 | — | A direct role or permission comparison was added |
| `authz.guard_removed` | authorization | change | 0.85 | high | A guard was removed from an endpoint |
| `authz.guard_added` | authorization | change | 0.40 | — | A guard was added to an endpoint |
| `authz.logic_changed` | authorization | change | 0.70 | — | Authorization logic changed |
| `validation.removed` | validation | change | 0.65 | — | Input validation was removed |
| `db.write_changed` | schema_changes | change | 0.40 | — | A database write changed outside a transaction (persistent data mutation) |
| `db.write_changed_in_tx` | transaction_boundaries | change | 0.45 | — | A database write inside a transaction changed |
| `tx.boundary_changed` | transaction_boundaries | change | 0.60 | — | A transaction boundary changed |
| `api.breaking_change` | public_api_contracts | change | 0.60 | — | Breaking API contract change (removed endpoint, field or parameter, narrowed type) |
| `api.additive_change` | public_api_contracts | change | 0.35 | — | Additive API contract change |
| `async.floating_promise` | concurrency | change | 0.50 | — | A floating promise was introduced, or an await on a database write was removed |
| `endpoint.reachable` | public_api_contracts | graph | 0.30 | — | The change reaches a public HTTP endpoint (amplifies authorization, authentication and validation by 1.15) |
| `queue.produce_changed` | queues | framework | 0.45 | — | A queue producer changed |
| `queue.consume_changed` | background_jobs | framework | 0.45 | — | A queue consumer or job handler changed |
| `external.call_added` | external_network_calls | framework | 0.40 | — | An outbound network call was added |
| `fs.access_changed` | filesystem_access | framework | 0.40 | — | Filesystem access changed |
| `crypto.changed` | cryptography | framework | 0.60 | — | Cryptographic calls changed (crypto, bcrypt, argon2, jsonwebtoken, jose) |
| `auth.jwt_verify_removed` | authentication | framework | 0.80 | high | A JWT verification call was removed |
| `auth.authn_changed` | authentication | framework | 0.70 | — | Authentication strategy or guard changed |
| `serialization.changed` | serialization | framework | 0.40 | — | Serialization behaviour changed |
| `payments.changed` | payments | framework | 0.65 | — | Payment code changed |
| `concurrency.shared_state` | concurrency | framework | 0.45 | — | Shared mutable state written, or concurrent database writes |

## Scoring (RISK-004)

* Per signal: `contribution = clamp(weight × confidence × amplifier, 0, 0.95)`. The amplifier is
  1.15 for authorization, authentication and validation signals of a symbol that has
  `endpoint.reachable`, and 1.0 otherwise.
* Within a category the maximum contribution counts, so many weak signals of one kind never
  stack. Across categories contributions combine by noisy-OR:
  `score = 1 − Π(1 − max_contribution(category))`.
* Levels: `low < 0.25 ≤ medium < 0.50 ≤ high < 0.75 ≤ critical`. A signal's floor raises the
  level (`level = max(threshold level, floors)`), and then the score to the level's minimum
  (`medium 0.25`, `high 0.50`, `critical 0.75`).
* File-subject signals (paths, manifests, secrets) apply to the symbols of that file. A symbol
  classified low-risk (RISK-006) has the confidence of its path signals multiplied by 0.3;
  change and graph signals are never dampened.
* A cluster combines the category maxima of its members by the same noisy-OR. The pull request
  score is the maximum over clusters (a large PR is not riskier per se), plus 0.1 when more
  than 50 files changed. No signal means score 0.0, level low.
* Contributions are rounded to four decimals. The assessment carries the effects of the pull
  request and of every cluster (below) and an `input_hash`: `blake3` over the change, impact and
  policy hashes, the rules version, the effects configuration and the signals, so a changed
  input never reuses a cached assessment.
* Worked example (PRD §151): `authz.call_removed` 0.85 × 1.15 → 0.95 (clamped),
  `path.configured` (`src/auth/**: critical`) 0.8, `endpoint.reachable` 0.3:
  `1 − 0.05 × 0.2 × 0.7 = 0.993` → critical.

## Effects (RISK-005)

| Level | Reviewers (if enabled) | Depth | Context × | Impact × | Model tier | Verification | Threshold Δ | Test inspection |
|---|---|---|---|---|---|---|---|---|
| low | correctness (non-cosmetic only) | light | 0.5 | 0.5 | FAST_REASONER | structural (stages 1–4) | +0.05 | no |
| medium | correctness | standard | 1.0 | 1.0 | REVIEW_REASONER | standard (1–7, deterministic contradiction search) | 0 | if untested |
| high | correctness + category reviewers | deep | 1.25 | 1.5 | REVIEW_REASONER | full (1–8, verifier) | 0 | yes |
| critical | correctness + security + category reviewers | deep | 1.5 | 2.0 | REVIEW_REASONER (+ DEEP_REASONER when enabled) | full | −0.03 (never below 0.55) | yes |

Category reviewers: authorization, authentication, cryptography, permissions, secrets,
validation and payments → `security`; database migrations, schema changes and transaction
boundaries → `correctness` with the `database_safety` focus; concurrency, queues and background
jobs → `correctness` + `performance`; public API contracts → `architecture`; untested changed
symbols → `test`. A reviewer disabled by configuration is removed and listed in
`disabled_by_configuration`. Authorization and secrets signals force full verification whatever
the configuration says. Intent (CHG-009) is not an input.

## Low-risk changes (RISK-006, PRD §39)

| Kind | Detection |
|---|---|
| formatting_or_comments | cosmetic symbol (hashes equal) or whitespace/comment-only file |
| pure_rename | rename with similarity ≥ 0.95 and an unchanged body, or a file rename with similarity 1.0 |
| generated | generated or vendored file |
| import_sort | only import lines changed and the multiset of specifiers and bindings is equal |
| simple_constant | only the literal initializer of a constant or variable changed |
| snapshot_update | only `*.snap` / `__snapshots__/` files changed |

Overrides (any one makes the change not low-risk): an exported symbol renamed while callers
outside the PR exist, or an API contract change; any change class on the symbol; a simple
constant read by authorization, validation, cryptography or payment code, or used as a
configuration default; a file under a `high` or `critical` risk path; an import reordering that
moves side-effect imports. A degraded parse is never low-risk.

## Error handling (decision)

PRD §37 has no error-handling category. `exception_handling_changed` (including swallowed
exceptions) therefore produces **no risk-score signal**; it influences reviewer routing through
the change class (REV-002) instead.
