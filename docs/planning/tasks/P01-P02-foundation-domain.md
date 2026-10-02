# Phases 1–2 — Foundation and Core Domain Model

**Parent:** [MASTER_IMPLEMENTATION_PLAN.md](../MASTER_IMPLEMENTATION_PLAN.md) §9 (Phase 1 = FND-001..008, Phase 2 = DOM-001..010)
**Governing docs:** [target-architecture](../../architecture/target-architecture.md) §2, §2.1, §3.2, §4.1, §4.3, §5, §8 · ADR-001, ADR-002, ADR-005, ADR-008, ADR-011, ADR-012, ADR-014, ADR-015 · PRD §49, §52, §105, §108, §109, §149, §150 · gap analysis §P.
**Milestone:** M1 Foundation. Its exit criteria: `engine/scripts/cargo.sh test` green, `pnpm -r test` green, `docker compose up` brings pg/redis/qdrant/openobserve/minio up healthy, and migrations apply.

## Conventions used in this file

- Paths are relative to the repository root `reviewgraph/`.
- `cargo.sh` means `engine/scripts/cargo.sh`. No cargo command in this file runs on the host toolchain (ADR-001).
- The **Global Definition of Done** in master plan §10 applies to every task, on top of the task's own DoD.
- Status markers: ☐ todo · ◐ in progress · ☑ done. A task is marked ☑ only after its acceptance criteria have been run and passed.
- **What already exists (seen 2026-10-02, 11:57–11:58).** A partial engine skeleton is already in the tree. FND-001 and FND-002 **reconcile** it and must not recreate it:
  - `engine/Cargo.toml`:
    - workspace with `members = ["crates/*", "apps/*"]`
    - `rust-version = "1.85"`
    - lints: `unsafe_code=forbid`, clippy `unwrap_used`/`expect_used`/`panic`/`dbg_macro`/`todo` = deny
    - `[workspace.dependencies]` covering the stack (chrono, schemars 0.8, sqlx 0.8 with `chrono`+`uuid`+`migrate`, …)
  - `engine/clippy.toml`, which allows unwrap/expect/panic in tests.
  - 17 crates with one-line `lib.rs` files and no dependencies.
  - 3 apps whose `main.rs` stubs depend only on `anyhow`.
  - `engine/docker/dev.Dockerfile`.
  - `engine/scripts/cargo.sh` and `engine/scripts/cargo.ps1`.
  - An empty `engine/migrations/`.
  - Not yet present: `rust-toolchain.toml`, `Cargo.lock`, `apps/`, `packages/`, `infra/`, `fixtures/`.
- **Library choices fixed by this file** (to match the existing workspace dependencies):
  - timestamps: `chrono::DateTime<Utc>`
  - JSON Schema: `schemars` 0.8 (draft-07)
  - IDs: `uuid` v7 for generated entity IDs, so the `uuid` workspace dependency gains the `v7` feature
  - errors: `thiserror` 2 in libraries, `anyhow` only in apps


## Phase 1 — Foundation

## Task index

| ID | Title |
|---|---|
| FND-001 | Create engine Cargo workspace skeleton |
| FND-002 | Container build wrapper `engine/scripts/cargo.sh` (+ `.ps1`) and engine dev Dockerfile |
| FND-003 | TypeScript pnpm workspace: `apps/api`, `apps/web`, `packages/contracts`, `packages/config` |
| FND-004 | Crate dependency-direction enforcement |
| FND-005 | Local infra compose `infra/compose/docker-compose.yml` (+ `docker-compose.test.yml`) |
| FND-006 | Root task runner (package.json scripts + Makefile) |
| FND-007 | Contracts pipeline: Rust → JSON Schema → TypeScript, with drift check |
| FND-008 | Fixture repository builder |
| DOM-001 | Typed IDs |
| DOM-002 | Error taxonomy |
| DOM-003 | Version constants & Provenance |
| DOM-004 | Repository / RepositorySnapshot / SourceFile entities |
| DOM-005 | PullRequest / ChangedFile / ChangedSymbol / ChangeCluster entities |
| DOM-006 | Finding entities and the FindingState lifecycle |
| DOM-007 | Evidence model |
| DOM-008 | ReviewRun / ReviewerRun entities and the ReviewState machine |
| DOM-009 | Initial PostgreSQL migrations |
| DOM-010 | Port the legacy fail-safe publication decision |

---

### FND-001 — Create engine Cargo workspace skeleton
Status: ☑

> **Implementation note:** Done. `rust-version` stays 1.85 as the MSRV floor; the toolchain is pinned to 1.97 by `engine/rust-toolchain.toml`. An `xtask` member was added for FND-004.

**Task ID:** FND-001

**Title:** Create (reconcile) the engine Cargo workspace skeleton

**Problem:** Domain code needs a workspace whose crate boundaries match target-architecture §2 before any of it lands. The skeleton that exists today falls short in five ways:
- It declares `rust-version = "1.85"`, but the mandated toolchain is 1.97 (master plan §5).
- It has no `rust-toolchain.toml`.
- No crate declares its intra-workspace dependency edges, so FND-004 has nothing to verify.
- There is no committed `Cargo.lock`. The legacy Dockerfile had the same defect, which made builds irreproducible (audit §3.3).
- The app binaries do not parse arguments and have no `--version`.

**Why it exists:** Crate boundaries are how Principle 6 ("domain crates have no I/O") and the §2.1 DAG are enforced. Workspace lints that are present from day one avoid a later sweep to remove `unwrap()`.

**Scope:**
- Bring `engine/Cargo.toml` to the target state described below.
- Add `rust-toolchain.toml` and `rustfmt.toml`.
- Declare in each crate's `Cargo.toml` exactly the internal dependencies allowed by the table in Implementation details. Unused dependencies are fine at this stage.
- Give each `lib.rs` a crate-level doc comment that states its responsibility (copied from target-arch §2) and its "must not" rules.
- Give each app a `clap` parser with `--version`.
- Generate and commit `engine/Cargo.lock`.

**Explicit non-scope:**
- Domain types (DOM-*).
- Dependency enforcement tooling (FND-004).
- Container script changes (FND-002).
- Any external dependency in library crates. Libraries get none in this task.

**Files/modules expected to change:**
- `engine/Cargo.toml`
- `engine/crates/{review-core,telemetry,repository,analysis-ir,lang-typescript,codegraph,graph-storage,incremental,diff-engine,impact,semantic,context-engine,model-gateway,reviewers,verification,profile,pipeline}/Cargo.toml` and `src/lib.rs`
- `engine/apps/{review-cli,review-worker,review-engine}/Cargo.toml` and `src/main.rs`
- `engine/.gitignore` (verify it still contains `/target`)

**New files/modules expected:**
- `engine/rust-toolchain.toml`
- `engine/rustfmt.toml`
- `engine/Cargo.lock`
- `engine/apps/review-cli/tests/version.rs`

**Dependencies (task IDs):** None. This reconciles the existing skeleton. The commands in its acceptance criteria use the existing `cargo.sh`, which already works for this purpose.

**Implementation details:**

`rust-toolchain.toml`:
```toml
[toolchain]
channel = "1.97"
components = ["rustfmt", "clippy"]
profile = "minimal"
```

`rustfmt.toml`:
```toml
edition = "2021"
max_width = 100
newline_style = "Unix"
use_field_init_shorthand = true
```

Changes to `engine/Cargo.toml`:
- `[workspace.package] rust-version = "1.97"`.
- Add `"v7"` to the features of the `uuid` workspace dependency.
- Add these workspace dependencies:
  - `semver = { version = "1", features = ["serde"] }`
  - `cargo_metadata = "0.19"`
  - `criterion = { version = "0.5", default-features = false }`
  - `hex = "0.4"`
- Add `unimplemented = "deny"` to `[workspace.lints.clippy]`. The existing lints stay as they are.

Allowed internal dependencies, declared now and enforced by FND-004. This is the target-arch §2.1 DAG, with one addition: `telemetry` is allowed under every crate except `review-core` and `analysis-ir`.

| Crate | Direct internal deps |
|---|---|
| review-core | — |
| telemetry | review-core |
| repository | review-core, telemetry |
| analysis-ir | review-core |
| lang-typescript | review-core, analysis-ir, telemetry |
| codegraph | review-core, analysis-ir, telemetry |
| graph-storage | review-core, codegraph, telemetry |
| incremental | review-core, analysis-ir, codegraph, graph-storage, telemetry |
| diff-engine | review-core, repository, analysis-ir, codegraph, telemetry |
| impact | review-core, codegraph, diff-engine, telemetry |
| semantic | review-core, codegraph, telemetry |
| profile | review-core, repository, codegraph, telemetry |
| context-engine | review-core, codegraph, impact, semantic, profile, telemetry |
| model-gateway | review-core, telemetry |
| reviewers | review-core, context-engine, model-gateway, telemetry |
| verification | review-core, codegraph, diff-engine, impact, model-gateway, telemetry |
| pipeline | every library crate above |
| apps (review-cli, review-worker, review-engine) | any library crate, never another app |

Each app gets `clap.workspace = true` plus the following, with the right name per app (`review`, `review-worker`, `review-engine`):
```rust
#[derive(clap::Parser, Debug)]
#[command(name = "review", version, about = "ReviewGraph CLI")]
struct Cli {}
fn main() -> anyhow::Result<()> { let _cli = <Cli as clap::Parser>::parse(); Ok(()) }
```

The clippy.toml test allowances do not cover helper functions in `tests/*.rs`. Each integration-test file therefore starts with:
```rust
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
```

**Data model changes:** None.

**API/protocol changes:** None. Each binary gains `--version` and `--help`.

**Concurrency semantics:** None.

**Failure behavior:** Build or lint failures exit non-zero. No step may be wrapped in `|| true`.

**Idempotency considerations:**
- `Cargo.lock` is committed, so repeated builds resolve the same versions.
- `cargo.sh build` is a no-op when nothing has changed.

**Security considerations:**
- `unsafe_code = "forbid"` applies workspace-wide.
- `publish = false` prevents an accidental crates.io publish.
- The lock file pins transitive dependencies. Supply-chain scanning comes in CI-002.

**Observability additions:** None.

**Tests required:**
- `review_cli_prints_version`, in `apps/review-cli/tests/version.rs`. It runs `env!("CARGO_BIN_EXE_review")` with `--version` and asserts that stdout starts with `review 0.1.0`.
- Workspace compile check: `cargo test --workspace --all-targets` compiles all 20 packages.

**Benchmarks if applicable:** Record the cold and warm `cargo.sh build --workspace` wall times in the PR description, as a baseline against ADR-001's 2m46s probe. There is no threshold.

**Acceptance criteria:**
- `engine/scripts/cargo.sh metadata --format-version 1 --no-deps` lists 20 workspace members.
- `engine/scripts/cargo.sh fmt --all --check` exits 0.
- `engine/scripts/cargo.sh clippy --workspace --all-targets -- -D warnings` exits 0.
- `engine/scripts/cargo.sh test --workspace` exits 0, and `review_cli_prints_version` passes.
- `engine/scripts/cargo.sh run -q -p review-cli -- --version` prints `review 0.1.0`.
- `engine/Cargo.lock` exists and is committed.
- `grep -c 'rust-version = "1.97"' engine/Cargo.toml` prints `1`.

**Definition of done:**
- The acceptance criteria pass in the container.
- Every crate's `Cargo.toml` matches the dependency table exactly.
- The table is copied into `docs/architecture/target-architecture.md` §2.1 as the authoritative list. This is a docs edit in the same change.

---

---

### FND-002 — Container build wrapper `engine/scripts/cargo.sh` (+ `.ps1`) and engine dev Dockerfile
Status: ☑

> **Implementation note:** Done. `engine/scripts/run.sh` is the generic container runner and `cargo.sh` delegates to it. `REVIEWGRAPH_DOCKER_NETWORK` joins a compose network. The dev image also installs `sqlx-cli`, and `cargo-deny` installs without `|| true`.

**Task ID:** FND-002

**Title:** Harden the container build wrapper and the engine dev image

**Problem:** The existing wrapper works, but has six defects:
- `dev.Dockerfile` installs `cargo-deny` with `|| true`, which hides a failed install.
- It lacks `sqlx-cli` (FND-006 migrate) and `cargo-insta` (golden tests).
- It installs Debian's Node 18, which is neither the project's Node 24 nor used.
- The image tag `reviewgraph-engine-dev:1` is fixed, so editing the Dockerfile never triggers a rebuild.
- `cargo.ps1` calls `bash`, which on Windows can resolve to the WSL launcher `C:\Windows\System32\bash.exe`.
- Containers can only reach services through `host.docker.internal`. That fails on Linux hosts for ports bound to 127.0.0.1, which is what FND-005 uses.
- Git inside the container also refuses bind-mounted repositories with "dubious ownership", which will break `gix`/git-based tests.

**Why it exists:** ADR-001 makes the container the only build environment. Every later task's acceptance criteria call this wrapper, so it must be correct, reproducible and fast on warm runs.

**Scope:**
- Rewrite `dev.Dockerfile` with pinned tool versions.
- Add a generic runner, `engine/scripts/run.sh`, that executes any command in the dev image. `cargo.sh` becomes a thin call to `run.sh cargo "$@"`.
- Add native PowerShell equivalents, `run.ps1` and `cargo.ps1`.
- Derive the image tag from a content hash.
- Add optional joining of a Docker network.
- Map the host UID/GID on Linux.
- Add a dry-run mode for tests.

**Explicit non-scope:**
- Production images (`infra/docker/*`, DEV/CI tasks).
- CI runners (CI-001).
- Compose services (FND-005).

**Files/modules expected to change:**
- `engine/docker/dev.Dockerfile`
- `engine/scripts/cargo.sh`
- `engine/scripts/cargo.ps1`

**New files/modules expected:**
- `engine/scripts/run.sh`
- `engine/scripts/run.ps1`
- `engine/scripts/tests/run_sh_test.sh`

**Dependencies (task IDs):** FND-001, for `rust-toolchain.toml`, which is part of the image hash.

**Implementation details:**

`dev.Dockerfile`. The dev image stays at `engine/docker/` because it is dev-only. Production images belong in `infra/docker/`.
```dockerfile
FROM rust:1-bookworm
ARG RUST_TOOLCHAIN=1.97
ARG CARGO_DENY_VERSION=0.18.3        # exact pins; bump deliberately
ARG SQLX_CLI_VERSION=0.8.6
ARG CARGO_INSTA_VERSION=1.43.1
RUN rustup toolchain install "$RUST_TOOLCHAIN" --profile minimal --component rustfmt,clippy \
 && rustup default "$RUST_TOOLCHAIN"
RUN apt-get update && apt-get install -y --no-install-recommends git postgresql-client ca-certificates \
 && rm -rf /var/lib/apt/lists/*
RUN cargo install --locked cargo-deny --version "$CARGO_DENY_VERSION"
RUN cargo install --locked sqlx-cli --version "$SQLX_CLI_VERSION" --no-default-features --features rustls,postgres
RUN cargo install --locked cargo-insta --version "$CARGO_INSTA_VERSION"
RUN git config --system --add safe.directory '*' && chmod -R a+rwX /usr/local/cargo
ENV CARGO_TERM_COLOR=always CARGO_TARGET_DIR=/target
WORKDIR /repo/engine
```
The exact patch versions are confirmed at implementation time. Whatever is installed is what gets recorded in the ARGs.

`run.sh` steps:
1. Resolve `ENGINE_DIR` and `REPO_DIR`. Convert paths with `cygpath -m` when it is present (Git Bash).
2. Compute `TAG = sha256(dev.Dockerfile ‖ rust-toolchain.toml)[:12]` and `IMAGE=${REVIEWGRAPH_BUILD_IMAGE:-reviewgraph-engine-dev:$TAG}`. Build the image if it is missing.
3. Preflight: if `docker version --format '{{.Server.Version}}'` fails, print `error: Docker daemon not reachable — engine builds require Docker (ADR-001)` and exit 2.
4. Mount these volumes, all kept from the current script:

   | Source | Container path |
   |---|---|
   | `$REPO_DIR` (bind mount) | `/repo` |
   | `rg-cargo-registry` | `/usr/local/cargo/registry` |
   | `rg-cargo-git` | `/usr/local/cargo/git` |
   | `rg-engine-target` | `/target` |
5. If `REVIEWGRAPH_DOCKER_NETWORK` is set, add `--network "$REVIEWGRAPH_DOCKER_NETWORK"`. Otherwise add `--add-host host.docker.internal:host-gateway`.
6. On Linux (`uname -s` = `Linux`), add `--user "$(id -u):$(id -g)" -e HOME=/tmp`, so files written into `/repo` (insta snapshots, contract schemas) are owned by the host user.
7. Pass environment variables through by **name only**, using `-e NAME` and never `-e NAME=value`, and only when they are set on the host. The current list is kept, plus `REVIEWGRAPH_TEST_*` and `SQLX_OFFLINE`.
8. Use `-it` when stdin and stdout are TTYs, `-i` otherwise.
9. If `REVIEWGRAPH_DRY_RUN=1`, print the docker argv one argument per line and exit 0.
10. Otherwise run `exec docker run …`, so the command's exit code propagates.

`cargo.sh`: `exec "$(dirname "$0")/run.sh" cargo "$@"`.

`run.ps1` is a native port of the same steps. It uses `docker run` directly and never invokes `bash`. `cargo.ps1` calls `run.ps1 cargo @args`.

**Data model changes:** None.

**API/protocol changes:** These environment variables become a documented interface:
- `REVIEWGRAPH_BUILD_IMAGE`
- `REVIEWGRAPH_DOCKER_NETWORK`
- `REVIEWGRAPH_DRY_RUN`

**Concurrency semantics:**
- Two concurrent `cargo.sh` invocations share the target volume. Cargo's own lock on the build directory serializes them ("Blocking waiting for file lock").
- Concurrent image builds of the same tag are idempotent.

**Failure behavior:**
- No Docker daemon: exit 2 with a message.
- Image build failure: the `docker build` exit code is passed through, and no partial tag is left behind.
- Otherwise the exit code is the inner command's. For example, cargo test failures exit 101.

**Idempotency considerations:**
- The image tag is a pure function of its inputs, so unchanged inputs never rebuild.
- Named volumes persist between runs. `docker volume rm rg-engine-target` is the documented reset.

**Security considerations:**
- Secrets are passed by name and never appear in argv or in dry-run output. `ps` on the host shows `-e ANTHROPIC_API_KEY`, never the value.
- The container runs without `--privileged`.
- `safe.directory '*'` applies only inside the dev image.
- The repo is mounted read-write because builds write snapshots. No host paths outside the repo are mounted.

**Observability additions:** None. A one-line `building <image> ...` notice goes to stderr.

**Tests required** (in `engine/scripts/tests/run_sh_test.sh`, plain bash with asserts, run in CI and locally):
- `dry_run_mounts_repo_and_named_volumes`
- `dry_run_joins_network_when_env_set`
- `dry_run_uses_host_gateway_without_network`
- `env_passthrough_by_name_only`: sets `ANTHROPIC_API_KEY=sk-test-123` and asserts the output contains `ANTHROPIC_API_KEY` but not `sk-test-123`.
- `unset_env_not_passed`
- `image_tag_changes_when_dockerfile_changes`: runs against a temporary copy.
- `exit_code_propagates`: `run.sh sh -c 'exit 7'` must exit 7. This is an integration case and needs Docker.
- PowerShell parity: `pwsh -File engine/scripts/run.ps1` with `REVIEWGRAPH_DRY_RUN=1` gives the same argv set as `run.sh`, modulo path formatting.

