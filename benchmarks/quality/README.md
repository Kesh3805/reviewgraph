# Quality benchmark corpus

Labelled cases used by `review eval` (EVAL-001..EVAL-006). Harness code lives in
`engine/apps/review-cli/src/eval/`.

## Case layout

```
cases/<case_id>/
  case.yaml            # labels, validated against schema/case.v1.schema.json
  base/**              # plain file tree committed as "base"
  head.patch           # unified diff applied with `git apply --index`, committed as "head"
  review-config.yaml   # optional, written to .review/config.yaml in the base tree
```

`build.sh <case_dir> <out_dir>` creates a git repo with fixed author and dates, so base and head
SHAs are identical on every machine. It prints `base_sha head_sha`.

## case.yaml

- `id` equals the directory name and matches `^[a-z]+-[0-9]{3}-[a-z0-9-]+$`.
- `classes`: `correctness_regression`, `safe_change`, `security`, `architecture`, `fp_trap`,
  `test_gap`, `concurrency`, `transaction`, `api_contract`.
- `expected_findings`, `forbidden_findings`, `optional_findings`: entries with `id`, `categories`,
  optional `severity_at_least`, `symbol` (a SymbolId), `location {path, line, window}`,
  `claim_any_of` (lowercase keywords), `must_publish`.
- `expect_no_published_findings: true` implies no expected findings, and holds exactly when
  `classes` contains `safe_change`.
- Cases prefixed `example-` are excluded from metrics.

Validate with `review eval validate [--cases <dir>]`; it exits 1 on any invalid case. The corpus
hash is blake3 over the sorted `(case_id, blake3(dir contents))` pairs.

## Matcher (EVAL-002)

A produced finding matches an expectation when `category_ok AND (symbol_ok OR location_ok) AND
claim_ok`. Category `any` matches everything. Compatible category pairs (closed table, versioned
by `MATCHER_VERSION`): see `engine/apps/review-cli/src/eval/category_compat.rs`.
