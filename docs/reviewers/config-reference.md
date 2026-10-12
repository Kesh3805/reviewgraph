# `.review/config.yaml` reference (schema v1)

The repository owns its review policy in `.review/config.yaml`. The file is version-controlled,
parsed by `profile::config` (POL-001) and bound to every snapshot (POL-002). The JSON Schema is
published as `ReviewConfigV1` in `packages/contracts/schemas/ReviewConfigV1.schema.json`; add

```yaml
# yaml-language-server: $schema=https://raw.githubusercontent.com/Kesh3805/reviewgraph/main/packages/contracts/schemas/ReviewConfigV1.schema.json
```

at the top of the file for editor completion. `review init` writes a starter file that validates.

## Loading rules

- The file is capped at 256 KiB. Alias expansion is capped (32 levels, 100 000 nodes), so YAML
  "billion laughs" documents are rejected.
- Every section denies unknown keys. The error names the key path and suggests the closest key:
  `error rules.queue_job: unknown key `queue_job` (did you mean queue_jobs)`.
- Every path and glob must stay inside the repository: no leading `/`, no drive letter, no `..`.
- **An invalid file fails closed to the defaults.** The review still runs, the errors are shown
  in the summary and in `review doctor`, and **no suppression from the invalid file is applied**.
- Normalization expands defaults and sorts keys. `config_hash = blake3(canonical_json(normalized))`,
  so comments, whitespace, key order and explicitly written defaults never change it. No file means
  the defaults and the defaults' hash. An invalid file gets a distinct hash derived from its bytes.

## Which config governs a review

The config is read **from the snapshot tree** of the reviewed commit, never from the default
branch. A pull request is reviewed under its **base** policy. When the head edits `rules`,
`suppressions`, `architecture`, `conventions`, `knowledge_sources`, `review.reviewers`,
`review.confidence` or `review.risk`, the PR raises the risk signal `review_policy_changed` and the
summary says "This PR changes review policy". A PR therefore cannot weaken, disable or suppress
its own review.

Only `ignore`, `generated`, `review.generated.ignore` and `index` change the graph; editing them
triggers a full rebuild. Rule-only edits never rebuild the graph.

## Keys

| Key | Type | Default | Meaning |
|---|---|---|---|
| `version` | int | required | Must be `1`. |
| `ignore` | globs | `[]` | Extra ignore globs on top of `.gitignore` and `.reviewignore`. |
| `generated.include` / `generated.exclude` | globs | `[]` | Force files into / out of the generated class. |
| `index.tolerance.{max_failed_ratio,max_failed_files,min_parsed_files}` | number | indexer default | Parse-failure tolerance. |
| `review.reviewers.{correctness,security,tests,performance,architecture}` | bool | `true` | Reviewer toggles. |
| `review.reviewers.maintainability` | bool | `false` | Opt-in. |
| `review.confidence.minimum_publish` | float | `0.72` | Must be in `[0.55, 1]`. |
| `review.confidence.per_reviewer.<reviewer>` | float | `maintainability: 0.90` | Per-reviewer threshold, floor `0.55` (`0.85` for maintainability). |
| `review.budgets.{max_symbols,max_context_tokens,max_model_calls,max_review_seconds}` | int | `100, 40000, 30, 300` | Per-review budgets. |
| `review.generated.ignore` | globs | `[]` | Indexed but never reviewed. |
| `review.risk.paths` | glob → `critical\|high\|medium\|low` | `{}` | Path risk overrides. |
| `review.privacy.external_models` | bool | `true` | `false` restricts model calls to eligible local providers (ADR-010). |
| `review.publish.{inline_cap,summary,check_run}` | int, bool, bool | `25, true, true` | Publication. |
| `architecture.layers` | name → globs | `{}` | Declared layers. Rules may also use the inferable role names (`controller(s)`, `service(s)`, `repository/repositories`, `entity/entities`, `dto(s)`, `guard(s)`, `processor(s)`, `module(s)`, `config(s)`, `util(s)`, `test(s)`). |
| `rules.*` | see below | off | Explicit rules. |
| `conventions.exceptions` | globs | `[]` | Paths excluded from every inferred convention. |
| `suppressions` | list | `[]` | See below. |
| `knowledge_sources` | list of `{id, kind: markdown_vault\|adr, path}` | `[]` | Documentation rules (PROF-007). |

## Rules (PRD §66, evaluated by POL-003)

Rules are evaluated **only over the pull request's change**. A violation that already exists in the
base is never reported. A rule whose inputs are missing reports `NOT_EXECUTED` with a reason; it
never passes silently.

### `rules.forbidden_dependencies`

```yaml
rules:
  forbidden_dependencies:
    - { id: no-ctrl-repo, from: controllers, to: repositories, severity: high,
        reason: "Controllers go through services.", fix: "Inject the service instead." }
```

A new `IMPORTS`, `DEPENDS_ON` or `CALLS` edge from a file of layer `from` to a symbol of layer
`to` is a violation. Layers come from `architecture.layers` first, then from the inferred roles.
`from`/`to` must name a declared or inferable layer. `severity` defaults to `high`; `reason` is
required and rendered into the comment, `fix` is the corrective direction.

### `rules.database`

```yaml
rules:
  database: { migrations_only: true, migration_paths: ["migrations/**"] }
```

A schema-affecting change outside `migration_paths` is a violation: a changed entity column
without a migration in the same PR, `synchronize: true` in configuration, or raw DDL
(`CREATE TABLE`, `ALTER TABLE`, ...) in non-migration code. Without `migration_paths` and without
detected migrations the rule is `NOT_EXECUTED`.

### `rules.tests`

```yaml
rules:
  tests: { public_api_changes_require_tests: true }
```

A changed API endpoint contract (route, DTO or response type) with no mapped test changed or added
in the PR is a violation. `NOT_EXECUTED` when no test framework was detected.

### `rules.queue_jobs`

```yaml
rules:
  queue_jobs: { require_deterministic_id: true }
```

A new or changed job-producing call whose `jobId` is missing or random (`uuid()`, `Date.now()`,
`Math.random()`, `nanoid()`, `randomUUID()`) is a violation.

### `rules.security`

```yaml
rules:
  security: { authorization_symbols: ["PermissionService.check"] }
```

Symbols that perform authorization. The security reviewer and its verification checks treat a
call to one of them as an authorization step.

## Suppressions (POL-006)

```yaml
suppressions:
  - { id: s1, type: path, value: "src/legacy/**", reason: "Frozen legacy code.",
      owner: "@team", expires: 2027-01-01 }
```

| `type` | `value` matches |
|---|---|
| `type` | the finding category, e.g. `security.input_validation` (exact, or a `prefix.*` pattern) |
| `path` | a glob on the anchor path |
| `symbol` | a symbol id prefix, following renames through lineage |
| `rule` | a rule id |
| `fingerprint` | the root-cause fingerprint |

Every suppression needs a `reason`. Ids must be unique. An expired suppression (`expires` today or
earlier) never matches and is reported as a warning. `type` and `path` suppressions never match a
`critical` finding unless `allow_critical: true` is set. Suppressions are read from the base policy
only; every match is recorded with the suppression id.