**Benchmarks if applicable:** Warm no-op `cargo.sh build --workspace` takes ≤ 15 s on the dev machine. Record cold and warm times.

**Acceptance criteria:**
- `engine/scripts/cargo.sh --version` prints `cargo 1.97.`
- `engine/scripts/run.sh cargo deny --version`, `run.sh sqlx --version` and `run.sh cargo insta --version` all exit 0.
- `bash engine/scripts/tests/run_sh_test.sh` exits 0.
- `pwsh -NoProfile -File engine/scripts/cargo.ps1 --version` prints `cargo 1.97.` and never spawns `bash`.
- `engine/scripts/cargo.sh test -p does-not-exist; echo $?` prints `101`.
- After FND-005: `REVIEWGRAPH_DOCKER_NETWORK=reviewgraph_default engine/scripts/run.sh pg_isready -h postgres -p 5432` exits 0.

**Definition of done:**
- The acceptance criteria pass on the Windows host from both Git Bash and PowerShell.
- No `|| true` remains in the Dockerfile.
- The variables are documented in a header comment in `run.sh`. `docs/operations/local-development.md` gets the same documentation in DEV-001.

---

---

### FND-003 — TypeScript pnpm workspace: `apps/api`, `apps/web`, `packages/contracts`, `packages/config`
Status: ☑

> **Implementation note:** Done.

**Task ID:** FND-003

**Title:** Create the TypeScript pnpm workspace with shared tsconfig, eslint and prettier

**Problem:** There is no TypeScript workspace. The only TS code is the legacy the legacy prototype (audit §7) Vite app. The NestJS API (API-001), the Next.js web app (WEB-001) and the generated contracts (FND-007) need one strict, shared configuration and a `pnpm -r lint typecheck test` that works from the first commit.

**Why it exists:** M1 requires `pnpm -r test` to pass. If shared config does not exist up front, each app invents its own strictness, and those settings drift.

**Scope:**
- Root `package.json`, `pnpm-workspace.yaml` and `.npmrc`.
- `packages/config` holding the shared tsconfig presets, the ESLint 9 flat config and the Prettier config.
- Placeholder packages for `apps/api`, `apps/web` and `packages/contracts`. Each has `lint`, `typecheck` and `test` scripts and one trivial test.
- A committed `pnpm-lock.yaml`.

**Explicit non-scope:**
- NestJS bootstrap (API-001).
- Next.js app (WEB-001).
- Contract generation (FND-007).
- Root orchestration scripts for Rust and compose (FND-006).
- the legacy prototype (audit §7), which stays outside the workspace.

**Files/modules expected to change:** `.gitignore`, which gains `.pnpm-store/`, `**/dist/`, `**/.next/`, `**/coverage/` and `*.tsbuildinfo`.

**New files/modules expected:**
- `package.json`
- `pnpm-workspace.yaml`
- `.npmrc`
- `.nvmrc` (`24`)
- `packages/config/{package.json,tsconfig.base.json,tsconfig.node.json,tsconfig.nest.json,tsconfig.next.json,eslint.config.js,prettier.config.js}`
- `apps/api/{package.json,tsconfig.json,jest.config.ts,src/index.ts,test/service-name.spec.ts}`
- `apps/web/{package.json,tsconfig.json,vitest.config.ts,src/index.ts,src/index.test.ts}`
- `packages/contracts/{package.json,tsconfig.json,src/index.ts,src/index.test.ts}`
- `pnpm-lock.yaml`

**Dependencies (task IDs):** None.

**Implementation details:**

Root `package.json`:
```json
{ "name": "reviewgraph", "private": true, "packageManager": "pnpm@10.<pinned>",
  "engines": { "node": ">=24 <25" },
  "scripts": { "lint": "pnpm -r lint", "typecheck": "pnpm -r typecheck", "test:ts": "pnpm -r test",
               "format": "prettier --write .", "format:check": "prettier --check ." } }
```

`pnpm-workspace.yaml`:
```yaml
packages: ["apps/api", "apps/web", "packages/*"]
onlyBuiltDependencies: []        # pnpm 10 blocks dependency lifecycle scripts by default; allow-list explicitly
```

`.npmrc`: `engine-strict=true`, `save-exact=true`.

`tsconfig.base.json` compiler options:
- `strict`, `noUncheckedIndexedAccess`, `exactOptionalPropertyTypes`, `noImplicitOverride`, `noPropertyAccessFromIndexSignature`, `useUnknownInCatchVariables`, `verbatimModuleSyntax`, `isolatedModules`, `forceConsistentCasingInFileNames`
- `target: "ES2023"`, `module: "NodeNext"`, `moduleResolution: "NodeNext"`
- `skipLibCheck: true`, `declaration: true`, `sourceMap: true`

The presets:

| Preset | Adds |
|---|---|
| `tsconfig.nest.json` | `experimentalDecorators: true`, `emitDecoratorMetadata: true`. The output is CJS because the package has no `"type": "module"`; `verbatimModuleSyntax` is turned off for Nest compatibility. |
| `tsconfig.next.json` | `jsx: "preserve"`, `module: "ESNext"`, `moduleResolution: "Bundler"`, `lib: ["dom","dom.iterable","ES2023"]` |

ESLint: the `typescript-eslint` `strictTypeChecked` and `stylisticTypeChecked` presets, plus:
- `@typescript-eslint/no-floating-promises: error`
- `@typescript-eslint/no-explicit-any: error`
- `no-console: ["error", { allow: ["warn", "error"] }]`
- `eslint-config-prettier` last

It ignores `dist`, `.next`, `coverage` and `src/generated/**`.

Prettier: `{ singleQuote: true, trailingComma: "all", printWidth: 100 }`.

Test runners: `apps/api` uses Jest with `ts-jest`, as ADR-002 requires. `apps/web` and `packages/*` use Vitest.

Package names: `@reviewgraph/api`, `@reviewgraph/web`, `@reviewgraph/contracts`, `@reviewgraph/config`.

Each package defines these scripts:
- `lint: eslint .`
- `typecheck: tsc --noEmit -p tsconfig.json`
- `test`

Placeholder sources export `export const PACKAGE_NAME = '@reviewgraph/<x>' as const;`.

**Data model changes:** None.

**API/protocol changes:** None.

**Concurrency semantics:** `pnpm -r` runs packages in topological order with default concurrency. The placeholder tests share no state.

**Failure behavior:**
- Any lint, type or test failure fails the recursive command, because pnpm propagates a non-zero exit code.
- `engine-strict` rejects Node versions other than 24.

**Idempotency considerations:** `pnpm install --frozen-lockfile` is reproducible. The lockfile is committed.

**Security considerations:**
- pnpm 10's default block on dependency lifecycle scripts stays on. Only packages listed explicitly in `onlyBuiltDependencies` may build.
- `save-exact` stops silent range upgrades.
- `pnpm audit` is wired in CI-002.

**Observability additions:** None.

**Tests required:**
- `apps/api/test/service-name.spec.ts`: `exports its package name`.
- `apps/web/src/index.test.ts`: `exports its package name`.
- `packages/contracts/src/index.test.ts`: `exports its package name`.
- Lint self-check: a file containing `const x: any = 1` makes `pnpm -F @reviewgraph/api lint` fail. Verify manually once and record the result in the PR; do not commit the file.

**Benchmarks if applicable:** None.

**Acceptance criteria:**
- `pnpm install --frozen-lockfile` exits 0 on Node 24 and pnpm 10.
- `pnpm -r lint`, `pnpm -r typecheck` and `pnpm -r test` exit 0.
- `pnpm format:check` exits 0.
- `node -e "require('./packages/config/tsconfig.base.json')"` parses, and `compilerOptions.strict === true`.
- `pnpm ls -r --depth -1` lists exactly the four workspace packages and does not list the legacy prototype (audit §7).

**Definition of done:**
- The acceptance criteria pass on the host.
- The lockfile is committed.
- The ignores are added to `.gitignore`.

---

---

### FND-004 — Crate dependency-direction enforcement
Status: ☑

> **Implementation note:** Done. The DAG check lives in `engine/xtask` (`tests/dependency_dag.rs`) rather than `engine/tools/arch-tests`. `cargo deny` bans are in `engine/deny.toml`.

**Task ID:** FND-004

**Title:** Enforce the §2.1 crate DAG with `cargo deny` bans and a workspace metadata test

**Problem:** The DAG in target-arch §2.1 is documentation only. Nothing stops a later change from making `review-core` depend on `sqlx`, `reviewers` depend on `reqwest`, or `codegraph` depend on `lang-typescript`. Each of these would break an architectural rule silently: no I/O in core (§2.1), no provider clients in reviewers (ADR-009), language-neutral graph (Invariant 5).

**Why it exists:** Gap analysis §A/§2: "Crate DAG (target-arch §2.1) + `cargo deny` bans (FND-004)". The enforcement must be in place before the first domain code lands.

**Scope:**
- An `engine/deny.toml` `[bans]` section. Only the bans section; CI-002 adds advisories and licenses.
- A test-only workspace package, `engine/tools/arch-tests`. It reads `cargo metadata` and checks the allowed table from FND-001, plus external-crate rules.
- Add `tools/arch-tests` to the workspace members.

**Explicit non-scope:**
- License and advisory policy (CI-002).
- Module-level layering inside a crate.
- TypeScript import boundaries (API-001 adds `eslint-plugin-boundaries`).

**Files/modules expected to change:** `engine/Cargo.toml`, where members gains `"tools/arch-tests"`.

**New files/modules expected:**
- `engine/deny.toml`
- `engine/tools/arch-tests/Cargo.toml`
- `engine/tools/arch-tests/src/lib.rs` (checker)
- `engine/tools/arch-tests/src/rules.rs` (the allowed table)
- `engine/tools/arch-tests/tests/dependency_dag.rs`

**Dependencies (task IDs):** FND-001, FND-002 (the `cargo-deny` binary in the image).

**Implementation details:**

`deny.toml`:
```toml
[graph]
all-features = true
[bans]
multiple-versions = "warn"
wildcards = "deny"
allow-wildcard-paths = true          # workspace path deps (publish = false)
deny = [
  # no async runtime / network / db in pure crates — wrappers = the only crates allowed to pull them in directly
  { crate = "reqwest",  wrappers = ["model-gateway", "semantic", "review-cli", "review-worker", "review-engine"] },
  { crate = "sqlx",     wrappers = ["graph-storage", "pipeline", "review-cli", "review-worker", "review-engine"] },
  { crate = "axum",     wrappers = ["review-engine"] },
  { crate = "anyhow",   wrappers = ["review-cli", "review-worker", "review-engine"] },
  { crate = "tree-sitter-typescript", wrappers = ["lang-typescript"] },
  { crate = "tree-sitter", wrappers = ["lang-typescript"] },
  { crate = "pipeline", wrappers = ["review-cli", "review-worker", "review-engine"] },
  { crate = "lang-typescript", wrappers = ["pipeline", "review-cli", "review-worker", "review-engine"] },
]
```
At implementation time, check whether cargo-deny applies `wrappers` to workspace path crates. If it does not, the arch test below is the authoritative check, and the deny entries for internal crates are deleted with a comment saying so.

`arch-tests` checker API:
```rust
pub struct DepGraph { pub members: BTreeMap<String, MemberDeps> }
pub struct MemberDeps { pub internal: BTreeSet<String>, pub external: BTreeSet<String>, pub is_app: bool }
pub enum Violation {
    UnknownMember(String),
    ForbiddenInternalEdge { from: String, to: String },
    LibraryDependsOnApp { from: String, to: String },
    ForbiddenExternal { crate_name: String, dep: String },
}
pub fn load(manifest: &Path) -> Result<DepGraph, cargo_metadata::Error>;
pub fn check(graph: &DepGraph, rules: &Rules) -> Vec<Violation>;
```

`Rules`:
- `allowed_internal: BTreeMap<&str, &[&str]>`, the FND-001 table verbatim.
- `review_core_external_allowlist = ["serde", "serde_json", "thiserror", "uuid", "chrono", "blake3", "hex", "schemars", "semver"]`.
- `analysis_ir_external_allowlist`, the same list.

Normal and build dependencies are checked. Dev dependencies are checked only for the rule "no dependency on an app".

**Data model changes:** None.

**API/protocol changes:** None.

**Concurrency semantics:** The test spawns `cargo metadata` once, which takes cargo's lock briefly. It is safe under `cargo test` parallelism because the test loads the graph once through `std::sync::OnceLock`.

**Failure behavior:**
- A violation fails the test, and the message lists every violation in the form `ForbiddenInternalEdge codegraph -> lang-typescript (allowed: review-core, analysis-ir, telemetry)`.
- A `cargo metadata` failure fails the test with the cargo error. It is never skipped.

**Idempotency considerations:** Pure check. Results are deterministic because `BTreeMap`/`BTreeSet` ordering is used.

**Security considerations:** This is a supply-chain boundary. Keeping `reqwest` out of `reviewers` and `verification` means a reviewer cannot exfiltrate data to an arbitrary endpoint (ADR-009).

**Observability additions:** None.

**Tests required** (in `tests/dependency_dag.rs` and the `src/lib.rs` unit tests):
- `workspace_matches_allowed_dag`, against the real workspace.
- `every_member_is_known`
- `review_core_has_no_io_dependencies`
- `rejects_core_depending_on_codegraph`, on a synthetic graph.
- `rejects_library_depending_on_app`, on a synthetic graph.
- `rejects_unknown_member`, on a synthetic graph.
- `rejects_reqwest_in_reviewers`, on a synthetic graph.

**Benchmarks if applicable:** None.

**Acceptance criteria:**
- `engine/scripts/cargo.sh test -p arch-tests` exits 0.
- `engine/scripts/run.sh cargo deny check bans` exits 0.
- Negative check, done manually and recorded in the PR: temporarily add `codegraph.workspace = true` to `review-core/Cargo.toml`. Both commands then fail, and the test output names `review-core -> codegraph`. Revert afterwards.

**Definition of done:**
- Both checks run in `pnpm lint` (FND-006) and in CI-001.
- target-arch §2.1 links to `engine/tools/arch-tests/src/rules.rs` as the machine-checked copy of the DAG.

---

---

### FND-005 — Local infra compose `infra/compose/docker-compose.yml` (+ `docker-compose.test.yml`)
Status: ☑

> **Implementation note:** Done, with one deviation: **MinIO images are no longer publicly pullable** (docker.io `minio/minio` is denied and quay.io returns 401, checked 2026-10-02). Local object storage is therefore **SeaweedFS** (`chrislusf/seaweedfs:4.48`, S3 API on `127.0.0.1:29000`, bucket `reviewgraph-artifacts` created by the idempotent `objectstore-init`). The OpenObserve root password default was changed to satisfy its password-strength rule. Where this file says MinIO, read "the S3-compatible object store".

**Task ID:** FND-005

**Title:** Local infrastructure compose (postgres16, redis7, qdrant, openobserve, minio) with healthchecks, plus an ephemeral test compose

**Problem:** None of the backing services exist locally. The legacy the legacy prototype (audit §7) runs only postgres, on `127.0.0.1:15432`, and records that host ports 5432 and 5433 are already in use on this machine. Integration tests (DOM-009, GS-*, SEM-*) and M1 need all five services, healthy and reachable from both the host and the engine container.

**Why it exists:** It is the M1 exit criterion. Every integration test depends on it. A separate test compose keeps CI and local test runs from touching developer data.

**Scope:**
- A dev compose file with named volumes, loopback-only ports, healthchecks, a one-shot MinIO bucket init, and an OpenObserve readiness sidecar.
- A test compose file with tmpfs storage, distinct ports and a fixed network name.
- `infra/compose/.env.example`.

**Explicit non-scope:**
- Application services (`api`, `engine`, `worker`, `web`), which DEV-001 adds.
- OpenObserve dashboards and alerts (OBS-007/008).
- Production deployment.

**Files/modules expected to change:** `.gitignore`, which gains `infra/compose/.env`.

**New files/modules expected:**
- `infra/compose/docker-compose.yml`
- `infra/compose/docker-compose.test.yml`
- `infra/compose/.env.example`
- `infra/compose/postgres/init/00-extensions.sql`

**Dependencies (task IDs):** None.

**Implementation details:**

Project names:
- `name: reviewgraph`, so the default network is `reviewgraph_default`.
- `name: reviewgraph-test`, with an explicitly named network `reviewgraph-test-net`.

Images are pinned through variables with exact defaults. Never use `latest`. The exact patch tags are confirmed at implementation time.

| Service | Image var (default) | Host port (dev) | Host port (test) | Volume (dev) |
|---|---|---|---|---|
| postgres | `RG_POSTGRES_IMAGE` (`postgres:16.x-bookworm`) | `127.0.0.1:${RG_PG_PORT:-25432}:5432` | `127.0.0.1:35432` | `rg-pg-data:/var/lib/postgresql/data` |
| redis | `RG_REDIS_IMAGE` (`redis:7.4.x-alpine`) | `127.0.0.1:26379` | `127.0.0.1:36379` | none (ephemeral by design) |
| qdrant | `RG_QDRANT_IMAGE` (`qdrant/qdrant:v1.x.y`) | `127.0.0.1:26333` (HTTP), `127.0.0.1:26334` (gRPC) | `127.0.0.1:36333` | `rg-qdrant-data:/qdrant/storage` |
| openobserve | `RG_OPENOBSERVE_IMAGE` (`public.ecr.aws/zinclabs/openobserve:v0.x.y`) | `127.0.0.1:25080` | not in test compose | `rg-oo-data:/data` |
| minio | `RG_MINIO_IMAGE` (`minio/minio:RELEASE.<date>`) | `127.0.0.1:29000` (API), `127.0.0.1:29001` (console) | `127.0.0.1:39000` | `rg-minio-data:/data` |

Postgres:
- `POSTGRES_USER=${RG_PG_USER:-reviewgraph}`
- `POSTGRES_PASSWORD=${RG_PG_PASSWORD:-reviewgraph-dev}`
- `POSTGRES_DB=reviewgraph`
- Command: `postgres -c log_min_duration_statement=500 -c max_connections=200`
- `00-extensions.sql` runs `CREATE EXTENSION IF NOT EXISTS pg_stat_statements;` and `CREATE DATABASE reviewgraph_shadow;`. The shadow database is for `#[sqlx::test]` template use.
- `gen_random_uuid()` is built in, so it needs no extension.

Redis: `redis-server --save "" --appendonly no --maxmemory 256mb --maxmemory-policy noeviction`. Idempotency keys and locks must never be evicted silently. Running out of memory is an explicit error instead.

Healthchecks use `interval: 5s`, `timeout: 3s`, `retries: 30`, `start_period: 5s`:

| Service | Healthcheck |
|---|---|
| postgres | `pg_isready -U $$POSTGRES_USER -d reviewgraph` |
| redis | `redis-cli ping` |
| qdrant | `bash -c 'exec 3<>/dev/tcp/127.0.0.1/6333; printf "GET /readyz HTTP/1.0\r\n\r\n" >&3; grep -q " 200 " <&3'` |
| minio | `mc ready local` |
| openobserve | No probe tooling in the image is assumed. A sidecar service, `openobserve-ready` (`curlimages/curl:<pinned>`, `command: sleep infinity`), has the healthcheck `curl -fsS http://openobserve:5080/healthz`, so `docker compose up --wait` blocks on it. |

`minio-init` is a one-shot `minio/mc` service with `depends_on: minio: service_healthy`. It runs:
- `mc alias set local http://minio:9000 $$MINIO_ROOT_USER $$MINIO_ROOT_PASSWORD`
- `mc mb --ignore-existing local/reviewgraph-artifacts`
- `mc anonymous set none local/reviewgraph-artifacts`

