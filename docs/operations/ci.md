# Continuous integration

GitHub Actions is the build machine. Nothing in this repository requires Docker on a developer
machine: workflows use runner-provided service containers, and images are built only on the CI
runner (see the Docker section).

All third-party actions are pinned by commit SHA with the tag in a trailing comment. To bump one,
resolve the new tag with `gh api repos/<owner>/<action>/commits/<tag> --jq .sha` and replace both
the SHA and the comment in every workflow (`grep -rn "uses: <owner>/<action>" .github/workflows`).

## Rust (`rust.yml`)

Jobs `fmt`, `clippy` (`-D warnings`), `test`, `integration`, `contracts`, `deny`. The toolchain is
read from `engine/rust-toolchain.toml`; `scripts/ci/check-toolchain-parity.sh` fails when it
disagrees with the pin in `engine/docker/dev.Dockerfile` or the active `rustc`. Reproduce locally
with `cd engine && cargo fmt --all --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked`
(or `pnpm test:rust` and `pnpm lint:rust`). The host needs a C compiler for the tree-sitter crates.

## TypeScript (`ts.yml`)

Jobs `lint`, `typecheck`, `unit`, `format`, Node 24 and pnpm 10.15, ubuntu-24.04 only.

- Install is `pnpm install --frozen-lockfile --ignore-scripts`; a stale lockfile fails at install.
- On pull requests `lint` uses `scripts/ci/affected-filter.mjs` to run only affected packages and
  their dependents; changes to the lockfile, root config, `packages/config` or the workflow select
  everything. `main` always runs everything.
- `unit` runs `pnpm -r --if-present test` and the `scripts/**` and `tools/**` node tests with
  `CI=true` and `TZ=UTC`. It writes JUnit XML to `test-results/` (jest through
  `scripts/ci/jest-junit-reporter.cjs`, vitest through its built-in reporter, node:test through
  `--test-reporter=junit`) and an advisory coverage summary for `apps/api` to `coverage/api/`. Both
  are uploaded as the `ts-test-results` artifact and summarized (counts, slowest 10 tests,
  coverage) in the job summary by `scripts/ci/junit-summary.mjs`.
- Coverage thresholds are advisory until a baseline is recorded; enforcement is a follow-up.
- Locally: `pnpm lint:ts`, `pnpm test:ts`. The JUnit and coverage outputs are opt-in through
  `JUNIT_DIR` and `COVERAGE=1`, so plain local runs are unchanged.

## Integration (`integration.yml`)

Jobs `guard`, `api`, `security`, `rust`. Backing services are runner service containers bound to
loopback: Postgres 16 on 35432, Redis on 36379 and (Rust job) Qdrant on 36333, with no persisted
state. Each job waits for its services with `scripts/ci/wait-for-stack.sh` (fails naming every
service that did not come up), applies `engine/migrations/*.sql` in order, runs its suite and, on
failure, uploads container logs and a Postgres slow-query excerpt from `scripts/ci/collect-logs.sh`.

- `guard` runs `scripts/ci/check-integration-gate.mjs`: every Rust test target that reads
  `DATABASE_URL` must be behind the `integration` feature or a skip guard, and the number of
  `#[ignore = "quarantine:ISSUE-n"]` tests must not exceed `scripts/ci/quarantine-budget.txt`.
- `api` also checks Kysely type drift (`node scripts/db-types.mjs --check`).
- `security` runs the tenant isolation suites (`pnpm -F @reviewgraph/api test:security`).
- Locally: install Postgres 16, Redis 7 and Qdrant natively, create the `reviewgraph` role and
  database with the test credentials above, then run `pnpm test:integration`. No Docker is used.
- Deviation from the task text: the object-store service for SEC-007/SEC-009 tests is not part of
  these jobs because it would need Docker Compose; those tests run against their in-memory fake.

## Benchmarks (`bench.yml`)

- `smoke` (pull requests and manual runs): `cargo bench --workspace --no-run --locked` proves every
  bench compiles, a tiny criterion profile runs for `codegraph` and `incremental` (timings are
  informational because shared runners are noisy; a crash or panic fails), and the compare scripts'
  tests run.
- `nightly` (02:17 UTC and manual): the full criterion run with `--save-baseline nightly`,
  converted by `scripts/ci/criterion-to-perf.mjs` to `perf-current.json` (`p95_ms` and `median_ms`
  per benchmark) and compared with `benchmarks/baselines/perf.json` by `scripts/ci/bench-compare.mjs`
  (p95 latency regression above 20 percent or memory above 15 percent fails). A failing scheduled
  run opens or updates one `perf-regression` issue. Artifacts: `perf-current.json`, `compare.txt`,
  criterion output.
- Updating baselines: download the `criterion` artifact of a green nightly (or manual) run, copy
  `perf-current.json` over `benchmarks/baselines/perf.json` and open a reviewed PR whose description
  justifies the change. Never edit baselines to make a run pass.
- Quality baseline (`benchmarks/baselines/quality.json`) is still a placeholder: the review-eval
  `run` command under the replay adapter and the QB-001 labeled corpus do not exist yet, so the
  quality smoke and nightly comparison are not wired. `bench-compare.mjs quality` is ready for it.

## Docker images (`docker.yml`)

Images are built only on the CI runner (`infra/docker/{engine,api,web}.Dockerfile`; see
`infra/docker/README.md`). Per image: build with Buildx and the GitHub Actions layer cache,
hardening checks (`scripts/ci/image-hardening-check.sh`), read-only rootfs smoke
(`scripts/ci/image-smoke.sh`), Trivy (HIGH and CRITICAL, fixed vulnerabilities, waivers from
`security/exceptions.toml`) and a syft CycloneDX SBOM uploaded as `sbom-<image>`. On `v*` tags the
`publish` job pushes `sha-<short>` and semver tags to `ghcr.io/<owner>/<repo>/<image>` with the
workflow token (`packages: write` only on that job; skipped for forks).
