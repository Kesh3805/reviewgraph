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