OpenObserve settings:
- `ZO_ROOT_USER_EMAIL=${RG_OO_EMAIL:-dev@reviewgraph.local}`
- `ZO_ROOT_USER_PASSWORD=${RG_OO_PASSWORD:-reviewgraph-dev-oo}`
- `ZO_DATA_DIR=/data`

Every service sets `security_opt: ["no-new-privileges:true"]` and `restart: unless-stopped` (dev only).

The test compose:
- Uses `tmpfs: /var/lib/postgresql/data` for postgres and `tmpfs: /qdrant/storage` for qdrant.
- Has no named volumes.
- Has `restart: "no"`.
- Uses the same healthchecks.

**Data model changes:** None. The schema arrives in DOM-009.

**API/protocol changes:** Connection conventions, written down in `.env.example`:

| Variable | Value |
|---|---|
| `DATABASE_URL` (host) | `postgres://reviewgraph:reviewgraph-dev@127.0.0.1:25432/reviewgraph` |
| `DATABASE_URL` (container, `REVIEWGRAPH_DOCKER_NETWORK=reviewgraph_default`) | `postgres://reviewgraph:reviewgraph-dev@postgres:5432/reviewgraph` |
| `REDIS_URL` | `redis://127.0.0.1:26379` |
| `QDRANT_URL` | `http://127.0.0.1:26333` |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | `http://127.0.0.1:25080/api/default` |
| `S3_ENDPOINT` | `http://127.0.0.1:29000` |

**Concurrency semantics:**
- The dev and test stacks can run at the same time, because their project names, ports and networks differ.
- `docker compose up --wait` returns only once every service with a healthcheck is healthy.

**Failure behavior:**
- If any service fails its healthcheck, `up --wait` exits non-zero and names the service.
- A port collision fails fast with Docker's bind error. Every port can be overridden through `RG_*_PORT`.

**Idempotency considerations:**
- `up` is re-runnable.
- `minio-init` uses `--ignore-existing`.
- `00-extensions.sql` runs only when the volume is first initialized, which is postgres's own semantics.
- `down -v` is the documented reset.

**Security considerations:**
- Every port binds to 127.0.0.1 only.
- The defaults are dev-only credentials and are never used outside local work. `.env` is gitignored.
- The MinIO bucket is private.
- No service gets `privileged` or the host network.
- The Redis no-eviction policy prevents security-relevant keys (webhook dedup) from vanishing silently.

**Observability additions:** OpenObserve itself becomes the local telemetry backend. Its endpoint is documented for OBS-001.

**Tests required:**
- `compose_config_valid`: `docker compose -f infra/compose/docker-compose.yml config -q` and the same for the test file.
- `compose_up_all_healthy`: `up --wait` followed by `docker compose ps --format json`. Every service reports `"Health":"healthy"`, and `minio-init` reports `"ExitCode":0`.
- `ports_loopback_only`: a script, `infra/compose/check-ports.sh`, asserts that every published port in `docker compose config --format json` has `host_ip == "127.0.0.1"`.

**Benchmarks if applicable:** Record the cold `up --wait` time to healthy (target < 90 s with images cached).

**Acceptance criteria:**
- `docker compose -f infra/compose/docker-compose.yml up -d --wait` exits 0.
- `psql postgres://reviewgraph:reviewgraph-dev@127.0.0.1:25432/reviewgraph -c 'select 1'` succeeds from the dev container with the network joined, or from the host if psql is installed.
- `curl -fsS http://127.0.0.1:26333/readyz` and `curl -fsS http://127.0.0.1:25080/healthz` succeed.
- `bash infra/compose/check-ports.sh` exits 0.
- `docker compose -f infra/compose/docker-compose.test.yml up -d --wait`, then `down -v`, leaves no `reviewgraph-test` volumes behind.

**Definition of done:**
- Both stacks come up healthy on the Windows host with Docker Desktop.
- The acceptance criteria pass.
- `.env.example` documents every variable.

---

---

### FND-006 — Root task runner (package.json scripts + Makefile)
Status: ☑

> **Implementation note:** Done. `scripts/rg.mjs` drives everything. On Windows the engine scripts run through Git Bash, never the WSL `bash.exe` shim. `migrate` targets `host.docker.internal:25432` from the build container.

**Task ID:** FND-006

**Title:** Root task runner: up/down/test/lint/fmt/migrate across Rust and TypeScript

**Problem:** Developers and CI need one vocabulary for everyday work across two languages, a container build and a compose stack. Raw invocations are long, and they differ between Git Bash and PowerShell. On Windows, `pnpm` runs scripts through `cmd.exe`, where `bash` may resolve to WSL.

**Why it exists:**
- Master plan §15 names `pnpm dev:up` as the local entry point.
- M1's exit criteria are phrased in terms of these commands.
- CI (CI-001) calls the same scripts, so local and CI behaviour cannot drift.

**Scope:**
- Root `package.json` scripts, which are the single source of truth.
- A cross-platform Node dispatcher that picks `cargo.sh` or `cargo.ps1`.
- A `Makefile` whose targets are thin aliases to the pnpm scripts.
- `migrate` through `sqlx-cli` in the dev container.

**Explicit non-scope:**
- CI workflows (CI-001).
- Starting app services (DEV-001).
- The production migration runner. DOM-009 adds `review-worker migrate`.

**Files/modules expected to change:** Root `package.json` (from FND-003).

**New files/modules expected:**
- `Makefile`
- `scripts/rg.mjs`
- `scripts/rg.test.mjs`

**Dependencies (task IDs):** FND-002, FND-003, FND-005.

**Implementation details:**

`scripts/rg.mjs` is a Node 24 ESM script with no dependencies. It runs `rg.mjs <cmd> [...args]`:
- `cargo …` spawns `bash engine/scripts/cargo.sh …` on non-Windows platforms. On win32 it spawns `pwsh -NoProfile -File engine/scripts/cargo.ps1 …`, falling back to `powershell.exe` if `pwsh` is missing.
- `run …` does the same through `run.sh` / `run.ps1`.
- `compose <dev|test> …` runs `docker compose -f infra/compose/docker-compose{,.test}.yml …`.

It uses `child_process.spawn` with `stdio: 'inherit'` and exits with the child's code. Without `shell: true`, no argument is shell-interpolated.

Scripts:

| Script | Command |
|---|---|
| `dev:up` | `node scripts/rg.mjs compose dev up -d --wait` |
| `dev:down` | `node scripts/rg.mjs compose dev down` |
| `dev:reset` | `node scripts/rg.mjs compose dev down -v` |
| `test:rust` | `node scripts/rg.mjs cargo test --workspace` |
| `test:ts` | `pnpm -r test` |
| `test` | `pnpm test:rust && pnpm test:ts` |
| `test:integration` | `compose test up -d --wait`, then `REVIEWGRAPH_DOCKER_NETWORK=reviewgraph-test-net DATABASE_URL=postgres://reviewgraph:reviewgraph-dev@postgres:5432/reviewgraph node scripts/rg.mjs cargo test --workspace --features integration`, then `compose test down -v`. The teardown runs through a `try/finally` inside `rg.mjs integration`. |
| `lint` | `rg cargo fmt --all --check && rg cargo clippy --workspace --all-targets -- -D warnings && rg run cargo deny check bans && pnpm -r lint && pnpm -r typecheck && pnpm format:check` |
| `fmt` | `rg cargo fmt --all && prettier --write .` |
| `migrate` | `REVIEWGRAPH_DOCKER_NETWORK=reviewgraph_default DATABASE_URL=postgres://…@postgres:5432/reviewgraph node scripts/rg.mjs run sqlx migrate run --source migrations` (cwd `/repo/engine`) |
| `migrate:info` | the same, with `sqlx migrate info` |

The integration feature: crates that own DB tests declare `[features] integration = []` and gate their tests with `#![cfg(feature = "integration")]`.

`Makefile`: the targets `up down reset test test-rust test-ts test-integration lint fmt migrate` each run `pnpm run <script>`. It is documented as optional, since Git Bash may not ship `make`.

**Data model changes:** None.

**API/protocol changes:** None. This is developer CLI vocabulary.

**Concurrency semantics:** The scripts run sequentially (`&&`). `test:integration` owns the test stack for the whole run, so two concurrent runs on one machine would collide on `reviewgraph-test`. That is documented as unsupported.

**Failure behavior:**
- The first failing step stops the chain with its exit code.
- `test:integration` always tears the test stack down, even when tests fail.
- `migrate` with the stack down fails with sqlx's connection error and a hint: `run pnpm dev:up first`.

**Idempotency considerations:**
- `migrate` is idempotent, because sqlx skips applied versions and verifies checksums.
- `dev:up` is re-runnable.

**Security considerations:**
- `rg.mjs` never uses `shell: true`, so there is no argument injection.
- The credentials in the scripts are the dev-only defaults from FND-005. Overrides come from the environment.

**Observability additions:** None.

**Tests required** (`scripts/rg.test.mjs`, run with `node --test`):
- `selects_ps1_on_win32`, with `process.platform` stubbed through an injected parameter.
- `selects_sh_elsewhere`
- `propagates_child_exit_code`
- `does_not_use_shell`

**Benchmarks if applicable:** None.

**Acceptance criteria:**
- From a fresh clone: `pnpm install && pnpm dev:up && pnpm migrate && pnpm lint && pnpm test` all exit 0. `migrate` reports that no migrations exist yet, or applies DOM-009's migrations once those exist.
- `pnpm test:integration` exits 0, and `docker volume ls` afterwards shows no `reviewgraph-test` volumes.
- `node --test scripts/rg.test.mjs` exits 0.
- These work from both PowerShell and Git Bash.

**Definition of done:**
- The acceptance criteria pass.
- The list of scripts appears in the root `README.md` under "Development". It is a short section; DEV-001 expands it.

---

---

### FND-007 — Contracts pipeline: Rust → JSON Schema → TypeScript, with drift check
Status: ☑

> **Implementation note:** `check.mjs` accepts `--schemas`, `--generated` and `--fresh <dir>`; `--fresh` skips the container export so the TS drift test runs without Docker (the default path still exports through `rg.mjs cargo`). Added workspace dependency `sha2` for the `index.json` hashes, `clap` for the CLI, and the `v7` feature on `uuid`. The `.tmp/` gitignore entry is `/packages/contracts/.tmp/`.

**Task ID:** FND-007

**Title:** Contracts pipeline: `review contracts export` writes `packages/contracts/schemas/*.json`, which generate TS types, with a drift check

**Problem:**
- Target-arch §1 and ADR-002 require shared shapes to be defined once, in Rust, and consumed in TypeScript. Examples are job payloads, finding states and `ReviewEvent`.
- No pipeline exists to do that. Without one, the NestJS publisher would hand-copy enums like `ReviewState` or `ReviewEvent`, which is exactly where the fail-safe could silently diverge (DOM-010).

**Why it exists:** It is the only coupling mechanism the architecture allows between the two planes, other than PostgreSQL rows and the HTTP API. ADR-012 also names `packages/contracts` as the home of payload schemas.

**Scope:**
- A `contracts export` subcommand in `review-cli`, backed by a type registry.
- A deterministic JSON writer.
- A TS generation script in `packages/contracts`.
- A `contracts:check` drift script.
- A seed contract type, `SchemaInfo`, so the pipeline is testable before the DOM types exist.

**Explicit non-scope:**
- Registering the domain types. DOM-006, DOM-008 and DOM-010 add their own types to the registry.
- Runtime validation in the API (API tasks).
- Kysely DB types, which are generated from the migrated DB in API-002.

**Files/modules expected to change:**
- `engine/apps/review-cli/Cargo.toml`, adding `review-core`, `schemars`, `serde_json`
- `engine/crates/review-core/Cargo.toml`, adding `serde` and `schemars`
- `packages/contracts/package.json`
- the root `package.json` scripts

**New files/modules expected:**
- `engine/crates/review-core/src/contracts.rs` (`SchemaInfo`)
- `engine/apps/review-cli/src/contracts.rs` (registry + export)
- `packages/contracts/scripts/generate.mjs`
- `packages/contracts/scripts/check.mjs`
- `packages/contracts/schemas/index.json`
- `packages/contracts/schemas/SchemaInfo.schema.json`
- `packages/contracts/src/generated/SchemaInfo.ts`
- `packages/contracts/src/generated/index.ts`

**Dependencies (task IDs):** FND-001, FND-002, FND-003.

**Implementation details:**

Seed type:
```rust
// review-core::contracts
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SchemaInfo { pub contracts_version: u32, pub types: Vec<String> }
pub const CONTRACTS_VERSION: u32 = 1;
```

Registry, in `review-cli::contracts`:
```rust
pub struct ContractType { pub name: &'static str, pub schema: fn() -> schemars::schema::RootSchema }
pub fn registry() -> Vec<ContractType> { vec![ ContractType { name: "SchemaInfo", schema: || schemars::schema_for!(review_core::contracts::SchemaInfo) } ] }
pub fn export(out_dir: &Path) -> anyhow::Result<Vec<PathBuf>>;
```

`export`:
- Asserts that the names are unique.
- For each type, serializes the schema through `serde_json::Value`, recursively sorting object keys, pretty-printed with 2 spaces plus a trailing `\n`.
- Sets `"$id": "urn:reviewgraph:contracts:<Name>"`.
- Writes `<Name>.schema.json`, plus `index.json` = `{ contracts_version, types: [sorted names], sha256: { <Name>: hex } }`.
- Deletes any `*.schema.json` in `out_dir` that is not in the registry, so stale schemas cannot linger.

CLI: `review contracts export --out <dir>`. The default directory is `../packages/contracts/schemas`, relative to `engine/`.

`generate.mjs` uses `json-schema-to-typescript` (pinned):
- One `compile(schema, name, { bannerComment: '/* GENERATED by packages/contracts/scripts/generate.mjs — do not edit */', additionalProperties: false, strictIndexSignatures: true, format: true })` per schema.
- Writes `src/generated/<Name>.ts` and a sorted barrel `src/generated/index.ts`.
- `src/index.ts` re-exports `./generated/index.js`.

`check.mjs`:
1. Run the export into `os.tmpdir()/rg-contracts-<pid>` through `rg.mjs cargo run -q -p review-cli -- contracts export --out /repo/<tmp-relative>`. The temporary directory lives under `packages/contracts/.tmp/` (gitignored), so the container can see it.
2. Byte-compare it with `schemas/`.
3. Generate TS from the temporary schemas into `.tmp/generated`, and byte-compare that with `src/generated`.
4. Print each drifted file and exit 1 on any difference.

Root scripts: `contracts:export`, `contracts:generate` and `contracts:check`. `lint` gains `pnpm contracts:check`.

**Data model changes:** None.

**API/protocol changes:** This creates the contracts package as an interface. The rules:
- Schema files are append-compatible within one `contracts_version`.
- Removing or renaming a field bumps `CONTRACTS_VERSION`.

**Concurrency semantics:** None. This is an offline generation step.

**Failure behavior:**
- Duplicate names, a non-serializable schema or an unwritable `out_dir` make export exit 1 with the reason.
- Drift makes the check exit 1 and print a list of files.
- There is no partial write: export writes into `<out>.tmp-<pid>`, then renames each file into place.

**Idempotency considerations:** Export is deterministic, because keys are sorted, there is no timestamp, and files are newline-terminated. Running it twice gives byte-identical output, and a test asserts this.

**Security considerations:**
- Generated TS carries only shape information.
- Every schema uses `deny_unknown_fields` / `additionalProperties: false`, so unknown fields are rejected at the boundary.

**Observability additions:** None.

**Tests required:**
- Rust unit tests in `review-cli`:
  - `export_is_deterministic`: export twice into tempdirs, and the bytes are equal.
  - `export_removes_stale_schemas`
  - `registry_names_unique`
  - `schema_info_schema_has_no_additional_properties`
- TS tests in `packages/contracts`:
  - `generated index exports SchemaInfo`, a type-level test through `tsc --noEmit` on a usage file.
  - `check detects drift`: the test copies `schemas/` to a temporary directory, mutates one schema, and runs `check.mjs --schemas <tmp>`. It expects exit 1.

**Benchmarks if applicable:** None.

**Acceptance criteria:**
- `pnpm contracts:export && pnpm contracts:generate && pnpm contracts:check` exits 0.
- Editing a field doc on `SchemaInfo` without re-exporting makes `pnpm contracts:check` exit 1. Verify manually, then revert.
- `pnpm -F @reviewgraph/contracts typecheck test` exits 0.

**Definition of done:**
- The pipeline runs end to end.
- `contracts:check` is part of `pnpm lint`.
- A README section in `packages/contracts/package.json` `description` states the rule "do not edit `src/generated`". No separate README file is needed.

---

---

### FND-008 — Fixture repository builder
Status: ☑

> **Implementation note:** `.gitignore` already ignored `/fixtures/build/`; `/fixtures/.build/` was added. `build.sh` also accepts `--src DIR` (fixture root override, used by the self-tests) and `--out DIR`. Executable and symlink modes are checked from the git index (symlinks additionally on disk) because file modes on a Windows checkout are not reliable. On the host with Git 2.49, `%aI` prints `2026-01-01T02:00:00Z` rather than `+00:00` (git formats a zero offset as `Z`); the stored commit date is identical. `build.sh --all` takes about 13 s on the Windows host (process spawn cost), above the 10 s target; it is about 13 s in the container as well because of container start-up. `host_and_container_same_shas` is verified by running `check.sh` in both environments (both pass against the same `EXPECTED_SHAS`) rather than being a `--self-test` case.

**Task ID:** FND-008

**Title:** Deterministic fixture git repositories built from plain-file step directories

**Problem:**
- The analyzer, graph, incremental, diff and identity tasks (TSA, CG, INC, DIFF, SID) all need small, real git repositories with scripted history: renames, moves, multi-commit edits.
- Committing nested `.git` directories is impossible.
- Building them ad hoc in each test gives SHAs that differ between machines, which breaks golden snapshots that embed commit SHAs.

**Why it exists:**
- Master plan §11 (fixture repositories) and ADR-006 ("adding a language means … fixture repositories").
- SID-006 (rename/move suite) and INC-012 (oracle) require reproducible histories.

**Scope:**
- A step-directory format.
- `fixtures/build.sh`, which produces deterministic git repositories under a gitignored build directory, plus `fixtures/check.sh`.
- Three initial fixtures: `ts-basic-calls`, `nest-app` and `rename-move`.
- Expected commit SHAs, committed.

**Explicit non-scope:**
- Pull-request golden scenarios (`fixtures/pull-requests/*`, for example auth-bypass), which belong to CHG/QB tasks.
- Expected IR or graph snapshots (TSA/CG golden tests).
- A Rust helper crate for loading fixtures. TSA-001 can add `fixtures` path resolution through `REVIEWGRAPH_FIXTURES`.

**Files/modules expected to change:**
- `.gitignore`, which gains `fixtures/.build/`.
- `.gitattributes`. If it does not exist, create it with `fixtures/repositories/** -text`.

**New files/modules expected:**
- `fixtures/build.sh`
- `fixtures/check.sh`
- `fixtures/repositories/<name>/steps/NNN-<slug>/…`
- `fixtures/repositories/<name>/EXPECTED_SHAS`

**Dependencies (task IDs):** FND-002. It is needed only to verify that the container build and the host build produce the same SHAs.

**Implementation details:**

Step format. `fixtures/repositories/<name>/steps/` contains directories `001-<slug>`, `002-<slug>`, … processed in `LC_ALL=C` lexical order. Each step can contain:
- `_commit.txt` (required): the commit message.
- `_ops.txt` (optional): one operation per line, executed **before** the overlay is copied:

  | Operation | Effect |
  |---|---|
  | `rm <path>` | `git rm -q` |
  | `mv <from> <to>` | `git mv` |
  | `branch <name>` | `git switch -c` |
  | `switch <name>` | `git switch` |
- Every other file is an overlay, copied over the working tree at the same relative path.

Files whose names start with `_` at the step root are control files and are never copied. Symlinks and executable bits are forbidden, and the build script rejects them.

`build.sh <name>|--all [--out DIR]` builds into `fixtures/.build/repos/<name>/`, which is deleted and recreated. For each repository:
```bash
export GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null TZ=UTC LC_ALL=C
git init -q --template= -b main "$dst"
git -C "$dst" config core.autocrlf false; config core.fileMode false; config core.symlinks false
git -C "$dst" config commit.gpgsign false; config user.name "ReviewGraph Fixtures"; config user.email fixtures@reviewgraph.invalid
# per step i (1-based): apply _ops, copy overlay (cp -R), git add -A
d=$(date -u -d "@$((1767225600 + i*3600))" +%Y-%m-%dT%H:%M:%SZ)   # 2026-01-01T00:00:00Z + i h
GIT_AUTHOR_DATE=$d GIT_COMMITTER_DATE=$d git -C "$dst" commit -q --no-verify -F "$step/_commit.txt"
git -C "$dst" tag "step-$NNN"
```
The `date -d` call is GNU-only. Git Bash ships GNU date, and the script fails clearly if `date -d` is unsupported.

After building, `build.sh` writes `fixtures/.build/repos/<name>.shas`, one `step-NNN <sha>` line per step.

`check.sh` builds `--all` and diffs every `.shas` file against the committed `EXPECTED_SHAS`. It exits 1 on a mismatch and prints the remedy: `if intentional, run fixtures/build.sh --all --update-expected`.

Initial fixtures:

1. **`ts-basic-calls`**
   - `001-initial`:
     - `package.json` and `tsconfig.json`
     - `src/math.ts`: `add`, `mul` and the class `Calculator`, whose `total()` calls `this.add()`
     - `src/util/format.ts`: a default export plus a named arrow function
     - `src/index.ts`: imports with aliases, `new Calculator()` and calls
     - `src/util/index.ts`: a barrel re-export
   - `002-modify-body`: the body of `mul` changes, and a new caller is added in `index.ts`.
2. **`nest-app`**
   - `001-initial`:
     - `src/app.module.ts`
     - `src/users/users.module.ts`, `users.controller.ts` (`@Controller('users')`, `@Get(':id')`, `@UseGuards(AuthGuard)`), `users.service.ts` (constructor-injected `UsersRepository`), `users.repository.ts` (TypeORM `@InjectRepository(User)`) and `user.entity.ts` (`@Entity('users')`, columns)
     - `src/auth/auth.guard.ts` (`CanActivate`)
     - `src/jobs/email.processor.ts` (BullMQ `@Processor('email')`, `process()`)
     - `src/users/users.service.spec.ts` (Jest `describe`/`it`, `Test.createTestingModule`)
   - `002-add-endpoint`: adds `@Post()` `create()`, which calls `usersService.create()` and then enqueues an `email` job.
3. **`rename-move`**
   - `001-initial`: `src/a/orders.ts` contains `function computeTotal`, and `class OrderService` has the method `submit`.
   - `002-rename-function`: `computeTotal` becomes `calculateTotal`, with the body unchanged.
   - `003-move-file`: `mv src/a/orders.ts src/b/orders.ts`, with the imports updated.
   - `004-rename-and-edit`: `submit` becomes `place`, with a one-line body change, giving token similarity ≥ 0.8.
   - `005-rename-class`: `OrderService` becomes `OrdersService`.

None of the fixture code needs `node_modules`. Only syntax matters.

**Data model changes:** None.

**API/protocol changes:** These become the documented interface for the fixture format and the build directory: `fixtures/.build/repos/<name>`, with tags `step-NNN`.

**Concurrency semantics:**
- Builds of different fixtures are independent.
- Building the same fixture concurrently is unsafe, because the build deletes and recreates the directory. `build.sh` takes `flock fixtures/.build/.lock` when `flock` is available, and is otherwise documented as single-run.

**Failure behavior:**
- A missing `_commit.txt`, a symlink, an executable file, an unknown op, or a git failure aborts with `set -euo pipefail` and names the step.
- A partially built repository is removed when the script exits on error (`trap`).

**Idempotency considerations:**
- Byte-identical inputs give identical SHAs on every run and every OS.
- Line endings are kept stable by `-text` in `.gitattributes` together with `autocrlf=false`.
- File modes are kept stable by `fileMode=false`, which makes new entries `100644`.

**Security considerations:**
- `GIT_CONFIG_GLOBAL=/dev/null` and `--template=` mean user hooks and config can never run.
- Fixture content contains no secrets. CI's secret scan (SEC-003) covers `fixtures/`.

**Observability additions:** None.

**Tests required:**
- `check.sh` is the test. Named cases, run by `check.sh --self-test`:
  - `build_twice_same_shas`
  - `host_and_container_same_shas`: run `bash fixtures/check.sh` on the host and through `engine/scripts/run.sh bash /repo/fixtures/check.sh`.
  - `rejects_symlink_in_step`, using a temporary fixture.
  - `rejects_missing_commit_message`
  - `rename_move_history_has_git_detectable_renames`: `git log --follow -M src/b/orders.ts` lists `step-001`.

**Benchmarks if applicable:** `build.sh --all` takes under 10 s.

**Acceptance criteria:**
- `bash fixtures/check.sh` exits 0 on the host, using Git Bash.
- `engine/scripts/run.sh bash /repo/fixtures/check.sh` exits 0 in the container.
- `git -C fixtures/.build/repos/rename-move tag` lists `step-001` through `step-005`.
- `git -C fixtures/.build/repos/nest-app log --format='%an %aI' -1` prints `ReviewGraph Fixtures 2026-01-01T02:00:00+00:00`.

**Definition of done:**
- The three fixtures and `EXPECTED_SHAS` are committed.
- Both host and container checks pass.
- The format is documented in a header comment of `build.sh`.

---

---

### DOM-001 — Typed IDs
Status: ☑

> **Implementation note:** The spec path `engine/tools/arch-tests` does not exist; the dependency-DAG test is `engine/xtask`, and it stays green (`review-core` has no internal dependencies). `CoreError` is introduced in `error.rs` with only `InvalidId` and `InvalidCommitSha` (adding the `thiserror` dependency here); DOM-002 extends it. UUID IDs deserialize through `FromStr` (hyphenated 36-character form only) so serde and `FromStr` give the same error. The workspace `schemars` dependency gained the `uuid1` and `chrono` features, and the workspace gained `hex` and `criterion`. Benchmark baseline: `SymbolKey::of` on a 63-byte ID is about 77 ns. Golden value for `ts:src/auth/auth.service#AuthService.authorize/method` is `97ac70ec2191e38555c6678614fc4699`.

**Task ID:** DOM-001

**Title:** Typed ID newtypes with serde- and sqlx-compatible representations

**Problem:** Entities are referenced across crates, the database and the API. Raw `Uuid`/`String` IDs can be swapped silently: a `RepositoryId` passed where an `OrganizationId` is expected is a tenant-isolation bug (R9). The legacy `finding_id` was positional (`AR-<pr>-<seq>`, `core.rs:403-405`) and unstable (audit §3.1).

**Why it exists:**
- PRD §149 domain objects.
- Master plan §13.1: every query is org- and repo-scoped. Typed IDs turn a mix-up into a compile error.
- ADR-005 defines `SymbolKey` and `SymbolId`.

**Scope:**
- UUID-backed IDs: `OrganizationId`, `UserId`, `RepositoryId`, `SnapshotId`, `FileVersionId`, `PullRequestId`, `ReviewRunId`, `ReviewerRunId`, `CandidateFindingId`, `VerifiedFindingId`, `PublishedFindingId`.
- `SymbolKey`: 128-bit, hex.
- `SymbolId`: an opaque canonical string.
- `CommitSha`.

**Explicit non-scope:**
- `SymbolId` grammar parsing and construction from IR (SID-001).
- Synthetic node IDs (CG-*).
- DB row structs (GS/API tasks).
- `ChangeClusterKey` (DOM-005).

**Files/modules expected to change:**
- `engine/crates/review-core/Cargo.toml`, adding `serde`, `uuid` (v7, serde), `blake3`, `hex`, `schemars`
- `engine/crates/review-core/src/lib.rs`

**New files/modules expected:**
- `engine/crates/review-core/src/ids.rs`
- `engine/crates/review-core/benches/symbol_key.rs`

**Dependencies (task IDs):** FND-001, FND-002.

**Implementation details:**

UUID IDs:
```rust
macro_rules! uuid_id { ($name:ident) => {
    #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
    #[serde(transparent)] pub struct $name(Uuid);
    impl $name {
        pub fn new() -> Self { Self(Uuid::now_v7()) }          // generated in-app; time-ordered for index locality
        pub const fn from_uuid(u: Uuid) -> Self { Self(u) }
        pub const fn as_uuid(&self) -> &Uuid { &self.0 }
        pub const fn into_uuid(self) -> Uuid { self.0 }
    }
    impl From<Uuid> for $name { … }  impl From<$name> for Uuid { … }   // enables #[sqlx(try_from = "Uuid")] downstream
    impl fmt::Display for $name { /* hyphenated lowercase */ }
    impl fmt::Debug for $name { /* "$name(<uuid>)" */ }
    impl FromStr for $name { type Err = CoreError; /* Uuid::parse_str, error kind = stringify!($name) */ }
}}
```

"sqlx-compatible" means that `review-core` stays sqlx-free. Persistence crates bind `id.as_uuid()` and decode with `#[sqlx(try_from = "Uuid")]` on row fields. This works without orphan-rule problems and without pulling sqlx into the pure crate. DOM-009's tests demonstrate the pattern.

`SymbolKey([u8; 16])`:
- `SymbolKey::of(id: &SymbolId) -> SymbolKey`, defined as `blake3::hash(id.as_str().as_bytes())` truncated to the first 16 bytes (ADR-005).
- `Display` and serde use 32 lowercase hex characters.
- `FromStr` accepts exactly 32 lowercase hex characters and rejects uppercase, so there is one canonical form.
- `as_bytes() -> &[u8; 16]`, so a future `bytea` column can be chosen in GS-002.

`SymbolId(String)`:
- `#[serde(transparent)]`.
- `SymbolId::from_canonical_unchecked(s: impl Into<String>)`, documented as "only SID-001's parser and storage decoding may call this".
- `as_str()`.
- `SymbolId` deliberately has no `FromStr`; SID-001 adds `SymbolId::parse`.

`CommitSha(String)`:
- Validated as 40 or 64 lowercase hex characters (SHA-1 or SHA-256 repositories).
- `FromStr` lowercases only if the input is already valid hex. Any other character is rejected.
- `short()` gives the first 12 characters.

`CoreError::InvalidId { kind: &'static str, reason: String }` is a temporary local error until DOM-002 consolidates it.

**Data model changes:** None. The representations map onto `uuid` and `text` columns in DOM-009.

**API/protocol changes:** The JSON representation is fixed:
- UUID IDs serialize as hyphenated lowercase strings.
- `SymbolKey` serializes as 32 hex characters.
- `SymbolId` serializes as the canonical string.

**Concurrency semantics:** All ID types are `Copy` or immutable and are `Send + Sync`, which a static assertion test checks. `Uuid::now_v7()` is thread-safe. Ordering within a millisecond across threads is not guaranteed, and nothing may depend on it.

**Failure behavior:** Parsing returns `Err(CoreError::InvalidId)` and never panics. Deserialization surfaces the same message through serde.

**Idempotency considerations:**
- `SymbolKey::of` is a pure function: the same `SymbolId` always gives the same key, on every platform and version. A golden test pins this.
- Entity IDs are generated once, at the entity's creation, and are never regenerated on retry. Callers persist them before any side effect.

**Security considerations:**
- Distinct types prevent cross-tenant ID confusion at compile time.
- `Debug` shows the type name, which helps audits.
- IDs carry no secrets.
- UUIDv7 leaks the creation time (millisecond). That is acceptable for internal IDs. Provider-facing identifiers stay provider-native.

**Observability additions:** None. `Display` impls are the canonical span-attribute form for `organization_id`, `repository_id`, `review_run_id` and the rest (target-arch §8), used by OBS-001.

**Tests required:**
- `uuid_ids_roundtrip_display_fromstr_serde`
- `uuid_ids_reject_malformed`
- `distinct_id_types_do_not_unify`: a `compile_fail` doctest passes a `RepositoryId` to `fn f(_: OrganizationId)`.
- `symbol_key_golden`: `SymbolKey::of(ts:src/auth/auth.service#AuthService.authorize/method)` equals a pinned 32-hex value, computed once and committed.
- `symbol_key_rejects_uppercase_and_wrong_length`
- `commit_sha_accepts_sha1_and_sha256_rejects_other`
- `ids_are_send_sync`
- A proptest, `symbol_key_display_parse_roundtrip`, over arbitrary 16-byte arrays.

**Benchmarks if applicable:** `benches/symbol_key.rs` (criterion) measures `SymbolKey::of` on a 64-byte ID. Target < 300 ns/op (1M symbols < 0.3 s). Record the baseline.

**Acceptance criteria:**
- `engine/scripts/cargo.sh test -p review-core ids` passes every test listed.
- `engine/scripts/cargo.sh test -p review-core --doc` passes, including the compile_fail doctest.
- `engine/scripts/cargo.sh bench -p review-core --bench symbol_key -- --quick` runs and reports under 300 ns.
- `engine/scripts/cargo.sh test -p arch-tests` is still green, so `review-core` dependencies are within the allowlist.

**Definition of done:**
- The tests and the bench pass.
- The golden `SymbolKey` value is committed.
- ADR-005 gains a link to `ids.rs`.

---

---

### DOM-002 — Error taxonomy
Status: ☑

> **Implementation note:** The arch-test rule lives in `engine/xtask` (the spec path `engine/tools/arch-tests/src/rules.rs` does not exist): `xtask::check` reports a violation when any non-app crate lists `anyhow` as a dependency, tested by `no_library_depends_on_anyhow` and `anyhow_rule_flags_libraries_but_not_apps`. No `cargo deny` entry was added, since a per-crate ban of `anyhow` cannot be expressed without banning it in the apps; `cargo deny check bans` passes. `ErrorClass` is re-exported at the crate root and the wire-name snapshot is `crates/review-core/src/snapshots/`. The `pipeline` crate counts as a library crate for the `anyhow` rule.

**Task ID:** DOM-002

**Title:** Error taxonomy: `thiserror` per library crate, `anyhow` only in apps, a shared `ErrorClass`, and a mapping guide

**Problem:**
- The legacy code used `anyhow` everywhere, and its transient-vs-permanent logic lived in string matching (`agent.rs` `is_transient`, audit §3.1).
- The new pipeline needs typed errors for three decisions: retry or not (PIPE-001), which HTTP status the review-engine returns, and which `FAILED_*` state a run enters.

**Why it exists:**
- Master plan §10 Global DoD: no panics for normal failures.
- ADR-009's typed `GatewayError` family.
- ADR-012's retry semantics.

**Scope:**
- `review_core::error::{CoreError, ErrorClass, Classify}`.
- An `error.rs` with a crate-level `Error` enum and a `Result<T>` alias in each of the other 16 library crates.
- The `anyhow` ban for library crates, through the FND-004 deny entry and an arch-test rule.
- The mapping guide document.

**Explicit non-scope:**
- Concrete domain error variants for later crates. Those crates add them in their own tasks.
- HTTP error middleware (API-001, review-engine tasks).

**Files/modules expected to change:**
- `engine/crates/review-core/src/lib.rs`
- `engine/crates/review-core/src/ids.rs`, which moves to `CoreError`
- `engine/crates/review-core/Cargo.toml`, adding `thiserror`
- `engine/crates/*/src/lib.rs` (16)
- `engine/crates/*/Cargo.toml`, adding `thiserror`
- `engine/tools/arch-tests/src/rules.rs`

**New files/modules expected:**
- `engine/crates/review-core/src/error.rs`
- `engine/crates/<each library>/src/error.rs`
- `docs/architecture/error-handling.md`

**Dependencies (task IDs):** DOM-001, FND-004.

**Implementation details:**
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorClass { InvalidInput, NotFound, Conflict, Transient, RateLimited, Permanent, Cancelled, Internal }
impl ErrorClass {
    pub const fn as_str(self) -> &'static str;            // span attribute `error.class`
    pub const fn is_retryable(self) -> bool { matches!(self, Self::Transient | Self::RateLimited) }
}
pub trait Classify { fn class(&self) -> ErrorClass; }

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CoreError {
    #[error("invalid {kind}: {reason}")] InvalidId { kind: &'static str, reason: String },
    #[error("invalid repository path {path:?}: {reason}")] InvalidRepoPath { path: String, reason: &'static str },
    #[error("invalid commit sha: {0}")] InvalidCommitSha(String),
    #[error("invalid {entity} transition {from} -> {to}")] InvalidTransition { entity: &'static str, from: &'static str, to: &'static str },
    #[error("{field} out of range: {value}")] OutOfRange { field: &'static str, value: String },
    #[error("invalid version {kind}: {reason}")] InvalidVersion { kind: &'static str, reason: String },
}
impl Classify for CoreError { /* InvalidTransition → Conflict; all others → InvalidInput */ }
```

Each library crate gets:
```rust
#[derive(Debug, thiserror::Error)] #[non_exhaustive]
pub enum Error { #[error(transparent)] Core(#[from] review_core::CoreError) }
pub type Result<T, E = Error> = std::result::Result<T, E>;
impl review_core::Classify for Error { fn class(&self) -> ErrorClass { match self { Error::Core(e) => e.class() } } }
```

Rules written into `error-handling.md`:
1. Library crates never expose `anyhow`, `Box<dyn Error>` or `String` errors.
2. Every library error implements `Classify`.
3. `#[non_exhaustive]` on all public error enums.
4. Wrap with context using `#[error("…: {source}")]` variants, not by formatting into strings.
5. Apps convert to `anyhow` only at the boundary (`main`, job handler top, HTTP handler).
6. Never log an error's `Display` if it may contain source code or secrets. Use the variant name plus IDs.

Mapping table in the doc:

| ErrorClass | HTTP (review-engine/API) | Job outcome (PIPE-001) | Run state effect |
|---|---|---|---|
| InvalidInput | 400 | `failed`, no retry | `FAILED_<stage>` |
| NotFound | 404 | `failed`, no retry | `FAILED_<stage>` |
| Conflict | 409 | no retry; the CAS was lost, so the stage aborts quietly (it is superseded or done elsewhere) | unchanged |
| Transient | 503 | retry with jittered backoff until `max_attempts`, then `dead` | `FAILED_<stage>` only when dead |
| RateLimited | 429 (`Retry-After`) | retry at `retry_after` | as for Transient |
| Permanent | 422 | `failed` | `FAILED_<stage>` |
| Cancelled | 409 | `cancelled` | `CANCELLED` / `SUPERSEDED` |
| Internal | 500 | `failed`, alert | `FAILED_<stage>` |

The arch-test gains a rule: no library crate lists `anyhow` among its normal dependencies.

**Data model changes:** None. `ErrorClass::as_str` values become the allowed values of `review_runs.failure_class` in DOM-009.

**API/protocol changes:** `ErrorClass` gets a stable snake_case wire form and is exported to contracts by DOM-008.

**Concurrency semantics:** Every error type is `Send + Sync + 'static`, which a test asserts. This is required to cross tokio tasks.

**Failure behavior:** This task defines failure behaviour for the whole system. The table above is normative.

**Idempotency considerations:**
- `is_retryable` is the only input to the automatic-retry decision.
- Conflict is never retried, because retrying a lost CAS would re-execute a stage that someone else has already advanced.

**Security considerations:**
- Error `Display` strings must not embed source text, tokens or prompts (rule 6).
- `CoreError::InvalidRepoPath` echoes the rejected path. Paths are not secret, but its length is capped at 256 characters in the message to bound log size.

**Observability additions:** It defines the span attribute `error.class`, set from `Classify::class()`, and the metric label `error_class`. OBS-002 records both.

**Tests required:**
- `error_class_wire_names_stable`, an insta snapshot of all variants.
- `only_transient_and_rate_limited_retryable`
- `invalid_transition_is_conflict`
- `core_error_is_send_sync_static`
- `each_library_error_wraps_core_error`, in each crate: `Error::from(CoreError::OutOfRange{..}).class() == InvalidInput`.
- arch-tests `no_library_depends_on_anyhow`

**Benchmarks if applicable:** None.

**Acceptance criteria:**
- `engine/scripts/cargo.sh test --workspace` is green.
- `engine/scripts/cargo.sh test -p arch-tests no_library_depends_on_anyhow` passes.
- `engine/scripts/run.sh cargo deny check bans` passes.
- `docs/architecture/error-handling.md` exists, with the mapping table and rules 1–6.

**Definition of done:**
- Every library crate has `error.rs` and implements `Classify`.
- The doc is linked from target-arch §2.
- The tests pass.

---

---

### DOM-003 — Version constants & Provenance
Status: ☑

> **Implementation note:** Added workspace dependency `semver` (serde feature). `EmbeddingSpace` and the semver wrappers deserialize through their validating constructors, so malformed input is rejected by serde as well. A shared `pub(crate) schema::string_pattern` helper now builds the hand-written string schemas (also used by `ids.rs`). `Invalidation` is a plain value type (not serialized). `cargo insta` was not run; the single snapshot was accepted with `INSTA_UPDATE=always` and `cargo test` leaves no `.snap.new` files.

**Task ID:** DOM-003

**Title:** Version types and the `Provenance` struct (ADR-015)

**Problem:**
- Derived data must be reproducible and must be invalidated selectively (PRD §15, §105, §121; ADR-015).
- No version types exist, and the legacy system recorded none.
- If each crate invents its own version representation, cache keys and the fingerprint (INIT-012) cannot be composed.

**Why it exists:**
- Invariant 6: intelligence is versioned and reproducible.
- ADR-015's provenance list and bump/invalidate table.

**Scope:**
- Types: `GraphSchemaVersion`, `AnalyzerVersion`, `ProfileVersion`, `ReviewerVersion`, `PromptVersion`, `VerificationVersion`, `EmbeddingSpace`, `ConfigHash`, `Language`.
- `Provenance` and `ModelProvenance`.
- `Provenance::invalidation_against(&Provenance) -> Invalidation`, implementing the ADR-015 table.

**Explicit non-scope:**
- The concrete version values. `codegraph::SCHEMA_VERSION` comes from CG-010, analyzer versions from TSA, and `verification_version` from VER-009.
- Fingerprint computation (INIT-012).
- `config_hash` normalization of `.review/config.yaml` (POL-001).

**Files/modules expected to change:**
- `engine/crates/review-core/src/lib.rs`
- `engine/crates/review-core/Cargo.toml`, adding `semver`

**New files/modules expected:**
- `engine/crates/review-core/src/version.rs`
- `engine/crates/review-core/src/language.rs`
- `engine/crates/review-core/src/provenance.rs`

**Dependencies (task IDs):** DOM-001, DOM-002.

**Implementation details:**
```rust
#[serde(rename_all = "lowercase")] pub enum Language { Typescript, Javascript, Python, Java, Go, Rust }
impl Language { pub const fn id_prefix(self) -> &'static str /* "ts","js","py","java","go","rs" — used by SymbolId */ }

#[serde(transparent)] pub struct GraphSchemaVersion(pub u32);
#[serde(transparent)] pub struct AnalyzerVersion(semver::Version);   // major bump ⇒ reparse that language
#[serde(transparent)] pub struct ProfileVersion(pub u32);
#[serde(transparent)] pub struct ReviewerVersion(semver::Version);
pub struct PromptVersion { pub prompt: PromptName, pub n: u32 }       // Display/serde "correctness/v3"
pub struct PromptName(String);                                          // ^[a-z][a-z0-9_-]{0,47}$
#[serde(transparent)] pub struct VerificationVersion(pub u32);
pub struct EmbeddingSpace { pub provider: String, pub model: String, pub dims: u32 }
impl EmbeddingSpace { pub fn collection_name(&self, n: u32) -> String }
#[serde(transparent)] pub struct ConfigHash([u8; 32]);                 // blake3; hex(64) on the wire

pub struct Provenance {
    pub commit_sha: CommitSha,
    pub graph_schema_version: GraphSchemaVersion,
    pub analyzer_versions: BTreeMap<Language, AnalyzerVersion>,       // BTreeMap ⇒ deterministic serialization
    pub config_hash: ConfigHash,
    pub profile_version: ProfileVersion,
    pub model: Option<ModelProvenance>,
}
pub struct ModelProvenance {
    pub embedding_space: Option<EmbeddingSpace>, pub reviewer_version: Option<ReviewerVersion>,
    pub prompt_version: Option<PromptVersion>, pub provider: Option<String>, pub model: Option<String>,
    pub verification_version: Option<VerificationVersion>,
}
pub struct Invalidation { pub full_rebuild: bool, pub reparse_languages: BTreeSet<Language>,
                          pub profile: bool, pub model_layers: bool }
impl Provenance { pub fn invalidation_against(&self, previous: &Provenance) -> Invalidation }
```

`collection_name(n)` follows ADR-008: `rg_{provider}_{model}_{dims}_v{n}`. Each component is lowercased, and every character outside `[a-z0-9]` is replaced with `_`. For example, `("voyage","voyage-code-3",1024)` gives `rg_voyage_voyage_code_3_1024_v1`. This deliberately follows the ADR rather than target-arch §3.9's `rg_{space}_v{n}`. The target-arch text is corrected in the same change.

`invalidation_against` rules (ADR-015):

| Condition | Result |
|---|---|
| `graph_schema_version` differs | `full_rebuild = true` |
| `config_hash` differs | `full_rebuild = true` (PRD §24 config change, conservative) |
| an analyzer **major** version differs for a language | that language is added to `reparse_languages` |
| a language is added or removed | that language is added to `reparse_languages` |
| a minor or patch analyzer bump | no reparse, because the analyzer contract requires minor bumps to be IR-compatible; this is documented on `AnalyzerVersion` |
| `profile_version` differs | `profile = true` |
| any `ModelProvenance` field differs | `model_layers = true` |
| `commit_sha` differs alone | no flags; incremental handles it |

**Data model changes:** None. `Provenance` serializes into `review_runs.provenance jsonb` (DOM-009) and `snapshots` columns (GS-002).

**API/protocol changes:** These wire forms become stable:
- `PromptVersion` as `"name/vN"`
- `ConfigHash` as 64 hex characters
- `Language` as lowercase strings

**Concurrency semantics:** Immutable value types, `Send + Sync`.

**Failure behavior:** Constructors validate their input:
- `PromptName` by regex.
- `EmbeddingSpace.dims` must be greater than 0, and `provider` and `model` must be non-empty.
- Semver parsing.

Violations return `CoreError::InvalidVersion`.

**Idempotency considerations:**
- Serialization is deterministic: `BTreeMap`/`BTreeSet` and no floats. A golden test asserts it. Cache keys and INIT-012 hash this serialization.
- `invalidation_against` is a pure function.

**Security considerations:** `provider` and `model` are free text that ends up in collection names. Sanitization to `[a-z0-9_]` prevents path- or URL-injection into Qdrant collection paths.

**Observability additions:** None. OBS uses `Display` for the `graph_schema_version` and `prompt_version` span attributes.

**Tests required:**
- `prompt_version_roundtrip_and_rejects_bad_names`
- `collection_name_sanitizes`
- `collection_name_matches_adr_008_example`
- `provenance_serialization_is_deterministic`: build it twice, with analyzer versions inserted in different orders, and the bytes are equal (insta snapshot).
- `schema_bump_forces_full_rebuild`
- `config_hash_change_forces_full_rebuild`
- `analyzer_major_bump_reparses_only_that_language`
- `analyzer_minor_bump_no_reparse`
- `model_change_invalidates_only_model_layers`
- `commit_change_alone_invalidates_nothing`

**Benchmarks if applicable:** None.

**Acceptance criteria:**
- `engine/scripts/cargo.sh test -p review-core version provenance` passes all the tests listed.
- `cargo insta test -p review-core` (through `run.sh`) shows no pending snapshots.

**Definition of done:**
- Tests pass.
- target-arch §3.9 has the corrected collection-name format.
- ADR-015 links to `provenance.rs`.

---

---

### DOM-004 — Repository / RepositorySnapshot / SourceFile entities
Status: ☐

**Task ID:** DOM-004

**Title:** `Repository`, `RepositorySnapshot` and `SourceFile` entities, plus the location primitives (`RepoPath`, `ContentHash`, `SourceRange`, `SourceLocation`)

**Problem:** Indexing (INIT, IDX, INC), storage (GS) and findings (DOM-006/007) all need the same repository, snapshot, file and location types. Without validated path and location types, path-traversal and off-by-one bugs spread into every consumer. The legacy code used free `String` files and single `u32` lines (`core.rs:73-104`).

**Why it exists:**
- PRD §149 (`Repository`, `RepositorySnapshot`, `SourceFile`).
- ADR-003/ADR-015 snapshot kinds.
- Every later location-bearing type depends on these primitives.

**Scope:**
- Entity structs and their invariants.
- `SnapshotKind`/`SnapshotStatus` enums.
- `RepoPath`, `ContentHash`, `Position`, `SourceRange`, `LineRange`, `DiffSide`, `SourceLocation`.
- `ProviderKind`.

**Explicit non-scope:**
- `Symbol`, `GraphNode`, `GraphEdge` (CG-*/analysis-ir).
- Snapshot persistence and delta-chain logic (GS-004).
- Snapshot compaction (INC).

**Files/modules expected to change:** `engine/crates/review-core/src/lib.rs`, `Cargo.toml` (adding `chrono`).

**New files/modules expected:**
- `engine/crates/review-core/src/location.rs`
- `engine/crates/review-core/src/repository.rs`

**Dependencies (task IDs):** DOM-003.

**Implementation details:**

`RepoPath(String)`. `RepoPath::new(s)` validates and normalizes. It rejects:
- an empty string
- a leading `/`
- a drive prefix (`^[A-Za-z]:`)
- a `\`
- a NUL byte
- any `.` or `..` segment
- an empty segment (`//`)
- a total length over 4096 bytes

Further rules:
- Only forward slashes are allowed, and the path is stored exactly as given. There is no case folding, because the repository may be case-sensitive.
- `RepoPath::module_path()` returns the path without its final extension (ADR-005's `module_path`). For example, `src/a.service.ts` gives `src/a.service`.
- `extension()` returns the final extension.

`ContentHash([u8; 32])` is blake3 over the raw file bytes. `ContentHash::of(bytes: &[u8])` is pure. On the wire it is 64 hex characters.

Positions and ranges:
- `Position { line: u32, column: u32 }`. Lines are **1-based**. Columns are **0-based UTF-8 byte offsets** within the line, matching tree-sitter columns. This is documented on the type.
- `SourceRange { start: Position, end: Position }`, with the invariant `start <= end`.
- `LineRange { start: u32, end: u32 }`, 1-based and inclusive, with the invariant `1 <= start <= end`.
- `DiffSide { Base, Head }`, serialized as `"base"` / `"head"`.
- `SourceLocation { path: RepoPath, side: DiffSide, lines: LineRange, range: Option<SourceRange> }`.

`ProviderKind { Github }`, serialized as `"github"`. It is `#[non_exhaustive]`, so GitLab and Bitbucket can be added later (MP-*).

```rust
pub struct Repository { pub id: RepositoryId, pub organization_id: OrganizationId, pub provider: ProviderKind,
    pub provider_repo_id: String, pub full_name: String, pub default_branch: String,
    pub visibility: Visibility /* public|private|internal */, pub archived: bool,
    pub created_at: DateTime<Utc>, pub updated_at: DateTime<Utc> }
pub enum SnapshotKind { Full, Delta { base: SnapshotId } }           // serde: {"kind":"full"} | {"kind":"delta","base":"…"}
pub enum SnapshotStatus { Building, Ready, Failed, Inconsistent }     // Inconsistent = consistency validator mismatch (ADR-004)
pub struct RepositorySnapshot { pub id: SnapshotId, pub repository_id: RepositoryId, pub commit_sha: CommitSha,
    pub kind: SnapshotKind, pub status: SnapshotStatus, pub provenance: Provenance,
    pub stats: SnapshotStats, pub created_at: DateTime<Utc> }
pub struct SnapshotStats { pub files: u64, pub symbols: u64, pub edges: u64, pub unresolved_refs: u64, pub parse_errors: u64 }
pub enum ParseStatus { Parsed, ParsedWithErrors, Skipped { reason: SkipReason }, Failed }
pub enum SkipReason { Generated, Binary, TooLarge, UnsupportedLanguage, Ignored }
pub struct SourceFile { pub file_version_id: FileVersionId, pub repository_id: RepositoryId, pub path: RepoPath,
    pub content_hash: ContentHash, pub language: Option<Language>, pub size_bytes: u64,
    pub analyzer_version: Option<AnalyzerVersion>, pub parse_status: ParseStatus, pub is_generated: bool }
```

A `Delta` snapshot's base is part of its type, so a delta without a base cannot be represented. That enforces ADR-003 at the type level.

`RepositorySnapshot::new_delta(base: &RepositorySnapshot, …)` returns an error if `base.repository_id != repository_id` (`CoreError::InvalidId { kind: "SnapshotBase", … }`) or if `base.status != Ready`.

**Data model changes:** None. These map onto `repositories` (DOM-009) and onto `snapshots` and `file_versions` (GS-002).

**API/protocol changes:** These wire forms are fixed and exported to contracts later by API tasks if needed:
- `RepoPath` serializes as a plain string.
- `SnapshotKind` uses its tagged form.

**Concurrency semantics:** Immutable value types, `Send + Sync`.

**Failure behavior:** Every constructor validates its input and returns `CoreError`, either `InvalidRepoPath` or `OutOfRange` for ranges. Deserializing an invalid `RepoPath` or range fails, through `#[serde(try_from = "String")]` or a custom `Deserialize`, so invalid values cannot enter through JSON either.

**Idempotency considerations:**
- `ContentHash::of` and `RepoPath::module_path` are pure.
- `SourceFile` identity is `(repository_id, path, content_hash, analyzer_version)` (ADR-003). The method `SourceFile::cache_key()` returns this tuple, so callers can dedupe.

**Security considerations:**
- `RepoPath` validation is the system's path-traversal guard. Every filesystem join in later crates must take a `RepoPath`, never a `&str`. The guard cannot be bypassed through JSON input, because serde validates too.
- `Repository` has no clone URL with credentials and no token fields.

**Observability additions:** None.

**Tests required:**
- `repo_path_rejects_traversal_absolute_backslash_nul_empty_segments`, table-driven with at least 12 cases, including `../x`, `a/../b`, `/etc/passwd`, `C:/x`, `a\\b`, `a//b`, `./a` and `a/.`.
- `repo_path_deserialize_validates`
- `module_path_strips_final_extension_only`
- `content_hash_golden`: the blake3 of `b"hello"` matches the pinned hex.
- `line_range_rejects_zero_and_inverted`
- `source_range_rejects_inverted`
- `snapshot_kind_serde_tagged_form`
- `delta_requires_ready_base_same_repo`
- A proptest, `repo_path_valid_inputs_roundtrip`, generating segments from `[a-zA-Z0-9_.-]+` that are neither `.` nor `..`.

**Benchmarks if applicable:** None.

**Acceptance criteria:**
- `engine/scripts/cargo.sh test -p review-core location repository` passes all the tests listed.
- `engine/scripts/cargo.sh clippy -p review-core -- -D warnings` passes.

**Definition of done:**
- Tests pass.
- The `Position` convention (1-based lines, 0-based byte columns) is documented on the type, and in target-arch §3.1 under the `ParsedUnit` ranges.

---

---

### DOM-005 — PullRequest / ChangedFile / ChangedSymbol / ChangeCluster entities
Status: ☐

**Task ID:** DOM-005

**Title:** Pull-request and change shell entities, populated by later phases

**Problem:** The diff engine (DIFF), the change model (CHG), impact and clustering (IMP), and the control plane all exchange PR and change types. Defining them late would force each producer to invent a local type, and the types would then have to be reconciled.

**Why it exists:**
- PRD §149 (`PullRequest`, `ChangedFile`, `ChangedSymbol`, `ChangeCluster`).
- Target-arch §3.6–3.7.
- `ChangeClusterKey` is needed by `reviewer_runs` (DOM-008/009).

**Scope:**
- Entity shells with their invariants:
  - `PullRequest`, `PrState`
  - `ChangedFile`, `FileChangeStatus`, `Hunk`
  - `ChangedSymbol`, `SymbolChange`
  - `ChangeCluster`, `ChangeClusterKey`

**Explicit non-scope:**
- Hunk line content and the full hunk model (DIFF-001).
- Change classes, the PRD §28 taxonomy (CHG-002).
- `PullRequestChangeModel` aggregation (CHG-001).
- Risk fields (RISK-*).
- Clustering algorithm (IMP-009).

**Files/modules expected to change:** `engine/crates/review-core/src/lib.rs`.

**New files/modules expected:**
- `engine/crates/review-core/src/pull_request.rs`
- `engine/crates/review-core/src/change.rs`

**Dependencies (task IDs):** DOM-004.

**Implementation details:**
```rust
pub enum PrState { Open, Closed, Merged }                                  // "open"|"closed"|"merged"
pub struct PullRequest { pub id: PullRequestId, pub organization_id: OrganizationId, pub repository_id: RepositoryId,
    pub provider_number: u64, pub title: String, pub author_login: String, pub base_ref: String, pub head_ref: String,
    pub base_sha: CommitSha, pub head_sha: CommitSha, pub merge_base_sha: Option<CommitSha>,
    pub state: PrState, pub draft: bool, pub updated_at: DateTime<Utc> }

pub enum FileChangeStatus { Added, Modified, Deleted, Renamed, Copied }
pub struct Hunk { pub old_start: u32, pub old_lines: u32, pub new_start: u32, pub new_lines: u32 }   // unified-diff header semantics
pub struct ChangedFile { pub path: RepoPath, pub old_path: Option<RepoPath>, pub status: FileChangeStatus,
    pub binary: bool, pub hunks: Vec<Hunk> }
impl ChangedFile { pub fn new(...) -> Result<Self, CoreError> }

pub enum SymbolChange { Added, Removed, Modified { signature: bool, body: bool, attrs: bool },
                        Renamed { from: SymbolId, similarity: f32 } }
pub struct ChangedSymbol { pub symbol_key: SymbolKey, pub symbol_id: SymbolId, pub path: RepoPath,
    pub side: DiffSide, pub range: SourceRange, pub change: SymbolChange }

pub struct ChangeClusterKey([u8; 16]);      // hex(32) on the wire
impl ChangeClusterKey { pub fn of(members: &[SymbolKey]) -> Self }
pub struct ChangeCluster { pub key: ChangeClusterKey, pub members: Vec<SymbolKey>, pub module: Option<String> }
```

`ChangedFile::new` enforces these invariants:
- `old_path.is_some()` exactly when the status is `Renamed` or `Copied`.
- `Added` has no hunk with `old_lines > 0`.
- `Deleted` has no hunk with `new_lines > 0`.
- A `binary` file has no hunks.

`SymbolChange::Modified` requires at least one flag to be true. `Renamed.similarity` must be in `[0, 1]` and finite.

`ChangeClusterKey::of(members)` sorts and dedupes the keys, then takes blake3 over the concatenated 16-byte keys, truncated to 16 bytes. The key therefore does not depend on member order.

`ChangeCluster::new` stores the members sorted and deduplicated.

`PullRequest.title` and `author_login` are provider data. They are stored, but they are never interpolated into model prompts without the redaction and escaping done by REV-001.

**Data model changes:** None. These map onto `pull_requests` (DOM-009). `ChangeClusterKey` maps onto `reviewer_runs.cluster_key text` (DOM-009).

**API/protocol changes:** `PrState` and `FileChangeStatus` get stable lowercase wire forms. The legacy `ReviewContext` (`core.rs:357-376`) is not ported; providers normalize into `PullRequest` instead (API-006).

**Concurrency semantics:** Immutable value types, `Send + Sync`.

**Failure behavior:** Invariant violations return `CoreError::OutOfRange` or `CoreError::InvalidId` and never panic.

**Idempotency considerations:** `ChangeClusterKey::of` is deterministic and order-independent. That makes `reviewer_runs` unique on `(review_run_id, reviewer, cluster_key)`, so a retried review stage maps to the same rows.

**Security considerations:** `old_path` and `path` are `RepoPath`, which is traversal-safe even for adversarial rename entries in a malicious PR.

**Observability additions:** None.

**Tests required:**
- `renamed_requires_old_path`
- `modified_forbids_old_path`
- `added_file_has_no_old_side_lines`
- `deleted_file_has_no_new_side_lines`
- `binary_has_no_hunks`
- `symbol_modified_requires_a_flag`
- `rename_similarity_bounds`
- `cluster_key_order_independent`, as a proptest over shuffled member lists.
- `cluster_key_dedupes`
- `pr_state_wire_names`

**Benchmarks if applicable:** None.

**Acceptance criteria:** `engine/scripts/cargo.sh test -p review-core pull_request change` passes all the tests listed.

**Definition of done:**
- Tests pass.
- Each shell type's doc comment names the task that populates it (DIFF-001, CHG-001, IMP-009).

---

---

### DOM-006 — Finding entities and the FindingState lifecycle
Status: ☐

**Task ID:** DOM-006

**Title:** `CandidateFinding` (PRD §49), `VerifiedFinding`, `PublishedFinding`, `FindingState` (PRD §150), `Severity`, `FindingCategory` and `ReviewerType`

**Problem:** The legacy `Finding` (`core.rs:73-104`) has these fields:
- a file and a single line
- a free-text category
- free-text evidence
- a *model-reported* confidence

It has no symbols, no lifecycle and no persisted suppression reason. PRD §49 and §150, and Invariant 2 ("LLM output never directly becomes an external finding"), require a typed candidate → verified → published chain in which every suppression is persisted.

**Why it exists:**
- Gap analysis §G (§49, §150).
- ADR-011: every candidate is persisted with its lifecycle state and suppression reason.
- These types are the contract between reviewers, verification, dedup and the publisher.

**Scope:**
- `Severity`, with a legacy P0–P4 mapping.
- `FindingCategory`, `ReviewerType`, `Confidence`.
- `CandidateFinding`, `FindingArtifact`, `FindingFingerprint` (v1).
- `FindingState`, with an explicit transition table.
- `Suppression` and `SuppressionReason`.
- `VerifiedFinding`, `PublicationBand`, `PublishedFinding`, `Placement`.
- Registering `FindingState`, `Severity`, `FindingCategory`, `ReviewerType`, `CandidateFinding`, `VerifiedFinding` and `PublishedFinding` in the contracts registry (FND-007).

**Explicit non-scope:**
- Verification stages and the confidence formula (VER-*).
- Root-cause dedup and its fingerprint (DED-001), which replaces v1 for cross-reviewer dedup.
- Priority scoring (DED-004).
- Comment rendering (GH-007).

**Files/modules expected to change:**
- `engine/crates/review-core/src/lib.rs`
- `engine/apps/review-cli/src/contracts.rs` (registry)
- `packages/contracts/schemas/*`
- `packages/contracts/src/generated/*` (regenerated)

**New files/modules expected:**
- `engine/crates/review-core/src/finding/mod.rs`
- `engine/crates/review-core/src/finding/severity.rs`
- `engine/crates/review-core/src/finding/state.rs`
- `engine/crates/review-core/src/finding/candidate.rs`
- `engine/crates/review-core/src/finding/verified.rs`
- `engine/crates/review-core/src/finding/published.rs`

**Dependencies (task IDs):** DOM-007 (Evidence), FND-007.

**Implementation details:**

`Severity { Info, Low, Medium, High, Critical }`:
- Serialized as `"info"`, `"low"`, `"medium"`, `"high"`, `"critical"`.
- `Ord` is ascending: `Info < … < Critical`. This means `sev >= Severity::Medium` reads naturally for the §55 band rule.
- The legacy order is the reverse (P0 < P1). The ported tests must flip their comparisons.
- `Severity::from_legacy(&str) -> Option<Severity>` maps P0→Critical, P1→High, P2→Medium, P3→Low, P4→Info. It is used only by the ported policy corpus.
- `Severity::parse` accepts only the five lowercase wire values. `"HIGH"` is rejected, which carries over the legacy §28 defect guard (`core.rs:458-466`).

`ReviewerType { Correctness, Security, Test, Architecture, Performance, Maintainability }`, serialized in snake_case.

`FindingCategory`:
- Variants: `Correctness, Security, Performance, Testing, Architecture, Maintainability, DataIntegrity, Concurrency, ApiContract, ErrorHandling`, serialized in snake_case.
- `primary_reviewer()` maps each category to the reviewer that normally emits it. A reviewer may emit any category.

`Confidence(f32)`: `Confidence::new(v)` requires `v.is_finite() && (0.0..=1.0).contains(&v)`. Serde validates the same way.

```rust
pub struct CandidateFinding {
    pub id: CandidateFindingId, pub organization_id: OrganizationId,
    pub review_run_id: ReviewRunId, pub reviewer_run_id: ReviewerRunId,
    pub category: FindingCategory, pub title: String /* 1..=200 chars */, pub description: String /* ≤ 8000 */,
    pub changed_location: SourceLocation, pub evidence: Vec<Evidence>, pub affected_symbols: Vec<SymbolId>,
    pub severity_candidate: Severity,
    pub confidence_candidate: Option<Confidence>,   // model self-report: INFORMATIONAL ONLY, never a publication input (PRD §54, ADR-011)
    pub reviewer: ReviewerType, pub reasoning_artifacts: Vec<FindingArtifact>,
    pub fingerprint: FindingFingerprint, pub state: FindingState, pub suppression: Option<Suppression>,
    pub created_at: DateTime<Utc>,
}
pub struct FindingArtifact { pub kind: ArtifactKind /* model_rationale|tool_output|graph_query */,
    pub summary: String /* ≤ 2000 chars */, pub content_hash: ContentHash, pub blob_key: Option<String> }
```
A `FindingArtifact` never holds a prompt. Large content lives in the object store under `blob_key`.

`FindingFingerprint(String)` is `"v1:" + hex(blake3("v1\0{reviewer}\0{category}\0{path}\0{lines.start}\0{normalize(title)}")[..16])`, where `normalize` lowercases, collapses whitespace and trims. **Severity is deliberately excluded.** Including it was a legacy defect: `store.rs` fingerprinted the severity label, so a re-rated finding counted as new.

`FindingState`, serialized in SCREAMING_SNAKE:
- `GENERATED`, `EVIDENCE_COLLECTED`, `VERIFIED`, `DEDUPLICATED`, `PRIORITIZED`, `PUBLISHED`
- `SUPPRESSED_LOW_CONFIDENCE`, `SUPPRESSED_DUPLICATE`, `SUPPRESSED_PREEXISTING`, `SUPPRESSED_NOT_ACTIONABLE`, `SUPPRESSED_POLICY`
- `INVALIDATED`

It exposes `FindingState::ALL: [FindingState; 12]`, `as_str`, `is_terminal`, `is_suppressed` and `can_transition_to(self, to) -> bool`, all driven by one `const ALLOWED: &[(FindingState, FindingState)]` table:

| From | Allowed to |
|---|---|
| GENERATED | EVIDENCE_COLLECTED, SUPPRESSED_NOT_ACTIONABLE (stage-1 structural gate or stage-7 failure), SUPPRESSED_PREEXISTING, SUPPRESSED_POLICY, INVALIDATED |
| EVIDENCE_COLLECTED | VERIFIED, SUPPRESSED_LOW_CONFIDENCE, SUPPRESSED_PREEXISTING, SUPPRESSED_NOT_ACTIONABLE, SUPPRESSED_POLICY, INVALIDATED |
| VERIFIED | DEDUPLICATED, SUPPRESSED_DUPLICATE, SUPPRESSED_POLICY, INVALIDATED |
| DEDUPLICATED | PRIORITIZED, SUPPRESSED_POLICY, INVALIDATED |
| PRIORITIZED | PUBLISHED, SUPPRESSED_POLICY (e.g. a PRD §60 comment-cap/policy filter), INVALIDATED |
| PUBLISHED, all SUPPRESSED_*, INVALIDATED | — (terminal) |

`PRIORITIZED` is a legal resting state at the end of a run, for findings in the *internal* band (0.55–0.70) or for findings in 0.70–0.85 below medium severity. The UI shows them, and they are not published.

```rust
pub enum SuppressionReason { LowConfidence { computed: Confidence, threshold: Confidence }, Duplicate { of: CandidateFindingId },
    Preexisting, NotActionable { gate: String }, Policy { rule: String }, Invalidated { superseded_by: Option<ReviewRunId> } }
pub struct Suppression { pub reason: SuppressionReason, pub detail: String /* ≤ 1000 */, pub stage: Option<u8> /* 1..=8 */ }
impl CandidateFinding { pub fn transition(&mut self, to: FindingState, suppression: Option<Suppression>) -> Result<(), CoreError> }
```

`transition` checks the table. It also requires `suppression.is_some()` exactly when `to` is suppressed or `INVALIDATED`, and the reason variant must match the target state. On violation it returns `CoreError::InvalidTransition`.

```rust
pub enum PublicationBand { Suppress, Internal, PublishIfMediumOrAbove, Publish }   // §55 bands; thresholds applied in VER-010
pub struct VerifiedFinding { pub id: VerifiedFindingId, pub candidate_id: CandidateFindingId, pub organization_id: OrganizationId,
    pub computed_confidence: Confidence, pub severity: Severity, pub band: PublicationBand,
    pub verification_version: VerificationVersion, pub stage_outcomes: Vec<StageOutcomeRecord>, pub evidence: Vec<Evidence> }
pub struct StageOutcomeRecord { pub stage: u8, pub outcome: StageOutcome /* pass|fail|inconclusive */, pub reason: Option<String> }
pub enum Placement { Inline, Summary }       // legacy core.rs:164-174 minus `Discarded` (now a suppression state)
pub struct PublishedFinding { pub id: PublishedFindingId, pub verified_finding_id: VerifiedFindingId, pub organization_id: OrganizationId,
    pub placement: Placement, pub location: Option<SourceLocation>, pub head_sha: CommitSha,
    pub provider_review_id: Option<String>, pub provider_comment_id: Option<String>, pub published_at: DateTime<Utc> }
```

`PublishedFinding::new` requires `location.is_some()` exactly when the placement is `Inline`. A `Summary` placement carries an out-of-diff finding, which is moved to the summary and never dropped (legacy `policy.rs:128-137`; INV-015).

**Data model changes:** None in the database. DOM-009 mirrors these enums as CHECK constraints.

**API/protocol changes:** The contracts gain `FindingState`, `Severity`, `FindingCategory`, `ReviewerType`, `CandidateFinding`, `VerifiedFinding` and `PublishedFinding`. Running `pnpm contracts:export && pnpm contracts:generate` updates the TS types.

**Concurrency semantics:** These are value types. Persisted transitions are compare-and-set in SQL (`UPDATE … WHERE id=$1 AND state=$expected`), done in VER/PIPE. This type only validates the edge.

**Failure behavior:** An invalid edge, a missing or extra suppression, an out-of-range confidence, or an over-long text returns `CoreError` and never panics.

**Idempotency considerations:**
- The v1 fingerprint is deterministic, which makes `(reviewer_run_id, fingerprint)` unique in DOM-009. Re-inserting a retried reviewer's candidates is then `ON CONFLICT DO NOTHING`.
- Applying a transition to a state the finding is already in is an error at this layer. The SQL CAS turns duplicates into no-ops.

**Security considerations:**
- The text fields are length-capped, which bounds storage abuse from model output.
- `FindingArtifact` cannot hold a prompt, because there is no such field. Prompts are never persisted in clear text (master plan §13.5).
- `confidence_candidate` is excluded from every decision. A test enforces this by making the decision functions (DOM-010, VER-010) signature-incapable of receiving it.

**Observability additions:** None. The VER/DED tasks emit `candidate_generated` and the `findings_suppressed_total{reason}` metric labelled with `FindingState::as_str`.

**Tests required:**
- `severity_order_ascending`
- `severity_rejects_uppercase_and_legacy_labels`
- `legacy_p_mapping_1_to_1`
- `finding_state_transition_table_exhaustive`: every pair in 12×12 (144) is compared against an independently written expected matrix in the test.
- `terminal_states_have_no_outgoing_edges`
- `every_non_terminal_can_be_invalidated`
- `published_reachable_only_via_prioritized`
- `suppression_required_iff_suppressed_target`
- `suppression_reason_must_match_state`
- `confidence_rejects_nan_and_out_of_range`
- `fingerprint_v1_golden`
- `fingerprint_ignores_severity`
- `fingerprint_normalizes_title_whitespace_and_case`
- `inline_placement_requires_location`
- `contracts_export_contains_finding_state_enum_values`, in `review-cli`: the exported enum equals `FindingState::ALL.map(as_str)`.

**Benchmarks if applicable:** None.

**Acceptance criteria:**
- `engine/scripts/cargo.sh test -p review-core finding` passes all the tests listed.
- `pnpm contracts:check` passes after regeneration.
- `packages/contracts/src/generated/FindingState.ts` contains exactly the 12 literal values.

**Definition of done:**
- Tests pass.
- The contracts are regenerated and committed.
- target-arch §4.3 links to `state.rs` as the authoritative lifecycle table.

---

---

### DOM-007 — Evidence model
Status: ☐

**Task ID:** DOM-007

**Title:** Typed evidence: the 12 PRD §52 kinds, strength, origin, verification status, source locations and symbol references

**Problem:** Legacy evidence was a free-text string, and its only gate was "≥ 40 chars" (`policy.rs:46-142`). PRD §52 requires typed evidence and that "every published finding must have at least one strong evidence source". ADR-011 requires reviewers to cite structured evidence that verification can check: symbol keys, ranges and claimed relations.

**Why it exists:**
- Gap analysis §G/§52 (DOM-007, VER-002).
- Verification stages 3 and 4 check claimed relations and cited ranges, and they need a machine-readable claim to check.

**Scope:**
- `EvidenceKind` (12 variants), `EvidenceStrength`, `EvidenceOrigin`, `EvidenceVerification`.
- `SymbolRef`, `ClaimedRelation`, `RelationClaim`.
- `Evidence`, with `effective_strength()`.
- `has_strong_evidence(&[Evidence])`.

**Explicit non-scope:**
- Gathering and verifying evidence (VER-002..VER-008).
- Mapping `ClaimedRelation` onto codegraph edge kinds (VER-003). `review-core` must not depend on `codegraph`.

**Files/modules expected to change:** `engine/crates/review-core/src/lib.rs`.

**New files/modules expected:** `engine/crates/review-core/src/evidence.rs`.

**Dependencies (task IDs):** DOM-004.

**Implementation details:**
```rust
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind { ChangedSource, CallerPath, CalleePath, TestBehavior, InterfaceContract, Configuration,
    DatabaseSchema, RepositoryConvention, CompilerDiagnostic, LintResult, StaticAnalysisResult, HistoricalRegression }
pub enum EvidenceStrength { Weak, Supporting, Strong }                       // Ord ascending
pub enum EvidenceOrigin { Reviewer { reviewer: ReviewerType }, Deterministic { tool: String }, Graph, Verification { stage: u8 } }
pub enum EvidenceVerification { Unverified, Confirmed { stage: u8 }, Refuted { stage: u8, reason: String } }
pub struct SymbolRef { pub id: SymbolId, pub key: SymbolKey, pub display: String /* ≤ 200 */ }
pub enum ClaimedRelation { Calls, Reaches, Tests, Implements, Extends, Overrides, DependsOn,
    ReadsConfig, ReadsTable, WritesTable, ProducesJob, ConsumesJob, HandlesRoute, GuardedBy }
pub struct RelationClaim { pub from: SymbolRef, pub to: SymbolRef, pub relation: ClaimedRelation,
                           pub via: Vec<SymbolRef> /* claimed path, ≤ 8 hops */ }
pub struct Evidence { pub kind: EvidenceKind, pub claimed_strength: EvidenceStrength, pub origin: EvidenceOrigin,
    pub verification: EvidenceVerification, pub claim: String /* 1..=500 chars */, pub location: Option<SourceLocation>,
    pub symbols: Vec<SymbolRef>, pub relation: Option<RelationClaim> }
```

`EvidenceKind::max_strength()` is `Supporting` for `RepositoryConvention` and `HistoricalRegression`, because neither is proof on its own (the R10 anti-reinforcement stance). It is `Strong` for every other kind.

`Evidence::effective_strength()`:
1. `Refuted` gives `Weak`.
2. Otherwise the strength is capped at `kind.max_strength()`.
3. With origin `Reviewer` and status `Unverified`, the strength is further capped at `Supporting`. Model-claimed evidence is never strong until verification confirms it.
4. `Deterministic`, `Graph` and `Verification` origins keep their capped claimed strength.

`pub fn has_strong_evidence(e: &[Evidence]) -> bool` returns whether any item's `effective_strength() == Strong`.

Construction checks:
- `claim` is 1–500 characters.
- `via.len() <= 8`.
- `kind ∈ {CallerPath, CalleePath}` requires `relation.is_some()`.
- `kind == ChangedSource` requires `location.is_some()`.

The map from `ClaimedRelation` to edge kinds (`Calls` → `CALLS`, `HandlesRoute` → `HANDLED_BY`/`ROUTES_TO`, and so on) is documented on the enum for VER-003 to implement.

**Data model changes:** None. `Evidence` serializes into `candidate_findings.evidence jsonb` and `verified_findings` (DOM-009).

**API/protocol changes:**
- `Evidence` and its sub-types are exported to contracts once DOM-006 registers `CandidateFinding`, since it is referenced from there.
- The reviewer output JSON schema (REV-001) is derived from these types.

**Concurrency semantics:** Value types, `Send + Sync`.

**Failure behavior:** Constructor and serde validation return `CoreError::OutOfRange` for over-long claims or paths. A missing required location or relation returns `CoreError::InvalidId { kind: "Evidence", … }`.

**Idempotency considerations:** `effective_strength` and `has_strong_evidence` are pure. The same evidence always gives the same verdict.

**Security considerations:**
- `claim` and `display` hold model-authored text. They are length-capped and are treated as untrusted when rendered, which GH-007 escapes.
- `Deterministic { tool }` is free text naming a tool. It never contains a command line with secrets; the analysis runner (PIPE-004) supplies a tool name only.

**Observability additions:** None.

**Tests required:**
- `twelve_evidence_kinds_wire_names`, an insta snapshot.
- `reviewer_unverified_capped_at_supporting`
- `confirmed_reviewer_evidence_can_be_strong`
- `refuted_is_weak`
- `convention_and_history_never_strong`
- `deterministic_lint_result_strong`
- `has_strong_evidence_requires_effective_not_claimed`
- `caller_path_requires_relation`
- `changed_source_requires_location`
- `via_path_capped_at_8`
- `claim_length_bounds`
- A proptest, `effective_strength_never_exceeds_claimed_or_kind_cap`.

**Benchmarks if applicable:** None.

**Acceptance criteria:** `engine/scripts/cargo.sh test -p review-core evidence` passes all the tests listed, and insta has no pending snapshots.

**Definition of done:**
- Tests pass.
- ADR-011 links to `evidence.rs`, and its "≥1 strong evidence" rule references `has_strong_evidence`.

---

---

### DOM-008 — ReviewRun / ReviewerRun entities and the ReviewState machine
Status: ☐

**Task ID:** DOM-008

**Title:** `ReviewRun`, `ReviewerRun` and the `ReviewState` enum (PRD §108), with an explicit allowed-transition table and exhaustive tests

**Problem:**
- The legacy machine had 13 states (`core.rs:263-310`). It mixed lifecycle states with outcomes (`Approved`, `ChangesRequested`, `Commented`) and had no transition table, so any state could be assigned at any time.
- PRD §108 defines a new lifecycle, and target-arch §4.1 requires CAS transitions with supersession from every active state.
- PRD §109 requires degraded completion to be recorded.

**Why it exists:**
- Gap analysis §M/§108 (DOM-008, PIPE-007).
- R12 (supersession races). The persisted CAS in PIPE-007 and SUP-003 needs one authoritative table of legal edges.

**Scope:**
- `ReviewState` (13 states) with `ALL`, `as_str`, `is_terminal`, `is_failed`, `is_active`, `can_transition_to` and the `ALLOWED` table.
- `ReviewTrigger`, `RunFailure`, `ReviewRun`, with an `apply_transition` method.
- `ReviewerRunState` and `ReviewerRun`, with model accounting fields (ADR-009).
- Export of `ReviewState`, `ReviewerRunState`, `ErrorClass` and `ReviewerType` to contracts.

**Explicit non-scope:**
- Persisting transitions with SQL CAS (PIPE-007).
- The supersession orchestration (SUP-001..003).
- Stage outputs (PIPE-005).
- Degraded-mode policy decisions (PIPE-008). This task only records them.

**Files/modules expected to change:**
- `engine/crates/review-core/src/lib.rs`
- `engine/apps/review-cli/src/contracts.rs`
- `packages/contracts/*` (regenerated)

**New files/modules expected:**
- `engine/crates/review-core/src/review/mod.rs`
- `engine/crates/review-core/src/review/state.rs`
- `engine/crates/review-core/src/review/run.rs`
- `engine/crates/review-core/src/review/reviewer_run.rs`

**Dependencies (task IDs):** DOM-005 (`ChangeClusterKey`), DOM-006 (`ReviewerType`).

**Implementation details:**

`ReviewState`, serialized in SCREAMING_SNAKE:
- `RECEIVED`, `INDEXING`, `ANALYZING`, `REVIEWING`, `VERIFYING`, `PUBLISHING`, `COMPLETED`
- `FAILED_INDEXING`, `FAILED_ANALYSIS`, `FAILED_REVIEW`, `FAILED_PUBLISH`
- `SUPERSEDED`, `CANCELLED`

`const ALLOWED: &[(ReviewState, ReviewState)]`:

| From | To |
|---|---|
| RECEIVED | INDEXING, SUPERSEDED, CANCELLED |
| INDEXING | ANALYZING, FAILED_INDEXING, SUPERSEDED, CANCELLED |
| ANALYZING | REVIEWING, PUBLISHING (no applicable reviewers: a summary-only publication, recorded as such, never a silent skip), FAILED_ANALYSIS, SUPERSEDED, CANCELLED |
| REVIEWING | VERIFYING, FAILED_REVIEW, SUPERSEDED, CANCELLED |
| VERIFYING | PUBLISHING, FAILED_REVIEW (verification is part of review), SUPERSEDED, CANCELLED |
| PUBLISHING | COMPLETED, FAILED_PUBLISH, SUPERSEDED, CANCELLED |
| COMPLETED, FAILED_*, SUPERSEDED, CANCELLED | — (terminal) |

Further decisions:
- **`FAILED_*` is terminal.** A manual retry creates a *new* run with `retry_of = Some(old)`. This keeps an audit trail and keeps the CAS simple.
- `SUPERSEDED` always carries `superseded_by: ReviewRunId`.
- `CANCELLED` covers a closed PR, a manual cancel and shutdown abandonment.

```rust
pub enum ReviewTrigger { Webhook, Manual, Reconciler, Cli }
pub struct RunFailure { pub class: ErrorClass, pub detail: String /* ≤ 2000, no source/secrets */ }
pub struct ReviewRun { pub id: ReviewRunId, pub organization_id: OrganizationId, pub repository_id: RepositoryId,
    pub pull_request_id: PullRequestId, pub base_sha: CommitSha, pub head_sha: CommitSha,
    pub merge_base_sha: Option<CommitSha>, pub state: ReviewState, pub trigger: ReviewTrigger,
    pub superseded_by: Option<ReviewRunId>, pub retry_of: Option<ReviewRunId>, pub failure: Option<RunFailure>,
    pub degraded_reviewers: Vec<ReviewerType>,               // PRD §109: missing reviewers recorded, sorted+dedup
    pub provenance: Option<Provenance>, pub trace_parent: Option<String>,
    pub created_at: DateTime<Utc>, pub updated_at: DateTime<Utc>, pub completed_at: Option<DateTime<Utc>> }
pub enum TransitionInput { Advance, Fail(RunFailure), Supersede { by: ReviewRunId }, Cancel }
impl ReviewRun { pub fn apply_transition(&mut self, to: ReviewState, input: TransitionInput, at: DateTime<Utc>) -> Result<(), CoreError> }
```

`apply_transition` validates the following, then sets `state`, `updated_at`, and `completed_at` when `to.is_terminal()`:
- The edge is in `ALLOWED`.
- `to.is_failed()` requires `Fail`.
- `to == SUPERSEDED` requires `Supersede { by }` with `by != self.id`.
- `to == CANCELLED` requires `Cancel`.
- Every other target requires `Advance`.

```rust
pub enum ReviewerRunState { Pending, Running, Succeeded, Failed, Skipped, TimedOut }   // snake_case
pub struct ReviewerRun { pub id: ReviewerRunId, pub organization_id: OrganizationId, pub review_run_id: ReviewRunId,
    pub reviewer: ReviewerType, pub cluster_key: Option<ChangeClusterKey>, pub state: ReviewerRunState,
    pub provider: Option<String>, pub model: Option<String>, pub prompt_version: Option<PromptVersion>,
    pub reviewer_version: Option<ReviewerVersion>, pub usage: TokenUsage, pub cost_usd_micros: u64,
    pub latency_ms: Option<u32>, pub error_class: Option<ErrorClass>,
    pub started_at: Option<DateTime<Utc>>, pub finished_at: Option<DateTime<Utc>> }
pub struct TokenUsage { pub input: u64, pub output: u64, pub cached_read: u64, pub cached_write: u64 }
```

`ReviewerRunState` transitions:
- `Pending → Running | Skipped`
- `Running → Succeeded | Failed | TimedOut`
- the rest are terminal

`error_class.is_some()` exactly when the state is `Failed` or `TimedOut`.

**Data model changes:** None in the database. DOM-009 mirrors the `as_str` values as CHECK constraints.

**API/protocol changes:** The contracts gain `ReviewState`, `ReviewerRunState`, `ReviewTrigger` and `ErrorClass`. The NestJS reviews module (API-010) consumes the generated union types and never hand-writes them.

**Concurrency semantics:**
- This type is the *validator* only. Persisted transitions use `UPDATE review_runs SET state=$to, updated_at=now() WHERE id=$1 AND organization_id=$2 AND state=$from RETURNING *`. Zero rows means the CAS was lost, which maps to `ErrorClass::Conflict` (DOM-002), so the stage aborts without side effects.
- Because every active state has an edge to `SUPERSEDED`, supersession can win against any stage. A stage that loses the CAS must not publish. SUP-003 enforces this at publish time inside the publish transaction.

**Failure behavior:** An illegal edge or a mismatched input returns `CoreError::InvalidTransition { entity: "ReviewRun", from, to }`, which is class Conflict. It never panics.

**Idempotency considerations:**
- A transition to the current state is rejected here. PIPE-007's CAS makes a duplicate delivery a zero-row update, which is treated as "already done" only when the observed state is at or after the target. That rule is documented for PIPE-007.
- `degraded_reviewers` is kept sorted and deduplicated, so recording the same missing reviewer twice is idempotent.

**Security considerations:**
- `RunFailure.detail` is capped and documented as "no source, prompts or tokens". It is shown in the UI.
- `trace_parent` is a W3C traceparent string, validated with `^[0-9a-f]{2}-[0-9a-f]{32}-[0-9a-f]{16}-[0-9a-f]{2}$`, so arbitrary header injection cannot be stored.

**Observability additions:**
- It defines the metric label values `state` = `ReviewState::as_str`, used by OBS-005 for `review_runs_total{state}` and `review_state_transitions_total{from,to}`.
- It defines the span attribute `review.state`.

**Tests required:**
- `review_state_transition_table_exhaustive`: every pair in 13×13 (169) is checked against an independent expected matrix in the test.
- `terminal_states_have_no_outgoing_edges`
- `every_active_state_can_be_superseded_and_cancelled`
- `every_state_reachable_from_received`, by BFS.
- `completed_reachable_only_via_publishing`
- `each_active_stage_maps_to_its_failure_state`
- `failed_requires_failure_input`
- `superseded_requires_other_run_id`
- `completed_at_set_on_terminal`
- `degraded_reviewers_sorted_dedup`
- `reviewer_run_error_class_iff_failed_or_timed_out`
- `trace_parent_validated`
- `review_state_wire_names_snapshot`
- A proptest, `random_walk_respects_table`: any sequence of random targets either errors or follows `ALLOWED`, and never leaves a terminal state.
- `contracts_export_contains_review_state_values`

**Benchmarks if applicable:** None.

**Acceptance criteria:**
- `engine/scripts/cargo.sh test -p review-core review` passes all the tests listed.
- `pnpm contracts:check` is green.
- `packages/contracts/src/generated/ReviewState.ts` has exactly 13 literals.

**Definition of done:**
- Tests pass.
- target-arch §4.1 states that `FAILED_*` is terminal (retry creates a new run) and that the ANALYZING→PUBLISHING edge exists, with a link to `state.rs`.

---

---

### DOM-009 — Initial PostgreSQL migrations
Status: ☐

**Task ID:** DOM-009

**Title:** Initial PostgreSQL schema (`engine/migrations`): tenancy, repositories, PRs, review runs, findings, feedback and webhook deliveries

**Problem:**
- The legacy store ran DDL on every connect through a `psql` subprocess. It had five ad-hoc tables, no FKs, no migrations, and interpolated SQL (`store.rs:55-176`, `store.rs:179-181`).
- The control plane (API-*), the pipeline (PIPE-*) and the publisher (GH-*) all need one migrated schema, with `organization_id` on every tenant table (master plan §13.1, R9).

**Why it exists:**
- ADR-014: `engine/migrations` is the only schema source for both languages.
- ADR-002: Kysely types are generated from it.
- M1 exit: "migrations apply".

**Scope:** Five forward-only sqlx migrations creating the tables below, plus:
- the `updated_at` trigger function and a trigger on every table
- the indexes
- composite tenant FKs
- CHECK constraints mirroring the Rust enums
- `review-worker migrate`
- an integration-test suite

**Explicit non-scope:**
- Graph tables: `file_versions`, `symbols`, `snapshots` and the rest (GS-002..005).
- The `jobs` table (PIPE-001).
- `stage_outputs` (PIPE-005).
- RLS policies (SEC-001/API-003). Only the columns they need are provided here.
- Kysely codegen (API-002).
- Retention jobs (SEC-007).

**Files/modules expected to change:**
- `engine/apps/review-worker/Cargo.toml`, adding `sqlx`, `tokio`, `clap`, `review-core`, and the `[features] integration = []` flag
- `engine/apps/review-worker/src/main.rs`

**New files/modules expected:**
- `engine/migrations/20261002000001_foundation.sql`
- `engine/migrations/20261002000002_repositories_pull_requests.sql`
- `engine/migrations/20261002000003_review_runs.sql`
- `engine/migrations/20261002000004_findings.sql`
- `engine/migrations/20261002000005_webhook_deliveries.sql`
- `engine/apps/review-worker/src/migrate.rs`
- `engine/apps/review-worker/tests/migrations.rs`

**Dependencies (task IDs):** DOM-006, DOM-008 (enum values), FND-005 (Postgres), FND-006 (`migrate` / `test:integration`).

**Implementation details:**

Conventions:
- Enums are stored as `text` + `CHECK`, not PG `ENUM`. Expand/contract changes are a single `ALTER TABLE … DROP/ADD CONSTRAINT`, with no `ALTER TYPE` lock.
- Timestamps are `timestamptz NOT NULL DEFAULT now()`.
- IDs are `uuid PRIMARY KEY DEFAULT gen_random_uuid()`. The app normally supplies v7 UUIDs (DOM-001).
- Every tenant table has `organization_id uuid NOT NULL` and `UNIQUE (id, organization_id)`. Children reference parents with **composite** FKs `(parent_id, organization_id)`, so a row can never point at another tenant's parent.
- Migrations are forward-only. Dev resets use `pnpm dev:reset`.

`…0001_foundation.sql`:
```sql
CREATE FUNCTION rg_set_updated_at() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN NEW.updated_at := now(); RETURN NEW; END $$;

CREATE TABLE organizations (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  slug text NOT NULL UNIQUE CHECK (slug ~ '^[a-z0-9][a-z0-9-]{0,62}$'),
  display_name text NOT NULL CHECK (length(display_name) BETWEEN 1 AND 200),
  created_at timestamptz NOT NULL DEFAULT now(), updated_at timestamptz NOT NULL DEFAULT now());
CREATE TABLE users (                                   -- global identity (a user may belong to many orgs): not a tenant table
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  provider text NOT NULL CHECK (provider IN ('github')), provider_user_id text NOT NULL,
  login text NOT NULL, display_name text, email text,
  created_at timestamptz NOT NULL DEFAULT now(), updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (provider, provider_user_id));
CREATE TABLE memberships (
  organization_id uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  role text NOT NULL CHECK (role IN ('owner','admin','member','viewer')),
  created_at timestamptz NOT NULL DEFAULT now(), updated_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (organization_id, user_id));
CREATE INDEX memberships_user_idx ON memberships (user_id);
CREATE TABLE provider_installations (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  organization_id uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  provider text NOT NULL CHECK (provider IN ('github')), provider_installation_id bigint NOT NULL,
  account_login text NOT NULL, account_type text NOT NULL CHECK (account_type IN ('organization','user')),
  permissions jsonb NOT NULL DEFAULT '{}'::jsonb, suspended_at timestamptz,   -- NO token columns, ever (§13.2)
  created_at timestamptz NOT NULL DEFAULT now(), updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (provider, provider_installation_id), UNIQUE (id, organization_id));
CREATE INDEX provider_installations_org_idx ON provider_installations (organization_id);
-- + CREATE TRIGGER <table>_set_updated_at BEFORE UPDATE ON <table> FOR EACH ROW EXECUTE FUNCTION rg_set_updated_at(); for each table
```

`…0002_repositories_pull_requests.sql`:
```sql
CREATE TABLE repositories (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(), organization_id uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  installation_id uuid NOT NULL, provider text NOT NULL CHECK (provider IN ('github')), provider_repo_id text NOT NULL,
  full_name text NOT NULL, default_branch text NOT NULL,
  visibility text NOT NULL CHECK (visibility IN ('public','private','internal')), archived boolean NOT NULL DEFAULT false,
  settings jsonb NOT NULL DEFAULT '{}'::jsonb, created_at timestamptz NOT NULL DEFAULT now(), updated_at timestamptz NOT NULL DEFAULT now(),
  FOREIGN KEY (installation_id, organization_id) REFERENCES provider_installations (id, organization_id) ON DELETE CASCADE,
  UNIQUE (organization_id, provider, provider_repo_id), UNIQUE (id, organization_id));
CREATE TABLE pull_requests (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(), organization_id uuid NOT NULL, repository_id uuid NOT NULL,
  provider_number integer NOT NULL CHECK (provider_number > 0), title text NOT NULL, author_login text NOT NULL,
  base_ref text NOT NULL, head_ref text NOT NULL,
  base_sha text NOT NULL CHECK (base_sha ~ '^([0-9a-f]{40}|[0-9a-f]{64})$'),
  head_sha text NOT NULL CHECK (head_sha ~ '^([0-9a-f]{40}|[0-9a-f]{64})$'),
  merge_base_sha text CHECK (merge_base_sha ~ '^([0-9a-f]{40}|[0-9a-f]{64})$'),
  state text NOT NULL CHECK (state IN ('open','closed','merged')), draft boolean NOT NULL DEFAULT false,
  created_at timestamptz NOT NULL DEFAULT now(), updated_at timestamptz NOT NULL DEFAULT now(),
  FOREIGN KEY (repository_id, organization_id) REFERENCES repositories (id, organization_id) ON DELETE CASCADE,
  UNIQUE (repository_id, provider_number), UNIQUE (id, organization_id));
CREATE INDEX pull_requests_org_repo_state_idx ON pull_requests (organization_id, repository_id, state);
```

`…0003_review_runs.sql`:
```sql
CREATE TABLE review_runs (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(), organization_id uuid NOT NULL, repository_id uuid NOT NULL,
  pull_request_id uuid NOT NULL, base_sha text NOT NULL, head_sha text NOT NULL, merge_base_sha text,
  state text NOT NULL CHECK (state IN ('RECEIVED','INDEXING','ANALYZING','REVIEWING','VERIFYING','PUBLISHING','COMPLETED',
        'FAILED_INDEXING','FAILED_ANALYSIS','FAILED_REVIEW','FAILED_PUBLISH','SUPERSEDED','CANCELLED')),
  trigger text NOT NULL CHECK (trigger IN ('webhook','manual','reconciler','cli')),
  superseded_by uuid REFERENCES review_runs(id) ON DELETE SET NULL,
  retry_of uuid REFERENCES review_runs(id) ON DELETE SET NULL,
  failure_class text CHECK (failure_class IN ('invalid_input','not_found','conflict','transient','rate_limited','permanent','cancelled','internal')),
  failure_detail text CHECK (length(failure_detail) <= 2000),
  degraded_reviewers text[] NOT NULL DEFAULT '{}', provenance jsonb NOT NULL DEFAULT '{}'::jsonb,
  trace_parent text CHECK (trace_parent ~ '^[0-9a-f]{2}-[0-9a-f]{32}-[0-9a-f]{16}-[0-9a-f]{2}$'),
  created_at timestamptz NOT NULL DEFAULT now(), updated_at timestamptz NOT NULL DEFAULT now(), completed_at timestamptz,
  FOREIGN KEY (pull_request_id, organization_id) REFERENCES pull_requests (id, organization_id) ON DELETE CASCADE,
  FOREIGN KEY (repository_id, organization_id) REFERENCES repositories (id, organization_id) ON DELETE CASCADE,
  UNIQUE (id, organization_id),
  CHECK ((state LIKE 'FAILED_%') = (failure_class IS NOT NULL)),
  CHECK ((state = 'SUPERSEDED') = (superseded_by IS NOT NULL)));
CREATE UNIQUE INDEX review_runs_one_active_per_pr ON review_runs (pull_request_id)
  WHERE state IN ('RECEIVED','INDEXING','ANALYZING','REVIEWING','VERIFYING','PUBLISHING');
CREATE UNIQUE INDEX review_runs_first_run_per_head ON review_runs (pull_request_id, head_sha) WHERE retry_of IS NULL;
CREATE INDEX review_runs_org_repo_created_idx ON review_runs (organization_id, repository_id, created_at DESC);
CREATE TABLE reviewer_runs (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(), organization_id uuid NOT NULL, review_run_id uuid NOT NULL,
  reviewer text NOT NULL CHECK (reviewer IN ('correctness','security','test','architecture','performance','maintainability')),
  cluster_key text CHECK (cluster_key ~ '^[0-9a-f]{32}$'),
  state text NOT NULL CHECK (state IN ('pending','running','succeeded','failed','skipped','timed_out')),
  provider text, model text, prompt_version text, reviewer_version text,
  input_tokens bigint NOT NULL DEFAULT 0, output_tokens bigint NOT NULL DEFAULT 0,
  cached_read_tokens bigint NOT NULL DEFAULT 0, cached_write_tokens bigint NOT NULL DEFAULT 0,
  cost_usd_micros bigint NOT NULL DEFAULT 0, latency_ms integer, error_class text,
  started_at timestamptz, finished_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(), updated_at timestamptz NOT NULL DEFAULT now(),
  FOREIGN KEY (review_run_id, organization_id) REFERENCES review_runs (id, organization_id) ON DELETE CASCADE,
  UNIQUE NULLS NOT DISTINCT (review_run_id, reviewer, cluster_key), UNIQUE (id, organization_id),
  CHECK ((state IN ('failed','timed_out')) = (error_class IS NOT NULL)));
```
`review_runs_one_active_per_pr` enforces supersession at the database level. A new head's run can only be inserted in the same transaction that moves the old active run to `SUPERSEDED` (SUP-001).

`…0004_findings.sql`. All three finding tables, plus `finding_feedback`, have `organization_id`, a composite FK to their parent, `UNIQUE (id, organization_id)` and an `updated_at` trigger.

`candidate_findings`:
- Columns:
  - `review_run_id`, `reviewer_run_id`
  - `reviewer`, `category`, both CHECKed to their enum values
  - `title`, CHECK length 1–200
  - `description`, CHECK ≤ 8000
  - `changed_path`, `changed_side` (CHECK `head|base`), `changed_start_line` / `changed_end_line` (CHECK 1 ≤ start ≤ end)
  - `severity_candidate` CHECK
  - `confidence_candidate real` CHECK 0..1
  - `affected_symbols text[]`
  - `evidence jsonb NOT NULL DEFAULT '[]'`
  - `reasoning_artifacts jsonb NOT NULL DEFAULT '[]'`
  - `fingerprint text NOT NULL CHECK (fingerprint ~ '^v1:[0-9a-f]{32}$')`
  - `state` CHECK, with the 12 `FindingState` values
  - `suppression jsonb`, `suppressed_at_stage smallint` CHECK 1..8
- Constraints and indexes:
  - `CHECK ((state LIKE 'SUPPRESSED_%' OR state = 'INVALIDATED') = (suppression IS NOT NULL))`
  - `UNIQUE (reviewer_run_id, fingerprint)`
  - `INDEX (review_run_id, state)`

`verified_findings`:
- `candidate_finding_id`, UNIQUE, with a composite FK.
- `review_run_id`.
- `computed_confidence real NOT NULL` CHECK 0..1.
- `severity` CHECK.
- `band` CHECK in `suppress|internal|publish_if_medium_or_above|publish`.
- `verification_version integer NOT NULL`.
- `stage_outcomes jsonb NOT NULL`.
- `evidence jsonb NOT NULL`.
- `priority_score real`.

`published_findings`:
- `verified_finding_id`, UNIQUE, with a composite FK.
- `review_run_id`, `pull_request_id`.
- `provider` CHECK.
- `placement` CHECK `inline|summary`.
- `path text`, `start_line int`, `end_line int`.
- `head_sha` CHECK.
- `provider_review_id text`, `provider_comment_id text`.
- `published_at timestamptz NOT NULL`.
- `CHECK ((placement = 'inline') = (path IS NOT NULL AND start_line IS NOT NULL))`.
- `UNIQUE (provider, provider_comment_id)`, which only constrains non-null values, as PG's default NULL semantics do.

`finding_feedback`:
- `published_finding_id`, with a composite FK.
- `user_id uuid REFERENCES users(id) ON DELETE SET NULL`.
- `source` CHECK in `web|provider_reaction|provider_reply|cli`.
- `verdict` CHECK in `useful|false_positive|already_handled|not_relevant|intentional` (PRD §70).
- `comment text` CHECK ≤ 2000.
- `UNIQUE (published_finding_id, user_id)`, so a user's verdict is upserted.

`…0005_webhook_deliveries.sql`:
```sql
CREATE TABLE webhook_deliveries (            -- ingress log; tenant resolved after normalization ⇒ organization_id NULLABLE by design
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  provider text NOT NULL CHECK (provider IN ('github')), delivery_id text NOT NULL,
  event text NOT NULL, action text, organization_id uuid REFERENCES organizations(id) ON DELETE CASCADE,
  provider_installation_id bigint, payload_sha256 text NOT NULL CHECK (payload_sha256 ~ '^[0-9a-f]{64}$'),
  signature_valid boolean NOT NULL,
  status text NOT NULL CHECK (status IN ('received','processed','ignored','rejected','failed')),
  error text CHECK (length(error) <= 2000), received_at timestamptz NOT NULL DEFAULT now(), processed_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(), updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (provider, delivery_id));          -- durable dedup backing the Redis SETNX fast path (target-arch §5)
CREATE INDEX webhook_deliveries_org_received_idx ON webhook_deliveries (organization_id, received_at DESC);
```
Raw payloads are **not** stored, only their hash. This minimizes retained provider data. GH tasks may add retention-bounded storage through an ADR if they need it.

`review-worker migrate`: `clap` subcommand `migrate`, reading `DATABASE_URL`, with `sqlx::migrate!("../../migrations").run(&pool).await`. sqlx takes a PG advisory lock and verifies checksums.

Tests use `#[sqlx::test(migrations = "../../migrations")]`, which gives a fresh database per test, gated by `#![cfg(feature = "integration")]`.

**Data model changes:** 13 tables:
- organizations
- users
- memberships
- provider_installations
- repositories
- pull_requests
- review_runs
- reviewer_runs
- candidate_findings
- verified_findings
- published_findings
- finding_feedback
- webhook_deliveries

Plus the function `rg_set_updated_at()` and the `<table>_set_updated_at` triggers.

**API/protocol changes:**
- Adds the `review-worker migrate` CLI.
- The schema becomes the source for Kysely codegen (API-002).

**Concurrency semantics:**
- Concurrent `migrate` runs serialize on sqlx's advisory lock. The second sees the migrations already applied and does nothing.
- The partial unique index makes "one active run per PR" race-free under concurrent webhook deliveries. The loser gets a unique violation, which maps to `ErrorClass::Conflict`.
- `UNIQUE (provider, delivery_id)` makes duplicate deliveries race-free.

**Failure behavior:**
- A migration error aborts in its transaction, and sqlx runs each migration in a transaction. The `_sqlx_migrations` table is not advanced past it.
- A checksum mismatch on an applied migration fails startup with a clear error. Applied migrations are never edited; fixes are new migrations.

**Idempotency considerations:**
- Re-running `migrate` is a no-op.
- `review_runs_first_run_per_head` prevents duplicate runs for the same head from replayed events.
- `UNIQUE (reviewer_run_id, fingerprint)` makes candidate re-insertion on retry idempotent.
- `webhook_deliveries` dedups by delivery ID.

**Security considerations:**
- Composite tenant FKs make a cross-tenant reference impossible at the database level, which is defence in depth under RLS (SEC-001).
- There are no credential or token columns anywhere. A test asserts that no column name matches `token|secret|password|private_key`.
- Every value that later reaches SQL from the app is bound, never interpolated, which fixes legacy `store.rs:179-181`.
- `webhook_deliveries.organization_id` is nullable only for pre-resolution ingress, and SEC-001 restricts that table to the service role.

**Observability additions:**
- The `migrate` subcommand logs each applied version through `tracing` (`migration_applied{version}`) and emits a `db_migrate` span.
- Full OTel wiring comes in OBS-001.

**Tests required** (in `apps/review-worker/tests/migrations.rs`, feature `integration`):
- `migrations_apply_on_empty_db`
- `migrate_twice_is_noop`
- `every_tenant_table_has_organization_id`: queries `information_schema.columns` for every table except `users`.
- `webhook_deliveries_org_nullable_by_design`
- `every_table_has_updated_at_trigger`: queries `pg_trigger`.
- `updated_at_trigger_bumps_timestamp`
- `cross_tenant_fk_rejected`: a PR whose `organization_id` is B but whose repository belongs to A fails with an FK violation.
- `review_state_check_matches_rust_enum`: inserting each `ReviewState::ALL.as_str()` succeeds, and inserting `'APPROVED'` fails.
- `finding_state_check_matches_rust_enum`
- `severity_check_matches_rust_enum`
- `error_class_check_matches_rust_enum`
- `one_active_run_per_pr_enforced`
- `superseded_requires_superseded_by`
- `failed_requires_failure_class`
- `suppressed_requires_suppression`
- `inline_published_requires_location`
- `duplicate_webhook_delivery_rejected`
- `candidate_reinsert_same_fingerprint_conflicts`
- `no_credential_columns`
- `uuid_ids_decode_via_try_from`: a row struct with `#[sqlx(try_from = "Uuid")] id: ReviewRunId` round-trips. This demonstrates the DOM-001 pattern.

**Benchmarks if applicable:** None.

**Acceptance criteria:**
- With the dev stack up, `pnpm migrate` exits 0, and `pnpm migrate:info` lists five applied versions.
- `pnpm test:integration` exits 0 with every test listed above passing.
- `engine/scripts/cargo.sh run -q -p review-worker -- migrate`, with the network joined and `DATABASE_URL` set, exits 0 and is a no-op on the second run.
- `psql … -c '\dt'` lists exactly the 13 tables plus `_sqlx_migrations`.

**Definition of done:**
- Migrations are committed.
- The integration tests pass in the test compose.
- `docs/graph-schema/` is not touched; that is GS.
- A schema overview table (table → owner task → tenant column) is added to target-arch §5 or linked from it.

---

---

### DOM-010 — Port the legacy fail-safe publication decision
Status: ☐

**Task ID:** DOM-010

**Title:** Fail-safe publication decision in `review-core`: `ReviewEvent::Comment` only, never APPROVE, never merge

**Problem:**
- The legacy `decision_for` (the legacy prototype rule (audit §8), the `Decision` enum plus the choke-point function) is the single place a review state becomes a GitHub event. Its tests (`core.rs:407-456`, `only_approved_state_can_approve` and `quota_failure_posts_nothing`) encode "failure is never approval".
- The new system replaces the 13-state legacy machine with DOM-008's `ReviewState`.
- Gap analysis §P decides that the MVP publishes a **COMMENT** review plus a check run with conclusion `neutral` (findings exist) or `success` (none), and never approves.
- Without a port, the publisher (GH-009, in TypeScript) would choose the event itself.

**Why it exists:**
- Invariants INV-011 (never merge) and INV-012 (failure is never approval), from gap analysis §P.
- Master plan §4 principle 10.
- Legacy audit §1.4 lists the choke point as KEEP.

**Scope:**
- `ReviewEvent`, with exactly one variant, `Comment`.
- `CheckConclusion { Success, Neutral }`.
- `Coverage { Complete, Degraded }`.
- `PublicationInput` and `PublicationDecision`.
- `publication_decision(state, &input) -> Option<PublicationDecision>`.
- Exhaustive tests, including the INV-011/INV-012 tests.
- Exporting `ReviewEvent` and `CheckConclusion` to contracts, so the TS side's type is literally `'COMMENT'`.

**Explicit non-scope:**
- Posting to GitHub (GH-009).
- The TypeScript no-merge permission-manifest test (GH-010).
- The cross-repo INV-011 source-scan test (INV task in P30-P37).
- Configurable blocking (`REQUEST_CHANGES`), which is post-MVP and needs a new ADR.

**Files/modules expected to change:**
- `engine/crates/review-core/src/lib.rs`
- `engine/apps/review-cli/src/contracts.rs`
- `packages/contracts/*` (regenerated)

**New files/modules expected:** `engine/crates/review-core/src/publication.rs`.

**Dependencies (task IDs):** DOM-008, FND-007.

**Implementation details:**
```rust
/// The ONLY provider review events ReviewGraph can express. There is deliberately no Approve,
/// RequestChanges or Merge variant — absent, not guarded (legacy core.rs:316-317, github.rs:5-6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReviewEvent { Comment }
impl ReviewEvent { pub const fn as_provider_event(self) -> &'static str { match self { ReviewEvent::Comment => "COMMENT" } } }

#[serde(rename_all = "snake_case")] pub enum CheckConclusion { Success, Neutral }   // never "failure" in MVP
pub enum Coverage { Complete, Degraded }
pub struct PublicationInput { pub publishable_findings: u32, pub coverage: Coverage }
pub struct PublicationDecision { pub event: ReviewEvent, pub check_conclusion: CheckConclusion }

/// Port of legacy `decision_for` (core.rs:341-349). Called by the publisher while the run is PUBLISHING,
/// inside the publish transaction (SUP-003). Every other state — in-flight, COMPLETED (already published),
/// every FAILED_*, SUPERSEDED, CANCELLED — returns None, which callers MUST treat as "post nothing".
pub fn publication_decision(state: ReviewState, input: &PublicationInput) -> Option<PublicationDecision> {
    match state {
        ReviewState::Publishing => Some(PublicationDecision {
            event: ReviewEvent::Comment,
            check_conclusion: match (input.publishable_findings, input.coverage) {
                (0, Coverage::Complete) => CheckConclusion::Success,
                _ => CheckConclusion::Neutral,          // findings, or degraded coverage: never a green "pass" on partial review
            },
        }),
        ReviewState::Received | ReviewState::Indexing | ReviewState::Analyzing | ReviewState::Reviewing
        | ReviewState::Verifying | ReviewState::Completed | ReviewState::FailedIndexing | ReviewState::FailedAnalysis
        | ReviewState::FailedReview | ReviewState::FailedPublish | ReviewState::Superseded | ReviewState::Cancelled => None,
    }
}
```

The match deliberately has **no wildcard arm**. Adding a `ReviewState` variant then fails to compile until someone decides what it maps to, which carries over the legacy test's intent (`core.rs:410-414`).

`PublicationInput` intentionally has no field for model confidence or model "approval" text. Its signature makes it impossible to feed self-reported completeness into the decision (INV-013).

Legacy-to-new mapping, recorded in the module doc:

| Legacy | New |
|---|---|
| `Approved → APPROVE` | removed |
| `ChangesRequested → REQUEST_CHANGES` | removed in the MVP |
| `Commented → COMMENT` | `Publishing → Comment` |
| `Failed{..}` / `Cancelled` / `Skipped` / in-flight → `None` | every non-PUBLISHING state → `None` |

**Data model changes:** None.

**API/protocol changes:** The contracts gain:
- `ReviewEvent`, whose JSON Schema enum is `["COMMENT"]`. The generated TS is `export type ReviewEvent = "COMMENT";`.
- `CheckConclusion`, with the enum `["success","neutral"]`.

GH-009 must type its Octokit call's `event` parameter as `ReviewEvent`.

**Concurrency semantics:**
- The function is pure.
- The race between supersession and publication is handled by the caller. The publisher reads the run state **inside** the publish transaction with `SELECT … FOR UPDATE`, calls `publication_decision`, and posts only on `Some`. A concurrent SUPERSEDED commit therefore yields `None` (SUP-003, R12).

**Failure behavior:**
- `None` is the fail-safe output. Callers must post nothing: no review, no comment and no check-run conclusion of success.
- A failed run's check run, if one exists, is finalized by GH tasks as `neutral` with an error summary, never `success`. This rule is documented here for GH-009.

**Idempotency considerations:**
- The function is deterministic.
- Once a run is `COMPLETED`, it returns `None`, so a replayed publish job cannot post a second review for the same run.

**Security considerations:**
- This is the system's structural guarantee that it can never approve or merge. No variant exists that could encode APPROVE, REQUEST_CHANGES or a merge, in either Rust or the generated TS.
- Deserializing `"APPROVE"` into `ReviewEvent` fails.

**Observability additions:** None in this crate. GH-009 emits the `publication` span with the attributes `review.event` (= `as_provider_event`) and `check.conclusion`, plus the counter `publication_decisions_total{decision="comment"|"none", state}`.

**Tests required:**
- `inv_011_review_event_cannot_represent_approve_or_merge`: an exhaustive `match` with no wildcard over `ReviewEvent` lists only `Comment`; `serde_json::from_str::<ReviewEvent>("\"APPROVE\"")`, `"\"REQUEST_CHANGES\""` and `"\"MERGE\""` all fail; and the exported JSON Schema enum equals `["COMMENT"]`.
- `inv_012_failure_states_publish_nothing`: for every `ReviewState` in `ALL` except `Publishing`, with every combination of `publishable_findings ∈ {0, 1, 50}` × `Coverage`, the result is `None`. Ported from legacy `only_approved_state_can_approve`.
- `inv_012_quota_style_failure_posts_nothing`: `FailedReview` with a `RunFailure` of class `RateLimited` gives `None`. Ported from legacy `quota_failure_posts_nothing`.
- `inv_012_degraded_run_never_reports_success`: `Publishing` with 0 findings and `Degraded` gives `Neutral`.
- `completed_run_cannot_republish`
- `publishing_with_findings_is_neutral_comment`
- `publishing_clean_complete_is_success_comment`
- `contracts_review_event_ts_is_comment_literal`: the generated `ReviewEvent.ts` contains `"COMMENT"` and none of `APPROVE`, `REQUEST_CHANGES` or `MERGE`. This runs in `packages/contracts` tests.

**Benchmarks if applicable:** None.

**Acceptance criteria:**
- `engine/scripts/cargo.sh test -p review-core publication` passes all the Rust tests listed.
- `pnpm -F @reviewgraph/contracts test` passes `contracts_review_event_ts_is_comment_literal`.
- `pnpm contracts:check` is green.
- `grep -rnE 'Approve|RequestChanges|Merge' engine/crates/review-core/src/publication.rs` matches only doc comments.

**Definition of done:**
- Tests pass, and the INV-011/INV-012 tests are listed in the invariant suite index (INV tasks, P30-P37 file) as implemented at the domain layer.
- The legacy reference `core.rs:316-349` and the gap-analysis §P decision are cited in the module doc.
- Master plan §4.10 links to `publication.rs`.
