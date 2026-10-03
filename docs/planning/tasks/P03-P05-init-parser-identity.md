# Phases 3–5 — Repository initialization, parser & language analyzer, stable symbol identity

**Plan:** [MASTER_IMPLEMENTATION_PLAN.md](../MASTER_IMPLEMENTATION_PLAN.md) §9, phases 3, 4 and 5.
**Architecture:** [target-architecture.md](../../architecture/target-architecture.md) §2, §3.1, §3.2, §3.4, §7.
**ADRs:** [ADR-005](../../decisions/ADR-005-stable-symbol-identity.md) (identity), [ADR-006](../../decisions/ADR-006-language-analyzer-protocol.md) (analyzer protocol), [ADR-007](../../decisions/ADR-007-tree-sitter-plus-semantic-enrichment.md) (tree-sitter + semantic), [ADR-015](../../decisions/ADR-015-repository-snapshot-versioning.md) (fingerprint/versioning), [ADR-014](../../decisions/ADR-014-postgresql-graph-persistence.md) (migrations).
**PRD:** §12–§20, §93–§99.

Status markers: ☐ todo · ◐ in progress · ☑ done (acceptance criteria executed and passed). The global Definition of Done in master plan §10 applies to every task in addition to its own.

---

## 0. Conventions used by every task in this file

### 0.1 Build and test

- The engine builds **only** inside the Linux container: `engine/scripts/cargo.sh <cargo args>` (ADR-001, FND tasks). Every command in an acceptance criterion below is run through it. "`cargo.sh`" below always means `engine/scripts/cargo.sh`.
- tree-sitter `0.25` and tree-sitter-typescript `0.23` are verified to build in that container (ADR-001 probe). Do not change those major/minor versions inside these phases.
- Fixture repositories live in `fixtures/repositories/<name>/` as plain files plus an optional scripted history. `fixtures/build.sh <name>` (Phase 1 foundation) materializes each into a real git repository under `fixtures/.build/<name>/` (gitignored). Tests call the Rust helper `review_test_support::fixture_repo("<name>") -> PathBuf`, which runs the build once per test process and caches the result. If the helper does not exist yet when the first task here starts, INIT-001 adds it to a `test-support` dev-only crate (`engine/crates/test-support`, `publish = false`, never a normal dependency).
- The external reference repository `reference-api` is **not** inside the workspace. Tests that use it are `#[ignore]` and read the path from `RG_REFERENCE_REPO_PATH`. They run with the checkout mounted read-only into the container, for example `-v <path-to-reference-repo>:/ext/reference-api:ro` and `RG_REFERENCE_REPO_PATH=/ext/reference-api` (mount option per FND-002). Those runs are acceptance evidence for M2. They are never CI-required.

### 0.2 Cross-file references to other phases

- `FND-*` and `DOM-*` refer to [P01-P02-foundation-domain.md](P01-P02-foundation-domain.md). Where a task here names a type that DOM may already have introduced (`RepoPath`, `Language`, `ContentHash`, `AnalyzerVersion`, `RepositoryId`), **extend the existing type and do not duplicate it**. If it is absent, the task here adds it to `review-core` at the stated path.
- `CG-*`, `GS-*`, `IDX-*`, `INC-*` refer to [P06-P08-graph-index-incremental.md](P06-P08-graph-index-incremental.md). Tasks in this file produce **IR and facts only**. Mapping facts onto graph node and edge kinds is the linker's job (CG). Each framework task states which node and edge kinds its facts are meant to produce, so CG can implement the mapping table without guessing.

### 0.3 Crates touched (target-architecture §2)

| Crate | Path | Role in these phases |
|---|---|---|
| `review-core` | `engine/crates/review-core` | `RepoPath`, `Language`, `ContentHash`, `SymbolKind`, `SymbolId`, `SymbolKey`, `ModulePath` (no I/O) |
| `repository` | `engine/crates/repository` | git state (gix), file walk, all `review init` detectors, `.review/` writer, fingerprint, `RepositoryFactsStore` port |
| `analysis-ir` | `engine/crates/analysis-ir` | `ParsedUnit` and every IR type; `LanguageAnalyzer`, `FrameworkAdapter`, `ModuleResolver`, `SemanticProvider` traits |
| `lang-typescript` | `engine/crates/lang-typescript` | tree-sitter TS/TSX/JS analyzer, TS naming rules, module resolver, framework adapters (`frameworks::{nestjs,typeorm,bullmq,jest,config}`), semantic helper client |
| `incremental` | `engine/crates/incremental` | per-file symbol diff, rename/move matcher, lineage records |
| `graph-storage` | `engine/crates/graph-storage` | Postgres adapter of `RepositoryFactsStore` only (INIT-013) |
| tools | `engine/tools/ts-semantic` | Node helper (TSA-010) |
| migrations | `engine/migrations` | INIT-013 migration |

Dependency direction stays as target-architecture §2.1: `repository`, `analysis-ir` → `review-core`; `lang-typescript` → `analysis-ir`; `incremental` → `analysis-ir` (+ `codegraph`/`graph-storage` later). `analysis-ir` must not depend on `repository`. Repository facts reach analyzers as plain data (`analysis_ir::FrameworkSignals`, `lang_typescript::resolve::TsResolverConfig`), converted at the composition root (`pipeline` / apps).

### 0.4 Shared constants introduced here

| Constant | Location | Initial value | Bump rule |
|---|---|---|---|
| `REPOSITORY_FACTS_SCHEMA` | `repository::facts` | `1` | any breaking change to `repository.json` shape |
| `IR_SCHEMA_VERSION` | `analysis_ir` | `1` | any change to serialized `ParsedUnit` shape |
| `lang_typescript::ANALYZER_VERSION` | `lang-typescript` | `0.1.0` | minor: new facts extracted; major: identity/hash rule change (forces re-parse of TS/JS only, ADR-015) |
| `HASH_DOMAIN_*` | `analysis_ir::hash` | `rg.body.v1`, `rg.sig.v1`, `rg.attr.v1`, `rg.shingle.v1` | changes only with an analyzer major bump |
| `FINGERPRINT_DOMAIN` | `repository::fingerprint` | `rg.fp.v1` | new fingerprint input |
| Framework confidence | `lang_typescript::frameworks::confidence` | see each NEST task | calibrated by benchmark later (CG-003 owns the global edge table) |

Linker confidences (`import` 0.95, `this_member` 0.95, `di_constructor` 0.85, `type_annotation` 0.8, `name_unique` 0.6, `name_ambiguous` 0.3, `framework` 0.9, `type_checker` 1.0) are **owned by `codegraph::confidence`** (CG-003). Tasks here only emit the hints that let the linker pick the right row.

### 0.5 Fixture repositories created by these phases

| Fixture | Created by | Purpose |
|---|---|---|
| `init-basic` | INIT-001 | single-package npm TS repo: git state, languages, manifests |
| `init-edge-cases` | INIT-002 | .gitignore/.reviewignore, binary, oversize, symlinks, sensitive files with canary content, LFS pointer, non-UTF-8 content, case collision |
| `monorepo-pnpm` | INIT-005 | pnpm workspace, `apps/api` (Nest), `apps/web` (Next), `packages/{shared,db}`, turbo.json, tsconfig `paths`, `workspace:*` deps |
| `polyglot-manifests` | INIT-004 | pyproject, requirements.txt, go.mod, Cargo workspace, pom.xml, build.gradle.kts, yarn berry lockfile |
| `ts-basic` | TSA-003 | every TS declaration form, imports/exports, CJS |
| `ts-edge` | TSA-002 | syntax errors, huge file, JSX, decorators-in-odd-places, overloads, declaration merging |
| `nest-api` | NEST-001 | NestJS app: modules, controllers, guards, DI, TypeORM, BullMQ, Jest, config — modelled on reference-api patterns (global prefix, URI versioning, `APP_GUARD`, `@Public()`, custom `WorkerHost` base, queue names from constants, `dataSource.transaction`) |
| `rename-move` | SID-006 | scripted history c0…c8 for identity tests |

### 0.6 PRD §13 coverage matrix (where each of the 22 init facts comes from)

| # | PRD §13 fact | Field in `RepositoryFacts` | Task |
|---|---|---|---|
| 1 | languages | `languages[]` | INIT-003 |
| 2 | frameworks | `frameworks[]` | INIT-006 |
| 3 | package managers | `package_managers[]` | INIT-004 |
| 4 | build systems | `build_systems[]` | INIT-004 |
| 5 | test frameworks | `frameworks[]` with `category = test` | INIT-006 |
| 6 | source roots | `layout.source_roots[]` | INIT-007 |
| 7 | test roots | `layout.test_roots[]`, `layout.test_globs[]` | INIT-007 |
| 8 | generated-code directories | `generated` | INIT-008 |
| 9 | dependency manifests | `manifests[]` | INIT-004 |
| 10 | monorepo workspaces | `workspaces` | INIT-005 |
| 11 | public entry points | `entrypoints[]` (`library`, `http_server`) | INIT-009 |
| 12 | API routes | not detectable without parsing: `api_routes = DeferredToIndex`; produced by NEST-002 at index time and summarized back into `repository.json` by IDX | NEST-002 / IDX |
| 13 | command-line entry points | `entrypoints[]` (`cli`, `script`) | INIT-009 |
| 14 | worker entry points | `entrypoints[]` (`worker`) | INIT-009 |
| 15 | database schemas/migrations | `migrations[]`, `schema_files[]` | INIT-009 |
| 16 | infrastructure configuration | `infra[]` | INIT-009 |
| 17 | auth boundaries where detectable | `auth.libraries[]` (INIT-006) at init; route-level boundaries from NEST-004 at index time | INIT-006 / NEST-004 |
| 18 | linting configuration | `tooling.lint[]` | INIT-007 |
| 19 | compiler configuration | `tooling.compiler[]`, `tsconfigs[]` | INIT-007 |
| 20 | CI configuration | `tooling.ci[]` | INIT-007 |
| 21 | architecture metadata | `docs.architecture[]`, `docs.adrs[]`, `docs.codeowners` | INIT-010 |
| 22 | rule-relevant documentation | `docs.rule_docs[]`, `docs.knowledge_vaults[]` | INIT-010 |

### 0.7 Task index

| ID | Title | Depends on | Status |
|---|---|---|---|
| INIT-001 | Repository discovery & git state via gix | FND-001, DOM-001 | ☐ |
| INIT-002 | File walker with ignore rules | INIT-001 | ☐ |
| INIT-003 | Language detection + per-language stats | INIT-002 | ☐ |
| INIT-004 | Package manager, manifest & build-system detection | INIT-002 | ☐ |
| INIT-005 | Monorepo/workspace detection | INIT-004 | ☐ |
| INIT-006 | Framework detection from manifests | INIT-004, INIT-005 | ☐ |
| INIT-007 | Source/test roots, tsconfig discovery, lint/compiler/CI | INIT-002, INIT-005 | ☐ |
| INIT-008 | Generated code detection | INIT-002, INIT-007 | ☐ |
| INIT-009 | Entrypoints, migrations, infra, env files (names only) | INIT-004, INIT-007 | ☐ |
| INIT-010 | Rule-doc & architecture metadata discovery | INIT-002 | ☐ |
| INIT-011 | `.review/` layout + `repository.json` writer | INIT-001..010 | ☐ |
| INIT-012 | Repository fingerprint (ADR-015) | INIT-011 | ☐ |
| INIT-013 | Persist init facts to PostgreSQL | INIT-011, INIT-012, DOM-009 | ☐ |
| TSA-001 | `analysis-ir` crate: IR types + traits | DOM-001 | ☐ |
| TSA-002 | tree-sitter TS/TSX/JS integration | TSA-001 | ☐ |
| TSA-003 | Declaration extraction | TSA-002, SID-002 (naming module, developed in lockstep) | ☐ |
| TSA-004 | Import/export extraction (ESM + CJS) | TSA-002 | ☐ |
| TSA-005 | Reference extraction with receiver hints | TSA-003, TSA-004 | ☐ |
| TSA-006 | Per-symbol SyntaxFacts | TSA-005 | ☐ |
| TSA-007 | Normalized body/signature/attribute hashes + shingles | TSA-003 | ☐ |
| TSA-008 | Golden IR test suite | TSA-003..007 | ☐ |
| TSA-009 | Module resolver | TSA-004, INIT-005, INIT-007 | ☐ |
| TSA-010 | ts-semantic helper behind `SemanticProvider` | TSA-001, TSA-005 | ☐ |
| NEST-001 | NestJS `@Module` facts | TSA-005 | ☐ |
| NEST-002 | Controllers + HTTP routes | NEST-001 | ☐ |
| NEST-003 | DI injection facts | TSA-005 | ☐ |
| NEST-004 | Guards/interceptors/pipes + global providers | NEST-001, NEST-002 | ☐ |
| NEST-005 | TypeORM entities + repository access | NEST-003, TSA-006 | ☐ |
| NEST-006 | BullMQ processors + producers | NEST-003 | ☐ |
| NEST-007 | Jest suites/cases + config/env reads | TSA-005 | ☐ |
| SID-001 | `SymbolId` canonical form + `SymbolKey` | TSA-001 | ☐ |
| SID-002 | TS qualified-name rules | SID-001, TSA-002 | ☐ |
| SID-003 | Overload/duplicate ordinal determinism | SID-002 | ☐ |
| SID-004 | Per-file symbol diff | SID-001, TSA-007 | ☐ |
| SID-005 | Rename/move matcher + lineage records | SID-004 | ☐ |
| SID-006 | Rename/move test suite | SID-005 | ☐ |

Critical-path members in this file (master plan §7): TSA-001 → TSA-003 → TSA-005 → SID-001 → SID-004. High-risk: SID-005, TSA-009.

---

## Phase 3 — Repository initialization (`engine/crates/repository`)

Every detector in this phase is a **pure function over already-walked data** (`FileInventory` plus bounded reads through `repository::read::BoundedReader`), except INIT-001 (git) and INIT-002 (walk). No detector executes repository code, spawns a process, or touches the network. Every detector returns `(Facts, Vec<InitWarning>)` and never fails the whole init for one malformed file.

## Task index

| ID | Title |
|---|---|
| INIT-001 | Repository discovery & git state via gix |
| INIT-002 | File walker with ignore rules |
| INIT-003 | Language detection + per-language stats |
| INIT-004 | Package manager, manifest & build-system detection |
| INIT-005 | Monorepo/workspace detection |
| INIT-006 | Framework detection from manifests |
| INIT-007 | Source roots, test roots, tsconfig discovery, lint/compiler/CI config |
| INIT-008 | Generated code detection |
| INIT-009 | Entrypoints, migration dirs, infra config, env files (names only) |
| INIT-010 | Rule-doc & architecture metadata discovery |
| INIT-011 | `.review/` layout + `repository.json` writer (RepositoryFacts) |
| INIT-012 | Repository fingerprint (ADR-015) |
| INIT-013 | Persist init facts to PostgreSQL via a RepositoryFactsStore port |
| TSA-001 | `analysis-ir` crate: IR types and analyzer traits |
| TSA-002 | tree-sitter TS/TSX/JS integration in `lang-typescript` |
| TSA-003 | Declaration extraction |
| TSA-004 | Import/export extraction (ESM + CJS) |
| TSA-005 | Reference extraction with receiver hints |
| TSA-006 | Per-symbol SyntaxFacts for change classification |
| TSA-007 | Normalized body_hash / signature_hash |
| TSA-008 | Golden IR test suite |
| TSA-009 | Module resolver |
| TSA-010 | ts-semantic helper (optional type-checker enrichment) |
| NEST-001 | NestJS adapter foundation and @Module facts |
| NEST-002 | Controllers and route decorators → HttpRoute facts |
| NEST-003 | Dependency injection facts and references |
| NEST-004 | Guards, interceptors, pipes → Middleware facts |
| NEST-005 | TypeORM entities, relations and repository access heuristics |
| NEST-006 | BullMQ processors, workers and producers |
| NEST-007 | Jest tests and configuration reads |
| SID-001 | SymbolId canonical form, SymbolKey, parse/format round-trip |
| SID-002 | Qualified-name rules for TypeScript |
| SID-003 | Overload and duplicate ordinal determinism |
| SID-004 | Per-file symbol diff |
| SID-005 | Rename/move matcher and symbol_lineage records |
| SID-006 | Rename/move test suite on fixtures/repositories/rename-move |

---

### INIT-001 — Repository discovery & git state via gix

Status: â

> **Implementation note:** Fixtures use the step-directory builder (FND-008) rather than a scripted history.sh, so the origin remote and refs/remotes/origin/HEAD are added by test helpers (review-test-support::add_origin). The test-support crate lives at engine/tools/test-support (package review-test-support) instead of engine/crates/ so the xtask dependency-direction table needs no new library entry. gix 0.88 needs the sha1 and dirwalk features in addition to those listed. Tracing spans are emitted with the tracing crate directly; metric instruments wait for the telemetry crate.

- **Task ID:** INIT-001
- **Title:** Repository discovery & git state via gix (open, HEAD, remotes, dirty status, default branch)
- **Problem:** `review init`, the indexer and the fingerprint all need a reliable answer to "which repository, at which commit, in which state". The legacy system shells out to `git` (audit §1), so user git config, hooks and locale can change the output, and credentials embedded in remote URLs leak into logs.
- **Why it exists:** PRD §12 (`review init --repository`), §15 (fingerprint needs `commit_sha`), target-architecture §3.6 (diff uses gix, never a subprocess). Every later INIT detector takes the discovered root as input.
- **Scope:**
  - Discover the repository from a path (walk up to the `.git` dir or file, linked worktrees included).
  - Resolve HEAD: commit id, symbolic branch or detached, unborn HEAD.
  - List remotes with credential-free URLs, and parse provider hint and `owner/name` slug.
  - Compute dirty status: staged, unstaged, untracked counts plus up to 20 sample paths.
  - Determine the default branch, recording which source decided it.
  - Detect shallow clones, bare repositories, submodules (paths only) and LFS-tracked patterns from `.gitattributes`.
  - Add the `test-support` crate with `fixture_repo()` if it does not exist yet.
  - Create fixture `init-basic`.
- **Explicit non-scope:** fetching, cloning, mirroring and checkout (PIPE/GH). Tree diffs (DIFF-001). Reading submodule contents. Provider API calls for the default branch (the provider value is accepted as an input only). Writing anything into `.git`.
- **Files/modules expected to change:** `engine/Cargo.toml` (workspace member `crates/repository`, workspace deps `gix`), `engine/crates/review-core/src/lib.rs` (re-export `RepoPath` if DOM did not add it).
- **New files/modules expected:**
  - `engine/crates/repository/Cargo.toml`, `src/lib.rs`, `src/error.rs`, `src/git.rs`, `src/remote_url.rs`
  - `engine/crates/repository/tests/git_state.rs`
  - `engine/crates/test-support/{Cargo.toml,src/lib.rs}` (if absent)
  - `fixtures/repositories/init-basic/` (files plus `history.sh` that makes 3 commits on `main`, a `feature/x` branch and an `origin` remote pointing at a local bare repo whose `HEAD` is `main`)
- **Dependencies:** FND-001 (workspace), FND-002 (`cargo.sh`), DOM-001 (`review-core`).
- **Implementation details:**
  - `gix` with `default-features = false`, features `["max-performance-safe", "status", "revision", "excludes", "attributes"]`. The exact feature names are verified against the pinned gix version in `Cargo.lock` during implementation. No `blocking-network-client`.
  - Public API:
    ```rust
    pub struct GitOpenOptions { pub allow_non_git: bool, pub provider_default_branch: Option<String> }
    pub fn discover(path: &Path, opts: &GitOpenOptions) -> Result<DiscoveredRepo, InitError>;
    pub struct DiscoveredRepo { pub root: PathBuf /* canonical worktree root */, pub git: Option<GitState> }
    pub struct GitState {
        pub head: HeadState,                 // Commit{ sha: CommitSha, branch: Option<String> } | Detached{ sha } | Unborn{ branch }
        pub remotes: Vec<RemoteFact>,        // sorted by name
        pub default_branch: Option<String>,
        pub default_branch_source: DefaultBranchSource, // OriginHead | Provider | WellKnownName | CurrentBranch | Unknown
        pub dirty: DirtyState,
        pub is_shallow: bool,
        pub is_linked_worktree: bool,
        pub submodules: Vec<RepoPath>,       // from .gitmodules, sorted
        pub lfs_patterns: Vec<String>,       // `.gitattributes` lines with filter=lfs
    }
    pub struct RemoteFact { pub name: String, pub url: RedactedUrl, pub provider: ProviderHint /* github|gitlab|bitbucket|azure|other|local */, pub slug: Option<String> }
    pub struct DirtyState { pub is_dirty: bool, pub staged: u32, pub unstaged: u32, pub untracked: u32, pub sample_paths: Vec<RepoPath> /* ≤20, sorted */ }
    ```
  - Discovery uses `gix::discover(path)`, then `repo.workdir()`. A bare repo returns `InitError::BareRepository`: init always runs on a worktree, and workers create one (PIPE). A missing repo returns `InitError::NotAGitRepository`, unless `allow_non_git`, in which case `git = None` and a warning `not_a_git_repository`.
  - Isolation from user config: open with `gix::open::Options::isolated()` (no global/system config, no `includeIf`). Only repository-local config is read, and only for `remote.*` and `core.bare`. This makes results identical on the CLI host and the worker.
  - Default branch, in order:
    1. symbolic target of `refs/remotes/origin/HEAD`;
    2. `provider_default_branch` if given;
    3. the first existing of `main`, `master`, `develop`, `trunk` (local, then `refs/remotes/origin/*`);
    4. the current branch.
  - Remote URL redaction (`remote_url.rs`):
    - Parse with `gix::url::parse`.
    - Drop `user`/`password` for http(s). Keep `git@` for scp-like ssh.
    - Strip query and fragment.
    - The `RedactedUrl` newtype has no constructor from a raw string except `redact()`.
    - Slug: `host/owner/name` with `.git` stripped. Hosts `github.com`, `gitlab.com`, `bitbucket.org` and `dev.azure.com` set the provider hint.
  - Dirty status through `repo.status(gix::progress::Discard)` index↔worktree and HEAD↔index iterators, using the same ignore rules as INIT-002 so untracked counts agree. Sample paths come from the first 20 in sorted order. Hard timeout: stop counting after 200k entries and set `dirty.truncated = true`. Do not hang on huge untracked trees.
  - Every path that leaves this module is a `RepoPath` (forward slashes, relative to root, no `..`, valid UTF-8). Non-UTF-8 paths become a warning `non_utf8_path` and are skipped.
- **Data model changes:** None (in-memory types only; persisted through INIT-011/INIT-013).
- **API/protocol changes:** New public Rust API `repository::git::{discover, GitState, ...}`.
- **Concurrency semantics:** `discover` is synchronous and read-only. `gix::Repository` is converted to `ThreadSafeRepository` when shared with the rayon walk (INIT-002). Concurrent inits on the same root are serialized by the `.review/.lock` taken in INIT-011, not here.
- **Failure behavior:** Typed `InitError::{NotAGitRepository, BareRepository, Io{path}, Git{op, source}}`. Corrupt refs or unreadable remotes produce warnings (`remote_unreadable`, `head_unreadable`), not failure, unless HEAD itself cannot be resolved and `allow_non_git == false`. No panics.
- **Idempotency considerations:** Pure read. Two calls on an unchanged repo return equal `GitState` (derive `PartialEq`, sorted vectors).
- **Security considerations:**
  - Credentials in remote URLs never reach logs, spans or `repository.json`. They are removed in `RedactedUrl` before any other code sees them.
  - Isolated config prevents `core.fsmonitor`/`core.hooksPath` from executing anything. gix does not run hooks.
  - Symlinked `.git` files pointing outside root are followed only by gix's own discovery. The worktree root is canonicalized and later path checks are relative to it.
- **Observability additions:** span `init.git_discover` (attrs `git.head_kind`, `git.is_shallow`, `git.dirty`, `git.remote_count`, `git.default_branch_source`). Metric `init_git_status_entries_total`. URLs are never attributes.
- **Tests required** (`tests/git_state.rs`, fixture `init-basic`):
  - `discover_from_nested_subdir_finds_root`
  - `head_on_branch_reports_sha_and_branch`
  - `detached_head_reports_detached`
  - `unborn_head_in_empty_repo`
  - `bare_repo_is_rejected`
  - `non_git_dir_rejected_unless_allowed`
  - `default_branch_from_origin_head`
  - `default_branch_falls_back_to_main_then_current`
  - `remote_url_credentials_are_redacted` (https with `user:token@`, and `x-access-token:` forms)
  - `scp_like_ssh_url_slug_parsed`
  - `dirty_counts_staged_unstaged_untracked`
  - `global_git_config_is_ignored` (sets `HOME` to a dir with a `.gitconfig` that defines a remote alias and asserts no effect)
  - `submodule_paths_listed_not_recursed`
  - `lfs_patterns_from_gitattributes`
  - Unit tests in `remote_url.rs` for 12 URL shapes.
- **Benchmarks if applicable:** None (covered by INIT-011 end-to-end timing).
- **Acceptance criteria:**
  - `engine/scripts/cargo.sh test -p repository --test git_state` passes.
  - `engine/scripts/cargo.sh clippy -p repository -- -D warnings` is clean.
  - `grep -rn "Command::new(\"git\")" engine/crates/repository` returns nothing.
  - The test `remote_url_credentials_are_redacted` asserts the token string is absent from `format!("{:?}", state)` and from the serialized JSON.
- **Definition of done:** All tests green. The `repository` crate is in the workspace with no `git` subprocess. `init-basic` builds through `fixtures/build.sh`. Global DoD met.

---

---

### INIT-002 — File walker with ignore rules

Status: â

> **Implementation note:** The fixture builder rejects symlinks and the root .gitignore would hide .env.* and *.key files, so init-edge-cases holds the plain-file content and review-test-support::edge_case_tree() adds symlinks, binary/oversized/LFS files, sensitive files with canary content, node_modules and a non-UTF-8 name at test time. walk_with() takes a FileSource so tests can prove which paths are opened (CountingFiles). IgnoreReason counts only what the walker can observe (built-in dirs, bad paths): files excluded by .gitignore/.reviewignore/config globs are never enumerated by the ignore crate. git.rs reads .gitmodules/.gitattributes directly because they are git metadata, not detector content; the read-path guard test allows it. Metrics are not emitted until the telemetry crate lands.

- **Task ID:** INIT-002
- **Title:** File walker with ignore rules (.gitignore via `ignore` crate, .reviewignore, size limits, binary detection, symlink policy)
- **Problem:** Every detector and the indexer need one authoritative, deterministic list of repository files. That list must respect .gitignore, skip vendored and huge content, never follow symlinks out of the tree, and never open files that are likely to hold secrets.
- **Why it exists:** PRD §13 (all detections run over the file set), §93 (generated directories), §110/§111 (secret handling). Determinism matters because the fingerprint and the parse cache depend on the same file set on the CLI host and on the worker.
- **Scope:**
  - `FileInventory` built with `ignore::WalkBuilder` (parallel walk, sorted output).
  - Ignore sources: `.gitignore` (nested), `.reviewignore` (same syntax, nested), `.review/config.yaml` `ignore:` globs (passed in as `Vec<String>`), and built-in always-ignored dirs.
  - Per-file classification: `Source | Binary | TooLarge | LfsPointer | Sensitive | Symlink`.
  - Bounded reader utility shared by all detectors.
  - Sensitive-path matcher (names only).
  - Case-collision and non-UTF-8 path warnings.
  - Fixture `init-edge-cases`.
- **Explicit non-scope:** Language detection (INIT-003), generated classification (INIT-008), content hashing for the parse cache (IDX-001 hashes while reading for parse), secret scanning of contents (SEC-003).
- **Files/modules expected to change:** `engine/crates/repository/src/lib.rs`, `engine/crates/repository/Cargo.toml` (deps `ignore`, `globset`, `rayon`).
- **New files/modules expected:** `engine/crates/repository/src/walk.rs`, `src/read.rs`, `src/sensitive.rs`, `tests/walk.rs`, `fixtures/repositories/init-edge-cases/` (with `build.sh` steps that create symlinks and a binary file at build time, because plain-file fixtures cannot hold symlinks portably on Windows hosts).
- **Dependencies:** INIT-001.
- **Implementation details:**
  - API:
    ```rust
    pub struct WalkOptions {
        pub max_analyze_bytes: u64,      // default 1 MiB: larger files -> TooLarge (counted, never parsed)
        pub hard_max_bytes: u64,         // default 16 MiB: never read at all
        pub max_files: u64,              // default 500_000 -> InitError::TooManyFiles (never silent truncation)
        pub extra_ignore_globs: Vec<String>, // from .review/config.yaml `ignore:`
        pub respect_local_excludes: bool,    // .git/info/exclude; default false for host/worker parity
    }
    pub fn walk(root: &Path, opts: &WalkOptions) -> Result<(FileInventory, Vec<InitWarning>), InitError>;
    pub struct FileInventory { pub root: PathBuf, pub entries: Vec<FileEntry> /* sorted by path */, pub ignored: BTreeMap<IgnoreReason, u64>, pub dirs: Vec<RepoPath> /* sorted, used by glob expansion in INIT-005 */ }
    pub struct FileEntry { pub path: RepoPath, pub size: u64, pub class: FileClass, pub symlink_target: Option<SymlinkTarget> }
    pub enum FileClass { Source, Binary, TooLarge, LfsPointer, Sensitive, Symlink }
    pub enum IgnoreReason { Gitignore, Reviewignore, ConfigGlob, BuiltinDir, NodeModules, Hidden /* unused: hidden files ARE walked */, NonUtf8Path, OutsideRoot }
    ```
  - `WalkBuilder` settings:
    - `.hidden(false)`, because `.github/` and `.agent/` must be seen.
    - `.git_ignore(true)`, `.git_global(false)`, `.git_exclude(opts.respect_local_excludes)`, `.parents(false)`, `.require_git(false)`, `.ignore(false)` (no `.ignore` files, so behaviour is not tool-specific).
    - `.add_custom_ignore_filename(".reviewignore")`, `.follow_links(false)`, `.same_file_system(true)`.
    - `filter_entry` drops built-in dirs `.git`, `node_modules`, `.review`, `.hg`, `.svn`, `bower_components`, `.yarn/cache`, `.pnpm-store` at any depth.
  - `extra_ignore_globs` are applied through `ignore::overrides::OverrideBuilder`, each added as `!{glob}`.
  - Walk with `build_parallel()` into a `Mutex<Vec<FileEntry>>`, then `sort_unstable_by(path)`. Output order is independent of thread scheduling.
  - Classification order (first match wins):
    1. symlink → `Symlink`. The target is read with `read_link`, never followed. Record `SymlinkTarget { relative: Option<RepoPath>, escapes_root: bool }`; `escapes_root` → warning `symlink_escapes_root`.
    2. `sensitive::is_sensitive(path)` → `Sensitive`. **The file is never opened.**
    3. `size > hard_max_bytes` → `TooLarge`, never opened.
    4. Known binary extension (static table: png jpg jpeg gif webp ico bmp tiff pdf zip gz tgz bz2 xz 7z rar jar war class so dylib dll exe bin o a wasm woff woff2 ttf otf eot mp3 mp4 mov avi webm sqlite db pyc parquet) → `Binary`.
    5. Read the first 8 KiB: a NUL byte → `Binary`; a git-lfs pointer (`version https://git-lfs.github.com/spec/v1`) → `LfsPointer`.
    6. `size > max_analyze_bytes` → `TooLarge`.
    7. Otherwise `Source`.
  - `sensitive.rs` glob table, matched on the basename (case-insensitive):
    - `.env`, `.env.*` except templates `.env.example|.env.sample|.env.template|.env.dist|.env.defaults`
    - `*.env`, `.envrc`
    - `*.pem`, `*.key`, `*.p12`, `*.pfx`, `*.jks`, `*.keystore`
    - `id_rsa*`, `id_ed25519*`, `*.ppk`
    - `credentials.json`, `*service-account*.json`, `*-key.json`, `gcs-key.json`
    - `.npmrc`, `.pypirc`, `.netrc`, `.git-credentials`
    - `secrets.y{a,}ml`, `*.tfstate`, `*.tfstate.backup`
    - `.claude/settings.local.json`

    Templates are classified `Source` (INIT-009 reads only their keys).
  - `read.rs`: `BoundedReader::read_prefix(&FileEntry, max: usize) -> Result<Vec<u8>, ReadError>` and `read_text(&FileEntry, max)`. Both refuse `Sensitive`, `Symlink`, `Binary` and `TooLarge` entries with `ReadError::Refused(class)`. All detector reads go through this type. That is how "never read sensitive files" is enforced in code, not by convention.
  - Path safety: every walked path is `strip_prefix(canonical_root)`; failure → `IgnoreReason::OutsideRoot`. `RepoPath::new` rejects `..` components, absolute paths, NUL and backslashes.
  - Case collisions: build a `HashMap<lowercase path, path>`; on a collision emit warning `case_collision` with both paths.
- **Data model changes:** None.
- **API/protocol changes:** New public API `repository::walk`, `repository::read`, `repository::sensitive`.
- **Concurrency semantics:** The walk uses `ignore`'s parallel walker (threads = `min(available_parallelism, 8)`). Results are merged and sorted, so they are deterministic. The `BoundedReader` is `Sync` and holds no state.
- **Failure behavior:**
  - A permission-denied entry → warning `unreadable_entry` with the path, and the walk continues.
  - More than `max_files` entries → `InitError::TooManyFiles { limit }`. The fix is configuration, never a silent cut.
  - A malformed `.gitignore`/`.reviewignore` line → warning `ignore_parse` with file and line; the line is skipped.
- **Idempotency considerations:** Same tree and options → byte-identical serialized `FileInventory` (test asserts equality across two walks with different thread counts).
- **Security considerations:**
  - Sensitive files and symlink targets are never opened.
  - The walk never leaves root.
  - `node_modules` is always skipped: no reading of third-party code at init.
  - The canary test proves sensitive content is not read: the fixture file's content is a unique string that must not appear in any output.
- **Observability additions:** span `init.walk` (attrs `walk.entries`, `walk.ignored`, `walk.threads`). Metrics `init_files_walked_total{class}` and `init_files_ignored_total{reason}`. Warnings are counted in `init_warnings_total{code}`.
- **Tests required** (`tests/walk.rs`, fixture `init-edge-cases`):
  - `gitignore_nested_rules_respected`
  - `reviewignore_applies_like_gitignore`
  - `config_ignore_globs_applied`
  - `node_modules_always_skipped_even_if_not_gitignored`
  - `hidden_dirs_like_github_are_walked`
  - `global_gitignore_not_applied`
  - `binary_by_extension_and_by_nul_sniff`
  - `lfs_pointer_detected`
  - `too_large_counted_not_read`
  - `hard_max_file_never_opened` (uses a `CountingFs` test double around the reader)
  - `symlink_not_followed_and_escape_warned`
  - `sensitive_files_classified_and_never_read` (canary `RG_CANARY_5f1c` in `gcs-key.json` and `.env.production`; asserts the canary is absent from `serde_json::to_string(&inventory)` and from captured logs)
  - `env_template_is_source_not_sensitive`
  - `case_collision_warned`
  - `too_many_files_is_explicit_error`
  - `output_sorted_and_deterministic_across_thread_counts`
  - Proptest `repo_path_rejects_traversal`
- **Benchmarks if applicable:** `benches/walk.rs` (criterion) on a generated 50k-file tree. Target ≤ 1.5 s warm cache in the container. Recorded, not gated, until PERF-002.
- **Acceptance criteria:**
  - `engine/scripts/cargo.sh test -p repository --test walk` passes.
  - The canary test passes.
  - The `#[ignore]` test `walk_reference_api` with `RG_REFERENCE_REPO_PATH` reports between 1,000 and 1,200 `Source` files (reference has 1,028 files indexed by the external codegraph) and classifies `gcs-key.json` as `Sensitive` (if it is not gitignored).
- **Definition of done:** All listed tests green. `BoundedReader` is the only file-content read path in `repository` (a test greps `src/` for `std::fs::read` / `File::open` outside `read.rs` and `walk.rs`). Global DoD met.

---

---

### INIT-003 — Language detection + per-language stats

Status: â

> **Implementation note:** review_core::Language already existed with six variants and lowercase wire names, so it was extended additively (Kotlin, Csharp, Ruby, Php, Shell, Sql, Yaml, Json, Toml, Markdown, Html, Css, Dockerfile, Terraform, Protobuf, Graphql, Prisma, Other) and keeps id_prefix() rather than gaining a second id_tag(). Other is a unit variant, so Makefile/Jenkinsfile are reported as Other without the make/groovy hint. detect_language returns Some(Other) for unknown extensions and None for extensionless files with no recognizable name or shebang; stats count the latter as Other. The polyglot-manifests assertion (typescript, python, go, rust, java, kotlin) is covered by an inline polyglot tree here and again against the polyglot-manifests fixture in INIT-004.

- **Task ID:** INIT-003
- **Title:** Language detection (extension/shebang) + per-language stats
- **Problem:** Routing files to analyzers, choosing a primary language and reporting coverage all need a stable language label per file.
- **Why it exists:** PRD §13 #1, §96 (language priority: only TS/JS has an analyzer in the MVP, so others must be reported as present-but-unanalyzed rather than silently dropped).
- **Scope:**
  - `Language` enum and `detect_language(path, prefix_bytes) -> Option<LanguageTag>`.
  - Extension table, special filenames, shebang for extensionless files.
  - Dialect for TS (`ts`/`tsx`/`dts`) and JS (`js`/`jsx`, `module_kind` hint from `.mjs`/`.cjs`).
  - Per-language stats: files, bytes, lines, analyzable.
  - `primary_language`.
- **Explicit non-scope:** Content-based classification (linguist-style Bayesian). Vendored/generated exclusion (INIT-008 marks them, and stats report both totals). Grammar selection (TSA-002 consumes the tag).
- **Files/modules expected to change:** `engine/crates/review-core/src/language.rs` (extend `Language` if DOM defined it), `engine/crates/repository/src/lib.rs`.
- **New files/modules expected:** `engine/crates/repository/src/language.rs`, `tests/languages.rs`.
- **Dependencies:** INIT-002.
- **Implementation details:**
  - `review_core::Language`: `TypeScript, JavaScript, Python, Go, Rust, Java, Kotlin, CSharp, Ruby, Php, Shell, Sql, Yaml, Json, Toml, Markdown, Html, Css, Dockerfile, Terraform, Protobuf, GraphQL, Prisma, Other`. Serialized lowercase (`typescript`). The short id tags used in `SymbolId` (`ts`, `js`, `py`, `go`, `rs`, `java`, ...) come from `Language::id_tag()`.
  - `LanguageTag { language: Language, dialect: Option<Dialect> }` where `Dialect::{Ts, Tsx, Dts, Js, Jsx, Mjs, Cjs}`.
  - Extension table (lowercase, longest suffix first, so `.d.ts` beats `.ts`):
    - `.d.ts .d.mts .d.cts` → TS/Dts
    - `.ts .mts .cts` → TS/Ts; `.tsx` → TS/Tsx
    - `.js` → JS/Js; `.mjs` → JS/Mjs; `.cjs` → JS/Cjs; `.jsx` → JS/Jsx
    - `.py .pyi`; `.go`; `.rs`; `.java`; `.kt .kts`; `.cs`; `.rb`; `.php`
    - `.sh .bash .zsh`; `.sql`; `.yml .yaml`; `.json .jsonc .json5`; `.toml`; `.md .mdx`; `.html .htm`; `.css .scss .sass .less`
    - `.tf .tfvars .hcl`; `.proto`; `.graphql .gql`; `.prisma`
  - Filenames: `Dockerfile`, `Dockerfile.*`, `*.dockerfile` → Dockerfile; `Makefile` → Other("make"); `Jenkinsfile` → Other("groovy").
  - Shebang (only when there is no extension): read 256 bytes through `BoundedReader`. Match `^#!\s*(/usr/bin/env\s+(-S\s+)?)?(\S*/)?(node|nodejs|bun)` → JS, `ts-node|tsx|deno` → TS, `python[0-9.]*` → Python, `bash|sh|zsh|dash` → Shell, `ruby` → Ruby.
  - `LanguageStat { language, files, bytes, lines, analyzable: bool, generated_files: u64 }`. `lines` counts `\n` in files of class `Source` (bounded read; `TooLarge` contributes bytes but not lines). Computed in parallel with rayon over entries. `analyzable = language ∈ registered analyzers`, a static list passed in by the caller (`&[Language::TypeScript, Language::JavaScript]` in the MVP).
  - `primary_language`: argmax by bytes over programming languages (exclude Json, Yaml, Toml, Markdown, Html, Css, Other, Sql). Ties are broken by enum order.
  - Sort stats by bytes desc, then language name.
- **Data model changes:** None.
- **API/protocol changes:** `repository::language::{detect_language, language_stats}`. `review_core::Language` gains variants (additive).
- **Concurrency semantics:** Line counting is a rayon `par_iter` over entries followed by a deterministic fold (sums only).
- **Failure behavior:** An unreadable file counts toward bytes, not lines, and raises warning `unreadable_entry`. Unknown extensions → `Language::Other` with no warning.
- **Idempotency considerations:** Pure function of the inventory.
- **Security considerations:** Uses `BoundedReader` only (sensitive files refused). Shebang reads are capped at 256 bytes.
- **Observability additions:** span `init.languages` (attrs `languages.count`, `languages.primary`). Metric `init_files_by_language_total{language}`.
- **Tests required** (`tests/languages.rs`, fixtures `init-basic`, `polyglot-manifests`):
  - `dts_beats_ts_suffix`
  - `tsx_and_jsx_dialects`
  - `mjs_cjs_dialects`
  - `shebang_node_and_tsnode_detected`
  - `shebang_ignored_when_extension_present`
  - `dockerfile_variants`
  - `stats_sorted_and_summed`
  - `too_large_counts_bytes_not_lines`
  - `primary_language_excludes_data_formats`
  - `analyzable_flag_follows_registered_analyzers`
  - Table-driven unit test over 60 paths.
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** `engine/scripts/cargo.sh test -p repository --test languages` passes. On `polyglot-manifests` the stats contain typescript, python, go, rust, java and kotlin, with `analyzable = true` only for typescript and javascript.
- **Definition of done:** Tests green. Global DoD met.

---

---

### INIT-004 — Package manager, manifest & build-system detection

Status: â

> **Implementation note:** RepoPath forbids the empty path, so directory scopes (scope_dir, later config_dir/paths_base) use the new repository::RepoDir, which is the empty string for the repository root. ManifestFact gained members (Maven modules, Gradle includes, Cargo workspace members) and meta (go version, poetry/ruff tool markers) fields, and the Python parsers keep names and ranges only. Build systems are separate kinds (Webpack, Vite, Rollup, Esbuild, Tsup, Swc) rather than one Swc bucket. polyglot-manifests and monorepo-pnpm fixtures were created here (monorepo-pnpm is also used by INIT-005).

- **Task ID:** INIT-004
- **Title:** Package manager & manifest detection (npm/pnpm/yarn/bun lockfiles; pyproject, go.mod, Cargo.toml, pom/gradle recorded)
- **Problem:** Framework detection, workspace detection, external-dependency nodes (`pkg:npm/...`) and the module resolver all need the declared dependencies and the package manager in use.
- **Why it exists:** PRD §13 #3, #4, #9. TSA-009 needs the dependency list to turn bare specifiers into `ExternalDependency` nodes with a version range.
- **Scope:**
  - Lockfile and package-manager detection: npm, pnpm, yarn classic and berry, bun; the `packageManager` field.
  - `package.json` parsing (every one outside ignored dirs).
  - Recorded-only manifests: `pyproject.toml`, `requirements*.txt`, `Pipfile`, `go.mod`, `go.work`, `Cargo.toml`, `pom.xml`, `build.gradle(.kts)`, `settings.gradle(.kts)`, `Gemfile`, `composer.json`, `*.csproj`.
  - Build-system facts.
  - JSONC helper (shared with INIT-007).
  - Fixture `polyglot-manifests`.
- **Explicit non-scope:** Lockfile resolution graphs (versions installed). Running any package manager. Vulnerability data. Workspace member expansion (INIT-005).
- **Files/modules expected to change:** `engine/crates/repository/Cargo.toml` (deps `serde_json`, `toml`, `quick-xml`, the workspace YAML crate chosen in FND, `semver`).
- **New files/modules expected:** `engine/crates/repository/src/manifests/{mod.rs,npm.rs,python.rs,go.rs,cargo.rs,jvm.rs,other.rs}`, `src/build_systems.rs`, `src/jsonc.rs`, `tests/manifests.rs`, `fixtures/repositories/polyglot-manifests/`.
- **Dependencies:** INIT-002.
- **Implementation details:**
  - Types:
    ```rust
    pub struct PackageManagerFact { pub kind: PackageManagerKind /* Npm|Pnpm|YarnClassic|YarnBerry|Bun|Pip|Poetry|Uv|GoModules|Cargo|Maven|Gradle|Bundler|Composer|NuGet */,
        pub lockfile: Option<RepoPath>, pub lockfile_version: Option<String>, pub declared: Option<String> /* packageManager field e.g. "pnpm@10.4.1" */, pub scope_dir: RepoPath }
    pub struct ManifestFact { pub path: RepoPath, pub ecosystem: Ecosystem /* npm|pypi|go|cargo|maven|gradle|rubygems|composer|nuget */,
        pub name: Option<String>, pub version: Option<String>, pub private: bool,
        pub dependencies: BTreeMap<String, DepSpec>, // DepSpec { range: String, kind: Prod|Dev|Peer|Optional, workspace_protocol: bool }
        pub npm: Option<NpmManifestExtras>, pub parse_status: ParseStatusLite /* Ok|Partial|Failed{reason} */ }
    pub struct NpmManifestExtras { pub module_type: Option<ModuleType> /* module|commonjs */, pub main: Option<String>, pub module: Option<String>, pub types: Option<String>,
        pub bin: BTreeMap<String, String>, pub exports: Option<serde_json::Value> /* kept verbatim, ≤64 KiB */, pub workspaces: Option<Vec<String>>,
        pub script_names: Vec<String>, pub script_entry_hints: BTreeMap<String, Vec<String>> /* path-like tokens only, see INIT-009 */, pub engines: BTreeMap<String,String> }
    pub struct BuildSystemFact { pub kind: BuildSystemKind, pub config: RepoPath, pub scope_dir: RepoPath }
    ```
  - npm lockfiles:
    - `package-lock.json` / `npm-shrinkwrap.json`: read ≤ 4 KiB and regex `"lockfileVersion"\s*:\s*(\d+)`.
    - `pnpm-lock.yaml`: first line `lockfileVersion: '9.0'`.
    - `yarn.lock`: `# yarn lockfile v1` → classic; `__metadata:` → berry.
    - `bun.lockb` (binary, presence only) or `bun.lock` (text).
    - Lockfiles are never fully parsed.
  - Package-manager choice per scope dir:
    1. the `packageManager` field;
    2. the single lockfile present;
    3. with several lockfiles, warning `multiple_lockfiles`, all reported, and priority `pnpm > yarn > bun > npm` for the `primary` flag.
  - `package.json`: parsed with `serde_json` into a lenient `serde_json::Value`, then extracted field by field. A wrong type on one field yields `Partial` plus warning `manifest_field_type`. Files > 2 MiB are not parsed (warning `manifest_too_large`).
  - Script values are **not** stored. `script_entry_hints` keeps only tokens matching `^(\./)?[\w@./-]+\.(m?[jt]s|c[jt]s)$` or `^(dist|build|src|lib)/[\w./-]+$`. A script line can embed secrets (`--password=`), so raw commands are dropped.
  - pyproject: `[project] name, version, dependencies[]` (PEP 508 names, parsed up to the first `[ <>=!~;`), `[tool.poetry] name, dependencies` → Poetry, `uv.lock` → Uv.
  - go.mod: `module` line, `go` line, count of `require` entries. Names stored, versions recorded.
  - Cargo.toml: `[package].name/version` or `[workspace]` (members expanded in INIT-005); dependency table names.
  - pom.xml: `quick-xml` streaming reader for `project/groupId`, `artifactId`, `version` and `modules/module`; stop after 1 MiB.
  - Gradle: regex `rootProject.name\s*=\s*["'](.+)["']` and `include\(?\s*["']([^"']+)["']` in settings files. Build files are recorded only.
  - Build systems, from config presence:
    - `nest-cli.json` → NestCli; `tsconfig.build.json` → Tsc
    - `webpack.config.*`, `vite.config.*`, `rollup.config.*`, `esbuild.*`, `tsup.config.*`, `.swcrc` → Swc
    - `turbo.json`, `nx.json`, `Makefile`, `WORKSPACE`/`MODULE.bazel`, `pom.xml`, `build.gradle*`, `Cargo.toml`, `go.mod`
    - `justfile`, `Taskfile.yml`
  - `jsonc.rs`: `pub fn parse_jsonc(bytes: &[u8]) -> Result<serde_json::Value, JsoncError>`. A hand-written state machine strips `//` and `/* */` comments outside strings and trailing commas before `}`/`]`, then hands off to `serde_json`. It is fuzz-tested by proptest: output equals `serde_json` for comment-free inputs.
- **Data model changes:** None.
- **API/protocol changes:** `repository::manifests::{detect_manifests, ManifestFact, PackageManagerFact}`, `repository::jsonc::parse_jsonc`.
- **Concurrency semantics:** Manifests are parsed in parallel (rayon) and sorted by path.
- **Failure behavior:** Malformed manifest → `parse_status = Failed{reason}` plus warning `manifest_parse` (path, line/col where available). It is never fatal.
- **Idempotency considerations:** Pure function of inventory plus file contents. `BTreeMap` everywhere.
- **Security considerations:** Script commands and `.npmrc` (sensitive) are never stored or read. `exports` is stored verbatim but capped. No registry access.
- **Observability additions:** span `init.manifests` (attrs `manifests.count`, `package_manager.primary`, `lockfiles.count`). Warnings → `init_warnings_total{code}`.
- **Tests required** (`tests/manifests.rs`; fixtures `init-basic`, `polyglot-manifests`, `monorepo-pnpm`):
  - `npm_lockfile_version_detected`
  - `pnpm_lock_and_package_manager_field`
  - `yarn_classic_vs_berry`
  - `bun_text_and_binary_lock`
  - `multiple_lockfiles_warns_and_prefers_package_manager_field`
  - `package_json_dependencies_by_kind`
  - `workspace_protocol_flagged`
  - `script_values_not_stored_only_entry_hints`
  - `malformed_package_json_is_partial_not_fatal`
  - `pyproject_pep621_and_poetry`
  - `go_mod_module_path`
  - `cargo_workspace_recorded`
  - `pom_modules_streamed`
  - `gradle_settings_includes`
  - `build_systems_detected`
  - jsonc unit tests: `jsonc_strips_line_and_block_comments`, `jsonc_keeps_comment_like_text_in_strings`, `jsonc_trailing_commas`, proptest `jsonc_equals_serde_for_plain_json`.
- **Benchmarks if applicable:** None.
- **Acceptance criteria:**
  - `engine/scripts/cargo.sh test -p repository --test manifests` passes.
  - On reference (`#[ignore]`, `RG_REFERENCE_REPO_PATH`): `package_managers[0].kind == Npm`, the dependencies include `@nestjs/core`, `typeorm` and `bullmq`, and no field of the serialized output contains the substring `node -r ts-node/register` (raw script text dropped).
- **Definition of done:** Tests green, JSONC helper reused by INIT-007. Global DoD met.

---

---

### INIT-005 — Monorepo/workspace detection

Status: â

> **Implementation note:** Workspace patterns are interpreted only at the repository root (pnpm-workspace.yaml, package.json workspaces, lerna.json, rush.json, nx project.json, Cargo [workspace], go.work); nested pnpm-workspace.yaml files raise nested_workspace_root. package_for is a linear longest-prefix scan over the (small) package list rather than a precomputed binary search. Dir scopes use RepoDir (root = empty string).

- **Task ID:** INIT-005
- **Title:** Monorepo/workspace detection (pnpm-workspace.yaml, package.json workspaces, nx/turbo/lerna, cargo workspace)
- **Problem:** Cross-package imports (`@acme/shared`) must resolve to source inside the repo, not to an external package. Review budgets must prioritise the changed workspace (PRD §94). Neither is possible without knowing the package boundaries.
- **Why it exists:** PRD §13 #10, §94. TSA-009 resolves workspace packages from this output. PROF/RISK use `package_for(path)`.
- **Scope:**
  - Workspace definition sources: `pnpm-workspace.yaml`, `package.json` `workspaces` (array and `{packages}` forms), `lerna.json`, `nx.json` + `project.json` files, `turbo.json` (presence and task names), `rush.json` (presence and `projects[].projectFolder`), Cargo `[workspace]`, `go.work`.
  - Glob expansion over `FileInventory.dirs`, including negations.
  - Internal dependency edges between packages; cycle warning.
  - `package_for(path)` lookup.
  - Fixture `monorepo-pnpm`.
- **Explicit non-scope:** Nested workspaces inside workspaces (reported as warning `nested_workspace_root`, not modelled). Build-graph semantics of nx/turbo tasks. Resolving packages from `node_modules`.
- **Files/modules expected to change:** `engine/crates/repository/src/lib.rs`.
- **New files/modules expected:** `engine/crates/repository/src/workspaces.rs`, `tests/workspaces.rs`, `fixtures/repositories/monorepo-pnpm/`.
- **Dependencies:** INIT-004 (manifests), INIT-002.
- **Implementation details:**
  - Types:
    ```rust
    pub struct WorkspaceLayout { pub is_monorepo: bool, pub tools: Vec<WorkspaceTool> /* Pnpm|NpmWorkspaces|Yarn|Lerna|Nx|Turbo|Rush|Cargo|GoWork */,
        pub root_package: Option<RepoPath>, pub packages: Vec<WorkspacePackage> /* sorted by dir */, pub internal_edges: Vec<(String, String)> /* sorted */, pub cycles: Vec<Vec<String>> }
    pub struct WorkspacePackage { pub name: String, pub dir: RepoPath, pub manifest: Option<RepoPath>, pub ecosystem: Ecosystem, pub version: Option<String>,
        pub private: bool, pub source: WorkspaceTool, pub internal_deps: Vec<String> }
    impl WorkspaceLayout { pub fn package_for(&self, path: &RepoPath) -> Option<&WorkspacePackage>; } // longest dir prefix; precomputed sorted Vec + binary search
    ```
  - Pattern semantics follow pnpm/npm:
    - `packages/*` matches direct child dirs that contain a `package.json`.
    - `packages/**` matches any depth.
    - `!**/test/**` excludes.
    - Patterns are compiled with `globset` (`literal_separator(true)`) and matched against `FileInventory.dirs`. The filesystem is not re-read.
    - A matched dir without a manifest is skipped with warning `workspace_dir_without_manifest`.
  - Package name: the manifest `name`, else the directory basename (warning `workspace_package_unnamed`). Duplicate names → warning `duplicate_workspace_package_name`; the first by dir order wins for resolution.
  - Nx: each `project.json` dir is a package (`name` from project.json). `nx.json` `workspaceLayout.{appsDir,libsDir}` is recorded.
  - Turbo adds no packages. It relies on package-manager workspaces; record `tools += Turbo` and the `tasks`/`pipeline` key names.
  - Cargo: `[workspace] members`/`exclude` globs. go.work: `use` directives.
  - Internal edges: for each package, every dependency whose name equals a workspace package name (any range, or `workspace:` protocol) → edge `(from, to)`. Cycles are found with Tarjan SCC (`petgraph` is not needed; about 40 lines). An SCC larger than 1 → `cycles` plus warning `workspace_cycle`.
  - `is_monorepo = packages.len() >= 2` (the root package is not counted unless it is itself listed).
- **Data model changes:** None.
- **API/protocol changes:** `repository::workspaces::{detect_workspaces, WorkspaceLayout}`.
- **Concurrency semantics:** None (single-threaded; small input).
- **Failure behavior:** A malformed `pnpm-workspace.yaml` → warning `workspace_config_parse`, falling back to `package.json` workspaces if present. Never fatal.
- **Idempotency considerations:** Pure and sorted.
- **Security considerations:** Reads only manifests already allowed by `BoundedReader`.
- **Observability additions:** span `init.workspaces` (attrs `workspace.tools`, `workspace.packages`, `workspace.cycles`).
- **Tests required** (`tests/workspaces.rs`; fixtures `monorepo-pnpm`, `polyglot-manifests`, `init-basic`):
  - `pnpm_workspace_globs_expand_to_packages`
  - `negated_glob_excludes_package`
  - `npm_workspaces_array_and_object_forms`
  - `nx_project_json_packages`
  - `turbo_recorded_without_packages`
  - `cargo_workspace_members_and_exclude`
  - `go_work_use_dirs`
  - `internal_edges_from_workspace_protocol`
  - `cycle_detected_and_warned`
  - `package_for_longest_prefix`
  - `single_package_repo_is_not_monorepo`
  - `dir_without_manifest_warned`
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** `engine/scripts/cargo.sh test -p repository --test workspaces` passes. On `monorepo-pnpm`: four packages (`@acme/api`, `@acme/web`, `@acme/shared`, `@acme/db`), edges `api→shared`, `api→db`, `web→shared`, and `package_for("apps/api/src/main.ts").name == "@acme/api"`.
- **Definition of done:** Tests green. Global DoD met.

---

---

### INIT-006 — Framework detection from manifests

Status: â

> **Implementation note:** The nest-api fixture does not exist yet (NEST-001), so framework tests use inline temp trees plus the monorepo-pnpm fixture. FrameworkId is a plain String alias. The jwt libraries (@nestjs/jwt, jsonwebtoken, jose) share one id, jwt, and passport/@nestjs/passport share passport. Wrapper detection (express/fastify through @nestjs/platform-*) is a separate VIA_RULES table.

- **Task ID:** INIT-006
- **Title:** Framework detection from manifests (NestJS, Express, Next.js, React, TypeORM, Prisma, BullMQ, Jest/Vitest/Mocha)
- **Problem:** Framework adapters (NEST-*) should run only where the framework is present (`FrameworkAdapter::detect`). Reviewers and risk rules also need to know the stack. Guessing from source text alone is slow and produces false positives.
- **Why it exists:** PRD §13 #2, #5, #17 (auth libraries); §97–§98 (modular adapters gated by detection); ADR-006 (`FrameworkAdapter::detect(&RepoFacts)`).
- **Scope:**
  - A static rule table mapping dependencies and config files to `FrameworkFact` with category, version range, parsed major, evidence and confidence. Detection runs per workspace package and at the root.
  - Frameworks: NestJS, Express, Fastify, Next.js, React, Vue (recorded), TypeORM, Prisma, Sequelize/Knex/MikroORM/Drizzle (recorded), BullMQ, Bull (legacy), Jest, Vitest, Mocha, Playwright/Cypress (recorded), `@nestjs/config`, Swagger, class-validator/zod/joi, passport/`@nestjs/passport`/`@nestjs/jwt`/jsonwebtoken (auth libraries).
  - `to_framework_signals()` returns `repository::FrameworkSignalsData`. The crate DAG forbids `repository` → `analysis-ir`, so the composition root maps it field-for-field into `analysis_ir::FrameworkSignals`. Both are plain `BTreeMap<FrameworkId, FrameworkPresence { scope_dirs, major, confidence }>`.
- **Explicit non-scope:** Source-level detection of framework usage (the adapters do that and can raise or lower confidence). Version-specific behaviour beyond recording `major`. Python/Java frameworks (Django, Spring) beyond recording a dependency name hit.
- **Files/modules expected to change:** `engine/crates/repository/src/lib.rs`.
- **New files/modules expected:** `engine/crates/repository/src/frameworks.rs`, `src/frameworks/rules.rs`, `tests/frameworks.rs`.
- **Dependencies:** INIT-004, INIT-005.
- **Implementation details:**
  - Types:
    ```rust
    pub struct FrameworkFact { pub id: FrameworkId /* stable lowercase: "nestjs","express","nextjs","react","typeorm","prisma","bullmq","bull","jest","vitest","mocha",... */,
        pub category: FrameworkCategory /* Web|Ui|Orm|Queue|Test|Auth|Validation|Config|Docs */, pub scope: RepoPath /* package dir or "" for root */,
        pub version_range: Option<String>, pub major: Option<u64>, pub confidence: f32, pub evidence: Vec<FrameworkEvidence>, pub via: Option<FrameworkId> }
    pub struct FrameworkEvidence { pub source: EvidenceSource /* Dependency{kind}|ConfigFile|SchemaFile */, pub path: RepoPath, pub detail: String }
    ```
  - Rule table rows: `(id, category, dependency names[], config globs[], runtime: bool)`. Examples:
    - nestjs: deps `@nestjs/core`, `@nestjs/common`; config `nest-cli.json`
    - express: `express`
    - nextjs: `next`; `next.config.{js,mjs,ts}`
    - react: `react`
    - typeorm: `typeorm`, `@nestjs/typeorm`; `ormconfig.*`, `**/data-source.ts`
    - prisma: `prisma`, `@prisma/client`; `**/schema.prisma`
    - bullmq: `bullmq`, `@nestjs/bullmq`; bull: `bull`, `@nestjs/bull`
    - jest: `jest`, `ts-jest`; `jest.config.{js,ts,mjs,cjs,json}` or a `jest` key in package.json
    - vitest: `vitest`; `vitest.config.*`; mocha: `mocha`; `.mocharc.*`
    - config: `@nestjs/config`, `dotenv`; auth: `passport`, `@nestjs/passport`, `@nestjs/jwt`, `jsonwebtoken`, `jose`
  - Confidence:
    - runtime framework in prod deps: 0.9
    - only in devDeps (for `runtime = true` rows): 0.6
    - test frameworks in devDeps: 0.9
    - config file only: 0.7
    - dependency + config: 1.0
    - The values sit in one `const` table in `rules.rs`.
  - `via`: `express` found only through `@nestjs/platform-express` → `via = nestjs`, `confidence = 0.9`. It is not reported as a standalone Express app (avoids running a future Express adapter on Nest code).
  - `major`: strip a leading `^`, `~`, `>=` or `=` and `workspace:`, then `semver::Version::parse` or `VersionReq`, taking the lowest major. Unparseable (`latest`, git URLs) → `None`.
  - Dedup: one fact per `(id, scope)`, merging evidence.
- **Data model changes:** None.
- **API/protocol changes:** `repository::frameworks::{detect_frameworks, FrameworkFact, to_framework_signals}`.
- **Concurrency semantics:** None.
- **Failure behavior:** Never fails. Missing manifests → empty list.
- **Idempotency considerations:** Pure and sorted by `(scope, id)`.
- **Security considerations:** None beyond the use of `BoundedReader`.
- **Observability additions:** span `init.frameworks` (attr `frameworks.ids`, comma-joined, ≤ 512 chars).
- **Tests required** (`tests/frameworks.rs`; fixtures `nest-api`, `monorepo-pnpm`, `init-basic`):
  - `nestjs_detected_with_major`
  - `express_via_nest_platform_not_standalone`
  - `nextjs_and_react_in_web_package_scope`
  - `typeorm_by_dependency_and_data_source`
  - `prisma_by_schema_file`
  - `bullmq_vs_legacy_bull`
  - `jest_from_package_json_key`
  - `vitest_config_only_confidence_0_7`
  - `runtime_framework_in_dev_deps_lower_confidence`
  - `auth_libraries_categorized`
  - `framework_signals_conversion_roundtrip`
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** `engine/scripts/cargo.sh test -p repository --test frameworks` passes. On reference (`#[ignore]`): `nestjs`, `typeorm`, `bullmq`, `jest`, `config` and `auth` are present; `express.via == nestjs`.
- **Definition of done:** Tests green. The rule table is documented inline (one row per framework, with the evidence it needs). Global DoD met.

---

---

### INIT-007 — Source roots, test roots, tsconfig discovery, lint/compiler/CI config

Status: â

> **Implementation note:** tsconfig facts add include_base (TypeScript resolves include/exclude/files relative to the config that defined them, which differs from config_dir when inherited) and a TsOwner result carrying owned_by_fallback from tsconfig_for. Extracted Jest config values keep <rootDir> semantics (rootDir joined with each root). Config scripts are scanned with a regex and never executed; the nest-api fixture does not exist yet so tests use inline trees plus monorepo-pnpm.

- **Task ID:** INIT-007
- **Title:** Source roots, test roots, tsconfig discovery (extends chain, paths/baseUrl), lint/compiler/CI config detection
- **Problem:** Several later stages depend on facts that live only in these configs:
  - The module resolver must apply `paths`/`baseUrl` from the tsconfig that governs a file. the reference consumer resolves every `@core/*`, `@features/*` and similar alias through `paths`.
  - Test mapping needs test roots and globs.
  - Profiles and risk need lint, compiler and CI facts.

  tsconfig files are JSONC with `extends` chains and per-file ownership, so naive parsing gets them wrong.
- **Why it exists:** PRD §13 #6, #7, #18, #19, #20; §24 (a tsconfig change is a full-rebuild trigger, so tsconfig facts go into `config_hash`, INIT-012); TSA-009.
- **Scope:**
  - tsconfig/jsconfig discovery, JSONC parsing, `extends` resolution (string and array forms, relative and package bases) and the effective-options merge.
  - File→tsconfig ownership function.
  - Source roots and test roots/globs.
  - Lint/format, compiler/runtime and CI config detection, plus git hooks.
- **Explicit non-scope:**
  - Running `tsc`.
  - Evaluating JS/TS config files. `jest.config.ts` and `eslint.config.mjs` are **never executed**; only static literal extraction is done.
  - Project references build graph. `references` is recorded only.
  - Interpreting lint rules (PROF).
- **Files/modules expected to change:** `engine/crates/repository/src/lib.rs`, `Cargo.toml` (`globset`).
- **New files/modules expected:** `engine/crates/repository/src/layout.rs`, `src/tsconfig.rs`, `src/tooling.rs`, `tests/tsconfig.rs`, `tests/layout_tooling.rs`.
- **Dependencies:** INIT-002, INIT-004 (jsonc), INIT-005 (package dirs).
- **Implementation details:**
  - tsconfig types:
    ```rust
    pub struct TsConfigFact { pub path: RepoPath, pub extends_chain: Vec<TsExtends> /* nearest first */, pub effective: TsEffectiveOptions, pub parse_status: ParseStatusLite }
    pub enum TsExtends { Local(RepoPath), Package { specifier: String, resolved: Option<RepoPath> } , Unresolved(String) }
    pub struct TsEffectiveOptions { pub base_url: Option<RepoPath>, pub paths: Vec<TsPathMapping> /* in declaration order */, pub paths_base: RepoPath /* dir of the config that defined paths */,
        pub module: Option<String>, pub module_resolution: Option<String>, pub target: Option<String>, pub jsx: Option<String>, pub strict: Option<bool>,
        pub experimental_decorators: Option<bool>, pub emit_decorator_metadata: Option<bool>, pub allow_js: Option<bool>, pub root_dir: Option<RepoPath>, pub out_dir: Option<RepoPath>,
        pub root_dirs: Vec<RepoPath>, pub include: Vec<String>, pub exclude: Vec<String>, pub files: Vec<RepoPath>, pub references: Vec<RepoPath>, pub config_dir: RepoPath }
    pub struct TsPathMapping { pub pattern: String, pub targets: Vec<String> }
    pub fn tsconfig_for(&self /* TsConfigSet */, file: &RepoPath) -> Option<&TsConfigFact>;
    ```
  - Discovery: every `tsconfig*.json` and `jsconfig.json` in the inventory, parsed with `jsonc::parse_jsonc`. This matters because reference's tsconfig is full of comments.
  - `extends` resolution, with the chain capped at 16 and a visited-set for cycle detection (warning `tsconfig_extends_cycle`):
    - A relative value (`./`, `../`) is resolved against the config dir, appending `.json` if missing.
    - A bare value (`@tsconfig/node20/tsconfig.json`) is looked up as `node_modules/<spec>` in the inventory. `node_modules` is not walked (INIT-002), so this resolves only if FND-provided checkouts include it, which they normally do not → `Package{resolved: None}` plus warning `tsconfig_base_unresolved`.
    - Array `extends` (TS ≥ 5.0) is applied left to right.
  - Merge (TS semantics):
    - `compilerOptions` keys: child overrides parent per key.
    - `paths` replaced wholesale, never merged.
    - `baseUrl`, `rootDir`, `outDir` and `paths` targets are resolved relative to the config file that **defined** them, kept as `paths_base`.
    - `include`/`exclude`/`files` are replaced, not merged, and relative to the defining config.
    - `references` are not inherited.
    - Default `include` when it is absent and `files` is absent: `["**/*"]`.
  - Ownership for `tsconfig_for(file)`: candidates are configs named exactly `tsconfig.json` (or `jsconfig.json`) in `file`'s ancestor dirs, nearest first. The first whose `files`/`include` globs match and whose `exclude` does not (`globset`, relative to `config_dir`) wins. If none match, the nearest `tsconfig.json` is used with `owned_by_fallback = true`. `tsconfig.build.json`, `tsconfig.spec.json` and similar are recorded but never chosen by ownership.
  - Source roots (`RootFact { path, source: RootSource, confidence }`):
    - `rootDir` of owning configs (0.9)
    - `nest-cli.json` `sourceRoot` (0.95) and `projects.*.sourceRoot`
    - workspace package `src/` dirs (0.8)
    - conventional `src/`, `lib/`, `app/` with ≥1 analyzable file (0.6)
  - Test roots:
    - `test/`, `tests/`, `__tests__/`, `e2e/`, `spec/` (0.8)
    - jest/vitest `roots`, `testMatch`, `testRegex`, extracted by static scan: `package.json` `jest` key (JSON, exact); `jest.config.json` (exact); `jest.config.{ts,js,mjs,cjs}` by regex over string literals in arrays following `roots:`, `testMatch:` and `testRegex:` (confidence 0.6, warning `config_static_extraction`)
    - `test/jest-e2e.json` (reference pattern) parsed as JSON.
  - Default `test_globs`: `**/*.{spec,test}.{ts,tsx,js,jsx,mts,cts,mjs,cjs}`, `**/*.e2e-spec.ts`, `**/__tests__/**/*.{ts,tsx,js,jsx}`, plus extracted `testMatch`.
  - Tooling:
    - `tooling.lint[]`:
      - eslint: `.eslintrc{,.js,.cjs,.json,.yml,.yaml}`, `eslint.config.{js,mjs,cjs,ts}`
      - biome: `biome.json{,c}`
      - prettier: `.prettierrc*`, `prettier.config.*`
      - others: `.editorconfig`, `tslint.json`, `.dependency-cruiser.{js,cjs}`, `knip.json`, `.jscpd.json`, `.stylelintrc*`
      - Python linters from `[tool.ruff|flake8|pylint|mypy]` in pyproject.
    - `tooling.compiler[]`:
      - tsconfig set, `.swcrc`, `babel.config.*`/`.babelrc`
      - `nest-cli.json` (`compilerOptions.builder`)
      - `.nvmrc`/`.node-version`/`engines.node`, `.python-version`, `rust-toolchain{,.toml}`
    - `tooling.ci[]`:
      - `.github/workflows/*.y{a,}ml`: parse YAML for workflow `name`, `on` keys, job ids, `uses:` action refs and `run:` command **first tokens only** (e.g. `npm`, `pnpm`, `npx`, `docker`), never full commands
      - `.gitlab-ci.yml`, `.circleci/config.yml`, `Jenkinsfile`, `azure-pipelines.yml`, `bitbucket-pipelines.yml`, `.buildkite/`, `cloudbuild.y{a,}ml` (presence + path)
    - `tooling.hooks[]`: `.husky/*`, `lint-staged.config.*`, `.pre-commit-config.yaml`, `lefthook.yml`.
- **Data model changes:** None.
- **API/protocol changes:** `repository::tsconfig::{TsConfigSet, TsConfigFact, tsconfig_for}`, `repository::layout::{RootFact, LayoutFacts}`, `repository::tooling::ToolingFacts`.
- **Concurrency semantics:** None (configs are few). `TsConfigSet` is immutable after construction and `Sync`, so TSA-009 can share it through an `Arc`.
- **Failure behavior:** A malformed config → `parse_status = Failed` plus warning `tsconfig_parse`/`ci_config_parse`. The fact is still listed. A missing extends base → warning, and the merge continues from what resolved.
- **Idempotency considerations:** Pure. `paths` keep declaration order (it is significant for TS) and everything else is sorted.
- **Security considerations:**
  - Config JS/TS files are never executed or `require`d.
  - CI `run:` commands are reduced to their first token, because CI commands can embed inline secrets.
  - `${{ secrets.X }}` names are not recorded.
- **Observability additions:** span `init.layout` (attrs `tsconfig.count`, `source_roots.count`, `test_roots.count`, `ci.providers`).
- **Tests required:**
  - `tests/tsconfig.rs` (fixtures `nest-api`, `monorepo-pnpm`, plus inline temp dirs):
    - `tsconfig_with_comments_and_trailing_commas_parses`
    - `extends_relative_chain_merges_compiler_options`
    - `paths_replaced_not_merged`
    - `paths_relative_to_defining_config`
    - `base_url_inherited_from_parent`
    - `extends_array_applied_left_to_right`
    - `extends_package_unresolved_warns`
    - `extends_cycle_detected`
    - `ownership_nearest_including_config_wins`
    - `excluded_file_falls_back_flagged`
    - `build_config_never_chosen_by_ownership`
    - `default_include_all`
  - `tests/layout_tooling.rs`:
    - `nest_cli_source_root_preferred`
    - `test_roots_from_jest_json_key`
    - `jest_config_ts_static_extraction_low_confidence`
    - `jest_e2e_json_testregex`
    - `eslint_flat_config_detected`
    - `github_workflow_jobs_and_first_tokens_only`
    - `ci_run_command_full_text_not_stored` (asserts a fixture secret-looking argument is absent)
    - `husky_hooks_detected`
- **Benchmarks if applicable:** None.
- **Acceptance criteria:**
  - `engine/scripts/cargo.sh test -p repository --test tsconfig --test layout_tooling` passes.
  - On reference (`#[ignore]`): `tsconfig_for("src/features/v1/account/account.controller.ts")` returns `tsconfig.json` with 7 `paths` mappings and `base_url == ""` (repo root); `emit_decorator_metadata == Some(true)`; `tooling.ci` lists `ci-cd.yml` and `pr-validation.yml`.
- **Definition of done:** Tests green. `TsConfigSet` is consumed by TSA-009 without re-parsing. Global DoD met.

---

---

### INIT-008 — Generated code detection

Status: â

> **Implementation note:** Header marker and pattern names are Strings (not static str) in GeneratedReason so the type is deserializable. The false-positive, @generated, Go-marker, vendor and linguist cases live in the init-edge-cases fixture (extended in INIT-002); dist/coverage/min.js/lockfile cases are added dynamically by edge_case_tree() because the root .gitignore excludes dist/ and coverage/. GitAttributesView is a small in-crate parser (globset with **/ prefixing) rather than gix-attributes.

- **Task ID:** INIT-008
- **Title:** Generated code detection (dirs like dist/build/generated, headers "@generated"/"DO NOT EDIT", config globs)
- **Problem:** Generated files inflate the graph and waste review budget. Findings on them are noise: PRD §93 says the generator input should be reviewed instead. Detection must be explainable (why was this file marked?) and overridable.
- **Why it exists:** PRD §13 #8, §93. Symbols get `is_generated` (target-architecture §3.4). REV-002 routing skips generated changes.
- **Scope:**
  - Per-file `GeneratedClass` with reason and confidence.
  - Sources, in precedence order:
    1. `.review/config.yaml` `generated.exclude` (force not generated)
    2. `generated.include`
    3. `.gitattributes` `linguist-generated` / `linguist-vendored` (true/false)
    4. header marker
    5. file-name pattern
    6. directory name
  - Summary of generated dirs. Vendored class kept separately.
- **Explicit non-scope:** Finding the generator input for a generated file (post-MVP; recorded as `generator_hint` only when a header names it, e.g. `Code generated by protoc-gen-go`). Minified-code heuristics beyond the `.min.` pattern.
- **Files/modules expected to change:** `engine/crates/repository/src/lib.rs`.
- **New files/modules expected:** `engine/crates/repository/src/generated.rs`, `tests/generated.rs`. Fixture additions in `init-edge-cases/` (`src/generated/client.ts` with `@generated`, `api.pb.go` with the Go marker, `vendor/lib.js`, a `.gitattributes` with `linguist-generated`, and a false-positive file whose comment says "DO NOT EDIT this constant without approval").
- **Dependencies:** INIT-002, INIT-007 (out dirs from tsconfig `outDir` are added to the directory list).
- **Implementation details:**
  - Types:
    ```rust
    pub enum GeneratedKind { Generated, Vendored, Lockfile, Minified }
    pub enum GeneratedReason { ConfigInclude(String), ConfigExclude(String), GitAttributes(String), Header{ marker: &'static str, line: u32 }, Pattern(&'static str), Directory(String), TsOutDir(RepoPath) }
    pub struct GeneratedClass { pub kind: GeneratedKind, pub reason: GeneratedReason, pub confidence: f32, pub generator_hint: Option<String> }
    pub struct GeneratedFacts { pub files: BTreeMap<RepoPath, GeneratedClass>, pub dirs: Vec<(RepoPath, GeneratedKind, u64 /* files */)>, pub config_globs: Vec<String> }
    pub fn classify_generated(inv: &FileInventory, ts: &TsConfigSet, attrs: &GitAttributesView, cfg: &GeneratedConfig, reader: &BoundedReader) -> (GeneratedFacts, Vec<InitWarning>);
    ```
  - Directory names (any path segment, exact match):
    - `dist`, `build`, `out`, `.next`, `.nuxt`, `.svelte-kit`, `coverage`, `generated`, `__generated__`, `gen`, `.turbo`, `.nx`, `.serverless`, `storybook-static`, `allure-report`, `allure-results` → Generated (0.8)
    - `vendor`, `third_party`, `third-party` → Vendored (0.8)
    - tsconfig `outDir` dirs → Generated (0.9)
  - Name patterns (`globset`, basename):
    - `*.generated.*`, `*.gen.{ts,js,go}` → 0.9
    - `*.pb.go`, `*_pb2.py`, `*_pb2_grpc.py`, `*.pb.{ts,js}`, `*_grpc_pb.{ts,js}` → 0.95
    - `*.min.{js,css}` → Minified 0.9
    - `*.d.ts` under an out dir → 0.9
    - lockfiles (`package-lock.json`, `pnpm-lock.yaml`, `yarn.lock`, `Cargo.lock`, `go.sum`, `poetry.lock`, `uv.lock`, `composer.lock`, `Gemfile.lock`) → Lockfile 1.0
    - `**/migrations/*.snap`, `__snapshots__/*.snap` → Generated 0.7
  - Header markers: read only the first 2 KiB / 20 lines of `Source` files, through `BoundedReader`, and consider only comment lines (`^\s*(//|#|/\*|\*|<!--|--)`):
    - canonical, 0.95: `@generated`; Go `^// Code generated .* DO NOT EDIT\.$`; `This file is automatically generated`; `AUTO-GENERATED FILE`
    - loose, 0.7, only when the same line also contains `generat`: `DO NOT EDIT`, `auto-generated`, `autogenerated`
    - `generator_hint` = capture group from `Code generated by (\S+)`.
  - `.gitattributes`: read the root and nested files (≤ 64 KiB each) and compile `path-pattern attr` lines with gitattributes glob semantics (`gix-attributes` via gix, or `globset` with `**/` prefixing for patterns without `/`). `-linguist-generated` / `linguist-generated=false` → force not generated (a ConfigExclude-equivalent at gitattributes level).
  - Precedence implementation: evaluate all sources, then pick by fixed precedence index. The winning reason is kept and losing reasons are discarded (deterministic and explainable).
- **Data model changes:** None here. `symbols.is_generated` (GS) is filled from this output by IDX.
- **API/protocol changes:** `repository::generated::{classify_generated, GeneratedFacts}`.
- **Concurrency semantics:** Header scanning runs in parallel (rayon) with a sorted merge.
- **Failure behavior:** Unreadable file → no header classification and warning `unreadable_entry`. Invalid config glob → warning `invalid_generated_glob`, glob skipped.
- **Idempotency considerations:** Pure, `BTreeMap` output.
- **Security considerations:** Only 2 KiB prefixes are read, through `BoundedReader` (sensitive files refused).
- **Observability additions:** span `init.generated` (attrs `generated.files`, `generated.dirs`, `generated.by_reason`). Metric `init_generated_files_total{kind,reason}`.
- **Tests required** (`tests/generated.rs`, fixture `init-edge-cases`):
  - `dist_dir_generated`
  - `ts_out_dir_generated`
  - `vendor_dir_vendored`
  - `at_generated_header_canonical`
  - `go_code_generated_marker_with_hint`
  - `do_not_edit_without_generated_word_not_flagged` (the false-positive file)
  - `header_outside_comment_ignored`
  - `lockfiles_classified`
  - `min_js_minified`
  - `gitattributes_linguist_generated_true_and_false`
  - `config_exclude_beats_header`
  - `config_include_glob`
  - `precedence_reason_is_explainable` (snapshot of the reasons map)
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** `engine/scripts/cargo.sh test -p repository --test generated` passes. The false-positive fixture file is **not** generated. The `@generated` client is generated with `reason = Header{marker:"@generated", line:1}`.
- **Definition of done:** Tests green. `GeneratedFacts` is serialized in `repository.json` (INIT-011). Global DoD met.

---

---

### INIT-009 — Entrypoints, migration dirs, infra config, env files (names only)

Status: â

> **Implementation note:** InitWarning gained a severity field (info/warning/high) because the spec's warning shape and env_file_committed require it. tracked_in_git comes from repository::git::tracked_among (index lookup through gix, no subprocess). Entrypoints are deduplicated per (path, kind) keeping the strongest source, so a bootstrap-call hit hides the weaker script/Dockerfile source for the same file. The nest-api fixture does not exist yet; tests use inline trees, monorepo-pnpm and init-edge-cases (canaries) instead.

- **Task ID:** INIT-009
- **Title:** Entrypoints (main.ts/bin/scripts/workers), migration dirs, infra config (Dockerfile, compose, terraform), env files (names only, never values)
- **Problem:** Impact analysis needs process roots (HTTP server, workers, CLIs) to say "this change reaches the API". Risk needs migration and infra locations. Configuration review needs to know which env variables exist, without ever reading secret values.
- **Why it exists:** PRD §13 #11, #13, #14, #15, #16. PRD §110/§111: secrets are never read or sent. RISK path rules consume migrations and infra.
- **Scope:**
  - Entrypoints:
    - `package.json` `main`/`bin`/`exports["."]`
    - `script_entry_hints` mapped from out dir back to source dir
    - `nest-cli.json` `entryFile`/`sourceRoot`/`projects`
    - conventional files
    - text-scan for Nest bootstrap calls, BullMQ `new Worker(`, and `Dockerfile` `CMD`/`ENTRYPOINT`, `Procfile`, `serverless.yml` handlers
  - Migration dirs and schema files.
  - Infra config files.
  - Env files: names, kind (template vs real) and tracked-in-git flag. For **templates only**, variable names.
  - The sensitive-file list (names only), surfaced from INIT-002.
- **Explicit non-scope:** Parsing TypeORM DataSource objects semantically (TSA/NEST do that at index time). Reading real `.env` files in any way. Terraform resource graphs. Kubernetes manifest semantics beyond `kind`.
- **Files/modules expected to change:** `engine/crates/repository/src/lib.rs`.
- **New files/modules expected:** `engine/crates/repository/src/entrypoints.rs`, `src/migrations.rs`, `src/infra.rs`, `src/env_files.rs`, `tests/entrypoints_infra.rs`. Fixture additions in `nest-api` (`src/main.ts` with `NestFactory.create`, `src/worker.ts` with `createApplicationContext`, `scripts/seed.ts`, `schema/migrations/20260101_init.sql`, `Dockerfile`, `docker-compose.yml`, `infra/main.tf`, `.env.example`, a tracked `.env.test` containing canary `RG_CANARY_ENV_77`).
- **Dependencies:** INIT-004 (manifests, entry hints), INIT-007 (tsconfig `outDir`/`rootDir` for dist→src mapping), INIT-002 (sensitive classification).
- **Implementation details:**
  - Types:
    ```rust
    pub struct EntrypointFact { pub path: RepoPath, pub kind: EntrypointKind /* HttpServer|Worker|Microservice|Cli|Script|Library|Serverless */, pub source: EntrySource /* PackageMain|PackageBin{name}|Script{name}|NestCli|Bootstrap{call}|Conventional|Dockerfile|Procfile|Serverless */, pub confidence: f32, pub package: Option<String> }
    pub struct MigrationDirFact { pub dir: RepoPath, pub tool_hint: Option<MigrationTool> /* TypeOrm|Prisma|Knex|Flyway|Liquibase|RawSql|Alembic|GoMigrate */, pub files: u64, pub naming: MigrationNaming /* TimestampPrefix|VersionPrefix|Other */ }
    pub struct InfraFact { pub path: RepoPath, pub kind: InfraKind /* Dockerfile|Compose|Terraform|Helm|Kubernetes|Serverless|CloudBuild|AppEngine|Fly|Vercel|Netlify|DockerIgnore */ }
    pub struct EnvFileFact { pub path: RepoPath, pub kind: EnvFileKind /* Template|Real */, pub tracked_in_git: bool, pub variable_names: Vec<String> /* Template only; Real => always empty */ }
    ```
  - Dist→src mapping: the hint `dist/main` or `dist/main.js` → strip the `outDir` prefix of the owning tsconfig, prepend `rootDir` (or `src` when `rootDir` is absent), and probe `.ts`, `.tsx`, `.js` in the inventory. the reference consumer's `node dist/main` → `src/main.ts`.
  - Bootstrap text-scan over TS/JS `Source` files ≤ 256 KiB, as a byte search (`memchr::memmem`) before any regex. Needles and kinds:
    - `NestFactory.create(` → HttpServer 0.9
    - `NestFactory.createApplicationContext(` → Worker 0.8
    - `NestFactory.createMicroservice(` → Microservice 0.9
    - `new Worker(` together with an `import ... from 'bullmq'` in the same file → Worker 0.8
    - `.listen(` with `express()` in the same file → HttpServer 0.7
    - `#!` shebang + `bin/` dir → Cli 0.9
  - Conventional files: `src/main.ts`, `src/index.ts`, `src/server.ts`, `src/app.ts`, `index.{ts,js}`, at 0.5, and only if not found by a stronger source.
  - `scripts/**/*.{ts,js}` → Script 0.6.
  - Dockerfile: parse `CMD`/`ENTRYPOINT` in exec or shell form and take path-like tokens (same regex as INIT-004) → mapped as above, kind HttpServer unless a worker keyword (`worker`, `queue`, `consumer`) is in the path.
  - Migration dirs:
    - directory names `migrations`, `migration`, `db/migrate`, `prisma/migrations`, `schema/migrations` (reference), `alembic/versions`, `db/migration` (Flyway, `V\d+__.*\.sql`)
    - any dir with ≥3 `.sql` files whose names start with a timestamp `^\d{8,14}[_-]`
    - Schema files: `schema.prisma`, `*.sql` with `schema` in the basename, `**/schema/*.sql`.
  - Infra:
    - `Dockerfile*`, `*.dockerfile`
    - `docker-compose*.y{a,}ml`, `compose.y{a,}ml`
    - `*.tf`; `Chart.yaml` → Helm
    - YAML files with both `apiVersion:` and `kind:` in the first 1 KiB → Kubernetes
    - `serverless.y{a,}ml`, `cloudbuild.y{a,}ml`, `app.yaml`, `fly.toml`, `vercel.json`, `netlify.toml`, `.dockerignore`
  - Env files:
    - Real files are `Sensitive` (INIT-002) and are **never opened**; only `path`, `kind = Real` and `tracked_in_git` (from the git index via INIT-001's `GitState`, via `repo.index()?.entry_by_path`) are recorded.
    - A tracked real env file → warning `env_file_committed` (severity high).
    - Templates are read through `BoundedReader` (≤ 64 KiB). Only the key group of `^\s*(?:export\s+)?([A-Za-z_][A-Za-z0-9_]*)\s*=` is kept. The value is discarded inside the regex loop and never stored.
- **Data model changes:** None.
- **API/protocol changes:** `repository::{entrypoints, migrations, infra, env_files}` public functions returning the facts above.
- **Concurrency semantics:** The bootstrap scan runs in parallel with rayon. The rest is single-threaded.
- **Failure behavior:** Unparseable Dockerfile or compose → the fact is still recorded with `parse_status` and a warning. Never fatal.
- **Idempotency considerations:** Pure, and sorted by path.
- **Security considerations:**
  - The defining security property of this task: **real env files and other sensitive files are never opened**. The canary test proves it by checking `repository.json`, logs and span attributes for the canary string.
  - Template values are not retained.
  - Committed secrets raise a warning but their content is not inspected (SEC-003 owns content scanning).
- **Observability additions:** span `init.entrypoints` (attrs `entrypoints.count`, `entrypoints.kinds`, `migrations.dirs`, `infra.count`, `env_files.real`, `env_files.templates`). Metric `init_env_files_committed_total`.
- **Tests required** (`tests/entrypoints_infra.rs`, fixtures `nest-api`, `monorepo-pnpm`, `init-edge-cases`):
  - `nest_bootstrap_http_server`
  - `application_context_is_worker`
  - `package_main_dist_maps_to_src`
  - `bin_entries_are_cli`
  - `conventional_main_only_when_no_stronger_source`
  - `dockerfile_cmd_mapped`
  - `migration_dir_schema_migrations_timestamped`
  - `prisma_migrations_tool_hint`
  - `infra_kinds_detected`
  - `kubernetes_yaml_by_header`
  - `env_template_keys_only`
  - `real_env_file_never_opened` (canary + a `CountingReader` asserting zero reads of `.env.test`)
  - `committed_env_file_warns`
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** `engine/scripts/cargo.sh test -p repository --test entrypoints_infra` passes. On reference (`#[ignore]`): `src/main.ts` is HttpServer; `schema/migrations` is a migration dir with `naming = TimestampPrefix`; `Dockerfile` is listed; `.env.template` is a Template whose `variable_names` is non-empty; no output field contains any value from `.env.template`.
- **Definition of done:** Tests green, including both canary tests. Global DoD met.

---

---

### INIT-010 — Rule-doc & architecture metadata discovery

Status: â

> **Implementation note:** KnowledgeVault.root is a RepoDir (a vault can be the repository root) and RuleDocCandidate.signals are Strings. The nest-api fixture does not exist yet, so tests build an inline tree that mirrors the spec fixture additions (docs/adr, .github/CODEOWNERS, .agent/acme with .obsidian, .claude settings canary).

- **Task ID:** INIT-010
- **Title:** Rule-doc & architecture metadata discovery (docs/adr, ARCHITECTURE.md, CONTRIBUTING, CODEOWNERS, knowledge vaults like .agent/*)
- **Problem:** Explicit, human-written rules outrank inferred conventions (target-architecture §3.10 precedence). PROF and POL need to know where those rules live. Teams keep them in many places: ADR folders, ARCHITECTURE.md, agent instruction files, and Obsidian-style vaults such as reference's `.agent/knowledge`.
- **Why it exists:** PRD §13 #21, #22; §65 (precedence: documented architecture > inferred); PROF-007 (vault adapter) needs vault roots.
- **Scope:**
  - ADR discovery: number, title, status.
  - Architecture docs, CONTRIBUTING, SECURITY.md, CODEOWNERS (parsed rules), PR templates.
  - Agent instruction files: `AGENTS.md`, `CLAUDE.md`, `.cursorrules`, `.cursor/rules/*.mdc`, `.github/copilot-instructions.md`, `.windsurfrules`, `SKILL.md`.
  - Rule-doc candidates with keyword scoring.
  - Knowledge vaults: `.agent/*/`, dirs containing `.obsidian/`, `.cursor/rules/`, `.claude/` (structure only).
- **Explicit non-scope:** Interpreting document content into rules (PROF/POL). Embedding docs (SEM). Reading `.claude/settings*.json` or any other config inside vault tool dirs.
- **Files/modules expected to change:** `engine/crates/repository/src/lib.rs`.
- **New files/modules expected:** `engine/crates/repository/src/docs_meta.rs`, `src/codeowners.rs`, `tests/docs_meta.rs`. Fixture additions in `nest-api` (`docs/adr/0001-use-nestjs.md`, `docs/architecture.md`, `.github/CODEOWNERS`, `AGENTS.md`, `.agent/acme/{index.md,security.md}`, `.agent/acme/.obsidian/app.json`).
- **Dependencies:** INIT-002.
- **Implementation details:**
  - Types:
    ```rust
    pub struct DocsFacts { pub adrs: Vec<AdrFact>, pub architecture: Vec<RepoPath>, pub contributing: Vec<RepoPath>, pub security_policy: Vec<RepoPath>, pub pr_templates: Vec<RepoPath>,
        pub agent_instructions: Vec<RepoPath>, pub rule_docs: Vec<RuleDocCandidate>, pub codeowners: Option<CodeownersFact>, pub knowledge_vaults: Vec<KnowledgeVault> }
    pub struct AdrFact { pub path: RepoPath, pub number: Option<u32>, pub title: Option<String>, pub status: Option<String> }
    pub struct RuleDocCandidate { pub path: RepoPath, pub score: f32, pub signals: Vec<&'static str> }
    pub struct CodeownersFact { pub path: RepoPath, pub rules: Vec<(String /*pattern*/, Vec<String> /*owners*/)>, pub invalid_lines: Vec<u32> }
    pub struct KnowledgeVault { pub root: RepoPath, pub kind: VaultKind /* AgentVault|Obsidian|CursorRules|ClaudeDir */, pub markdown_files: u64 }
    ```
  - ADR dirs: `docs/adr`, `docs/adrs`, `docs/decisions`, `doc/adr`, `adr`, `architecture/decisions`, plus any dir with ≥2 files matching `^(ADR[-_ ]?)?\d{3,4}[-_].*\.md$`. `number` comes from that match. `title` is the first `# ` heading in the first 4 KiB. `status` matches `(?i)^\**status:?\**\s*:?\s*(\w+)` in the first 40 lines.
  - Architecture: `ARCHITECTURE.md`, `docs/architecture*.md`, `docs/architecture/**/*.md`, `docs/system-overview.md`, `docs/design/**/*.md`. CONTRIBUTING: `CONTRIBUTING.md`, `.github/CONTRIBUTING.md`, `docs/CONTRIBUTING.md`.
  - CODEOWNERS, first found in `.github/`, root, `docs/` (GitHub's order). Line grammar: `pattern owner+`, `#` comments. Owners must match `@[\w-]+(/[\w-]+)?` or an email. Other lines → `invalid_lines`.
  - Rule-doc score over Markdown files ≤ 512 KiB in `docs/`, `rules/`, root and vault roots. Weights sum, capped at 1.0:
    - basename keywords (guideline, convention, standard, rule, style, security, testing, coding, review, policy) 0.4
    - located in `rules/` or `docs/` 0.2
    - agent instruction file 0.5
    - ≥3 imperative markers (`must`, `never`, `always`, `do not`) in the first 4 KiB 0.2

    Candidates with score ≥ 0.4 are kept, sorted by score desc then path, capped at 200.
  - Vaults:
    - every immediate child dir of `.agent/` → AgentVault
    - any dir containing a `.obsidian/` child → Obsidian vault rooted at that dir
    - `.cursor/rules` → CursorRules
    - `.claude/` → ClaudeDir, counting `*.md` only; `settings*.json` is not read
    - `markdown_files` = count of `*.md` under the root, from the inventory
  - Note: `.obsidian/` and `.claude/` contents are walked by INIT-002 (hidden files are walked), but only Markdown files are ever read here.
- **Data model changes:** None.
- **API/protocol changes:** `repository::docs_meta::{detect_docs, DocsFacts}`.
- **Concurrency semantics:** None.
- **Failure behavior:** Unreadable doc → listed without title and with a warning. Invalid CODEOWNERS lines are recorded, not fatal.
- **Idempotency considerations:** Pure and sorted.
- **Security considerations:**
  - Only Markdown prefixes are read; no JSON configs inside tool dirs.
  - CODEOWNERS handles are repository-public metadata. They are stored as written, never enriched with any external lookup.
- **Observability additions:** span `init.docs` (attrs `docs.adrs`, `docs.rule_docs`, `docs.vaults`, `docs.codeowners_rules`).
- **Tests required** (`tests/docs_meta.rs`, fixture `nest-api`):
  - `adr_number_title_status_extracted`
  - `adr_dir_inferred_from_numbered_files`
  - `architecture_docs_found`
  - `codeowners_github_location_precedence`
  - `codeowners_invalid_lines_recorded`
  - `agent_instruction_files_listed`
  - `rule_doc_scoring_and_cap`
  - `agent_vault_and_obsidian_vault_detected`
  - `claude_settings_not_read` (canary)
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** `engine/scripts/cargo.sh test -p repository --test docs_meta` passes. On reference (`#[ignore]`): `knowledge_vaults` contains `.agent/knowledge` (AgentVault); `docs.architecture` contains `docs/architecture.md`; `agent_instructions` contains `AGENTS.md` and `CLAUDE.md`; ADRs from `docs/decisions` are listed.
- **Definition of done:** Tests green. Global DoD met.

---

---

### INIT-011 — `.review/` layout + `repository.json` writer (RepositoryFacts)

Status: â

> **Implementation note:** The contract is exported through the existing review-cli registry as RepositoryFacts.schema.json (plus generated TypeScript) instead of a hand-named repository-facts.v1.schema.json; schema_matches_committed_contract compares it structurally. fd-lock was replaced by fs2 (advisory flock through a plain File) so no unsafe code is needed. InitWarning carries severity and the facts add warnings_truncated. .review/ changes never make the worktree dirty (git.rs ignores that directory) so a second init on a clean HEAD is UpToDate even though .review/config.yaml and .gitignore are untracked. nest-api does not exist yet, so the insta snapshot covers monorepo-pnpm and the pinned hash covers init-basic (copied to a stable directory name). The init_dump example and the review-cli dependency on repository are included. benches/init.rs was not added.

- **Task ID:** INIT-011
- **Title:** `.review/` layout + repository.json writer (RepositoryFacts) per PRD §14
- **Problem:** The detectors produce separate fact sets. Nothing yet composes them into one versioned, deterministic document, and nothing creates the persistent local state directory that the CLI, the file graph store (GS-006) and later phases write into.
- **Why it exists:** PRD §12 (`review init`, `--force`), §14 (layout is an implementation detail but the logical model is the contract). The `RepositoryFacts` JSON Schema becomes a shared contract in `packages/contracts` (target-architecture §1).
- **Scope:**
  - The `RepositoryFacts` aggregate type plus its JSON Schema (via `schemars`).
  - Orchestrator `repository::init::run`, which runs detectors INIT-001..010 in order and collects warnings.
  - `.review/` directory layout creation.
  - Starter `config.yaml`, written only if absent.
  - `.review/.gitignore`.
  - Atomic `repository.json` write.
  - Advisory lock.
  - "Up to date" short-circuit and `--force`.
  - `facts_hash`.
  - Example binary `init_dump` for manual verification.
- **Explicit non-scope:** The `review init` CLI command surface and output formatting (CLI-002). Profile files under `.review/profile/` (PROF). Graph snapshot files (GS-006). Fingerprint computation (INIT-012, which fills `fingerprint`). Remote storage (INIT-013).
- **Files/modules expected to change:** `engine/crates/repository/src/lib.rs`, `Cargo.toml` (`schemars`, `fd-lock` or `fs4`, `tempfile`), `packages/contracts/` (generated schema file; the generation script comes from FND/DOM).
- **New files/modules expected:**
  - `engine/crates/repository/src/facts.rs`, `src/init.rs`, `src/review_dir.rs`
  - `engine/crates/repository/examples/init_dump.rs`
  - `engine/crates/repository/tests/init_end_to_end.rs`
  - `packages/contracts/schemas/repository-facts.v1.schema.json` (generated)
  - Snapshot dir `engine/crates/repository/tests/snapshots/`
- **Dependencies:** INIT-001 … INIT-010.
- **Implementation details:**
  - Aggregate:
    ```rust
    #[derive(Serialize, Deserialize, JsonSchema, PartialEq, Debug)]
    pub struct RepositoryFacts {
        pub schema_version: u32,              // = REPOSITORY_FACTS_SCHEMA (1)
        pub tool_version: String,             // env!("CARGO_PKG_VERSION") of repository crate
        pub detected_at: String,              // RFC 3339 UTC; EXCLUDED from facts_hash
        pub root_name: String,                // basename only; absolute host paths are never stored
        pub git: Option<GitStateDto>,
        pub inventory: InventorySummary,      // totals by class/reason; NOT the per-file list
        pub languages: Vec<LanguageStat>, pub primary_language: Option<Language>,
        pub package_managers: Vec<PackageManagerFact>, pub manifests: Vec<ManifestFact>, pub build_systems: Vec<BuildSystemFact>,
        pub workspaces: WorkspaceLayout, pub frameworks: Vec<FrameworkFact>, pub auth: AuthFacts,
        pub layout: LayoutFacts, pub tsconfigs: Vec<TsConfigFact>, pub tooling: ToolingFacts,
        pub generated: GeneratedSummary,      // dirs + counts + config globs; per-file map goes to .review/cache/generated.json
        pub entrypoints: Vec<EntrypointFact>, pub migrations: Vec<MigrationDirFact>, pub schema_files: Vec<RepoPath>,
        pub infra: Vec<InfraFact>, pub env_files: Vec<EnvFileFact>, pub sensitive_files: Vec<RepoPath>,
        pub docs: DocsFacts,
        pub api_routes: DeferredToIndex,      // { status: "deferred_to_index" } until IDX writes a summary
        pub fingerprint: Option<String>,      // INIT-012
        pub facts_hash: String,               // blake3 of canonical JSON with detected_at, facts_hash, fingerprint blanked
        pub warnings: Vec<InitWarning>,       // { code, severity, path?, message } sorted by (code, path)
    }
    ```
  - Size caps keep the document ≤ 1 MiB:
    - `manifests[].dependencies` ≤ 2,000 per manifest
    - `env_files[].variable_names` ≤ 500
    - `warnings` ≤ 1,000, plus a `warnings_truncated` count
    - `rule_docs` ≤ 200

    Every truncation is recorded in `warnings` (`list_truncated`). Truncation is never silent.
  - Orchestrator:
    ```rust
    pub struct InitOptions { pub root: PathBuf, pub force: bool, pub allow_non_git: bool, pub provider_default_branch: Option<String>,
        pub walk: WalkOptions, pub analyzable_languages: Vec<Language>, pub write_review_dir: bool /* false for worker mode */ }
    pub enum InitOutcome { Written { facts: RepositoryFacts, path: PathBuf }, UpToDate { facts: RepositoryFacts }, Computed { facts: RepositoryFacts } /* write_review_dir=false */ }
    pub fn run(opts: &InitOptions) -> Result<InitOutcome, InitError>;
    ```
    Order: git → walk (reading `.review/config.yaml` `ignore:` and `generated:` first, if present) → languages → manifests → build systems → workspaces → frameworks → tsconfig/layout/tooling → generated → entrypoints/migrations/infra/env → docs → assemble → (INIT-012 fingerprint, called by the caller) → write.
  - "Up to date": when `!force` and an existing `repository.json` parses with the same `schema_version`, `tool_version` and `git.head.sha`, and `git.dirty.is_dirty == false`, return `UpToDate` without rewriting. Any parse failure of the existing file → treated as absent (warning `repository_json_unreadable`).
  - Layout created (idempotent `create_dir_all`):
    ```
    .review/{config.yaml?, repository.json, .gitignore, .lock,
             graph/{nodes,edges,indexes,metadata,snapshots}, ast, symbols, semantic,
             profile, snapshots, history, cache}
    ```
    `.review/.gitignore` contents: `*`, `!.gitignore`, `!config.yaml`. Only the config is meant to be committed.
  - Starter `config.yaml`: a commented template with `version: 1` and empty `ignore`, `generated`, `reviewers`, `confidence`, `rules` sections (the shape POL-001 will own). Written only when absent. **`--force` never overwrites `config.yaml`.** It is user-owned. `--force` recomputes facts only.
  - Atomic write:
    1. `tempfile::NamedTempFile::new_in(.review/)`
    2. write pretty JSON (2-space indent, struct field order, trailing `\n`)
    3. `sync_all`
    4. `persist(repository.json)` (rename)
    5. fsync the dir on Unix
  - Lock: exclusive non-blocking `fd-lock` on `.review/.lock`. If held → `InitError::Busy` (the CLI maps it to "another init is running").
  - `facts_hash` = `blake3("rg.facts.v1\0" ‖ canonical_json(facts with detected_at="", facts_hash="", fingerprint=null))`. Canonical JSON uses `serde_json` with struct order and sorted maps (already `BTreeMap`). A unit test pins one fixture's hash so accidental field reordering is caught (the snapshot is updated deliberately on schema bumps).
  - Schema export test: `schemars::schema_for!(RepositoryFacts)` is compared with the committed `packages/contracts/schemas/repository-facts.v1.schema.json`. A mismatch fails the test. Regenerate with `UPDATE_SCHEMAS=1`.
  - `examples/init_dump.rs`: `cargo.sh run -p repository --example init_dump -- <path> [--no-write]` prints the facts JSON. This is the verification tool until CLI-002 exists.
- **Data model changes:** New on-disk format `.review/repository.json` v1 and the `.review/` layout. No DB changes.
- **API/protocol changes:** `repository::init::{run, InitOptions, InitOutcome}`, `repository::facts::RepositoryFacts`. New contract `repository-facts.v1.schema.json`.
- **Concurrency semantics:** Single init per `.review/` via the advisory lock. Detectors run sequentially, each internally parallel. The atomic rename guarantees readers see either the old or the new `repository.json`, never a partial one.
- **Failure behavior:**
  - Any detector warning → collected, and the run continues.
  - Fatal only for: git open failure (when not allowed), walk fatal errors (`TooManyFiles`, root unreadable), lock busy, `.review/` not writable (`InitError::ReviewDirNotWritable`).
  - On a write failure the temp file is removed and the existing `repository.json` is untouched.
- **Idempotency considerations:**
  - Two runs on an unchanged repo produce identical `facts_hash`. The second run returns `UpToDate`.
  - `--force` rewrites with the same `facts_hash`; only `detected_at` differs.
- **Security considerations:**
  - No absolute host paths in `repository.json`: `root_name` only, and `RepoPath` everywhere.
  - No raw remote URLs (`RedactedUrl` only).
  - No env values, no script commands, no sensitive-file contents.
  - The per-file generated map is kept in `.review/cache/` because it can be large, and it is gitignored.
  - File mode `0644`; `.review/` is created `0755`.
- **Observability additions:** parent span `repository_init` (target-architecture §8 `repository_index` family; attrs `init.outcome`, `init.warnings`, `init.facts_hash`, `repository.primary_language`) with child spans from INIT-001..010 and `init.write_facts`. Metrics `init_duration_seconds` (histogram, label `outcome`) and `init_runs_total{outcome}`.
- **Tests required** (`tests/init_end_to_end.rs`; fixtures `init-basic`, `nest-api`, `monorepo-pnpm`, `init-edge-cases`):
  - `init_creates_full_review_layout`
  - `review_gitignore_ignores_all_but_config`
  - `config_yaml_written_only_when_absent`
  - `force_never_overwrites_config_yaml`
  - `second_run_is_up_to_date`
  - `dirty_worktree_never_up_to_date`
  - `force_recomputes_same_facts_hash`
  - `concurrent_init_second_gets_busy`
  - `atomic_write_leaves_old_file_on_failure` (read-only dir injection)
  - `no_absolute_paths_in_output` (asserts no string field starts with `/` or matches `^[A-Za-z]:\\`)
  - `canaries_absent_from_repository_json` (all canaries from INIT-002/009/010)
  - `list_truncation_is_recorded`
  - `schema_matches_committed_contract`
  - `facts_hash_pinned_for_init_basic`
  - insta snapshots `repository_json__nest_api` and `repository_json__monorepo_pnpm`, redacting `detected_at`, `tool_version` and `git.head.sha`
- **Benchmarks if applicable:** `benches/init.rs`: full `run` on `nest-api` and on a 20k-file generated tree. Targets: `nest-api` < 300 ms; on reference (manual, `#[ignore]`) < 2 s warm. The results are recorded in the PR description.
- **Acceptance criteria:**
  - `engine/scripts/cargo.sh test -p repository --test init_end_to_end` passes.
  - `engine/scripts/cargo.sh run -p repository --example init_dump -- fixtures/.build/nest-api` exits 0 and prints JSON with `schema_version == 1`.
  - `git -C fixtures/.build/nest-api status --porcelain` lists nothing under `.review/` except `.review/config.yaml` and `.review/.gitignore`.
  - On reference (`#[ignore]`), every PRD §13 row of the §0.6 matrix is non-empty or explicitly `deferred_to_index`.
- **Definition of done:** Tests and snapshots green. The contract schema is committed. The §0.6 coverage matrix is verified by a test (`prd_13_coverage_fields_present`). Global DoD met.

---

---

### INIT-012 — Repository fingerprint (ADR-015)

Status: ☐

- **Task ID:** INIT-012
- **Title:** Repository fingerprint (ADR-015)
- **Problem:** Caches, snapshots and reproducibility checks need one value that changes exactly when any input that affects derived intelligence changes. That covers the commit, analyzer versions, graph schema, config, parser versions and profile version. It must also be unambiguous: different inputs must never concatenate to the same bytes.
- **Why it exists:** PRD §15. ADR-015 defines it as the snapshot's cache-validity key. `snapshots.fingerprint` (target-architecture §3.4) and `review status` expose it.
- **Scope:**
  - `FingerprintInputs` and `compute_fingerprint()`.
  - `config_hash` computation (normalized `.review/config.yaml`, the effective tsconfig set, `.reviewignore` files, workspace definitions).
  - Dirty-worktree commit component.
  - Local repository id derivation for CLI mode.
  - `parser_versions` constant with a Cargo.lock consistency test (in `lang-typescript`).
- **Explicit non-scope:** Deciding what a fingerprint change invalidates (INC-011 owns rebuild triggers). Model-derived artifact versions (`prompt_version` etc.), which belong to review-level provenance (GW/REV).
- **Files/modules expected to change:** `engine/crates/repository/src/facts.rs` (fill `fingerprint`), `engine/crates/lang-typescript/src/lib.rs` (export `PARSER_VERSIONS`; if the crate does not exist yet, the constant is added when TSA-002 lands and the test is added there).
- **New files/modules expected:** `engine/crates/repository/src/fingerprint.rs`, `src/config_hash.rs`, `tests/fingerprint.rs`, `engine/crates/lang-typescript/tests/parser_versions.rs`.
- **Dependencies:** INIT-011 (facts), INIT-007 (tsconfig set), DOM-003 (version types `AnalyzerVersion`, `GraphSchemaVersion`, `ProfileVersion`).
- **Implementation details:**
  - API:
    ```rust
    pub struct FingerprintInputs<'a> {
        pub repository_id: &'a RepositoryId,             // hosted: UUID; CLI: RepositoryId::Local(hex)
        pub commit: CommitComponent,                      // Clean(CommitSha) | Dirty { head: CommitSha, worktree_hash: [u8; 32] } | NoGit { tree_hash: [u8; 32] }
        pub analyzer_versions: &'a BTreeMap<Language, AnalyzerVersion>,
        pub graph_schema_version: u32,
        pub config_hash: [u8; 32],
        pub parser_versions: &'a [(&'static str, &'static str)], // sorted by name
        pub profile_version: u32,
    }
    pub struct Fingerprint([u8; 32]);   // Display: "fp1:" + 64 lowercase hex
    pub fn compute_fingerprint(i: &FingerprintInputs) -> Fingerprint;
    ```
  - Encoding: `blake3::Hasher` keyed by nothing, starting with domain `b"rg.fp.v1\0"`. Each field is written as `tag: u8`, `len: u64 LE`, `bytes`. Tags, in fixed order: 1 repository_id, 2 commit (`clean:<sha>` / `dirty:<sha>:<hex>` / `nogit:<hex>`), 3 analyzer_versions (`lang=semver\n` lines sorted by language id tag), 4 graph_schema_version (u32 LE), 5 config_hash, 6 parser_versions (`name=version\n`), 7 profile_version (u32 LE). This is the ADR-015 field list in ADR order, made injective by length prefixes.
  - `worktree_hash` for dirty trees: blake3 over sorted `(path, blake3(content) | "deleted")` for every modified, staged or untracked file reported by INIT-001 (full list, not the 20-sample). Content is read through `BoundedReader`. Sensitive files contribute `sensitive:<size>:<mtime-free>`, i.e. only path and size, never content.
  - `config_hash` = blake3 over, in order:
    - (a) `.review/config.yaml` parsed and re-serialized as canonical JSON (sorted keys). Comments and formatting are therefore ignored. Absent → the literal `none`.
    - (b) for each tsconfig in `TsConfigSet` sorted by path: `path` + canonical JSON of `TsEffectiveOptions`.
    - (c) each `.reviewignore` path + raw bytes.
    - (d) workspace definition sources (`pnpm-workspace.yaml`, root `package.json` `workspaces` value, `nx.json`, Cargo `[workspace]`), as canonical JSON.

    Each part is length-prefixed as above. Because `TsEffectiveOptions` is post-merge, a change to a base tsconfig in an `extends` chain changes the hash.
  - Local repository id (CLI): `Local(hex(blake3("rg.repo.local.v1\0" ‖ normalized_origin_url)))[..32]`, where the normalized URL is a lowercase host plus path without `.git` (from `RedactedUrl`). Without a remote, use the canonical root path; record a warning `repository_id_from_path`, since such an id is not portable.
  - `lang_typescript::PARSER_VERSIONS = &[("tree-sitter", "0.25.x"), ("tree-sitter-typescript", "0.23.x")]`, filled with exact versions. The `parser_versions.rs` test parses `engine/Cargo.lock` (via `toml`) and asserts the exact versions match. It fails the build when a dependency bump forgets the constant.
- **Data model changes:** None in this task. `snapshots.fingerprint text` already exists in the GS schema. `repository_init_facts.fingerprint` is added in INIT-013.
- **API/protocol changes:** `repository::fingerprint::{compute_fingerprint, FingerprintInputs, Fingerprint, CommitComponent}`, `repository::config_hash::compute_config_hash`.
- **Concurrency semantics:** Pure functions. The dirty hash reads files in parallel (rayon) and sorts before hashing.
- **Failure behavior:** An unreadable dirty file contributes `unreadable:<path>` and a warning, so the fingerprint stays defined and still differs from clean. A malformed config.yaml contributes its raw bytes instead of canonical JSON, with warning `config_yaml_parse`.
- **Idempotency considerations:** Deterministic. Golden test vectors pin outputs.
- **Security considerations:** Sensitive file content never enters the hash, so the hash cannot be used as an oracle for secret values. Only path and size are used.
- **Observability additions:** span `init.fingerprint` (attrs `fingerprint` (the value, safe), `fingerprint.commit_kind`). Metric None.
- **Tests required** (`tests/fingerprint.rs`, fixture `init-basic`):
  - `golden_vector_v1` (fixed inputs → pinned hex)
  - `each_input_changes_fingerprint` (7 cases, one per field)
  - `length_prefix_prevents_concat_ambiguity` (`repo="ab", commit="c"` vs `repo="a", commit="bc"`)
  - `dirty_differs_from_clean_same_head`
  - `dirty_hash_changes_with_content`
  - `sensitive_file_content_not_hashed` (change the content, keep the size → same hash)
  - `config_yaml_formatting_and_comments_ignored`
  - `base_tsconfig_change_changes_config_hash`
  - `reviewignore_change_changes_config_hash`
  - `local_repository_id_stable_across_clone_locations`
  - `parser_versions_match_cargo_lock` (in lang-typescript)
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** `engine/scripts/cargo.sh test -p repository --test fingerprint` and `engine/scripts/cargo.sh test -p lang-typescript --test parser_versions` pass. `init_dump` on `nest-api` prints a `fingerprint` matching `^fp1:[0-9a-f]{64}$`, identical across two runs.
- **Definition of done:** Tests green. Golden vector committed. ADR-015 updated in the same change with the exact encoding (tags and length prefixes) if it differs from the ADR text. Global DoD met.

---

---

### INIT-013 — Persist init facts to PostgreSQL via a RepositoryFactsStore port

Status: ☐

- **Task ID:** INIT-013
- **Title:** Persist init facts to PostgreSQL (repository_init_facts table / repositories columns) via a RepositoryFactsStore port
- **Problem:** In hosted mode, workers run init on ephemeral checkouts. The API, the UI (Repository Intelligence screen) and later stages need the latest facts without re-running detection. Facts must be tenant-scoped and versioned per commit.
- **Why it exists:** PRD §14 ("production hosted deployments may store these objects remotely"), §106 (repository status API). ADR-014: migrations are the only schema source. Master plan §13: RLS on tenant tables.
- **Scope:**
  - `RepositoryFactsStore` port trait in `repository`.
  - Postgres adapter in `graph-storage`.
  - A file adapter (wraps INIT-011's `repository.json`) so the CLI and the worker share one interface.
  - sqlx migration: a `repository_init_facts` table, columns on `repositories`, and an RLS policy.
  - The adapter conformance test suite run against both adapters.
- **Explicit non-scope:** API endpoints exposing facts (API-009/010). Worker job wiring (PIPE). Retention and cleanup of old facts rows (SEC-007).
- **Files/modules expected to change:** `engine/crates/repository/src/lib.rs`, `engine/crates/graph-storage/Cargo.toml` (dep on `repository` for the facts type, allowed by §2.1 since `repository` depends only on `review-core`), `engine/crates/graph-storage/src/lib.rs`.
- **New files/modules expected:**
  - `engine/crates/repository/src/store.rs` (port + `FileFactsStore`)
  - `engine/crates/graph-storage/src/pg/repository_facts.rs`
  - `engine/migrations/<timestamp>_repository_init_facts.sql` (timestamp at implementation time, ordered after the DOM-009 base schema)
  - `engine/crates/graph-storage/tests/repository_facts_store.rs`
  - `engine/crates/repository/tests/store_conformance.rs` (the shared suite as a generic fn `run_conformance<S: RepositoryFactsStore>(make: impl Fn() -> S)`, exported under the `test-support` feature)
- **Dependencies:** INIT-011, INIT-012, DOM-009 (migrations framework + `repositories`/`organizations` tables), GS-001 (graph-storage crate skeleton and PG pool; if not yet present, this task creates `graph-storage` with only this module and the pool).
- **Implementation details:**
  - Port:
    ```rust
    pub struct RepoScope { pub organization_id: OrganizationId, pub repository_id: RepositoryId }
    #[async_trait::async_trait] // or native async fn in traits (Rust ≥1.75) with Send bounds via trait-variant
    pub trait RepositoryFactsStore: Send + Sync {
        async fn save(&self, scope: &RepoScope, facts: &RepositoryFacts) -> Result<SavedFacts, FactsStoreError>;
        async fn latest(&self, scope: &RepoScope) -> Result<Option<StoredFacts>, FactsStoreError>;
        async fn by_commit(&self, scope: &RepoScope, commit_sha: &CommitSha) -> Result<Option<StoredFacts>, FactsStoreError>;
    }
    pub struct SavedFacts { pub id: FactsId, pub created: bool /* false = identical row existed */ }
    pub struct StoredFacts { pub id: FactsId, pub facts: RepositoryFacts, pub created_at: OffsetDateTime }
    pub enum FactsStoreError { TooLarge{bytes}, SchemaUnsupported{version}, Conflict, Backend(String) }
    ```
  - Migration:
    ```sql
    CREATE TABLE repository_init_facts (
      id                   uuid PRIMARY KEY DEFAULT gen_random_uuid(),
      organization_id      uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
      repository_id        uuid NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
      commit_sha           text NOT NULL,
      facts_schema_version int  NOT NULL,
      tool_version         text NOT NULL,
      facts_hash           text NOT NULL,
      fingerprint          text,
      primary_language     text,
      is_monorepo          boolean NOT NULL,
      frameworks           text[] NOT NULL DEFAULT '{}',
      warnings_count       int  NOT NULL DEFAULT 0,
      facts                jsonb NOT NULL,
      detected_at          timestamptz NOT NULL,
      created_at           timestamptz NOT NULL DEFAULT now(),
      CONSTRAINT repository_init_facts_dedup UNIQUE (repository_id, commit_sha, facts_hash),
      CONSTRAINT repository_init_facts_size CHECK (pg_column_size(facts) <= 1048576)
    );
    CREATE INDEX repository_init_facts_latest ON repository_init_facts (repository_id, detected_at DESC);
    ALTER TABLE repository_init_facts ENABLE ROW LEVEL SECURITY;
    CREATE POLICY repository_init_facts_tenant ON repository_init_facts
      USING (organization_id = current_setting('app.organization_id', true)::uuid);
    ALTER TABLE repositories
      ADD COLUMN latest_init_facts_id uuid REFERENCES repository_init_facts(id) ON DELETE SET NULL,
      ADD COLUMN default_branch text,
      ADD COLUMN primary_language text,
      ADD COLUMN initialized_at timestamptz;
    ```
  - `save`, in one transaction:
    1. `SET LOCAL app.organization_id = $org`.
    2. `INSERT ... ON CONFLICT ON CONSTRAINT repository_init_facts_dedup DO NOTHING RETURNING id`. If no row comes back, `SELECT id` the existing one (`created = false`).
    3. Advance the pointer only forward: `UPDATE repositories SET latest_init_facts_id=$id, default_branch=$b, primary_language=$l, initialized_at=now() WHERE id=$repo AND organization_id=$org AND (latest_init_facts_id IS NULL OR (SELECT detected_at FROM repository_init_facts WHERE id = latest_init_facts_id) <= $detected_at)`. A late-finishing older init cannot overwrite a newer one.
    - All queries use `sqlx::query!` macros, checked offline (`.sqlx/` data committed per FND conventions). There is no string interpolation.
  - Serialized size is checked before insert: `serde_json::to_vec(&facts).len() > 1 MiB` → `TooLarge`. INIT-011 caps make this unreachable in practice; the test forces it.
  - `latest` → `SELECT ... WHERE repository_id=$1 AND organization_id=$2 ORDER BY detected_at DESC LIMIT 1`. A `facts_schema_version` greater than supported → `SchemaUnsupported`. Older versions are migrated in code through `RepositoryFacts::upgrade_from(v, json)`, a no-op for v1.
  - `FileFactsStore` implements the port over `.review/repository.json` (latest only; `by_commit` returns the file if its sha matches). The scope is ignored except for being asserted non-empty.
- **Data model changes:** New table `repository_init_facts` (with RLS). New `repositories` columns `latest_init_facts_id`, `default_branch`, `primary_language`, `initialized_at`. Kysely types are regenerated by the API's codegen step (ADR-002). The CI drift check will flag that step if it is not run.
- **API/protocol changes:** New port `repository::store::RepositoryFactsStore`, adapters `graph_storage::pg::PgRepositoryFactsStore` and `repository::store::FileFactsStore`.
- **Concurrency semantics:**
  - The unique constraint makes concurrent identical saves converge on one row.
  - The forward-only pointer update is a compare-and-set on `detected_at` inside the transaction.
  - Two different commits saved concurrently → both rows exist, and the pointer ends on the later `detected_at`.
- **Failure behavior:** Typed errors. Transient PG errors (connection reset, serialization failure `40001`) are retried up to 3× with jittered backoff inside the adapter. Other errors surface to the caller (the worker job fails and is retried by the queue, PIPE-001). A partially failed transaction rolls back, leaving no orphaned rows or pointer.
- **Idempotency considerations:** `save` is idempotent on `(repository_id, commit_sha, facts_hash)`. Re-running the same init job is a no-op apart from the `created=false` result.
- **Security considerations:**
  - RLS policy plus an explicit `organization_id` predicate on every query (defence in depth, master plan §13).
  - Facts contain no secrets by construction (INIT-002/009/011 tests).
  - The adapter never logs `facts`; it logs only the id and the hash.
- **Observability additions:** span `init.persist_facts` (attrs `organization_id`, `repository_id`, `commit_sha`, `facts.created`, `facts.bytes`). Metrics `repository_facts_saves_total{created}` and `repository_facts_save_duration_seconds`.
- **Tests required:**
  - `tests/store_conformance.rs` (run against both adapters):
    - `save_then_latest_roundtrip`
    - `save_identical_twice_is_idempotent`
    - `by_commit_returns_matching`
    - `newer_detected_at_wins_pointer`
    - `older_late_save_does_not_regress_pointer`
    - `too_large_rejected`
    - `unsupported_schema_version_rejected`
  - `graph-storage/tests/repository_facts_store.rs` (docker PG from `infra/compose/docker-compose.test.yml`):
    - `rls_hides_other_org_rows` (two orgs; querying with the wrong `app.organization_id` returns none)
    - `concurrent_identical_saves_single_row` (10 tasks)
    - `migration_applies_and_reverts_cleanly` (if FND defines down-migrations; otherwise `applies_on_empty_db`)
- **Benchmarks if applicable:** None.
- **Acceptance criteria:**
  - `engine/scripts/cargo.sh test -p repository --test store_conformance` passes.
  - `engine/scripts/cargo.sh test -p graph-storage --test repository_facts_store` passes with the test compose stack up.
  - `engine/scripts/cargo.sh sqlx migrate run` (or the FND-defined migrate command) applies cleanly on an empty DB.
  - `psql -c "\d repository_init_facts"` shows RLS enabled.
- **Definition of done:** Migration committed. Both adapters pass the conformance suite. Offline sqlx query data committed. Kysely type regeneration noted for the API task owner (API-002) if `apps/api` exists. Global DoD met.

---

---

### TSA-001 — `analysis-ir` crate: IR types and analyzer traits

Status: ☐

- **Task ID:** TSA-001
- **Title:** analysis-ir crate: ParsedUnit, IrSymbol, IrReference (kinds), IrImport/IrExport, SyntaxFact, IrFrameworkFact, ParseDiagnostic; LanguageAnalyzer/FrameworkAdapter/ModuleResolver/SemanticProvider traits (ADR-006)
- **Problem:** Several components need one shared, language-neutral contract: the graph linker, the incremental engine, the diff→symbol mapper and the change classifier. Without it, each would couple to tree-sitter or TypeScript specifics, which violates Invariant 5 (PRD §95).
- **Why it exists:** ADR-006, target-architecture §3.1. It is the first node on the critical path (master plan §7). The `ParsedUnit` cache key `(path, content_hash, analyzer_version)` (target-architecture §7) depends on this being a pure, serializable value.
- **Scope:**
  - All IR types with `serde` (+ `bincode`-compatible) derives and `schemars` for documentation.
  - `SymbolKind` and `RefKind` enums with stable string forms.
  - The four traits.
  - `AnalyzerRegistry` (pick analyzer by path).
  - `FrameworkSignals`.
  - IR validation function.
  - `IR_SCHEMA_VERSION`.
- **Explicit non-scope:** Any parsing (TSA-002+). `SymbolId` formatting (SID-001; `IrSymbol` stores the components). Graph node/edge kinds (CG-001/002). Hash computation (TSA-007; the types are defined here).
- **Files/modules expected to change:** `engine/Cargo.toml` (member), `engine/crates/review-core/src/lib.rs` (`SymbolKind`, `ModulePath`, `ContentHash`, `Hash128` if absent).
- **New files/modules expected:** `engine/crates/analysis-ir/{Cargo.toml, src/lib.rs, src/unit.rs, src/symbol.rs, src/reference.rs, src/module.rs, src/facts.rs, src/framework.rs, src/diagnostic.rs, src/traits.rs, src/registry.rs, src/validate.rs}`, `tests/ir_roundtrip.rs`.
- **Dependencies:** DOM-001 (`review-core`), DOM-003 (`AnalyzerVersion`).
- **Implementation details:**
  - Core types (abridged; every type derives `Clone, Debug, PartialEq, Eq, Serialize, Deserialize`):
    ```rust
    pub const IR_SCHEMA_VERSION: u16 = 1;
    pub struct SourceInput<'a> { pub path: RepoPath, pub module_path: ModulePath /* computed by caller, see SID-001 */, pub bytes: &'a [u8], pub content_hash: ContentHash, pub is_generated: bool }
    pub struct AnalyzerConfig { pub anonymous_functions: AnonymousFnPolicy /* Attribute (default) | Emit */, pub max_file_bytes: u64 /* default 1 MiB */,
        pub parse_timeout_ms: u32 /* default 2000 */, pub frameworks: FrameworkSignals, pub enabled_adapters: Option<BTreeSet<String>>, pub syntax_facts: bool /* default true */ }
    pub struct ParsedUnit {
        pub ir_schema: u16, pub file: RepoPath, pub module_path: ModulePath, pub language: Language, pub dialect: Option<Dialect>,
        pub content_hash: ContentHash, pub analyzer: AnalyzerId /* { name: "lang-typescript", version } */, pub status: ParseStatus,
        pub symbols: Vec<IrSymbol>,           // index == LocalId; symbols[0] is always the Module symbol
        pub references: Vec<IrReference>, pub imports: Vec<IrImport>, pub exports: Vec<IrExport>,
        pub framework: Vec<IrFrameworkFact>, pub facts: Vec<SymbolFacts> /* sorted by symbol */, pub diagnostics: Vec<ParseDiagnostic>, pub stats: UnitStats }
    pub enum ParseStatus { Ok, Partial { error_nodes: u32, missing_nodes: u32 }, Failed { reason: FailReason /* TooLarge|Timeout|Binary|NotUtf8Decodable|Internal */ } }
    pub struct LocalId(pub u32);
    pub struct IrSymbol {
        pub local_id: LocalId, pub kind: SymbolKind, pub name: String, pub qualified_name: QualifiedName /* Vec<String> segments */, pub ordinal: u16 /* SID-003; 0 = none */,
        pub parent: Option<LocalId>, pub range: SourceRange, pub name_range: Option<SourceRange>, pub body_range: Option<SourceRange>,
        pub signature: Option<String> /* display text, ≤ 512 chars, single-spaced */, pub signature_hash: Hash128, pub body_hash: Hash128, pub attr_hash: Hash128,
        pub body_shingles: ShingleSet /* TSA-007 */, pub body_token_count: u32,
        pub modifiers: Modifiers /* bitflags: EXPORTED, DEFAULT_EXPORT, ASYNC, STATIC, ABSTRACT, READONLY, DECLARE, GENERATOR, OPTIONAL, OVERRIDE, CONST_ENUM, AMBIENT */,
        pub visibility: Visibility /* Public|Protected|Private|EcmaPrivate */, pub decorators: Vec<IrDecorator>, pub type_params: Vec<String>,
        pub params: Vec<IrParam>, pub return_type: Option<String>, pub heritage: Heritage /* extends: Vec<TypeText>, implements: Vec<TypeText> */,
        pub declared_type: Option<String> /* properties/variables */, pub const_value: Option<ConstValue> /* String|Number|Bool for literal-initialized constants */,
        pub overload_signatures: Vec<String>, pub doc_hash: Option<Hash128>, pub has_errors: bool, pub attrs: BTreeMap<String, AttrValue> }
    pub enum SymbolKind { Module, Namespace, Class, Interface, Enum, EnumMember, TypeAlias, Function, Method, Getter, Setter, Constructor, Property, Field, Variable, Constant, Parameter }
    // as_id_str(): "module","namespace","class","interface","enum","enum_member","type_alias","function","method","get","set","constructor","property","field","variable","constant","parameter"
    pub struct IrDecorator { pub name: String /* callee text, e.g. "Get" or "Nest.Get" */, pub args: Vec<IrExpr>, pub range: SourceRange }
    pub enum IrExpr { Str(String), Num(String), Bool(bool), Null, Ident(String), Member(Vec<String>), Array(Vec<IrExpr>), Object(Vec<(String, IrExpr)>),
                      Call { callee: Vec<String>, args: Vec<IrExpr> }, Arrow { returns: Box<IrExpr> } /* () => X */, Template { raw: String, has_subst: bool }, Other(String /* ≤120 chars */) }
    pub struct IrParam { pub name: String, pub type_text: Option<String>, pub optional: bool, pub rest: bool, pub decorators: Vec<IrDecorator>,
                         pub property: Option<ParamProperty> /* constructor parameter property: { visibility, readonly } */ }
    ```
  - `IrExpr` is a **bounded** literal tree (depth ≤ 6, ≤ 64 elements per array/object, strings ≤ 1 KiB). Framework adapters need decorator arguments (`@Module({...})`, `@Controller({path, version})`) without keeping AST nodes. Truncation sets `Other("…")`.
  - References, imports and exports:
    ```rust
    pub struct IrReference { pub from: LocalId, pub kind: RefKind, pub name: String /* terminal identifier */, pub receiver: ReceiverHint,
        pub import_binding: Option<BindingRef> /* (import index, binding index) when `name` or receiver root is an imported binding */,
        pub range: SourceRange, pub arg_count: u16, pub in_test_block: bool, pub attrs: BTreeMap<String, AttrValue> }
    pub enum RefKind { Call, New, TypeRef, Extends, Implements, Decorator, DiInjection, FrameworkRef, ValueRead, JsxElement }
    pub enum ReceiverHint { None, This, Super, ThisField { field: String, declared_type: Option<String> }, Identifier { name: String, declared_type: Option<String> },
        ImportedNamespace { binding: BindingRef }, Chain { root: Box<ReceiverHint>, segments: Vec<String> }, CallResult { callee: String }, Computed, Unknown }
    pub struct IrImport { pub specifier: String, pub kind: ImportKind /* Esm|SideEffect|CjsRequire|Dynamic|ImportEquals */, pub type_only: bool, pub bindings: Vec<ImportBinding>, pub range: SourceRange }
    pub struct ImportBinding { pub local: String, pub imported: Imported /* Named(String)|Default|Namespace|CjsModule */, pub type_only: bool }
    pub enum IrExport { Local { symbol: LocalId, exported_as: String, type_only: bool }, Reexport { specifier: String, imported: Imported, exported_as: String, type_only: bool },
        StarReexport { specifier: String, as_namespace: Option<String>, type_only: bool }, DefaultExpr { expr: IrExpr }, CjsModuleExports { symbol: Option<LocalId>, expr: IrExpr },
        CjsExportsProperty { name: String, symbol: Option<LocalId> }, ExportAssignment { symbol: Option<LocalId> } }
    ```
  - Syntax facts (TSA-006 fills them):
    ```rust
    pub struct SymbolFacts { pub symbol: LocalId, pub facts: Vec<SyntaxFact> /* source order */ }
    pub struct SyntaxFact { pub kind: FactKind, pub key: String /* stable comparison key, no positions */, pub range: SourceRange, pub detail: BTreeMap<String, AttrValue> }
    pub enum FactKind { Call, New, Condition, Loop, Throw, TryCatch, Await, Return, DbWriteLike, DbReadLike, TransactionWrapper, GuardDecorator, ConfigRead, Assignment }
    ```
  - Framework facts (ADR-006, framework-neutral kinds):
    ```rust
    pub struct IrFrameworkFact { pub adapter: String /* "nestjs","typeorm","bullmq","jest","config" */, pub kind: FrameworkFactKind, pub symbol: Option<LocalId>,
                                 pub attrs: BTreeMap<String, AttrValue>, pub range: SourceRange, pub confidence: f32 }
    pub enum FrameworkFactKind { ModuleDeclaration, Controller, HttpRoute, HttpGlobalConfig, DiInjection, Middleware, MiddlewareBinding, GlobalProvider, RouteMetadata,
        MetadataDecoratorDefinition, OrmEntity, OrmColumn, OrmRelation, OrmAccess, QueueConsumer, QueueJobHandler, QueueProducer, QueueRegistration,
        TestSuite, TestCase, TestMock, ConfigRead, Custom(String) }
    ```
    `AttrValue` = `Str | Int | Float | Bool | List(Vec<AttrValue>) | Map(BTreeMap<String, AttrValue>) | Expr(IrExpr)`.
  - Diagnostics: `ParseDiagnostic { severity: Error|Warning|Info, code: DiagCode, message: String /* ≤200 chars, no source text */, range: Option<SourceRange> }`. `DiagCode`: `SyntaxError, MissingNode, FileTooLarge, ParseTimeout, ComputedMemberName, UnsupportedConstruct, DynamicImportNonLiteral, DuplicateSymbol, DepthLimit, ExprTruncated`.
  - Traits (ADR-006 verbatim, with concrete associated types):
    ```rust
    pub trait LanguageAnalyzer: Send + Sync {
        fn id(&self) -> AnalyzerId; fn language(&self) -> Language; fn supports(&self, path: &RepoPath) -> bool;
        fn analyze(&self, input: &SourceInput, cfg: &AnalyzerConfig) -> Result<ParsedUnit, AnalyzeError>;
    }
    pub trait FrameworkAdapter<Ctx>: Send + Sync {   // Ctx = language-specific context (lang_typescript::FrameworkCtx); IR output is neutral
        fn name(&self) -> &'static str; fn version(&self) -> u32; fn detect(&self, signals: &FrameworkSignals) -> bool; fn extract(&self, ctx: &mut Ctx);
    }
    pub trait ModuleResolver: Send + Sync { fn resolve(&self, from: &RepoPath, specifier: &str, kind: ResolveKind /* Import|TypeImport|Require|Dynamic */) -> Resolution; }
    pub enum Resolution { File { path: RepoPath, method: ResolutionMethod, confidence: f32 }, External { ecosystem: String, name: String, subpath: Option<String>, version_range: Option<String> },
                          Builtin { name: String }, Unresolved { reason: UnresolvedReason } }
    pub trait SemanticProvider: Send + Sync {
        fn capabilities(&self) -> SemanticCapabilities;
        fn resolve_ambiguous(&self, refs: &[AmbiguousRef], budget: SemanticBudget) -> Result<Vec<SemanticResolution>, SemanticError>;
    }
    ```
    `FrameworkAdapter` is generic over the language context, so `analysis-ir` stays free of tree-sitter while adapters keep typed access to the syntax tree (the ADR-006 "runs over the syntax tree inside an analyzer").
  - `AnalyzeError` is only for *infrastructure* failures (e.g. grammar load failure). Bad source never errors: it yields `ParseStatus::Partial/Failed` with diagnostics (ADR-006 "parse errors are tolerated").
  - `AnalyzerRegistry { analyzers: Vec<Arc<dyn LanguageAnalyzer>> }` with `for_path(&RepoPath) -> Option<&dyn LanguageAnalyzer>` (first match; registration order is fixed by the composition root).
  - `validate(&ParsedUnit) -> Result<(), Vec<IrViolation>>`. Checks:
    - `symbols[0].kind == Module`
    - every `parent`/`from`/`symbol` LocalId in range
    - parents precede children
    - child range ⊆ parent range
    - ranges within the file length
    - `facts` sorted and unique by symbol
    - `(qualified_name, kind, ordinal)` unique

    It runs in debug builds after every `analyze` and in all tests.
- **Data model changes:** None (IR is in-memory and cached by IDX-005 as a bincode blob). `IR_SCHEMA_VERSION` is part of the cache key.
- **API/protocol changes:** New crate public API as above. JSON Schema of `ParsedUnit` exported to `docs/graph-schema/ir.schema.json` for documentation (not a cross-language contract).
- **Concurrency semantics:** All types are `Send + Sync`. Traits require `Send + Sync`, so the indexer can share analyzers across rayon workers.
- **Failure behavior:** Not applicable to type definitions. `validate` returns all violations, not just the first.
- **Idempotency considerations:** Serialization is deterministic: `BTreeMap`s, and vectors in source order. `bincode` round-trip equality is tested.
- **Security considerations:** `ParseDiagnostic.message` must not embed source text (enforced by constructor helpers taking only static strings plus numbers). Source never reaches logs through diagnostics.
- **Observability additions:** None in this crate (pure types). Metrics are emitted by the analyzer.
- **Tests required** (`tests/ir_roundtrip.rs` + unit tests):
  - `parsed_unit_json_roundtrip`
  - `parsed_unit_bincode_roundtrip`
  - `symbol_kind_id_strings_are_stable` (pinned list)
  - `ref_kind_strings_stable`
  - `validate_rejects_child_outside_parent`
  - `validate_rejects_dangling_local_id`
  - `validate_rejects_duplicate_identity`
  - `validate_requires_module_symbol_first`
  - `ir_expr_depth_and_size_bounded`
  - `registry_picks_first_supporting_analyzer`
  - `diagnostic_message_constructor_rejects_long_text`
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** `engine/scripts/cargo.sh test -p analysis-ir` passes. `cargo.sh tree -p analysis-ir` shows no dependency on `tree-sitter`, `repository`, `sqlx` or `tokio`.
- **Definition of done:** Crate merged with docs on every public type. Crate-DAG test (FND-004) updated to include `analysis-ir`. Global DoD met.

---

---

### TSA-002 — tree-sitter TS/TSX/JS integration in `lang-typescript`

Status: ☐

- **Task ID:** TSA-002
- **Title:** tree-sitter TS/TSX/JS integration in lang-typescript: grammar selection by extension, parser reuse per thread, error-node tolerance
- **Problem:** Parsing must be fast (parser objects are expensive to create), bounded (a pathological file must not stall a worker) and tolerant (a file with a syntax error must still produce symbols). The two TypeScript grammars must be applied to the right files.
- **Why it exists:** ADR-007 (tree-sitter primary layer), ADR-006 (errors → diagnostics, partial symbols still emitted), PRD §118–§120 (latency/scale).
- **Scope:**
  - `TypeScriptAnalyzer` implementing `LanguageAnalyzer` (skeleton visitor that emits only the Module symbol; later tasks add extraction).
  - Grammar selection by `Dialect`.
  - Thread-local parser cache.
  - Parse with deadline via the progress callback.
  - Size/binary guards.
  - ERROR/MISSING node → `ParseDiagnostic`.
  - `ParseStatus` computation.
  - Node-kind constant table and grammar-consistency test.
  - Fixture `ts-edge`.
- **Explicit non-scope:** Incremental tree-sitter re-parse with old trees (not useful: the cache is keyed per content hash, and INC works at file granularity). Flow syntax. Vue/Svelte SFC extraction.
- **Files/modules expected to change:** `engine/Cargo.toml` (member + deps `tree-sitter = "0.25"`, `tree-sitter-typescript = "0.23"`).
- **New files/modules expected:** `engine/crates/lang-typescript/{Cargo.toml, src/lib.rs, src/analyzer.rs, src/parser_pool.rs, src/kinds.rs, src/diagnostics.rs, src/visit/mod.rs, src/text.rs}`, `tests/parse_tolerance.rs`, `tests/node_kinds.rs`, `fixtures/repositories/ts-edge/` (`syntax-error-mid-class.ts`, `unterminated-template.ts`, `huge-generated.ts` (built by `build.sh` to 3 MiB), `component.tsx`, `legacy.jsx`, `module.mjs`, `common.cjs`, `deep-nesting.ts` (500 nested blocks), `bom-crlf.ts` (UTF-8 BOM + CRLF), `latin1.js` (invalid UTF-8 byte in a string)).
- **Dependencies:** TSA-001.
- **Implementation details:**
  - Grammar map:
    - `Dialect::Ts | Dts` → `tree_sitter_typescript::LANGUAGE_TYPESCRIPT`
    - `Dialect::Tsx` → `LANGUAGE_TSX`
    - `Js | Mjs | Cjs | Jsx` → `LANGUAGE_TSX`. JS is a syntactic subset of TS, and `.js` files commonly contain JSX. The language tag stays `JavaScript` and ids use `js:` (SID-001).
    - `<T>expr` casts never occur in JS, so the TSX ambiguity is harmless.
  - `supports(path)`: extension ∈ {ts, mts, cts, tsx, js, mjs, cjs, jsx} (incl. `.d.ts`).
  - Parser pool:
    ```rust
    thread_local! { static PARSERS: RefCell<ParserPool> = RefCell::new(ParserPool::default()); }
    struct ParserPool { ts: Option<Parser>, tsx: Option<Parser> } // created lazily, set_language once
    ```
    `with_parser(grammar, |p| ...)`; after any cancelled parse, `p.reset()`, because tree-sitter resumes a cancelled parse on the next call unless reset.
  - Deadline: `parser.parse_with_options(&mut |off, _| &bytes[off.min(len)..], None, Some(ParseOptions::new().progress_callback(&mut |_state| if Instant::now() > deadline { ControlFlow::Break(()) } else { ControlFlow::Continue(()) })))`. The deprecated `set_timeout_micros` is not used. `None` result → `ParseStatus::Failed{Timeout}` plus `DiagCode::ParseTimeout`, with the Module symbol only.
  - Guards before parsing:
    - `bytes.len() > cfg.max_file_bytes` → `Failed{TooLarge}`
    - NUL in the first 8 KiB → `Failed{Binary}`
    - A UTF-8 BOM is skipped, with byte offsets adjusted so ranges are still file offsets: parse `&bytes[3..]` and add 3 to every byte offset in `text.rs` helpers.
    - Invalid UTF-8 is allowed (tree-sitter works on bytes). Identifier/string extraction uses `String::from_utf8_lossy`, and `DiagCode::UnsupportedConstruct("lossy_utf8")` is emitted once.
  - Error tolerance: after the parse, walk with a `TreeCursor`. For each `node.is_error()` → `SyntaxError` diagnostic (range = node range). For each `node.is_missing()` → `MissingNode` with `message = "missing <kind>"`. Count both. Diagnostics are capped at 50 per file (`+ N more` summary diagnostic).
    - `ParseStatus` = `Ok` if no errors, else `Partial{..}`.
    - Visitors keep descending **into** ERROR nodes, because declarations inside are often intact. Any symbol whose range overlaps an ERROR node gets `has_errors = true`.
  - Recursion limits: the visitor is iterative (explicit stack) with a depth cap of 256. Deeper subtrees are skipped with `DiagCode::DepthLimit`. No stack overflow on `deep-nesting.ts`.
  - `kinds.rs`: `pub const CLASS_DECLARATION: &str = "class_declaration";` etc. — every kind used by TSA-003..006 and the NEST adapters, plus field-name constants (`"name"`, `"body"`, `"parameters"`, `"return_type"`, `"value"`, `"function"`, `"object"`, `"property"`, `"arguments"`, `"source"`, `"declaration"`, `"decorator"`, `"type_parameters"`, `"constructor"`, `"left"`, `"right"`, `"condition"`, `"pattern"`, `"type"`, `"alias"`). `node_kinds.rs` checks each kind with `Language::id_for_node_kind(k, true)` and each field with `field_id_for_name` against **both** grammars, with a documented exception list for TS-only kinds absent from TSX (none expected).
  - `ANALYZER_VERSION: semver::Version = 0.1.0` and `PARSER_VERSIONS` (INIT-012) live in `lib.rs`.
- **Data model changes:** None.
- **API/protocol changes:** `lang_typescript::TypeScriptAnalyzer::new() -> Self`, implementing `analysis_ir::LanguageAnalyzer`.
- **Concurrency semantics:** One parser per grammar per thread, never shared across threads. `TypeScriptAnalyzer` itself is stateless and `Sync`. rayon workers each warm their own pool.
- **Failure behavior:**
  - Timeouts, oversize and binary input → `Failed` units with diagnostics, never `Err`.
  - `Err(AnalyzeError::GrammarLoad)` only if `set_language` fails, which is an ABI mismatch and therefore a deployment bug. It is surfaced loudly.
- **Idempotency considerations:** Same bytes → identical `ParsedUnit`. The parser pool state does not leak, because `reset()` runs after a cancellation. Tested by parsing a file, cancelling another one, then re-parsing the first and comparing.
- **Security considerations:**
  - Untrusted input is bounded by the size cap, the deadline, the depth cap and the diagnostic cap.
  - No source text in diagnostics.
  - tree-sitter is C code, so parse runs are fuzzed in CI later (SEC task). Here, a proptest feeds random byte strings and asserts no panic or abort.
- **Observability additions:**
  - Per-file spans are too many, so the indexer (IDX-001) creates `parse_batch` spans.
  - This crate emits metrics through `telemetry` handles passed in `AnalyzerConfig` or a global registry:
    - `parse_files_total{language,dialect,status}`
    - `parse_duration_seconds{language}` (histogram)
    - `parse_error_nodes_total`
    - `parse_timeouts_total`
  - `tracing::debug!` event `parse_failed` with path and reason (no content).
- **Tests required:**
  - `tests/parse_tolerance.rs` (fixture `ts-edge`):
    - `syntax_error_yields_partial_with_symbols_before_and_after`
    - `missing_node_reported`
    - `oversize_file_failed_too_large_module_symbol_only`
    - `timeout_returns_failed_and_parser_is_reusable` (deadline 1 ms on `huge-generated.ts`, then a normal file parses Ok)
    - `tsx_grammar_for_tsx_and_jsx`
    - `ts_grammar_for_ts_and_dts`
    - `js_uses_tsx_grammar_and_js_tag`
    - `bom_offsets_are_file_offsets`
    - `crlf_lines_counted_correctly`
    - `lossy_utf8_diagnostic_once`
    - `deep_nesting_no_stack_overflow`
    - `diagnostics_capped_at_50`
    - proptest `random_bytes_never_panic`
  - `tests/node_kinds.rs`: `node_kinds_exist_in_grammar`, `field_names_exist_in_grammar`.
- **Benchmarks if applicable:** `benches/parse.rs` (criterion): parse-only throughput on `nest-api` files and on a synthetic 300-line NestJS service ×1,000. Recorded baseline. Target ≥ 20 MB/s single-thread parse-only in the container. Regression gate later (PERF-003).
- **Acceptance criteria:** `engine/scripts/cargo.sh test -p lang-typescript --test parse_tolerance --test node_kinds` passes. `cargo.sh bench -p lang-typescript --bench parse -- --quick` runs and prints throughput.
- **Definition of done:** Tests green. Benchmark baseline recorded in `benchmarks/perf/baselines/parse.json`. Global DoD met.

---

---

### TSA-003 — Declaration extraction

Status: ☐

- **Task ID:** TSA-003
- **Title:** Declaration extraction: classes, interfaces, type aliases, enums+members, functions, methods, constructors, properties/fields, variables/constants, namespaces, arrow functions bound to const, default exports
- **Problem:** Symbols are the nodes of the graph and the unit of change classification. Missing or misnamed declarations mean missing impact.
- **Why it exists:** PRD §17 (node types), §97 (TS reference analyzer). It is on the critical path. The external codegraph on reference-api finds 1,479 methods, 1,170 functions, 1,006 constants, 1,938 properties, 497 classes, 493 interfaces, 321 enum members, 111 type aliases and 67 enums. This is the parity reference for IDX-006.
- **Scope:**
  - The visitor emits `IrSymbol` for every construct in the table below, with parent links, ranges, name ranges, body ranges, modifiers, visibility, decorators (as `IrDecorator` with `IrExpr` args), params (incl. constructor parameter properties), return type text, heritage text, `declared_type`, `const_value`, overload signatures, and qualified names via `naming` (SID-002).
  - Also: fixture `ts-basic`; creating the synthetic `property` symbols for constructor parameter properties.
- **Explicit non-scope:** References (TSA-005). Hashes (TSA-007; this task leaves the hash fields zero until TSA-007). Local variables inside function bodies (not symbols). Parameters as separate symbols (`SymbolKind::Parameter` reserved, not emitted in MVP). Type-level members of type literals (`type X = { a: string }` members are not symbols).
- **Files/modules expected to change:** `engine/crates/lang-typescript/src/visit/mod.rs`, `src/kinds.rs`.
- **New files/modules expected:** `engine/crates/lang-typescript/src/visit/declarations.rs`, `src/visit/modifiers.rs`, `src/visit/expr.rs` (`IrExpr` builder), `tests/declarations.rs`, `fixtures/repositories/ts-basic/src/{shapes.ts, services/user.service.ts, util/strings.ts, enums.ts, namespaces.ts, defaults/{anon-class.ts, anon-fn.ts, named-class.ts, object.ts, ident.ts}, objects.ts, accessors.ts, overloads.ts, ambient.d.ts}`.
- **Dependencies:** TSA-002. SID-002 (the `naming` module is developed in the same PR or immediately before; TSA-003 calls `naming::qualify`).
- **Implementation details:**
  - Construct → symbol table (tree-sitter-typescript 0.23 node kinds):

    | Construct | Node kind(s) | SymbolKind | Name source | Notes |
    |---|---|---|---|---|
    | class | `class_declaration`, `abstract_class_declaration` | Class | field `name` (`type_identifier`) | heritage from `class_heritage` → `extends_clause` (`value`) / `implements_clause` (types); `ABSTRACT` modifier |
    | class expr bound to const | `variable_declarator{value: class}` | Class | declarator name | `attrs.binding = "const_class"` |
    | interface | `interface_declaration` | Interface | `name` | `extends_type_clause` → heritage.extends |
    | interface members | `property_signature`, `method_signature` inside `interface_body` | Property / Method | `name` | `OPTIONAL` from `?`; method overloads in interfaces fold into one symbol (SID-003) |
    | type alias | `type_alias_declaration` | TypeAlias | `name` | `body_range` = `value` |
    | enum | `enum_declaration` | Enum | `name` | `CONST_ENUM` if preceded by `const` |
    | enum member | `property_identifier` / `enum_assignment` in `enum_body` | EnumMember | name / `name` field | `const_value` from a literal `value` |
    | function | `function_declaration`, `generator_function_declaration` | Function | `name` | `ASYNC`, `GENERATOR` |
    | function overload / ambient | `function_signature` | (folded) | `name` | appended to the following implementation's `overload_signatures`; if there is no implementation (ambient/`.d.ts`), it becomes the Function symbol itself |
    | method | `method_definition` in `class_body` | Method / Getter / Setter / Constructor | `name` | `get`/`set` keyword child → Getter/Setter; name `constructor` → Constructor; `static`, `accessibility_modifier`, `override_modifier`, `async`, `*`; `private_property_identifier` (`#x`) → `Visibility::EcmaPrivate` |
    | method overload | `method_signature` in `class_body` | (folded) | `name` | appended to the implementing `method_definition` with the same name and staticness |
    | abstract method | `abstract_method_signature` | Method | `name` | `ABSTRACT` |
    | property | `public_field_definition` | Property | `name` | `declared_type` from `type`; `READONLY`, `STATIC`, `DECLARE`, `OPTIONAL`; if `value` is an `arrow_function`/`function_expression` → kind Method with `attrs.binding = "arrow_property"` (common Nest pattern `handler = async () => {}`) |
    | constructor param property | `required_parameter`/`optional_parameter` with `accessibility_modifier` or `readonly` inside a constructor's `formal_parameters` | Property (synthetic, parent = class) | `pattern` identifier | `attrs.from_constructor_param = true`, `declared_type` = param type; enables `ThisField` typing (TSA-005) |
    | module const/let/var | `lexical_declaration` (`const`/`let`) or `variable_declaration` (`var`) at module or namespace scope → `variable_declarator` | Constant (const) / Variable (let, var) | `name` (identifier) | destructuring patterns: one symbol per bound identifier (`object_pattern`/`array_pattern` leaves), `attrs.destructured = true`; `const_value` when `value` is a `string`, `number`, `true`/`false`, a `template_string` without `template_substitution`, or `as const` of those |
    | const-bound function | `variable_declarator{value: arrow_function | function_expression}` | Function | declarator name | `attrs.binding = "const_arrow"` or `"const_function_expr"`; params/return type from the function node; body = function body |
    | object-literal methods | `variable_declarator{value: object}` at module scope (const only) → `object` → `method_definition` / `pair{value: arrow_function|function_expression}` | Method (shorthand) / Function (pair) | `property_identifier`/`string` key | qualified `obj.key`, nested objects up to depth 2 (`obj.a.b`); computed keys skipped with `ComputedMemberName`; the object itself is a Constant symbol and the parent |
    | namespace | `internal_module` (`namespace X {}`), `module` (`module X {}`) | Namespace | `name` (`identifier` or `nested_identifier` `A.B.C` → three nested namespace symbols) | `declare module 'pkg'` (string name) → Namespace named by the literal content, `AMBIENT`; `declare global` → Namespace `global` |
    | ambient wrappers | `ambient_declaration` | — | — | unwrap and set `DECLARE`/`AMBIENT` on the inner symbol |
    | export wrappers | `export_statement{declaration}` | — | — | unwrap and set `EXPORTED`; `export default <decl>` sets `DEFAULT_EXPORT` |
    | default exports | `export_statement` with `default` + `value` | see SID-002 | — | named class/function → its name; anonymous class/function/arrow → name `default` (Class/Function); object literal → Constant `default`; identifier → no symbol (`IrExport::DefaultExpr`, TSA-004) |

  - Module symbol: `symbols[0]` = `SymbolKind::Module`, name = basename without extension, qualified name `["__module__"]`, range = the whole file. Module-level statements that are not declarations (top-level calls, `describe(...)`, `app.use(...)`) are attributed to it by TSA-005/006.
  - Anonymous functions (callbacks, IIFEs, function arguments) are **not** symbols under the default `AnonymousFnPolicy::Attribute`. Their references and facts belong to the nearest enclosing symbol. Under `Emit` they become Function symbols named `<anonymous>` with ordinals (SID-002). `Emit` exists for experiments only.
  - Decorators: in tree-sitter-typescript, `decorator` nodes are children of the class declaration (before `class`), of `method_definition`/`public_field_definition` (preceding siblings within `class_body` in some grammar versions), and of parameters. The visitor collects decorator nodes that are immediate previous siblings in `class_body` **and** decorator children, then attaches them to the next member. `node_kinds.rs` plus a fixture case pins which form 0.23 produces.
  - `IrDecorator.name` is the callee text, i.e. `call_expression.function` or a bare `identifier` / `member_expression`, joined by `.`. `args` come from `expr::to_ir_expr(arguments)`.
  - Signature display text: `{async }{name}{<T>}({params}): {ret}`, whitespace collapsed to single spaces and truncated to 512 chars at a char boundary.
  - Visibility: TS `accessibility_modifier` → Public/Protected/Private; `#name` → EcmaPrivate; default Public. `EXPORTED` reflects module export only.
  - Symbols are emitted in source order (pre-order). Parent always precedes child. Ordinals come from SID-003 after the visitor finishes, in a per-file pass.
- **Data model changes:** None.
- **API/protocol changes:** None (internal to the analyzer; output is `ParsedUnit.symbols`).
- **Concurrency semantics:** None (per-file pure).
- **Failure behavior:**
  - A construct without a name (e.g. `class {}` not in a default export) → no symbol, plus `UnsupportedConstruct` info diagnostic.
  - Computed member names → skipped plus `ComputedMemberName`.
  - Symbols inside ERROR nodes are still emitted with `has_errors`.
- **Idempotency considerations:** Deterministic source-order emission. No `HashMap` iteration in output paths (enforced by a clippy `disallowed_types` entry for `std::collections::HashMap` in `visit/`, allowing `HashMap` only behind sorted conversions).
- **Security considerations:** `const_value` strings are capped at 1 KiB. Constants that look like secrets (name matches `(?i)(secret|token|password|api[_-]?key|private[_-]?key)`) get `const_value = None` and `attrs.redacted = true`, so literal credentials in source never enter the IR or the graph DB.
- **Observability additions:** Metric `ir_symbols_total{kind}`, incremented per unit.
- **Tests required** (`tests/declarations.rs`, fixture `ts-basic` + inline snippets):
  - `class_with_heritage_and_decorators`
  - `abstract_class_and_abstract_method`
  - `interface_members_and_extends`
  - `type_alias_body_range`
  - `enum_members_with_const_values`
  - `const_enum_modifier`
  - `function_overloads_fold_into_implementation`
  - `ambient_function_signature_is_symbol`
  - `method_kinds_getter_setter_constructor_static`
  - `ecma_private_member`
  - `class_property_with_arrow_is_method`
  - `constructor_parameter_properties_become_properties`
  - `module_const_let_var_kinds`
  - `destructured_module_consts_one_symbol_each`
  - `const_arrow_and_function_expression_are_functions`
  - `object_literal_methods_qualified`
  - `computed_object_key_skipped_with_diagnostic`
  - `nested_namespace_a_b_c`
  - `declare_module_string_namespace`
  - `declare_global_namespace`
  - `export_default_variants` (5 fixture files)
  - `module_symbol_first`
  - `anonymous_callbacks_not_symbols_by_default`
  - `anonymous_emit_policy_emits_ordinals`
  - `secret_like_const_value_redacted`
  - `decorators_attached_to_correct_member` (incl. stacked decorators and decorators on parameters)
  - `symbols_inside_error_region_flagged`
- **Benchmarks if applicable:** Extend `benches/parse.rs` with `parse_and_extract_declarations`. Target ≥ 300 files/s single-thread on the synthetic 300-line service.
- **Acceptance criteria:** `engine/scripts/cargo.sh test -p lang-typescript --test declarations` passes. `validate()` passes for every unit in `ts-basic`, `ts-edge` and `nest-api`. On reference (`#[ignore]` `declarations_reference_counts`), per-kind counts are within ±10% of the external codegraph for class, interface, enum, enum_member, type_alias and method. Differences are written to `target/reference-decl-diff.txt` for IDX-006.
- **Definition of done:** Tests green. Kind table documented in `docs/languages/typescript.md` (created in this task as the language reference: construct → kind → naming). Global DoD met.

---

---

### TSA-004 — Import/export extraction (ESM + CJS)

Status: ☐

- **Task ID:** TSA-004
- **Title:** Import/export extraction (ESM named/default/namespace, re-exports `export * from`, type-only imports, CJS require/module.exports)
- **Problem:** Import bindings are the highest-confidence resolution signal (`import` 0.95). Exports define which symbols other files can reach, and barrels (`export * from`) chain them. CJS still appears in scripts and config files. Getting bindings wrong breaks the linker's main path.
- **Why it exists:** PRD §97 ("ES modules, CommonJS where required"). Target-architecture §3.1 confidence table. TSA-009 consumes specifiers. CG linker consumes bindings and exports.
- **Scope:**
  - `IrImport` for: ESM default, named (with alias), namespace, side-effect, `import type`, inline `type` specifiers, `import x = require()`, dynamic `import('literal')`, CJS `require('literal')` in variable declarators (plain, destructured, member access `require('x').y`) and bare `require()` statements.
  - `IrExport` for: exported declarations, `export { a, b as c }`, `export { x } from`, `export * from`, `export * as ns from`, `export type {}`, `export default <expr|decl>`, `export =`, `module.exports = ...`, `module.exports.x = ...`, `exports.x = ...`.
  - The binding index used by references.
- **Explicit non-scope:** Resolving specifiers to files (TSA-009). `require` with non-literal arguments (diagnostic only). AMD/UMD. `import.meta`. JSON imports' content.
- **Files/modules expected to change:** `engine/crates/lang-typescript/src/visit/mod.rs`.
- **New files/modules expected:** `engine/crates/lang-typescript/src/visit/modules.rs`, `src/bindings.rs` (`BindingTable`: local name → `BindingRef`, scoped to module level), `tests/imports_exports.rs`, fixture files `ts-basic/src/modules/{esm.ts, barrel.ts, types-only.ts, reexports.ts, dynamic.ts}`, `fixtures/repositories/js-cjs/{lib/a.js, lib/b.js, index.js, config.cjs}`.
- **Dependencies:** TSA-002 (TSA-003 for `IrExport::Local` symbol ids).
- **Implementation details:**
  - ESM: `import_statement` → `source` (string; strip quotes, unescape) and optional `import_clause` children:
    - `identifier` → `Imported::Default`
    - `namespace_import` → `Imported::Namespace` (local = its identifier)
    - `named_imports` → `import_specifier` (`name`, optional `alias`) → `Named(name)`, local = alias or name
  - Type-only: the `type` keyword directly after `import` → `IrImport.type_only = true`, and all bindings type-only. An inline `type` before an `import_specifier` → that binding's `type_only = true`.
  - `import_statement` without a clause → `ImportKind::SideEffect`.
  - `import x = require('y')` (`import_require_clause`) → `ImportEquals` with one binding `CjsModule`.
  - Dynamic: `call_expression` whose `function` is the `import` keyword node and whose first argument is `string` or a no-substitution `template_string` → `ImportKind::Dynamic`, no bindings, plus `attrs.awaited` if under `await`. A non-literal argument → `DynamicImportNonLiteral` diagnostic.
  - CJS (any scope, because require inside functions is common in scripts; bindings are only registered in the module-level `BindingTable` when at module scope):
    - `variable_declarator{name: identifier, value: call_expression(require, string)}` → `CjsRequire`, binding `CjsModule`.
    - `name: object_pattern` → one `Named` binding per `shorthand_property_identifier_pattern` / `pair_pattern(key, value identifier)`.
    - `value: member_expression(call require, property)` → `Named(property)`.
    - An `expression_statement` of bare `require('x')` → `SideEffect`, `kind = CjsRequire`.
    - `require` shadowed by a local binding named `require` in scope → not an import (checked via `BindingTable` + function params).
  - Exports:
    - `export_statement{declaration}` → `Local{symbol, exported_as = name}` for each declared symbol (destructuring → each).
    - `export_clause` without `source` → `Local` for each `export_specifier` (`name` → lookup local symbol by name; if not found, i.e. an exported import binding, emit `Reexport` with the binding's specifier and `imported`; this keeps barrels that import-then-export correct).
    - `export_clause` with `source` → `Reexport` per specifier (`export { default } from` → `imported = Default`; `export { default as X }`).
    - `export * from 'x'` → `StarReexport{as_namespace: None}`; `export * as ns from 'x'` (`namespace_export`) → `Some(ns)`.
    - `export type { ... }` → `type_only`.
    - `export default` → `Local` for a declaration, `DefaultExpr` for an expression.
    - `export = X` (`export_statement` with `=`, TS) → `ExportAssignment`.
  - CJS exports:
    - `assignment_expression{left: member_expression(module, exports)}` → `CjsModuleExports{expr}`; if the right side is an identifier bound to a local symbol → `symbol`; an object literal → expand `CjsExportsProperty` per key.
    - `left: member_expression(member_expression(module, exports), name)` or `member_expression(exports, name)` → `CjsExportsProperty`.
  - Specifier normalization: keep verbatim (no path normalization here; TSA-009 does that). Reject specifiers longer than 1 KiB (diagnostic) to bound memory.
  - `BindingTable` maps module-level local names to `BindingRef{import_idx, binding_idx}`. TSA-005 uses it to set `IrReference.import_binding` when a reference's name or receiver root is an imported binding.
- **Data model changes:** None.
- **API/protocol changes:** None (internal; output `ParsedUnit.imports/exports`).
- **Concurrency semantics:** None.
- **Failure behavior:** Malformed import inside an ERROR node → emitted if `source` is intact, else skipped with a `SyntaxError` diagnostic already present. Never fails the unit.
- **Idempotency considerations:** Source order is preserved. Indices are stable for identical input.
- **Security considerations:** None beyond the specifier length cap.
- **Observability additions:** Metrics `ir_imports_total{kind}` and `ir_exports_total{kind}`.
- **Tests required** (`tests/imports_exports.rs`; fixtures `ts-basic`, `js-cjs`):
  - `esm_default_named_namespace`
  - `named_import_alias_local_name`
  - `side_effect_import`
  - `import_type_whole_statement`
  - `inline_type_specifier_only_that_binding`
  - `import_equals_require`
  - `dynamic_import_literal_and_nonliteral_diagnostic`
  - `require_plain_destructured_member`
  - `require_shadowed_is_not_import`
  - `bare_require_side_effect`
  - `export_declarations_local`
  - `export_clause_of_imported_binding_is_reexport`
  - `export_from_named_and_default`
  - `export_star_and_star_as`
  - `export_type_clause`
  - `export_default_expr_vs_decl`
  - `export_equals`
  - `module_exports_identifier_and_object`
  - `exports_property_assignments`
  - `binding_table_lookup`
- **Benchmarks if applicable:** None (covered by parse benches).
- **Acceptance criteria:** `engine/scripts/cargo.sh test -p lang-typescript --test imports_exports` passes. On reference (`#[ignore]`), the total ESM import bindings are within ±5% of the external codegraph's 6,740 import nodes.
- **Definition of done:** Tests green. `docs/languages/typescript.md` import/export section written. Global DoD met.

---

---

### TSA-005 — Reference extraction with receiver hints

Status: ☐

- **Task ID:** TSA-005
- **Title:** Reference extraction: calls with receiver hints (identifier, this.method, this.field.method, imported namespace member, chained), new, type references, extends/implements, decorators
- **Problem:** Call and type edges are what impact analysis walks. The linker can only reach good precision if the analyzer records *how* a callee was referenced:
  - `this.repo.save()` must be tied to the declared type of `repo`.
  - `ns.fn()` must be tied to the namespace import.

  The external codegraph left 56,465 call references unresolved on reference-api, the measured cost of weak receiver hints.
- **Why it exists:** Target-architecture §3.1 confidence table (`this_member` 0.95, `di_constructor` 0.85, `type_annotation` 0.8, `import` 0.95). PRD §18 (CALLS, USES_TYPE, EXTENDS, IMPLEMENTS). Risk R1 in master plan §6.
- **Scope:**
  - `IrReference` emission from the enclosing symbol (innermost symbol whose range contains the node, else Module) for: calls, `new`, type references (annotations, generics, heritage), extends/implements, decorators, JSX elements (capitalized tags), value reads of imported bindings (identifier used as a value, e.g. passed as an argument: `TypeOrmModule.forFeature([User])`).
  - A receiver-hint classifier, using a per-function local type environment:
    - parameters with type annotations
    - `const x: T = ...` / `const x = new T()`
    - `this.field` types from class properties and constructor parameter properties
- **Explicit non-scope:**
  - Resolving references to targets (CG linker).
  - Type inference beyond the direct forms listed (no return-type flow, no generics instantiation except recording the head and args).
  - DI-specific facts (NEST-003 adds `DiInjection` refs).
  - Calls to builtins are still emitted, and the linker classifies them as external.
- **Files/modules expected to change:** `engine/crates/lang-typescript/src/visit/mod.rs`.
- **New files/modules expected:** `engine/crates/lang-typescript/src/visit/references.rs`, `src/visit/receiver.rs`, `src/visit/type_env.rs`, `tests/references.rs`, fixture files `ts-basic/src/refs/{calls.ts, chains.ts, types.ts, heritage.ts, jsx.tsx, namespace-import.ts}`.
- **Dependencies:** TSA-003, TSA-004.
- **Implementation details:**
  - Calls: `call_expression{function: F, arguments}`. Unwrap `F` through `parenthesized_expression`, `non_null_expression` (`x!`), `as_expression`, `satisfies_expression` and `type_assertion`. Also handle optional chaining (`a?.b()` is `member_expression` with `optional_chain`; record `attrs.optional = true`). Classification:

    | `F` shape | `name` | `ReceiverHint` |
    |---|---|---|
    | `identifier` x | x | `None`; `import_binding` set if x is imported |
    | `member_expression(this, p)` | p | `This` |
    | `member_expression(super, p)` | p | `Super` |
    | `member_expression(member_expression(this, f), p)` | p | `ThisField{ field: f, declared_type: class_env[f] }` |
    | `member_expression(identifier o, p)`, o imported namespace/default/CjsModule binding | p | `ImportedNamespace{binding}` |
    | `member_expression(identifier o, p)`, o local/param | p | `Identifier{name: o, declared_type: fn_env[o]}` |
    | `member_expression(identifier O, p)`, O capitalized and not local | p | `Identifier{name: O, declared_type: None}` + `attrs.static_candidate = true` (`Foo.create()`) |
    | `member_expression(member_expression(...), p)`, deeper chains | p | `Chain{ root: <hint of innermost object>, segments: [middle property names] }` (e.g. `this.a.b.c()` → root `This`, segments `[a, b]`) |
    | `member_expression(call_expression(g), p)` | p | `CallResult{callee: text(g) ≤ 120}` (builder chains: `qb.where().andWhere()`) |
    | `subscript_expression` / anything else | text tail if identifier else `"<computed>"` | `Computed` / `Unknown` |
    | `super` (call) | `super` | `Super`, `kind = Call`, `attrs.super_ctor = true` |

  - `new_expression{constructor}` → `kind = New`, same classification as calls.
  - Types: every `type_identifier` (and `nested_type_identifier` `A.B` → name `B`, `Chain` root namespace hint) inside `type_annotation`, `type_arguments`, `return_type`, `extends_type_clause`, `type_alias_declaration.value`, `as_expression` type, `implements_clause` → `kind = TypeRef`, except inside heritage, where `Extends`/`Implements` are used instead.
    - Class `extends_clause.value` is an *expression* (`identifier` / `member_expression` / `call_expression` such as `AuthGuard('jwt')` or `mixin(Base)`): identifier → `Extends` name; call → `Extends` with name = callee identifier and `attrs.extends_call = true`.
    - Generic heads and arguments: `Repository<User>` → TypeRef `Repository` plus TypeRef `User` with `attrs.type_arg_of = "Repository"`.
    - Builtin/primitive types (`string`, `number`, `Promise`, `Array`, `Record`, `Partial`, ...) are emitted too (the linker drops them via the lib table). This keeps the analyzer table-free.
  - Decorators → `kind = Decorator`, name = the last segment of the decorator callee, `ImportedNamespace`/`None` hint by binding, emitted from the decorated symbol (not the parent).
  - JSX: `jsx_opening_element` / `jsx_self_closing_element` with a capitalized `identifier`/`member_expression` name → `JsxElement`.
  - Value reads: an identifier in expression position that is an imported binding and is not a call callee → `ValueRead` (captures `providers: [UserService]`, `forFeature([User])`, `@UseGuards(JwtGuard)` args, callbacks passed by reference). Non-imported locals are not emitted (noise).
  - Type environment (`type_env.rs`), class-level `class_env: field → declared_type`, from:
    - `public_field_definition` with a `type`
    - constructor parameter properties
    - `public_field_definition` with `value: new_expression(T)` → `T`
  - Function-level `fn_env`, with a stack per function scope (shadowing respected):
    - parameters with type annotations
    - `const/let x: T`
    - `const x = new T(...)`
    - `const x = await this.f.g()` → none (no inference)
  - Declared types are reduced to the head identifier text (`Repository<User>` → `Repository<User>` kept verbatim plus a parsed `head` and `args` in `attrs`).
  - `in_test_block`: true when the reference is lexically inside a `describe`/`it`/`test` callback (helps test mapping; computed by checking the call stack of enclosing `call_expression` callee names).
  - References from anonymous callbacks are attributed to the enclosing symbol, with `attrs.in_callback = true`.
  - Dedup: none. Each occurrence is a reference with its own range. The linker aggregates them into one edge with occurrence count (CG).
- **Data model changes:** None.
- **API/protocol changes:** None (internal; output `ParsedUnit.references`).
- **Concurrency semantics:** None.
- **Failure behavior:** Unclassifiable callee → `Unknown` hint, still emitted. Never fails.
- **Idempotency considerations:** Source order. Environment maps are `BTreeMap` or scoped `Vec`s.
- **Security considerations:** `CallResult.callee` text is capped at 120 chars and contains code text only (no literals): string literal arguments are not included in the receiver text.
- **Observability additions:** Metrics `ir_references_total{kind,receiver}` (receiver ∈ none, this, super, this_field, identifier, imported_ns, chain, call_result, computed, unknown) and `ir_receiver_typed_ratio` (gauge per batch, set by IDX: share of member calls with a `declared_type`).
- **Tests required** (`tests/references.rs`, fixture `ts-basic/src/refs`):
  - `bare_call_imported_binding`
  - `this_method_call`
  - `super_method_and_super_ctor`
  - `this_field_call_typed_from_ctor_param_property`
  - `this_field_call_typed_from_field_declaration`
  - `this_field_initialized_with_new`
  - `param_typed_identifier_receiver`
  - `local_const_new_typed_receiver`
  - `shadowed_local_overrides_outer_type`
  - `imported_namespace_member_call`
  - `default_import_member_call_is_imported_namespace_hint`
  - `capitalized_static_candidate`
  - `deep_chain_segments`
  - `builder_chain_call_result`
  - `optional_chain_flagged`
  - `non_null_and_as_unwrapped`
  - `new_expression_classified`
  - `type_refs_in_annotations_generics_returns`
  - `nested_type_identifier`
  - `extends_identifier_and_extends_call_mixin`
  - `implements_multiple`
  - `decorator_refs_from_decorated_symbol`
  - `jsx_component_refs_only_capitalized`
  - `imported_value_reads_in_arrays_and_args`
  - `callback_refs_attributed_to_enclosing_with_flag`
  - `module_level_calls_from_module_symbol`
  - `in_test_block_flag`
- **Benchmarks if applicable:** `parse_and_extract_all` bench (declarations + references). Target ≥ 250 files/s single-thread on the synthetic service.
- **Acceptance criteria:** `engine/scripts/cargo.sh test -p lang-typescript --test references` passes. On `nest-api`, every `this.<field>.<method>()` call in services has `declared_type` set (test `nest_api_member_calls_all_typed`). On reference (`#[ignore]`), the share of `ThisField` calls with a `declared_type` is ≥ 0.9, written to `target/reference-receiver-stats.json`.
- **Definition of done:** Tests green. The receiver-hint table is documented in `docs/languages/typescript.md`. Global DoD met.

---

---

### TSA-006 — Per-symbol SyntaxFacts for change classification

Status: ☐

- **Task ID:** TSA-006
- **Title:** Per-symbol SyntaxFacts: calls made, conditions, loops, throws, try/catch, awaits, returns, DB-write-like calls, transaction wrappers (plus guard decorators, config reads, validation calls) for deterministic change classification
- **Problem:** The change classifier (CHG, PRD §28) must decide *what kind of change* a hunk made (`condition_changed`, `exception_handling_changed`, `database_write_changed`, `transaction_boundary_changed`, `call_added`, ...) without an LLM. It can only do this by comparing a stable, position-free summary of the base and head versions of each symbol. Line-level diffs and `body_hash` alone say "changed", never "how".
- **Why it exists:** Target-architecture §3.1 (`syntax_facts per symbol`) and §3.6 (change classification compares `SyntaxFact` sets of base and head). PRD §28 category list. The auth-bypass golden scenario depends on seeing a removed guard condition / removed guard decorator as an `authorization_changed` signal.
- **Scope:**
  - Populate `ParsedUnit.facts: Vec<SymbolFacts>` (type defined in TSA-001) with `SyntaxFact`s of kinds `Call, New, Condition, Loop, Throw, TryCatch, Await, Return, DbWriteLike, DbReadLike, TransactionWrapper, GuardDecorator, ConfigRead (process.env only), Assignment (this.x = / property writes)`.
  - A stable `key` per fact (no positions) and a `detail` map; facts are attributed to the innermost enclosing *symbol* (callbacks and anonymous functions fold into it, same rule as TSA-005).
  - `facts::compare_keys(base, head) -> FactDelta` helper in `analysis-ir` (multiset difference by `(kind, key)`), used later by CHG.
- **Explicit non-scope:** Mapping deltas to PRD §28 categories (CHG tasks). Framework-semantic facts (routes, guards as classes, ORM entities: NEST-*). Data-flow or type inference. Operator-level changes inside a condition beyond a normalized token hash.
- **Files/modules expected to change:** `engine/crates/lang-typescript/src/visit/mod.rs` (invoke fact walker, honour `cfg.syntax_facts`), `engine/crates/lang-typescript/src/kinds.rs`, `engine/crates/analysis-ir/src/facts.rs` (add `FactDelta`, `compare_keys`, key-format doc).
- **New files/modules expected:** `engine/crates/lang-typescript/src/visit/facts.rs`, `src/visit/db_heuristics.rs` (name tables + receiver test), `src/visit/fact_keys.rs`, `tests/syntax_facts.rs`, fixture files `fixtures/repositories/ts-basic/src/facts/{calls.ts, control.ts, errors.ts, db.service.ts, transactions.ts, guards.ts, env.ts}`.
- **Dependencies (task IDs):** TSA-003 (symbol ranges), TSA-005 (call classification, receiver hints, shared walker), TSA-007 (`normalized_tokens`, used for condition/loop/return keys).
- **Implementation details:**
  - Single pass sharing the TSA-005 walker; each node is routed once to `FactSink::push(symbol, fact)`. Facts are emitted in source order per symbol; `SymbolFacts` sorted by `LocalId`; symbols without facts are omitted.
  - Key formats (position-free, deterministic; `h8` = first 8 hex of `blake3` over `normalized_tokens(node)`):
    - `Call`: `call:{receiver_text}.{name}/{argc}` where `receiver_text` is the dotted receiver chain with `this` kept and literals dropped (`this.repo.save/1`); `New`: `new:{Name}/{argc}`.
    - `Condition` (nodes `if_statement`, `ternary_expression`, `switch_statement`/`switch_case`): `if:{h8}` / `ternary:{h8}` / `switch:{h8}`, `detail`: `has_else`, `early_exit` (consequent is `return_statement`/`throw_statement`: a guard clause), `compares_null`, `negated`, `idents` (≤ 8 identifiers, sorted).
    - `Loop` (`for_statement`, `for_in_statement`, `while_statement`, `do_statement`, `.forEach(` calls): `loop:{for|for_of|for_in|while|do|foreach}:{h8 of header}`; `detail.awaits_inside`.
    - `Throw`: `throw:{ClassName}` for `throw new X(...)`, else `throw:expr`. `TryCatch` (`try_statement`): `try:{catch|nocatch}:{finally|nofinally}`, `detail`: `empty_catch`, `rethrows`, `catch_param`.
    - `Await` (`await_expression`): `await:{callee key}` (or `await:expr`); `for await` also yields `Await`. `Return`: `return:{void|null|undefined|true|false|lit|ident|obj|call:{callee}|expr:{h8}}`.
    - `Assignment`: `assign:this.{field}` or `assign:{ident}` for `assignment_expression`/`augmented_assignment_expression` targets that are members.
  - `DbWriteLike` / `DbReadLike`: `db_heuristics.rs` tables. Write method names `{save, insert, update, upsert, delete, remove, softDelete, softRemove, restore, increment, decrement, create (only on Model-like receiver), destroy, bulkCreate, execute (when chain contains insert/update/delete builder step), query (when first string arg starts with INSERT|UPDATE|DELETE|ALTER|DROP|TRUNCATE)}`; read names `{find, findOne, findOneBy, findBy, findAndCount, count, exists, query (SELECT), getMany, getOne, getRawMany}`. A call qualifies only if the receiver passes the test: declared type head matches `(?i)(Repository|EntityManager|DataSource|QueryRunner|Model|Prisma|Knex)$`, or receiver name/field matches `(?i)(repo|repository|manager|em|db|dataSource|queryRunner|prisma|knex)`, or the chain contains `createQueryBuilder`. `detail`: `method`, `entity` (generic arg or first-arg identifier, if any), `receiver_declared_type`, `confidence` (0.9 typed Repository, 0.75 name-only, 0.6 raw SQL). Key: `dbw:{method}:{entity|?}`.
  - `TransactionWrapper`: calls `.transaction(cb)`, `.runInTransaction(`, `queryRunner.{startTransaction,commitTransaction,rollbackTransaction}`, `@Transactional(...)` decorator (symbol-level, attributed to the decorated symbol). Key `tx:{callee}`. Facts for calls inside the callback stay on the enclosing symbol and carry `detail.in_transaction = true`.
  - `GuardDecorator`: decorators whose last-segment name matches the configurable list `AnalyzerConfig.guard_decorator_names` (default regex `^(UseGuards|Roles?|Permissions?|Public|Auth\w*|Authorize\w*|Skip\w*Auth\w*)$`); key `guard:{Name}:{h8 of args}`. Validation calls (`validate*`, `assert*`, `plainToInstance`, `new ValidationPipe`, `.parse(`/`.safeParse(` on zod-like receivers) are `Call` facts with `detail.validation = true`.
  - `ConfigRead`: `process.env.NAME`, `process.env['NAME']`, `const { NAME } = process.env` → key `env:NAME` (names only, never values).
  - Caps: ≤ 2,000 facts per symbol; beyond that a single `UnsupportedConstruct` info diagnostic and no further facts for that symbol. Module-level statements attach to the Module symbol.
- **Data model changes:** None (`facts` persisted inside the cached IR blob, IDX-005). Bump `ANALYZER_VERSION` minor.
- **API/protocol changes:** `analysis_ir::facts::{FactDelta, compare_keys}` added; key grammar documented in `docs/languages/typescript.md`.
- **Concurrency semantics:** Pure per file; no shared state.
- **Failure behavior:** Facts inside ERROR nodes are emitted only if the node kind is intact; a fact never causes the unit to fail. Unknown shapes are skipped silently except for the 2,000-fact cap.
- **Idempotency considerations:** Source-order emission; sets (`idents`) sorted; `h8` computed from normalized tokens so reformatting (whitespace/comments/quote style) does not change keys (tested).
- **Security considerations:** Keys and details never contain string-literal contents except enumerated short identifiers (`env:NAME`, decorator names); SQL text is classified by first keyword only and not stored; secrets in arguments are hashed, not stored.
- **Observability additions:** `ir_syntax_facts_total{kind}`; `ir_fact_cap_hits_total`.
- **Tests required** (`tests/syntax_facts.rs`, fixture `ts-basic/src/facts`):
  - `calls_and_new_keys_have_receiver_and_arity`, `condition_fact_early_exit_and_compares_null`, `ternary_and_switch_conditions`, `loop_kinds_and_foreach`, `throw_class_vs_expr`, `try_catch_finally_flags_and_empty_catch`, `await_and_for_await`, `return_shapes`, `db_write_typed_repository_save`, `db_write_name_only_lower_confidence`, `db_raw_query_classified_by_verb`, `query_builder_chain_write`, `non_db_save_not_flagged` (e.g. `fileStore.save` stays Call), `transaction_callback_marks_inner_calls`, `transactional_decorator_fact`, `guard_decorator_default_and_custom_list`, `process_env_read_names_only`, `callback_facts_attributed_to_enclosing_symbol`, `module_level_facts_on_module_symbol`, `fact_keys_stable_under_reformat_and_comments`, `fact_cap_enforced`, `compare_keys_multiset_delta`, `syntax_facts_disabled_by_config`.
- **Benchmarks if applicable:** Extend `benches/parse.rs` with `parse_extract_all_with_facts`; overhead of facts ≤ 25% over TSA-005 baseline.
- **Acceptance criteria:** `engine/scripts/cargo.sh test -p lang-typescript --test syntax_facts` and `-p analysis-ir facts` pass; `validate()` accepts `facts` for all fixture units; reformatting a fixture file leaves every fact key unchanged.
- **Definition of done:** Tests green; fact-key grammar and heuristic tables documented; golden IR snapshots (TSA-008) updated; global DoD met.

---

---

### TSA-007 — Normalized body_hash / signature_hash

Status: ☐

- **Task ID:** TSA-007
- **Title:** Normalized `body_hash` / `signature_hash` / `attr_hash` (token stream excluding comments and whitespace, blake3) and token-shingle sketch
- **Problem:** Change classification and rename detection need hashes that change when behaviour changes and do not change when formatting, comments, quote style or trailing commas change. Hashing raw byte ranges would flag every Prettier run as a modification and break the rename matcher.
- **Why it exists:** ADR-005 (signature is an attribute; `signature_hash` and `body_hash` "a normalized token hash that ignores whitespace and comments" drive classification); target-architecture §3.2; SID-004/SID-005 consume `body_hash`, `signature_hash`, `body_shingles`.
- **Scope:**
  - Language-neutral hashing primitives in `analysis-ir` (`TokenHasher`, `ShingleSet`, `jaccard`, domain separators).
  - TypeScript token-stream extraction in `lang-typescript` (leaf-node walk with normalization rules).
  - Filling `IrSymbol.{body_hash, signature_hash, attr_hash, body_shingles, body_token_count}` for every symbol (TSA-003 left them zero).
- **Explicit non-scope:** Semantic equivalence (renamed locals, reordered statements), AST-level canonicalization, change-class computation (CHG), the matcher (SID-005).
- **Files/modules expected to change:** `engine/crates/analysis-ir/src/symbol.rs` (final `ShingleSet` shape), `engine/crates/lang-typescript/src/visit/declarations.rs` (call hasher after each symbol is complete), `engine/crates/review-core/src/lib.rs` (`Hash128` if absent).
- **New files/modules expected:** `engine/crates/analysis-ir/src/hashing.rs`, `engine/crates/lang-typescript/src/tokens.rs`, `src/visit/hashes.rs`, `tests/hashes.rs`, `benches/hashing.rs`, fixture files `fixtures/repositories/ts-basic/src/hashing/{base.ts, reformatted.ts, comments-only.ts, semantic-change.ts, quotes.ts, container.ts}`.
- **Dependencies (task IDs):** TSA-003 (symbol ranges, signature parts), TSA-001 (types).
- **Implementation details:**
  - Token stream: depth-first walk over the **leaf** nodes (named or anonymous) of the relevant range using a `TreeCursor`; skip `comment` nodes and zero-width `MISSING` nodes. Each token is `(class, text)` with classes `Ident`, `Keyword`, `Punct`, `Str`, `Num`, `Regex`, `Template`.
  - Normalization rules (all unit-tested): string literals of either quote style become `Str(content)` with the escapes of the original preserved; numbers lowercased, numeric separators removed; trailing commas before `)`, `]`, `}` and a final `;` of a statement dropped (ASI-insensitive); template string chunks kept verbatim; JSX text collapsed to single spaces; parentheses and everything else kept (they are semantic). Identifier text is kept: renamed locals therefore do change the hash (documented).
  - Hash function: `blake3::Hasher` with a domain prefix per hash (`b"rg.body.v1\0"`, `b"rg.sig.v1\0"`, `b"rg.attr.v1\0"`), each token fed as `class_byte || len_u32_le || bytes`; digest truncated to the first 16 bytes → `Hash128` (same truncation as `SymbolKey`). Empty stream hashes to a fixed non-zero value (domain prefix only) so "no body" is distinguishable from the unset zero hash.
  - `body_hash` per kind: function/method/getter/setter/constructor → tokens of `body_range` (arrow expression bodies included); property/variable/constant → initializer tokens; type alias → `value` tokens; enum member → initializer. **Containers** (class, interface, enum, namespace, module): tokens of the body with each child symbol's range replaced by one placeholder token `Ident("<child:{kind}:{name}>")`, so editing a method does not change the class's `body_hash`, but adding, removing or renaming a member does.
  - `signature_hash`: kind tag, `ASYNC/STATIC/ABSTRACT/READONLY/OPTIONAL/GENERATOR` modifiers, visibility, type parameters (with constraints/defaults), each param as `(name, type tokens, optional, rest, has_default flag)` (default *expressions* excluded: they are body-like), return type tokens, `declared_type`, heritage tokens (class/interface), then `overload_signatures` in source order. The symbol's own name is **excluded** so a pure rename keeps its signature hash (SID-005 compares names separately).
  - `attr_hash`: decorators (name + argument `IrExpr` canonical form, in order), `EXPORTED/DEFAULT_EXPORT/DECLARE/OVERRIDE` flags, `doc_hash` excluded. Decorator changes thus classify as attribute changes, not body changes.
  - `ShingleSet`: token 3-grams of the body stream, each hashed (`blake3` first 4 bytes → `u32`), de-duplicated and kept as a **bottom-k sketch** (k = 256, sorted ascending; exact when ≤ 256 distinct). `jaccard(a, b) -> f32` uses the standard bottom-k estimator (merge, take k smallest of the union, count shared). `body_token_count` = total tokens. Bodies with < 3 tokens get a 1-gram set.
- **Data model changes:** None (fields already in `IrSymbol`); `IR_SCHEMA_VERSION` unchanged unless `ShingleSet` layout changes (then bump to 2 and note in the cache key).
- **API/protocol changes:** `analysis_ir::hashing::{TokenHasher, Token, TokenClass, ShingleSet, jaccard}` public; `lang_typescript::tokens::tokens_of(node, src) -> impl Iterator<Item = Token>`. The normalization version is `HASH_NORMALIZATION_VERSION = 1`, folded into `ANALYZER_VERSION` bumps.
- **Concurrency semantics:** Pure functions; no shared state.
- **Failure behavior:** Symbols with `has_errors = true` still get hashes (over available tokens) and `attrs.hash_partial = true`; the incremental differ treats partial hashes as "modified" when unequal. Never fails the unit.
- **Idempotency considerations:** Same tokens → same hash on all platforms (little-endian framing explicit). Golden vectors pin three hashes so an accidental normalization change is caught.
- **Security considerations:** Hashes are one-way; string contents are not retrievable. Secret-looking constants (TSA-003 redaction) are still hashed so a changed secret is detected without storing it.
- **Observability additions:** `ir_hash_tokens_total` counter; no per-symbol logging.
- **Tests required** (`tests/hashes.rs` + unit tests in `analysis-ir`):
  - `reformat_does_not_change_body_hash`, `comments_do_not_change_body_hash`, `quote_style_normalized`, `trailing_comma_and_semicolon_normalized`, `crlf_vs_lf_same_hash`, `semantic_change_changes_body_hash`, `local_rename_changes_body_hash`, `param_type_change_changes_signature_not_body`, `body_change_keeps_signature_hash`, `rename_keeps_signature_hash`, `return_type_change_changes_signature_hash`, `default_value_expression_in_body_domain_not_signature`, `decorator_change_changes_attr_hash_only`, `class_hash_ignores_method_body_edits`, `class_hash_changes_when_member_added`, `overload_signatures_in_signature_hash`, `empty_body_hash_is_non_zero_and_stable`, `hash_golden_vectors`, `shingle_jaccard_identical_is_one`, `shingle_jaccard_small_edit_above_0_8`, `shingle_jaccard_unrelated_below_0_2`, `bottom_k_estimate_close_to_exact` (property test, error ≤ 0.05).
- **Benchmarks if applicable:** `benches/hashing.rs`: hashing throughput for a 300-line service; target ≥ 150 MB/s of token bytes and < 15% added to extract-all time.
- **Acceptance criteria:** `engine/scripts/cargo.sh test -p analysis-ir hashing` and `-p lang-typescript --test hashes` pass; reformatting every `ts-basic` file with a fixed formatter yields identical hashes for all symbols.
- **Definition of done:** Tests green; normalization rules documented in `docs/languages/typescript.md`; baseline recorded in `benchmarks/perf/baselines/hashing.json`; global DoD met.

---

---

### TSA-008 — Golden IR test suite

Status: ☐

- **Task ID:** TSA-008
- **Title:** Golden IR test suite: `insta` snapshots per fixture file, canonical text rendering, determinism and invariant checks
- **Problem:** The IR is the contract every downstream stage consumes. Per-feature unit tests cannot show that a change in one visitor silently altered the output for unrelated constructs. A reviewable snapshot of the whole IR per fixture file makes any behavioural drift visible in a diff.
- **Why it exists:** ADR-006 consequences ("fixture repositories, golden IR tests" are required for every language). Master plan Milestone M2 ("golden IR/graph tests"). Risk R1: precision regressions must be caught before the linker.
- **Scope:**
  - A canonical, line-oriented renderer `render_ir(&ParsedUnit) -> String` (stable, human-readable, not raw JSON).
  - One `insta` snapshot per fixture source file under `fixtures/repositories/` (all TypeScript/JavaScript files of `ts-basic`, `ts-edge`, `js-cjs`, `nest-api`, `ts-resolve`).
  - Cross-cutting invariants executed on every fixture: `validate()`, determinism, LF/CRLF equivalence, thread-order independence, bincode round-trip.
  - A manifest check that every fixture file has a snapshot and no snapshot is orphaned.
- **Explicit non-scope:** Graph-level or linker snapshots (CG/IDX tasks). Performance gates. Authoring new fixtures beyond adding the golden harness (feature tasks own their fixtures).
- **Files/modules expected to change:** `engine/crates/lang-typescript/Cargo.toml` (dev-deps `insta` with features `glob`, `yaml` off; `walkdir`), `engine/scripts/` CI snippet (`cargo insta test --check`).
- **New files/modules expected:** `engine/crates/analysis-ir/src/render.rs` (feature `render`), `engine/crates/lang-typescript/tests/golden_ir.rs`, `tests/golden_invariants.rs`, `tests/support/mod.rs` (fixture walker, analyzer factory), `engine/crates/lang-typescript/tests/snapshots/*.snap`, `fixtures/repositories/golden-manifest.txt` (sorted fixture file list, checked in), `docs/languages/golden-ir.md` (review policy).
- **Dependencies (task IDs):** TSA-003, TSA-004, TSA-005, TSA-006, TSA-007. NEST-001..007 add their own `nest-api` files, and each NEST task updates the snapshots it affects.
- **Implementation details:**
  - Renderer sections, in this fixed order: `# file / lang / dialect / status / analyzer`; `## symbols` as an indented tree (`{indent}{kind} {qualified_name}{~ordinal} [{modifiers}] {visibility} L{start}-{end} sig={sig_hash8} body={body_hash8} attr={attr_hash8} tokens={n}`), decorators and params on continuation lines; `## references` (`from -> kind name receiver=... binding=... argc=... L:C`); `## imports`; `## exports`; `## framework` (`adapter kind symbol attrs-as-sorted-k=v confidence`); `## facts` (`symbol: kind key`); `## diagnostics`. Hashes are rendered as the first 8 hex characters (full hashes are covered by TSA-007 golden vectors). Maps are sorted; floats printed with 2 decimals; paths forward-slash.
  - Harness: `insta::glob!("../../../../fixtures/repositories", "**/*.{ts,tsx,js,jsx,mjs,cjs,mts,cts}", |path| { ... assert_snapshot!(render_ir(&unit)) })` with `insta::with_settings!({ snapshot_path => "snapshots", prepend_module_to_snapshot => false, omit_expression => true })`. Snapshot name = fixture-relative path with `/` → `__`.
  - Files larger than 256 KiB (e.g. `huge-generated.ts`) are excluded by a documented skip list and asserted separately by size/status only.
  - Invariants (`golden_invariants.rs`, run per file): (1) `validate(&unit)` is `Ok`; (2) analyzing twice yields `==` units and identical `bincode`; (3) the same file with LF→CRLF conversion yields identical symbols (names, kinds, hashes) and identical facts keys (positions excluded); (4) analyzing all fixtures on 1 thread and on a shuffled 8-thread rayon pool gives identical rendered output; (5) `bincode` and JSON round-trips are lossless.
  - CI policy: `INSTA_UPDATE=no` in CI (`cargo insta test --check`); developers regenerate with `cargo insta test --review`. A snapshot diff must be explained in the PR description (documented in `docs/languages/golden-ir.md`).
- **Data model changes:** None.
- **API/protocol changes:** `analysis_ir::render::render_ir` behind the `render` feature (dev/test only; excluded from release builds).
- **Concurrency semantics:** Tests run files in parallel via rayon only in invariant (4); snapshots are written only in single-threaded `--review` mode.
- **Failure behavior:** A missing snapshot fails in CI (no auto-accept). The manifest test reports the exact missing/orphan paths.
- **Idempotency considerations:** Rendering is a pure function of the IR; no timestamps, absolute paths or hash-map iteration. Running the suite twice produces zero snapshot changes.
- **Security considerations:** Fixtures contain no real credentials; the renderer prints redacted constants as `<redacted>`, matching TSA-003 policy. The NestJS fixtures are synthetic, modelled on a reference NestJS repository without copying code.
- **Observability additions:** None (test-only). The suite prints a summary line `golden: N files, M symbols, K references`.
- **Tests required:**
  - `golden_ir_snapshots` (glob over every fixture file), `every_fixture_file_has_a_snapshot`, `no_orphan_snapshots`, `manifest_matches_filesystem`, `validate_all_fixture_units`, `analysis_is_deterministic`, `crlf_variant_equivalent`, `thread_order_independent`, `bincode_and_json_roundtrip_all_units`, `render_is_stable_for_empty_unit`, `render_sorts_attr_maps`, `large_files_skip_list_asserted_by_status`.
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** `engine/scripts/cargo.sh test -p lang-typescript --test golden_ir --test golden_invariants` passes with no `.snap.new` files; deleting any one snapshot makes `every_fixture_file_has_a_snapshot` fail; changing a visitor rule produces a readable snapshot diff.
- **Definition of done:** Snapshots committed for all fixtures existing at merge; policy doc written; CI step added to the Rust job (CI-00x); every later TSA/NEST task lists "golden snapshots updated" in its DoD; global DoD met.

---

---

### TSA-009 — Module resolver

Status: ☐

- **Task ID:** TSA-009
- **Title:** TypeScript module resolver: relative paths, extension and index probing, tsconfig `paths`/`baseUrl`/`extends`, workspace packages, bare specifiers → `ExternalDependency`
- **Problem:** Import specifiers are strings. The linker's best edge (`import`, 0.95) exists only if a specifier is mapped to a file in the repository. Wrong or missing resolution loses cross-file edges (R1) or, worse, links the wrong file. Monorepos with path aliases (`@app/*`) and workspace packages are the norm in NestJS repositories.
- **Why it exists:** ADR-006 (`ModuleResolver` port), target-architecture §3.1 (cross-file resolution consumes a `ModuleResolver`: tsconfig `paths`/`baseUrl`, node resolution, workspace packages) and §7 (resolution cache keyed `(snapshot_id, config_hash)`). Master plan critical-path node "TSA-009 / CG-005".
- **Scope:**
  - `TsModuleResolver` implementing `analysis_ir::ModuleResolver` (TSA-001 `Resolution`, `ResolutionMethod`, `UnresolvedReason`, `ResolveKind`).
  - Pure config parsers: `tsconfig.json` (JSONC, `extends` chain), `package.json` (`name`, `main`, `types`, `module`, `exports` basic string/`.`/subpath map, `workspaces`, dependency maps), `pnpm-workspace.yaml` (`packages:` globs).
  - Resolution order and confidence table; Node builtins; external package name/subpath extraction with `version_range`.
  - A `config_hash` over every config input, exposed for cache keys and the full-rebuild trigger (PRD §24).
- **Explicit non-scope:** Reading `node_modules` (never resolved; externals only). Yarn PnP, `imports` (`#internal`) maps beyond a simple exact map, conditional `exports` conditions other than `import|require|default|types`, `typesVersions`, `rootDirs`, `moduleSuffixes`. TypeScript project references semantics.
- **Files/modules expected to change:** `engine/crates/lang-typescript/src/lib.rs` (re-export), `Cargo.toml` (`jsonc-parser`, `serde_json`, `globset`, `dashmap`).
- **New files/modules expected:** `engine/crates/lang-typescript/src/resolver/{mod.rs, tsconfig.rs, package_json.rs, workspace.rs, probe.rs, external.rs, builtins.rs, cache.rs}`, `tests/resolver.rs`, fixtures `fixtures/repositories/ts-resolve/` (`tsconfig.json`, `tsconfig.base.json`, `src/{a.ts, b.tsx, c.d.ts, dir/index.ts, dir2/package.json, esm/x.ts, aliased/foo.ts, lib/util.ts}`) and `fixtures/repositories/ts-monorepo/` (`package.json` with workspaces, `pnpm-workspace.yaml`, `packages/{core,api}/{package.json, tsconfig.json, src/index.ts}`, `apps/svc/...`).
- **Dependencies (task IDs):** TSA-001 (trait + `Resolution`), TSA-004 (specifier text). INIT-002 (walker supplies the file set). No dependency on `codegraph`.
- **Implementation details:**
  - Inputs: `ResolverInputs { files: Arc<FileSet /* sorted set of RepoPath */>, configs: Vec<(RepoPath, Arc<[u8]>)> /* tsconfig*/package.json/pnpm-workspace bytes, supplied by the caller */ }`. The resolver performs **no I/O**; the indexer reads config files and passes bytes (keeps purity and determinism).
  - Scope selection: for an importing file, the nearest ancestor `tsconfig.json` (honouring `include`/`exclude` globs only to break ties between sibling configs) provides `baseUrl`/`paths`; `extends` (string or array, relative chains, cycle guard depth 8, package `extends` ignored with a diagnostic) merges `compilerOptions` with child override. `paths` targets and `baseUrl` are resolved relative to the config that declared them.
  - Algorithm for `resolve(from, spec, kind)`: (1) `node:` prefix or name in the Node builtin list → `Builtin`. (2) Relative (`./`, `../`, `.`, `..`) → join to `from`'s directory, normalize, reject if it escapes the repo root (`Unresolved(OutsideRepo)`), then `probe`. (3) `paths` match: pattern with at most one `*`, longest literal-prefix wins, targets tried in listed order, each target probed (`TsPath`, 0.98). (4) `baseUrl` + spec probed (`BaseUrl`, 0.95). (5) Workspace package: spec's package name (scope-aware) in the workspace map → `exports["."]`/`types`/`module`/`main` → file, else `src/index.{ts,tsx,js}`; subpath `pkg/sub` → `exports["./sub"]`, else `{dir}/sub` / `{dir}/src/sub` probed (`Workspace`, 0.95). (6) Otherwise bare → `External { ecosystem: "npm", name, subpath, version_range }` with the version range from the nearest `package.json` (`dependencies` > `devDependencies` > `peerDependencies`).
  - `probe(base)`: if `base` has a known extension and exists → hit; ESM `.js`/`.mjs`/`.cjs`/`.jsx` specifiers also try `.ts`/`.mts`/`.cts`/`.tsx` first (`Extension`, 0.98); else try `base + [".ts", ".tsx", ".d.ts", ".js", ".jsx", ".mjs", ".cjs", ".mts", ".cts", ".json"]` (`Extension`, 1.0, ordered, first hit wins); then directory: `{base}/package.json` `types|main` (inside repo), then `{base}/index` + the same extension list (`Index`, 1.0). `.d.ts` is a valid result only when no `.ts`/`.tsx` sibling exists.
  - Determinism: candidate order is fixed; `FileSet` lookups are exact-case; ties never resolved by directory iteration order.
  - Cache: `ResolutionCache { map: DashMap<(ScopeId, SmolStr, ResolveKind), Resolution> }`, owned per `(snapshot, config_hash)`; dropped when `config_hash` changes. `config_hash = blake3(sorted (path, content_hash) of all config inputs)`.
- **Data model changes:** None. `config_hash` is stored in `snapshots.config_hash` (GS) by the indexer.
- **API/protocol changes:** `lang_typescript::resolver::{TsModuleResolver, ResolverInputs, config_hash}`; implements `ModuleResolver`.
- **Concurrency semantics:** `Send + Sync`; `resolve` takes `&self`; the cache is sharded and lock-free for reads. The resolver is immutable after construction.
- **Failure behavior:** Malformed tsconfig/package.json → config ignored plus a `ResolverDiagnostic` (surfaced as a repository-level warning), never a panic. Unresolvable → `Unresolved{reason: NotFound|OutsideRepo|ConfigError|Ambiguous}` (never an error). Missing `extends` target → partial config with diagnostic.
- **Idempotency considerations:** Pure function of `(inputs, from, spec, kind)`; adding an unrelated file never changes resolution of other specifiers except where that file is a candidate.
- **Security considerations:** Path normalization rejects traversal outside the repo and absolute paths; specifiers longer than 1 KiB are `Unresolved`; no filesystem or network access; no `require`/`import` of config JS (e.g. `jest.config.js`, `.eslintrc.js` are never executed).
- **Observability additions:** `module_resolution_total{method}` (relative, extension, index, ts_path, base_url, workspace, external, builtin, unresolved), `module_resolution_cache_hit_ratio` gauge, `resolver_config_errors_total`.
- **Tests required** (`tests/resolver.rs`, fixtures `ts-resolve`, `ts-monorepo`):
  - `relative_with_explicit_extension`, `relative_extension_probe_order`, `esm_js_specifier_maps_to_ts`, `directory_index_resolution`, `directory_package_json_types_field`, `dts_only_when_no_ts_sibling`, `path_escaping_repo_rejected`, `tsconfig_paths_wildcard`, `tsconfig_paths_longest_prefix_wins`, `tsconfig_paths_multiple_targets_first_existing`, `tsconfig_baseurl_non_relative`, `tsconfig_extends_chain_and_override`, `tsconfig_extends_cycle_guarded`, `jsonc_comments_and_trailing_commas`, `nearest_tsconfig_per_package`, `workspace_package_main_and_exports`, `workspace_subpath_and_src_fallback`, `pnpm_workspace_globs`, `bare_specifier_external_with_version_range`, `scoped_package_subpath_split`, `node_builtin_with_and_without_prefix`, `unresolved_not_found_reason`, `case_sensitive_lookup`, `config_hash_changes_on_tsconfig_edit`, `cache_returns_same_resolution`, `malformed_tsconfig_diagnostic_not_panic`.
- **Benchmarks if applicable:** `benches/resolver.rs`: 100k resolutions over a synthetic 20k-file `FileSet` with 30 path aliases; target ≥ 1M resolutions/s with warm cache, ≥ 200k/s cold.
- **Acceptance criteria:** `engine/scripts/cargo.sh test -p lang-typescript --test resolver` passes. On the reference NestJS repository (`#[ignore]`), ≥ 99% of relative and alias imports resolve to files, and the unresolved list is written to `target/resolver-unresolved.txt`.
- **Definition of done:** Tests green; resolution order and confidence table documented in `docs/languages/typescript.md`; `config_hash` wired into the full-rebuild trigger spec (IDX/INC); global DoD met.

---

---

### TSA-010 — ts-semantic helper (optional type-checker enrichment)

Status: ☐

- **Task ID:** TSA-010
- **Title:** `engine/tools/ts-semantic` Node helper (JSON lines over stdio, version handshake, one capability: resolve method call on a typed receiver) behind `SemanticProvider`; disabled by default
- **Problem:** tree-sitter cannot resolve `a.save()` when `a` is typed through an interface, an inherited member, a generic or an alias. Those calls stay `name_ambiguous` (0.3). A type checker can answer exactly, but running it for everything is too slow and needs the target repository's dependencies installed.
- **Why it exists:** ADR-007 (optional Node helper, invoked only for ambiguous references in changed/impacted regions, never over the whole repository, disabled by default in MVP), ADR-006 (compiler helpers: out of process, JSON lines over stdio, version handshake, timeout, always optional), target-architecture §3.1. Gives the benchmark (BEN/QB) a with/without comparison for edge precision.
- **Scope:**
  - The Node tool: handshake, one method `resolve_method_call`, structured errors, graceful shutdown.
  - Rust `ProcessSemanticProvider` implementing `analysis_ir::SemanticProvider`, plus `NoopSemanticProvider`, `SemanticConfig` (default `enabled = false`) and result caching.
  - Protocol spec document and conformance tests with a mock helper.
- **Explicit non-scope:** Other capabilities (type-of-expression, find implementations, rename). Whole-program analysis. Installing target dependencies. Wiring into the linker (CG-005 calls the trait; enabling per repository is a later benchmark-driven decision). Python/Go helpers.
- **Files/modules expected to change:** `engine/crates/analysis-ir/src/traits.rs` (finalize `AmbiguousRef`, `SemanticResolution`, `SemanticCapabilities`, `SemanticBudget`, `SemanticError`), `engine/Cargo.toml`, `engine/scripts/` (tool build step).
- **New files/modules expected:** `engine/tools/ts-semantic/{package.json, tsconfig.json, src/main.ts, src/protocol.ts, src/program.ts, src/resolve.ts, test/*.test.ts, README.md}` (pinned `typescript`, built to `dist/main.js`), `engine/crates/lang-typescript/src/semantic/{mod.rs, process.rs, protocol.rs, cache.rs, noop.rs}`, `tests/semantic_protocol.rs`, `tests/support/mock_helper.js`, `docs/languages/ts-semantic-protocol.md`, fixture `fixtures/repositories/ts-semantic/` (interface-typed receiver, inherited method, generic `Repository<T>`, type alias, overloaded method, union receiver).
- **Dependencies (task IDs):** TSA-001 (trait), TSA-005 (receiver hints produce the `AmbiguousRef`s), TSA-009 (file set / tsconfig scope). Soft: CG-005 consumes it.
- **Implementation details:**
  - Wire format: one JSON object per line (UTF-8, `\n`-terminated, ≤ 1 MiB per line). Request `{"id":N,"method":"hello"|"resolve_method_call"|"shutdown","params":{...}}`; response `{"id":N,"result":{...}}` or `{"id":N,"error":{"code":"...","message":"..."}}`. stderr is free-form logs, never parsed.
  - Handshake (first message, mandatory): `hello{protocol:1, client:"review-engine/<ver>"}` → `{protocol:1, tool:"ts-semantic", version:"0.1.0", typescript:"<ver>", capabilities:["resolve_method_call"]}`. Rust rejects a protocol mismatch (`SemanticError::ProtocolMismatch`) and an unknown capability set; the helper rejects any call before `hello` with `not_initialized`.
  - `resolve_method_call` params: `{root, tsconfig?, files:[{path, content_hash}], refs:[{ref_id, file, line, col, method}]}` where `(line, col)` is the position of the callee's member name (from `IrReference.range`). Result per ref: `{ref_id, target:{file, name, container, line, col, declared_in_interface:bool}|null, reason:"resolved"|"no_type"|"any"|"external"|"not_found"|"union_ambiguous"}`. Targets outside `root` or in `node_modules`/lib files return `null` with `reason:"external"`.
  - Program construction: `ts.createProgram` with root files limited to `files` plus transitive imports resolved by the compiler (cap `maxFiles` 2000), `noEmit`, `skipLibCheck`, `allowJs`, `types: []`, tsconfig `plugins` ignored, `checkJs` off. Programs are cached in-process by `blake3-of-file-set` hash and reused across requests; at most 2 retained.
  - Rust provider: spawns `node dist/main.js` (path from `SemanticConfig { enabled: false, node_path, tool_path, request_timeout_ms: 10_000, startup_timeout_ms: 5_000, max_refs_per_request: 200, max_restarts: 2, max_program_files: 2000 }`) with `env_clear()` plus minimal `PATH`, `cwd` = tool dir, stdin/stdout piped; one reader thread; requests serialized by a mutex. `resolve_ambiguous` batches refs by file set, returns `SemanticResolution { ref_id, target, confidence: 1.0, resolved_by: TypeChecker }` for resolved ones only.
  - Cache: `(blake3(sorted (path, content_hash)), ref position)` → resolution, in-process LRU 10k entries (target-architecture: "results cached by the hash of the file-version set").
  - `capabilities()` returns an empty set when disabled or when the handshake failed, so the linker skips the call without cost.
- **Data model changes:** None. (Edges carry `resolved_by=type_checker`, written by the linker.)
- **API/protocol changes:** New documented protocol v1 (`docs/languages/ts-semantic-protocol.md`); `SemanticConfig` added to engine config (`semantic.enabled = false`).
- **Concurrency semantics:** One helper process per provider instance; requests are serialized; the provider is `Send + Sync`. A hung call is cancelled by killing the process; the next call restarts it (≤ `max_restarts` per run, then the provider degrades to Noop for the remainder of the run).
- **Failure behavior:** Every failure (spawn error, timeout, malformed line, crash, protocol mismatch, over-budget) returns `Err(SemanticError)`; the linker keeps heuristic edges. Nothing here may fail an index or a review run.
- **Idempotency considerations:** Same file set + refs → same results (cache and deterministic ordering of refs by `ref_id`). The helper holds no state that outlives a request except the program cache keyed by content.
- **Security considerations:** Target code is parsed, never executed: no `require` of repo files, no tsconfig plugins, no `postinstall`, no network. Paths are validated to stay under `root` (helper rejects `..` and absolute paths outside). Child runs with a cleared environment (no tokens). Response size and ref counts capped. Source text is read from the checkout by the helper only for listed files.
- **Observability additions:** `semantic_requests_total{status}`, `semantic_request_duration_seconds`, `semantic_refs_resolved_total{reason}`, `semantic_helper_restarts_total`, `semantic_cache_hits_total`; span `semantic_resolve` with `refs`, `files` attributes.
- **Tests required:**
  - Rust (`tests/semantic_protocol.rs`, mock helper): `handshake_ok`, `protocol_mismatch_rejected`, `call_before_hello_rejected`, `timeout_kills_and_restarts`, `crash_degrades_after_max_restarts`, `oversize_line_rejected`, `malformed_json_is_error_not_panic`, `results_cached_by_file_set_hash`, `disabled_provider_is_noop_with_empty_capabilities`, `env_is_cleared_for_child` (helper echoes env).
  - Node (`node --test`): `resolves_interface_receiver_to_interface_method`, `resolves_inherited_method_to_base_class`, `resolves_generic_repository_method`, `type_alias_receiver`, `union_receiver_reports_ambiguous`, `any_receiver_returns_null`, `external_lib_target_is_null`, `path_outside_root_rejected`.
  - `#[ignore]` integration `ts_semantic_end_to_end_fixture` (requires `node`).
- **Benchmarks if applicable:** `benches/semantic.rs` (ignored by default): latency for 50 refs over a 200-file fixture program; recorded to feed the ADR-007 enablement decision.
- **Acceptance criteria:** With `enabled=false`, no process is ever spawned (test asserts). With `enabled=true` on `ts-semantic`, all 6 labelled refs resolve to their expected declarations and each result carries confidence 1.0. Killing the helper mid-request returns `Err` and a subsequent call succeeds.
- **Definition of done:** Tool builds in the engine container image (`npm ci && npm run build`), tests green, protocol doc published, default-off asserted in config tests; global DoD met.

---

---

### NEST-001 — NestJS adapter foundation and @Module facts

Status: ☐

- **Task ID:** NEST-001
- **Title:** NestJS `@Module` facts (imports / providers / controllers / exports, dynamic modules, `@Global`) and the adapter scaffolding (`FrameworkCtx`, decorator-origin resolution)
- **Problem:** NestJS wiring lives in module metadata. Without it, the graph cannot say which providers a module owns, which controllers are mounted, or which modules depend on which, so DI resolution, guard scoping and impact across modules are blind.
- **Why it exists:** PRD §97/§98 (NestJS modules/controllers/providers), ADR-006 (framework facts as neutral `IrFrameworkFact`, adapters run inside the analyzer), target-architecture §3.1/§2.1 (NestJS knowledge only in `lang-typescript`). This task also creates the shared scaffolding used by NEST-002..007.
- **Scope:**
  - `frameworks/mod.rs`: `FrameworkCtx<'t>` (tree, source bytes, `BindingTable`, symbol table, `unit` under construction, sink for facts), adapter registry honouring `AnalyzerConfig.enabled_adapters`, `FrameworkAdapter<FrameworkCtx>` impl `NestJsAdapter` (`detect`: `FrameworkSignals` has `@nestjs/core` or `@nestjs/common`, or the file imports `@nestjs/*`).
  - Decorator-origin resolution: `ctx.decorator_origin(&IrDecorator) -> Origin { NestCommon(name) | Aliased | Unbound }`.
  - Emit `FrameworkFactKind::ModuleDeclaration` for each class decorated `@Module({...})`, attrs per the table below, plus `Global` flag and dynamic-module static methods.
  - Create `docs/graph-schema/framework-facts.md` section `module` if CG-006 has not.
- **Explicit non-scope:** Mapping module facts to graph edges (CG-006 follow-up; this task asserts facts only). Controllers/routes (NEST-002), DI tokens (NEST-003), guards (NEST-004), `forFeature`/`registerQueue` interpretation (NEST-005/006; this task preserves their arguments raw). Runtime module resolution or `ModuleRef`.
- **Files/modules expected to change:** `engine/crates/lang-typescript/src/lib.rs` and `src/analyzer.rs` (run adapters after declarations/imports/references), `src/visit/expr.rs` (expose `to_ir_expr` for decorator args), `docs/languages/typescript.md`.
- **New files/modules expected:** `engine/crates/lang-typescript/src/frameworks/{mod.rs, ctx.rs, decorators.rs, nestjs/mod.rs, nestjs/module.rs}`, `tests/nest_module.rs`, fixture `fixtures/repositories/nest-api/` created here with `src/{main.ts, app.module.ts, users/users.module.ts, users/users.service.ts, auth/auth.module.ts, common/common.module.ts, queue/queue.module.ts, db/database.module.ts}` (synthetic, modelled on a reference NestJS repository without copying it).
- **Dependencies (task IDs):** TSA-003, TSA-004, TSA-005 (decorator `IrExpr` args, `BindingTable`).
- **Implementation details:**
  - Origin rule: decorator callee `Module` whose binding is `import { Module } from '@nestjs/common'` (or alias, or `Nest.Module` via namespace import) → confidence 0.95. An identifier named exactly `Module` with no binding (barrel re-exports) → 0.7. Anything else is ignored.
  - Fact shape (`adapter = "nestjs"`, `kind = ModuleDeclaration`, `symbol` = the class `LocalId`, range = decorator range):
    - `module_name` (class name), `global: Bool`, `dynamic: Bool` (class has a static method returning an object with a `module` key).
    - `imports`, `providers`, `controllers`, `exports`: each `AttrValue::List` of `Map` entries `{ name, form, import_specifier?, imported?, args? }` where `form ∈ class | for_root | for_root_async | for_feature | register | register_queue | provider_object | forward_ref | spread | dynamic_expr`. `name` is the identifier text (class entries) or the callee root (`TypeOrmModule`); `args` keeps the raw `IrExpr` list for `forRoot/forFeature/register*` calls so NEST-005/006 can interpret them.
    - Provider objects `{ provide, useClass | useExisting | useFactory | useValue, inject, scope }`: `provide` stored as `Expr` (identifier, string or `APP_*` constant), `use_class` as name, `inject` as list of names, `use_value`/`use_factory` marked `Other` (never evaluated).
  - Resolution to files is not done here; each entry carries its import binding (`import_specifier`, `imported`) via `BindingTable` so the linker (CG-005) resolves it with the module resolver (TSA-009).
  - `@Global()` on the same class sets `global = true`. `@Module` arguments that are not an object literal (e.g. a shared constant) → fact emitted with empty lists and `attrs.metadata_ref = Expr` plus an `UnsupportedConstruct` diagnostic.
  - Dynamic module returns: for `static forRoot(...)` methods in module classes, walk `return { module: X, providers: [...], imports: [...], exports: [...] }` and emit a second `ModuleDeclaration` with `attrs.dynamic_method = "forRoot"`, same entry shape.
  - Spread/computed entries: `...OTHER_PROVIDERS` → `form = spread`, `name` = identifier; arrays built by calls (`[...providers()]`) → `dynamic_expr`; nothing is evaluated.
  - Fact `confidence`: 0.95 for fully literal metadata, 0.7 when any `spread`/`dynamic_expr` entry exists.
- **Data model changes:** None.
- **API/protocol changes:** Attribute contract for `module` facts documented in `docs/graph-schema/framework-facts.md`; `FrameworkCtx` and `NestJsAdapter` are crate-internal APIs, adapters selected via `AnalyzerConfig.enabled_adapters` (`nestjs`).
- **Concurrency semantics:** Adapters run per file inside the analyzer; no shared state.
- **Failure behavior:** Malformed metadata → fact with what could be extracted plus a diagnostic; never fails the unit.
- **Idempotency considerations:** Entries in source order; maps sorted; same file → same facts.
- **Security considerations:** `useValue` literals (may hold secrets) are never stored: `use_value` is recorded as `kind` only (`object`, `string`, `number`, `ident`).
- **Observability additions:** `framework_facts_emitted_total{adapter="nestjs",kind="module_declaration"}`, `framework_fact_issues_total{adapter,code}`.
- **Tests required** (`tests/nest_module.rs`, fixture `nest-api`):
  - `module_with_all_four_arrays`, `import_binding_captured_for_each_class_entry`, `alias_import_of_module_decorator`, `unbound_module_decorator_lower_confidence`, `for_root_for_feature_register_args_preserved`, `provider_object_use_class_use_existing_use_factory`, `provide_token_string_and_symbol_constants`, `global_decorator_sets_flag`, `dynamic_module_static_for_root`, `spread_entries_marked_and_confidence_lowered`, `non_literal_metadata_diagnostic`, `forward_ref_entries`, `non_nest_decorator_named_module_ignored`, `detect_false_without_nest_signals`, `use_value_secret_not_stored`, `enabled_adapters_filter_disables_nestjs`.
- **Benchmarks if applicable:** Covered by `parse_extract_all`; adapter overhead ≤ 10% on `nest-api`.
- **Acceptance criteria:** `engine/scripts/cargo.sh test -p lang-typescript --test nest_module` passes; `nest-api/app.module.ts` yields a `ModuleDeclaration` whose `imports` contain every module class referenced in source; golden snapshots updated (TSA-008).
- **Definition of done:** Tests green; contract documented; scaffolding reviewed as reusable for NEST-002..007; global DoD met.

---

---

### NEST-002 — Controllers and route decorators → HttpRoute facts

Status: ☐

- **Task ID:** NEST-002
- **Title:** Controllers and route decorators (`@Controller` prefix, `@Get/@Post/@Put/@Patch/@Delete/@All/@Options/@Head/@Sse`, versioning, global prefix) → `Controller`/`HttpRoute`/`HttpGlobalConfig` facts feeding `APIEndpoint` + `HANDLED_BY`/`ROUTES_TO`
- **Problem:** Impact analysis must answer "which public endpoints can this change reach?" and the verifier must check "is this endpoint guarded?". That needs each handler's HTTP method and full path. NestJS builds the path from a controller prefix, a handler path, an optional version and an application-level global prefix that lives in `main.ts`.
- **Why it exists:** PRD §98 (`@Get('/users/:id')` → `APIEndpoint → HANDLED_BY → UserController.getUser`), target-architecture §3.3 (synthetic node `http:{METHOD} {normalized_path}`), CG-006 contract (`route` category: `symbol`, `method`, `path`, `controller`; global prefix as a global fact).
- **Scope:**
  - Facts: `Controller` (class), `HttpRoute` (handler), `HttpGlobalConfig` (`setGlobalPrefix`, `enableVersioning`).
  - Path assembly and normalization; versioning attrs; multiple paths per route; `@HttpCode`, `@Header`, `@Redirect`, `@Sse` metadata.
  - Small extension of the CG-006 mapper hook `apply_globals` to consume versioning global config (coordinated change in `codegraph/src/framework/http.rs`, no NestJS identifiers added).
- **Explicit non-scope:** Inherited routes from abstract base controllers (diagnostic only), `RouterModule.register` path nesting (recorded as `attrs.router_module_hint` for a later task), GraphQL resolvers, microservice `@MessagePattern`, WebSocket gateways, applyDecorators-composed routes, OpenAPI decorators.
- **Files/modules expected to change:** `engine/crates/lang-typescript/src/frameworks/nestjs/mod.rs`, `docs/graph-schema/framework-facts.md` (route/global-config rows), `engine/crates/codegraph/src/framework/http.rs` (versioning global).
- **New files/modules expected:** `engine/crates/lang-typescript/src/frameworks/nestjs/{controller.rs, routes.rs, global_config.rs, path.rs}`, `tests/nest_routes.rs`, fixture files `nest-api/src/{users/users.controller.ts, orders/orders.controller.ts, health/health.controller.ts, versioned/v.controller.ts, dynamic/dynamic-paths.controller.ts}`, `nest-api/src/main.ts` (extended).
- **Dependencies (task IDs):** NEST-001 (scaffolding), TSA-003 (decorators, `const_value`), TSA-004.
- **Implementation details:**
  - `@Controller` forms: none → prefix `""`; string; array of strings; `{ path, version, host }`. Origin check per NEST-001 (`@nestjs/common` binding, 0.95; unbound 0.7). Fact `Controller` attrs: `symbol`, `prefixes` (List<Str>), `version?` (Str|List|`NEUTRAL`), `host?`.
  - Route decorators: `Get, Post, Put, Patch, Delete, All, Options, Head` → method (`All` → `ALL`); `Sse(path)` → `GET` with `attrs.sse = true`. Argument: none → `""`; string; array of strings (one fact per path); a no-substitution template or an identifier/member resolving to a same-file `const_value` (TSA-003) is substituted; otherwise `path = "{<expr text ≤ 60 chars>}"`, `attrs.path_dynamic = true`, confidence lowered to 0.5. Only methods in a `@Controller` class (or classes whose heritage is a known controller in the same file) emit.
  - Fact `HttpRoute` attrs: `symbol` (handler), `controller` (class symbol), `method`, `path` (controller prefix + handler path, normalized), `raw_path`, `controller_prefix`, `version?` (handler `@Version(...)` overrides controller), `status_code?` (`@HttpCode(n)`), `redirect?`, `header_names` (names only), `sse`, `dynamic`.
  - Normalization (`path.rs`): join segments with `/`, collapse repeated `/`, ensure leading `/`, drop trailing `/` (root stays `/`), strip regex parts from params (`:id(\\d+)` → `:id`), keep `*` wildcards, no lowercasing of literal segments (NestJS is case-sensitive by default).
  - Global config: `app.setGlobalPrefix('api', { exclude: [...] })` → `HttpGlobalConfig{ kind: "global_prefix", prefix, exclude: List }`; `app.enableVersioning({ type: VersioningType.URI|HEADER|MEDIA_TYPE|CUSTOM, defaultVersion, prefix: 'v', header, key })` → `{ kind: "versioning", type, default_version, prefix }`. Detected in any file by a call on an identifier whose `declared_type` is `INestApplication`/`NestExpressApplication` or assigned from `NestFactory.create(...)`; `symbol` = enclosing function (usually `bootstrap`).
  - Endpoint ID produced by the linker (not here): `http:{METHOD} {global_prefix}{/v{version} if URI}{path}`; the adapter applies only controller-level parts so incremental updates can re-run the global parts (CG-006 `apply_globals`).
  - Fact confidence: 0.95 literal; 0.5 dynamic path. Handler symbol attrs get `attrs.http_method` hint for reviewers.
- **Data model changes:** None.
- **API/protocol changes:** `route`/`controller`/`global config` attribute contract rows in `docs/graph-schema/framework-facts.md`.
- **Concurrency semantics:** Per-file pure; globals are returned as facts and applied once by the linker.
- **Failure behavior:** Unrecognized argument shapes degrade to `path_dynamic`; a route decorator on a non-controller class is ignored with an info diagnostic. Never fails the unit.
- **Idempotency considerations:** Source-order facts; one fact per (handler, path) with deterministic order; identical re-analysis yields identical endpoints.
- **Security considerations:** Routes are metadata only; header values and redirect URLs are not stored. Public-endpoint exposure is judged later from guards (NEST-004) and global config.
- **Observability additions:** `framework_facts_emitted_total{adapter="nestjs",kind="http_route"}`, `nest_routes_dynamic_total`.
- **Tests required** (`tests/nest_routes.rs`):
  - `controller_prefix_forms_string_array_object`, `empty_controller_prefix`, `all_http_method_decorators`, `route_path_array_creates_multiple_facts`, `path_normalization_slashes_and_trailing`, `param_regex_stripped`, `handler_version_overrides_controller_version`, `version_neutral_symbol`, `const_path_resolved_same_file`, `dynamic_path_flagged_low_confidence`, `sse_route_is_get_with_flag`, `http_code_and_redirect_attrs`, `route_in_non_controller_ignored`, `set_global_prefix_with_exclude`, `enable_versioning_uri_default_version`, `global_config_in_non_main_file`, `alias_imported_decorators`, `abstract_base_controller_routes_diagnostic`, and an end-to-end `nest_api_endpoint_inventory` (codegraph mapper test using `nest-api`, asserting nodes `http:GET /api/v1/users/:id` and `HANDLED_BY` + `ROUTES_TO` edges).
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** `engine/scripts/cargo.sh test -p lang-typescript --test nest_routes` and the codegraph mapping test pass; on the reference NestJS repository (`#[ignore]`) the extracted endpoint count equals the count of route decorators found by an independent regex scan (written to `target/nest-route-counts.txt`).
- **Definition of done:** Tests green; golden snapshots updated; contract rows documented; global DoD met.

---

---

### NEST-003 — Dependency injection facts and references

Status: ☐

- **Task ID:** NEST-003
- **Title:** DI: constructor parameter types and `@Inject` / `@InjectRepository` / `@InjectQueue` / `@InjectDataSource` tokens → `DiInjection` facts and `RefKind::DiInjection` references (`resolved_by = di_constructor`)
- **Problem:** In NestJS most cross-class calls go through injected fields (`this.users.find()`), not imports of instances. Linking those calls requires knowing what each constructor parameter *is*: a class by type, or a token (`'CONFIG'`, `getRepositoryToken(User)`, a queue name) that a module registered elsewhere.
- **Why it exists:** Target-architecture §3.1 confidence table (`di_constructor` 0.85, one of the core edge sources); ADR-007 (constructor-typed DI 0.7–0.9 in the prototype). TSA-005 types `this.field` receivers; this task additionally records *injection edges* (consumer class → provider) and token semantics for TypeORM/BullMQ.
- **Scope:**
  - `DiInjection` framework facts and `IrReference{kind: DiInjection}` from the consuming class for: constructor parameters (typed), parameter properties, `@Inject(token)`, `@Inject(forwardRef(() => X))`, `@Optional()`, `@Self/@SkipSelf/@Host` (recorded as flags), property injection (`@Inject() private foo: Foo`), and the `@Inject*` family from `@nestjs/*` packages.
  - Token classification and entity/queue extraction for `@InjectRepository(E)`, `@InjectEntityManager()`, `@InjectDataSource()`, `@InjectQueue('q')`, `@InjectModel(X)`.
- **Explicit non-scope:** Matching the token to a provider registration (linker/CG-006 using NEST-001 `provide` entries). `ModuleRef.get()`, `moduleRef.resolve()`, request-scoped provider semantics, custom factory `inject: [...]` arrays beyond recording names (done in NEST-001).
- **Files/modules expected to change:** `engine/crates/lang-typescript/src/frameworks/nestjs/mod.rs`, `src/visit/references.rs` (allow adapter-added references, no duplicate `TypeRef` for the same parameter), `docs/graph-schema/framework-facts.md`.
- **New files/modules expected:** `engine/crates/lang-typescript/src/frameworks/nestjs/{di.rs, tokens.rs}`, `tests/nest_di.rs`, fixtures `nest-api/src/{users/users.service.ts (extended), orders/orders.service.ts, di/{tokens.ts, custom-provider.service.ts, optional.service.ts, forward-ref.service.ts, property-injection.service.ts}}`.
- **Dependencies (task IDs):** NEST-001, TSA-003 (constructor params, parameter properties), TSA-005 (type environment).
- **Implementation details:**
  - Eligibility: class has a Nest class decorator (`Injectable, Controller, Module, Catch, Processor, WebSocketGateway, Resolver, Global`-adjacent list) **or** a base `*Guard/*Interceptor/*Pipe/*Filter` heritage; plain classes are skipped (avoids DTO/entity noise), unless a parameter carries an `@Inject*` decorator.
  - One fact per constructor parameter (`kind = DiInjection`, `symbol` = the class, range = parameter), attrs: `param_index`, `param_name`, `type` (head identifier), `type_args` (List<Str>), `token` (Expr or absent), `token_kind ∈ class | string | symbol | forward_ref | typeorm_repository | typeorm_entity_manager | typeorm_data_source | bullmq_queue | model | unknown`, `optional`, `scope_flags` (self/skip_self/host), `property` (field name when a parameter property; matches the TSA-003 synthetic property), `entity?` (for repository tokens), `queue?` (queue name literal or `{expr}`), `confidence`.
  - Token precedence: an explicit `@Inject(T)` wins over the type annotation. Interface or primitive typed params without a token get `token_kind = unknown`, confidence 0.3 (type cannot be a DI token at runtime).
  - Reference emission: `IrReference{ kind: DiInjection, name: type head (or class identifier inside the token), receiver: None, import_binding, attrs: { resolved_by_hint: "di_constructor", token_kind, property } }` from the consuming class symbol; for `forwardRef(() => X)` the arrow's return identifier is the name.
  - `@InjectRepository(User)` → `entity = "User"`, `token_kind = typeorm_repository`; also sets `attrs.di_entity = "User"` on the TSA-003 property symbol so NEST-005 can type `this.repo` even when the declared type lacks generics. `@InjectQueue('mail')` → `queue = "mail"`.
  - Fact confidence: class-typed param 0.9; explicit class token 0.9; string/symbol token 0.7 (needs provider match); unknown 0.3. The linker maps class-typed ones to `resolved_by = di_constructor` (0.85) per the confidence table.
  - Property injection: `public_field_definition` with `@Inject()`; no index (`param_index = -1`, `property` set).
- **Data model changes:** None.
- **API/protocol changes:** `di` attribute contract row in `docs/graph-schema/framework-facts.md` (additive).
- **Concurrency semantics:** Per-file pure.
- **Failure behavior:** Parameters whose type is a destructured pattern or missing annotation emit `token_kind = unknown` and an info diagnostic; no failure.
- **Idempotency considerations:** Facts and references in parameter order; no duplicate reference with TSA-005's generic `TypeRef` (the adapter marks the TypeRef `attrs.di = true` instead of emitting twice, covered by a test).
- **Security considerations:** Token string literals are kept (they are identifiers, not secrets); tokens whose text matches the TSA-003 secret regex are replaced by `<redacted>`.
- **Observability additions:** `framework_facts_emitted_total{adapter="nestjs",kind="di_injection"}`, `nest_di_tokens_total{token_kind}`.
- **Tests required** (`tests/nest_di.rs`):
  - `constructor_class_typed_params`, `parameter_property_links_field`, `explicit_inject_string_token`, `explicit_inject_symbol_constant_token`, `inject_overrides_type_annotation`, `forward_ref_unwraps_arrow`, `optional_flag`, `inject_repository_extracts_entity`, `inject_queue_extracts_name`, `inject_data_source_and_entity_manager`, `property_injection`, `non_injectable_class_skipped`, `interface_typed_param_without_token_low_confidence`, `no_duplicate_typeref_for_di_param`, `generic_type_args_recorded`, `unbound_inject_decorator_lower_confidence`, `secret_like_token_redacted`.
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** `engine/scripts/cargo.sh test -p lang-typescript --test nest_di` passes; every constructor parameter of every `@Injectable` class in `nest-api` yields exactly one `DiInjection` fact; on the reference NestJS repository (`#[ignore]`), ≥ 95% of injectable-class constructor parameters get `confidence ≥ 0.7` (stats to `target/nest-di-stats.json`).
- **Definition of done:** Tests green; golden snapshots updated; contract row documented; global DoD met.

---

---

### NEST-004 — Guards, interceptors, pipes → Middleware facts

Status: ☐

- **Task ID:** NEST-004
- **Title:** Guards / interceptors / pipes / filters (`@UseGuards`, `@UseInterceptors`, `@UsePipes`, `@UseFilters`, `APP_GUARD`-style global providers, `useGlobal*`, public/role metadata) → `Middleware`, `MiddlewareBinding`, `GlobalProvider`, `RouteMetadata` facts (`AUTHORIZES` / `VALIDATES`)
- **Problem:** The highest-value review finding class (authorization bypass) needs to know which endpoints are protected by which guard, at which scope, and which are explicitly public. The binding can sit on the method, on the controller, or globally in a module provider or in `main.ts`, and metadata decorators (`@Public()`, `@Roles('admin')`) change a guard's behaviour without changing the guard's code.
- **Why it exists:** PRD §18 (`AUTHORIZES`, `VALIDATES`), §98, auth-bypass golden scenario; CG-006 contract (`guard`: `symbol`, `targets` or `scope=global`; controller-level guards fan out). Target-architecture §3.7 (guards feed risk and verification contradiction search "upstream guard on all paths").
- **Scope:**
  - Middleware class detection: `role ∈ guard | interceptor | pipe | filter | middleware`, by `implements CanActivate | NestInterceptor | PipeTransform | ExceptionFilter | NestMiddleware`, by `extends AuthGuard(...)`/known base names, or by `@Catch` (filter).
  - Bindings: `@UseGuards/@UseInterceptors/@UsePipes/@UseFilters` on class, method and parameter pipes (`@Body(ValidationPipe)`, `@Query('id', ParseIntPipe)`).
  - Global bindings: provider objects with `provide: APP_GUARD|APP_INTERCEPTOR|APP_PIPE|APP_FILTER`, and `app.useGlobalGuards/Interceptors/Pipes/Filters(...)`.
  - Metadata: `@SetMetadata(key, value)`, custom decorators defined with `SetMetadata`, `@Public()`, `@Roles(...)`, `Reflector` reads inside guards.
- **Explicit non-scope:** Evaluating guard logic (TSA-006 `Condition`/`Return` facts on `canActivate` are consumed by CHG/VER), Passport strategy resolution, `MiddlewareConsumer.apply().forRoutes()` (recorded as `Middleware` only when a class is bound; route matching deferred), WebSocket/GraphQL guards.
- **Files/modules expected to change:** `engine/crates/lang-typescript/src/frameworks/nestjs/mod.rs`, `docs/graph-schema/framework-facts.md`, `engine/crates/codegraph/src/framework/auth.rs` (extend CG-006 mapping: `pipe` → `VALIDATES`; `interceptor`/`filter` → node refinement only).
- **New files/modules expected:** `engine/crates/lang-typescript/src/frameworks/nestjs/{middleware.rs, bindings.rs, global_providers.rs, metadata.rs}`, `tests/nest_middleware.rs`, fixtures `nest-api/src/auth/{jwt-auth.guard.ts, roles.guard.ts, public.decorator.ts, roles.decorator.ts, auth.controller.ts}`, `nest-api/src/common/{logging.interceptor.ts, validation.pipe.ts, http-exception.filter.ts}`, `nest-api/src/users/users.controller.ts` (bindings), `nest-api/src/main.ts` (`useGlobalPipes`).
- **Dependencies (task IDs):** NEST-001, NEST-002 (handler/controller symbols), NEST-003 (provider tokens), TSA-005, TSA-006 (guard-decorator facts).
- **Implementation details:**
  - `Middleware` fact attrs: `symbol` (class), `role`, `name`, `entry_symbol` (LocalId of `canActivate`/`intercept`/`transform`/`catch`/`use`, if present), `uses_reflector` (class has a `Reflector` DI param), `reads_metadata_keys` (string keys passed to `reflector.get/getAllAndOverride`), `implements` (List). Confidence 0.95 by `implements`, 0.8 by naming + method presence, 0.6 by name only.
  - `MiddlewareBinding` attrs: `role`, `scope ∈ class | method | param | global`, `target` (class or handler `LocalId`; absent for global), `middleware` (List<Map{name, form: class|instance|call|forward_ref, import_specifier?, imported?}>), `order` (argument index). `@UseGuards(new X())` → `form = instance`; `@UseGuards(AuthGuard('jwt'))` → `form = call`, name `AuthGuard`, `args` `[Str('jwt')]`.
  - Global: provider-object entries from NEST-001 whose `provide` is `APP_GUARD|APP_INTERCEPTOR|APP_PIPE|APP_FILTER` become `GlobalProvider{ role, middleware, via: "module_provider", module: <class symbol> }` with the guard class resolved via its import binding; `app.useGlobal*()` calls become `GlobalProvider{ via: "main" }`. The linker emits `AUTHORIZES` from global guards to **all** endpoints (CG-006 `GlobalFact::GlobalGuard`), minus `@Public()` endpoints flagged below.
  - Metadata: `MetadataDecoratorDefinition{ name, key, kind: set_metadata | create_decorator }` for `export const Roles = (...r) => SetMetadata('roles', r)` and `Reflector.createDecorator`; `RouteMetadata{ symbol, key, value: Expr, via_decorator }` for each usage (`@Public()` → key from the definition in the same file or by the conventional name `IS_PUBLIC_KEY`/`isPublic`, flagged `convention_based = true`, confidence 0.6 when the definition is in another file).
  - Edge semantics handed to CG-006: guard → `AUTHORIZES` endpoint; pipe → `VALIDATES` endpoint (handler/param); interceptor/filter → bound-to relation kept in node attrs only.
  - Class-level bindings are emitted once with `scope = class`; fan-out to handlers is the linker's job, which keeps incremental updates cheap when a handler is added.
- **Data model changes:** None (edges are written by the linker).
- **API/protocol changes:** `guard`/`pipe`/`global_provider`/`route_metadata` attribute contract in `docs/graph-schema/framework-facts.md`.
- **Concurrency semantics:** Per-file pure; global facts are collected by the linker after all files.
- **Failure behavior:** Unknown middleware expression → binding with `form = dynamic_expr`, confidence 0.4, no edge. Missing class resolution is a linker issue (`FactIssue`), not an analyzer error.
- **Idempotency considerations:** Facts in decorator and argument order; the same file always yields the same binding order (guard order matters semantically).
- **Security considerations:** This data decides "is the endpoint guarded", so under-reporting must be conservative: any binding the adapter cannot parse is emitted as `unknown` rather than dropped, so the verifier cannot claim "no guard" on an endpoint with an unparsed binding.
- **Observability additions:** `framework_facts_emitted_total{adapter="nestjs",kind="middleware|middleware_binding|global_provider|route_metadata"}`, `nest_unparsed_bindings_total`.
- **Tests required** (`tests/nest_middleware.rs`):
  - `guard_class_by_implements_can_activate`, `guard_by_authguard_extends_call`, `interceptor_pipe_filter_roles`, `use_guards_on_method_and_class_scopes`, `use_guards_instance_and_call_forms`, `guard_order_preserved`, `param_level_pipe_binding`, `app_guard_provider_global`, `use_global_pipes_in_main`, `set_metadata_decorator_definition`, `public_decorator_route_metadata`, `roles_metadata_with_values`, `reflector_keys_collected`, `unparsed_binding_kept_as_unknown`, `alias_and_namespace_imports`, `non_nest_use_guards_name_ignored`, and codegraph test `guard_authorizes_fanout_respects_public` over `nest-api`.
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** `engine/scripts/cargo.sh test -p lang-typescript --test nest_middleware` and the codegraph mapping test pass; on `nest-api` every endpoint has either an `AUTHORIZES` edge or an explicit `@Public` metadata fact, and the planted unguarded endpoint is reported as neither.
- **Definition of done:** Tests green; golden snapshots updated; contract documented; global DoD met.

---

---

### NEST-005 — TypeORM entities, relations and repository access heuristics

Status: ☐

- **Task ID:** NEST-005
- **Title:** TypeORM `@Entity/@Column/relations` + repository call heuristics (save/update/delete/insert/createQueryBuilder/query) → `OrmEntity`/`OrmColumn`/`OrmRelation`/`OrmAccess` facts → `DatabaseEntity`/`DatabaseTable`, `READS_TABLE`/`WRITES_TABLE`
- **Problem:** Database writes are where correctness and security regressions are costliest (missing transaction, wrong table, delete without filter). To flag them, the graph needs the table behind each entity and which code reads or writes it, even though calls go through injected repositories with generic types.
- **Why it exists:** PRD §17 (`DatabaseEntity`, `DatabaseTable`, `DatabaseColumn`), §18 (`READS_TABLE`, `WRITES_TABLE`), §97 (TypeORM entities); target-architecture §3.3 (`db:{schema}.{table}` node IDs); CG-006 contract (`entity`: `symbol`, `table`, `schema?`; `db_access`: `symbol`, `entity`, `op`).
- **Scope:**
  - Entity facts: `@Entity(name | {name, schema, database})`, `@ViewEntity`, `@ChildEntity`, with default table-name inference.
  - Column facts (`@Column`, `@PrimaryColumn`, `@PrimaryGeneratedColumn`, `@Create/Update/DeleteDateColumn`, `@VersionColumn`, `@Index`, `@Unique`) and relation facts (`@OneToOne/@OneToMany/@ManyToOne/@ManyToMany`, `@JoinColumn`, `@JoinTable`).
  - Access facts for repository, `EntityManager`, `DataSource` and QueryBuilder calls and raw SQL strings, with `op = read|write` and entity resolution.
- **Explicit non-scope:** Migrations (`MigrationInterface` classes; `Migration` nodes later), Prisma/Sequelize/Knex/Mongoose (extension points only; TSA-006 already flags name-based writes), schema inference from SQL, custom `NamingStrategy`, query result shape analysis, `.env` or datasource config reads (NEST-007).
- **Files/modules expected to change:** `engine/crates/lang-typescript/src/frameworks/mod.rs` (register `typeorm`), `docs/graph-schema/framework-facts.md`.
- **New files/modules expected:** `engine/crates/lang-typescript/src/frameworks/typeorm/{mod.rs, entity.rs, column.rs, relation.rs, access.rs, naming.rs, sql.rs}`, `tests/typeorm.rs`, fixtures `nest-api/src/db/entities/{user.entity.ts, order.entity.ts, audit-log.entity.ts, base.entity.ts}`, `nest-api/src/users/users.service.ts` and `orders/orders.service.ts` (access patterns), `nest-api/src/db/raw-sql.service.ts`.
- **Dependencies (task IDs):** NEST-003 (`di_entity`, repository tokens), TSA-005 (receiver `declared_type` with generics), TSA-006 (`DbWriteLike` heuristics and receiver tests, shared in `db_heuristics`), NEST-001.
- **Implementation details:**
  - `OrmEntity` attrs: `symbol`, `entity` (class name), `table`, `schema?`, `table_inferred` (true when no name given), `kind ∈ entity | view | child`, `extends` (base entity name). Default table name = TypeORM `snakeCase(className)` (`UserAccount` → `user_account`), confidence 0.7 when inferred, 0.95 when explicit. Origin check: `import { Entity } from 'typeorm'`.
  - `OrmColumn` attrs: `symbol` (the property), `entity`, `property`, `column` (option `name` or property name), `column_type`, `primary`, `generated`, `nullable`, `unique`, `length`, `kind ∈ column | primary | created_at | updated_at | deleted_at | version | index`. Defaults are not stored.
  - `OrmRelation` attrs: `symbol` (property), `entity`, `property`, `relation ∈ one_to_one | one_to_many | many_to_one | many_to_many`, `target` (from `() => Type`), `inverse_property` (second arrow), `join_column` / `join_table` names, `cascade` (bool or list), `eager`, `nullable`.
  - `OrmAccess` (one per qualifying call): attrs `symbol` (enclosing symbol), `entity` (type name or `None`), `op`, `method`, `via ∈ repository | manager | query_builder | raw_sql`, `receiver_declared_type`, `table_hint?`, `in_transaction` (from TSA-006), `confidence`.
    - Writes: `save, insert, update, upsert, delete, remove, softDelete, softRemove, restore, increment, decrement, clear, recover` plus QueryBuilder chains containing `.insert()/.update()/.delete()/.softDelete()/.restore()`. Reads: `find, findOne, findOneBy, findBy, findAndCount, findOneOrFail, count, countBy, exist(s), sum, average, minimum, maximum`, builder terminals `getOne/getMany/getRawOne/getRawMany/getCount/stream`. Non-DB `create/merge/preload` ignored. `query(sql)`: first keyword `SELECT/WITH` → read; `INSERT/UPDATE/DELETE/TRUNCATE/ALTER/DROP/CREATE` → write; table names extracted by regex after `FROM|JOIN|INTO|UPDATE|TABLE` into `table_hint`; the SQL text itself is not stored.
    - Entity resolution order: generic argument of the receiver's declared type (`Repository<User>`, 0.9) → `di_entity` of the injected property (0.9) → first argument identifier for manager calls (`manager.save(User, x)`, 0.85) → `getRepository(User)` / `createQueryBuilder(User, 'u')` / `.from(User)`, `.into(User)`, `.update(User)` (0.8) → `Repository` typed without generics: entity absent, confidence 0.4. Calls on receivers failing the TSA-006 receiver test are not emitted.
  - Confidence by `via`: repository typed 0.9; manager 0.85; query_builder 0.75; raw_sql 0.6. The linker resolves `entity` through the entity fact to `db:{schema}.{table}` (`schema` defaults to `public`).
- **Data model changes:** None.
- **API/protocol changes:** `entity`, `column`, `relation`, `db_access` rows in `docs/graph-schema/framework-facts.md` (`db_access.op` is `read|write` exactly as CG-006).
- **Concurrency semantics:** Per-file pure; entity→table lookup is the linker's job.
- **Failure behavior:** Entity without decorator args → inferred name; unresolved entity → fact kept with `entity = None` (CG-006 emits `FactIssue::UnresolvedEntity`, no edge).
- **Idempotency considerations:** Facts in source order; column/relation facts ordered by property position.
- **Security considerations:** Raw SQL strings are never stored (only verb and table names). `@Column({ default: ... })` values are not recorded. Column names that match secret patterns are still recorded as names only (they are schema, not data).
- **Observability additions:** `framework_facts_emitted_total{adapter="typeorm",kind}`, `typeorm_access_total{op,via}`, `typeorm_unresolved_entity_total`.
- **Tests required** (`tests/typeorm.rs`):
  - `entity_explicit_name_and_schema`, `entity_default_snake_case_inferred`, `entity_object_form`, `view_and_child_entity`, `column_kinds_and_options`, `relations_with_inverse_and_join`, `inherited_base_entity_columns_noted`, `repository_save_write_with_entity_from_generic`, `find_read_ops`, `inject_repository_token_resolves_entity`, `manager_save_first_arg_entity`, `get_repository_call`, `query_builder_insert_update_delete_chain`, `query_builder_select_read`, `raw_query_verbs_and_table_hint`, `sql_text_not_stored`, `non_db_save_ignored`, `unresolved_entity_low_confidence`, `transaction_flag_propagated`, plus codegraph test `db_access_edges_with_ops` on `nest-api`.
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** `engine/scripts/cargo.sh test -p lang-typescript --test typeorm` passes; on `nest-api` each `@Entity` yields one table node with the expected name and every labelled repository call has the expected `op` and entity (precision/recall ≥ 0.95 on the labelled set `fixtures/repositories/nest-api/expected/db-access.json`).
- **Definition of done:** Tests green; heuristic tables documented; golden snapshots updated; global DoD met.

---

---

### NEST-006 — BullMQ processors, workers and producers

Status: ☐

- **Task ID:** NEST-006
- **Title:** BullMQ `@Processor` / `@Process` / `WorkerHost` + `queue.add` / `addBulk` → `QueueRegistration`/`QueueConsumer`/`QueueJobHandler`/`QueueProducer` facts → `Queue`, `QueueProducer`, `QueueConsumer`, `JobHandler` nodes, `PRODUCES_JOB`/`CONSUMES_JOB`
- **Problem:** Queues are an implicit coupling: a change to a processor's payload handling breaks producers elsewhere, and nothing in imports connects them. Reviewers and impact analysis need producer ↔ consumer links by queue and job name, plus the idempotency options (job ids) that repository rules often mandate.
- **Why it exists:** PRD §17 (`Queue`, `QueueProducer`, `QueueConsumer`, `JobHandler`), §18 (`PRODUCES_JOB`, `CONSUMES_JOB`), §97 (BullMQ processors/producers); CG-006 contract (`queue_producer`/`queue_consumer`: `symbol`, `queue`, `job?`; synthetic node `queue:{name}`); POL rules about queue job ids.
- **Scope:**
  - Registration: `BullModule.registerQueue({name})` / `registerQueueAsync`, `BullModule.forRoot`, `new Queue('name')`, `new Worker('name', handler)`, `FlowProducer`.
  - Consumers: `@Processor('q' | {name, concurrency})` classes (extending `WorkerHost` for `@nestjs/bullmq`, or legacy `@nestjs/bull`), `@Process('job' | {name} | none)` handlers, and `process(job)` bodies that switch on `job.name`, `@OnWorkerEvent` / `@OnQueueEvent` handlers (recorded as events, not job handlers).
  - Producers: `queue.add(name, data, opts)`, `queue.addBulk([{name, data, opts}])`, `queue.upsertJobScheduler`, flow `add({name, queueName, children})` (depth ≤ 3).
- **Explicit non-scope:** Redis connection config (NEST-007 handles env reads), job payload type inference, retry/backoff semantics, delayed-job timing, other brokers (RabbitMQ, SQS, Kafka), Bull Board.
- **Files/modules expected to change:** `engine/crates/lang-typescript/src/frameworks/mod.rs` (register `bullmq`), `docs/graph-schema/framework-facts.md`.
- **New files/modules expected:** `engine/crates/lang-typescript/src/frameworks/bullmq/{mod.rs, registration.rs, consumer.rs, producer.rs, queue_name.rs}`, `tests/bullmq.rs`, fixtures `nest-api/src/queue/{mail.processor.ts, mail.producer.service.ts, legacy-bull.processor.ts, flow.service.ts, queue.constants.ts, queue.module.ts}`.
- **Dependencies (task IDs):** NEST-001 (`register*` args), NEST-003 (`@InjectQueue` token with `queue`), TSA-005 (`declared_type` `Queue`), TSA-003 (`const_value` for names).
- **Implementation details:**
  - Queue-name resolution (`queue_name.rs`): string literal → exact; same-file `const` or enum-member `const_value` (`QueueNames.MAIL`) → exact (0.9); template without substitutions → exact; anything else → `queue = "{<expr>}"`, attr `queue_dynamic = true`, confidence 0.5. Cross-file constants stay dynamic here; the linker may later substitute via `IrSymbol.const_value`.
  - `QueueRegistration` attrs: `queue`, `symbol` (module class or enclosing function), `via ∈ register_queue | new_queue | new_worker | flow`, `options_keys` (names only: `defaultJobOptions`, `limiter`, ...), `default_job_id_set` when `defaultJobOptions.jobId` exists.
  - `QueueConsumer` (class) attrs: `symbol`, `queue`, `concurrency?`, `style ∈ worker_host | legacy_bull | worker_instance`, `process_symbol?`. `QueueJobHandler` (method) attrs: `symbol`, `queue`, `job` (name or `*` for the default `@Process()`), `via ∈ process_decorator | switch_case | worker_host_process`, `event?`. For `process(job)` in `WorkerHost` subclasses, each `case 'x':` of a `switch (job.name)` yields a handler fact `job = 'x'` with `symbol` = the `process` method and confidence 0.8; no switch → one fact `job = '*'`.
  - `QueueProducer` attrs: `symbol` (enclosing method/function), `queue`, `job` (first arg literal or `{expr}`), `method ∈ add | add_bulk | upsert_scheduler | flow_add`, `job_id_set` (opts has `jobId` or `deduplication`), `job_id_expr_kind`, `delay_set`, `repeat_set`, `receiver_declared_type`, `confidence`.
    - Receiver qualification: declared type head `Queue`/`FlowProducer`, or ThisField typed `Queue` whose property carries `di_token_kind = bullmq_queue` (queue name from NEST-003), or the variable assigned from `new Queue('x')`. `this.queue.add` where the property's `@InjectQueue('mail')` name is known → 0.9; type-only `Queue` without a known name → queue unresolved, 0.5.
    - `addBulk([...])`: one producer fact per array element with a literal `name` (cap 50), else a single fact with `job = '{bulk}'`.
  - Linker (CG-006) creates `queue:{name}` nodes, refines classes to `QueueConsumer` and methods to `JobHandler`, adds `PRODUCES_JOB`/`CONSUMES_JOB`, and collects `attrs.jobs`; this task owns only facts plus the mapper test.
- **Data model changes:** None.
- **API/protocol changes:** `queue_producer`, `queue_consumer`, `queue_registration` rows in `docs/graph-schema/framework-facts.md` (additive attrs `job_id_set`, `style`, `via`).
- **Concurrency semantics:** Per-file pure.
- **Failure behavior:** Unresolvable queue → dynamic fact with confidence 0.5, never dropped silently (reviewers still see a producer exists). Malformed decorators → diagnostics.
- **Idempotency considerations:** Deterministic source order; bulk elements in array order.
- **Security considerations:** Job payloads and Redis connection options are never stored (only option key names). Queue names are identifiers, not secrets.
- **Observability additions:** `framework_facts_emitted_total{adapter="bullmq",kind}`, `bullmq_dynamic_queue_total`.
- **Tests required** (`tests/bullmq.rs`):
  - `processor_string_and_object_forms`, `worker_host_process_switch_cases_to_handlers`, `worker_host_without_switch_star_handler`, `legacy_process_decorator_named_and_default`, `on_worker_event_not_a_job_handler`, `register_queue_single_and_array_and_async`, `new_queue_and_new_worker`, `producer_add_via_inject_queue`, `producer_add_bulk_elements`, `job_id_option_flag`, `queue_name_from_same_file_const_and_enum`, `dynamic_queue_name_low_confidence`, `flow_producer_children_depth_cap`, `non_bullmq_queue_class_ignored`, `unbound_processor_decorator_lower_confidence`, plus codegraph test `producer_and_consumer_share_queue_node` over `nest-api`.
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** `engine/scripts/cargo.sh test -p lang-typescript --test bullmq` passes; in `nest-api` every `queue.add` job name that has a matching `@Process`/switch case is linked producer → queue → handler (asserted in the mapper test), and the planted orphan producer (no consumer) is reported by a graph query.
- **Definition of done:** Tests green; golden snapshots updated; contract rows documented; global DoD met.

---

---

### NEST-007 — Jest tests and configuration reads

Status: ☐

- **Task ID:** NEST-007
- **Title:** Jest `describe` / `it` / `test` → `TestSuite`/`TestCase`/`TestMock` facts; `ConfigService.get` / `process.env` reads → `ConfigRead` facts (`EnvironmentVariable`, `READS_CONFIG`)
- **Problem:** Reviewers need to know which tests exercise a changed symbol (and whether any do), and which environment variables a change reads. Test bodies are anonymous callbacks, so neither tests nor config reads are symbols; they exist only as facts attributed to the module symbol or enclosing function.
- **Why it exists:** PRD §17 (`TestSuite`, `TestCase`, `EnvironmentVariable`, `Configuration`), §18 (`TESTS`, `READS_CONFIG`), §97 (Jest tests), §99 (test mapping signals: direct imports, method invocation, test naming, mock references, path conventions); target-architecture §3.3 (`test:{file}#{suite path} › {name}`, `env:{NAME}`); CG-006 contract (`test_suite`/`test_case`: `suite_path`, `name`, `range`; `env_read`: `symbol`, `name`).
- **Scope:**
  - Jest structure: `describe`, `describe.each/.skip/.only/.todo`, `xdescribe/fdescribe`, `it/test` and their `.each/.skip/.only/.todo/.concurrent` forms, `xit/fit/xtest`, nesting, test-file detection.
  - Mocks: `jest.mock('spec')`, `jest.spyOn(obj, 'method')`, `jest.fn`, `Test.createTestingModule({providers, imports}).overrideProvider(X).useValue/useClass`, `{ provide: X, useValue: mock }`.
  - Config reads: `ConfigService.get/getOrThrow<T>('KEY', default?)`, `process.env.X`, `process.env['X']`, `const { X, Y } = process.env`, `Reflect`-free; env name vs config key classification.
- **Explicit non-scope:** Coverage data, test-to-symbol edge computation (`TESTS` edges come from call edges in CG-006/IMP-005), snapshot files, Vitest/Mocha dialects (extension via the same table later), reading `.env` or config files, evaluating `registerAs` factories.
- **Files/modules expected to change:** `engine/crates/lang-typescript/src/frameworks/mod.rs` (register `jest`, `config`), `engine/crates/lang-typescript/src/visit/facts.rs` (expose `push_config_read` so SyntaxFact `ConfigRead` includes ConfigService reads, TSA-006), `docs/graph-schema/framework-facts.md`.
- **New files/modules expected:** `engine/crates/lang-typescript/src/frameworks/jest/{mod.rs, suite.rs, mocks.rs, testfile.rs}`, `src/frameworks/config/{mod.rs, config_service.rs, process_env.rs}`, `tests/jest_facts.rs`, `tests/config_reads.rs`, fixtures `nest-api/src/users/{users.service.spec.ts, users.controller.spec.ts}`, `nest-api/test/{app.e2e-spec.ts, each-tables.spec.ts, mocks.spec.ts}`, `nest-api/src/config/{app.config.ts, env.reader.ts}`.
- **Dependencies (task IDs):** NEST-001 (scaffolding), NEST-003 (ConfigService DI field typing), TSA-005 (`in_test_block`, receiver hints), TSA-006 (ConfigRead SyntaxFact).
- **Implementation details:**
  - Test-file detection: path matches `*.spec.{ts,tsx,js}`, `*.test.*`, `*.e2e-spec.*`, `*.e2e.*`, or lives under `__tests__/` or a top-level `test/`/`tests/` directory; recorded on each fact as `is_test_file`. Facts are also emitted for `describe/it` calls in non-test paths (confidence 0.7).
  - `TestSuite` attrs: `suite_path` (List<Str>, outermost first), `name`, `modifier ∈ none | skip | only | todo | each`, `range`, `symbol` = Module symbol (callbacks fold into it per TSA-005), `is_test_file`. `TestCase` attrs identical plus `index` (position among siblings), `concurrent`. Names: string literal; template without substitution; with substitution → `${…}` placeholder, `name_dynamic = true`; `.each` tables → one fact with `name` template and `each = true`. Duplicate sibling names get a deterministic ` #2`, ` #3` suffix (source order) so the `test:{file}#{suite path} › {name}` IDs stay unique.
  - Origin: callee identifiers `describe/it/test/xit/...` with no local shadowing, or `@jest/globals` imports; `jest.*` calls detected by the root identifier `jest` or an import from `@jest/globals`.
  - `TestMock` attrs: `symbol`, `kind ∈ module_mock | spy | fn | testing_module | override_provider | provide_value`, `target` (specifier string for `jest.mock`, `Class.method` text for `spyOn`), `provider` (class name for `{provide: X}` / `overrideProvider(X)`), `value_kind` (`object|ident|call|string`, never the value). Mock references feed the test-mapping signal in PRD §99.
  - `ConfigRead` attrs (adapter `config`): `symbol` (enclosing symbol or Module), `source ∈ process_env | config_service | config_service_namespaced`, `name` (the key), `is_env_name` (matches `^[A-Z][A-Z0-9_]*$`), `has_default`, `or_throw`, `declared_type_arg?`. `process.env.X`, `process.env['X']`, `process.env?.X`, destructuring from `process.env` (one fact per bound key; renames use the original key), and `const env = process.env` followed by `env.X` in the same function scope. `ConfigService` reads require the receiver to be typed `ConfigService` (declared type or ThisField; 0.9) or named `configService|config|cfg` (0.7). Dot-notation keys (`database.host`) are `config_service_namespaced`, not env vars (no `env:` node).
  - Graph handoff (CG-006): `env_read` with `source = process_env` or `is_env_name` becomes `env:{NAME}` plus `READS_CONFIG`; namespaced keys are retained as attrs only until a `Configuration` node task exists.
  - TSA-006 `ConfigRead` SyntaxFacts are extended with ConfigService reads through `push_config_read`, so change classification sees both.
- **Data model changes:** None.
- **API/protocol changes:** `test_suite`, `test_case`, `test_mock`, `env_read` rows in `docs/graph-schema/framework-facts.md` (adds `modifier`, `each`, `is_test_file`, `source`, `has_default`).
- **Concurrency semantics:** Per-file pure.
- **Failure behavior:** Unresolvable names produce dynamic placeholders with confidence 0.5; malformed `describe` (non-function callback) → fact still emitted without children.
- **Idempotency considerations:** Source-order facts; suffix disambiguation is deterministic; the same file yields identical test IDs on re-analysis.
- **Security considerations:** Only variable names are captured. Defaults, `.env` contents, mock return values and secret-looking literals are never stored; any `name` that does not match `^[A-Za-z0-9_.:-]{1,128}$` is dropped (a config key built from user text is not recorded).
- **Observability additions:** `framework_facts_emitted_total{adapter="jest|config",kind}`, `config_reads_total{source}`, `jest_dynamic_names_total`.
- **Tests required** (`tests/jest_facts.rs`, `tests/config_reads.rs`):
  - Jest: `nested_describe_suite_path`, `it_and_test_cases`, `modifiers_skip_only_todo`, `x_and_f_prefixed_aliases`, `each_tables_single_fact`, `template_name_placeholder`, `duplicate_names_suffixed_deterministically`, `jest_globals_import_origin`, `shadowed_describe_ignored`, `non_test_file_lower_confidence`, `jest_mock_module_specifier`, `spy_on_target`, `testing_module_override_provider`, `provide_value_mock_kind_only`, `in_test_block_refs_attributed_to_module`.
  - Config: `process_env_dot_and_bracket`, `process_env_destructuring`, `env_alias_variable`, `config_service_get_with_default`, `get_or_throw_flag`, `config_service_typed_field_receiver`, `config_service_named_receiver_lower_confidence`, `namespaced_key_not_env`, `lowercase_key_not_env_name`, `invalid_key_dropped`, `values_never_stored`, plus codegraph test `env_read_nodes_without_values`.
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** `engine/scripts/cargo.sh test -p lang-typescript --test jest_facts --test config_reads` passes; each `it` in `nest-api` specs yields a unique `test:` ID; each `process.env.X` in `nest-api` yields an `env:X` read with no value.
- **Definition of done:** Tests green; golden snapshots updated; contract rows documented; the full NEST adapter set (NEST-001..007) listed in `docs/languages/typescript.md`; global DoD met.

---

---

### SID-001 — SymbolId canonical form, SymbolKey, parse/format round-trip

Status: ☐

- **Task ID:** SID-001
- **Title:** `SymbolId` canonical form `{lang}:{module_path}#{qualified_name}/{kind}[~{n}]`, `SymbolKey` (blake3, 128-bit hex), parse/format round-trip with escaping (ADR-005)
- **Problem:** The prototype hashed `file_path + qualified_name`, so moving a file changed every ID; line numbers cannot be used. All persistence, diffing, lineage and finding history key off symbol identity, so the string grammar and its hash must be exact, escaped unambiguously and stable forever (a change invalidates every stored graph).
- **Why it exists:** ADR-005 (canonical ID, `SymbolKey = hex(blake3(SymbolId)[..16])`, repository as a namespace column rather than part of the string), target-architecture §3.2, PRD §20. Required by CG-001, GS-002, SID-002..006, INC.
- **Scope:**
  - `SymbolId` construction from parts, `SymbolId::parse` (strict), `Display`/`as_str`, `SymbolIdParts { lang, module_path, qualified_name: Vec<String>, kind, ordinal }`.
  - Segment escaping rules and the module-path derivation (`module_path_for(RepoPath, Language) -> ModulePath`).
  - `identity::symbol_id_of(unit, local_id) -> SymbolId` in `analysis-ir` (assembles parts from `IrSymbol`).
  - Golden test vectors for `SymbolKey`.
- **Explicit non-scope:** Qualified-name construction rules for TypeScript constructs (SID-002), ordinal assignment (SID-003), diffing/matching (SID-004/005). Synthetic node IDs (CG-001). The `SymbolKey::of` hash itself already exists in `review-core` (DOM-001); this task pins and tests it.
- **Files/modules expected to change:** `engine/crates/review-core/src/ids.rs` (add `SymbolId::parse`, remove the "unchecked" escape hatch from public use), `engine/crates/review-core/src/lib.rs`, `engine/crates/analysis-ir/src/lib.rs`.
- **New files/modules expected:** `engine/crates/review-core/src/symbol_id.rs` (grammar, escape, parts), `engine/crates/analysis-ir/src/identity.rs`, `engine/crates/review-core/tests/symbol_id.rs`, `docs/graph-schema/symbol-id.md` (grammar + escaping + examples), `engine/crates/review-core/tests/data/symbol_key_vectors.json`.
- **Dependencies (task IDs):** DOM-001 (`SymbolId`, `SymbolKey`), TSA-001 (`IrSymbol`, `SymbolKind::as_id_str`).
- **Implementation details:**
  - Grammar: `id = lang ":" module_path "#" qn "/" kind [ "~" ordinal ]`; `qn = seg *( "." seg )`; `ordinal = 1*DIGIT` (decimal, ≥ 1, no leading zeros; absence means "no ordinal"). Example `ts:src/auth/auth.service#AuthService.authorize/method`; module symbol `ts:src/auth/auth.service#__module__/module`.
  - `lang` ∈ `[a-z][a-z0-9_]{0,15}`; TypeScript files use `ts`, JavaScript files (including JSX, `.mjs`, `.cjs`) use `js` (TSA-002). Reserved CG-001 prefixes (`repo, dir, file, package, http, queue, db, env, pkg, test`) are rejected as languages.
  - `module_path`: repo-relative, forward slashes, no leading `./` or `/`, no `..`, final extension stripped for `.ts .tsx .mts .cts .js .jsx .mjs .cjs`; `.d.ts` → `.d` (so `foo.ts` and `foo.d.ts` differ); if two files in one language would map to the same module path (`a.ts` vs `a.tsx`), the later one in byte-lexicographic order of the full path keeps its extension (`src/a.tsx`); `module_path_collisions(paths)` returns these for IDX to report. Paths are NFC-normalized; case is preserved.
  - Escaping: within `module_path` and each qualified-name segment, the bytes `%`, `#`, `/`, `~`, `.` (segments only; `.` is a legal separator inside module paths via file names, so escaped only in segments), ASCII control characters and space are percent-encoded as `%XX` uppercase; everything else (including non-ASCII UTF-8) is kept literally. `/` inside the module path separator is the directory separator and is not escaped. Parsing splits on the **first** `#`, the **last** `/` before an optional `~`, and unescapes after splitting, so an escaped `%2F` or `%23` never changes structure.
  - `kind` is exactly a `SymbolKind::as_id_str()` value (pinned list from TSA-001); unknown kinds fail to parse.
  - `SymbolId::parse(&str) -> Result<SymbolId, SymbolIdError>` is strict: it re-formats the parsed parts and requires equality with the input (canonical-form check), rejecting non-canonical escapes (lowercase hex, unnecessary encoding of a safe char). `parse(format(parts)) == parts` for all valid parts (property test).
  - Maximum lengths: module path 1,024 bytes, segment 256 bytes, whole ID 2,048 bytes; over-length parts fail with `SymbolIdError::TooLong` rather than truncating (truncation would create collisions).
  - `SymbolKey::of(&SymbolId)` stays `blake3(id.as_str().as_bytes())[..16]`; vectors `[(id, key_hex)]` are committed and asserted so no future refactor changes keys.
- **Data model changes:** None here; GS-002 stores `symbol_id text` and `symbol_key` per this grammar. The repository is a namespace column, not in the string.
- **API/protocol changes:** `review_core::{SymbolId::parse, SymbolIdParts, SymbolIdError}`, `analysis_ir::identity::symbol_id_of`. JSON representation unchanged (canonical string).
- **Concurrency semantics:** Pure, allocation-only functions; all types `Send + Sync`.
- **Failure behavior:** `parse` returns `Err(SymbolIdError::{Empty, BadLang, BadModulePath, BadSegment, BadKind, BadOrdinal, NonCanonical, TooLong})`; `symbol_id_of` is infallible for validated IR (over-length parts degrade to a deterministic truncated-segment-plus-hash form `seg…~h8` documented as `Lossy` and counted by a metric).
- **Idempotency considerations:** `format` is a pure function of parts; identical sources on different machines/OS yield identical IDs (path separators normalized before formatting; test on backslash input).
- **Security considerations:** IDs may contain attacker-controlled names; escaping guarantees they cannot inject structure (`#`, `/`, `~`) or control characters into logs, SQL keys or URLs. IDs are never used to build file paths.
- **Observability additions:** `symbol_id_lossy_total` counter; no per-ID logging.
- **Tests required** (`tests/symbol_id.rs`, plus unit tests):
  - `example_from_adr_005_formats_exactly`, `module_symbol_id`, `parse_format_roundtrip_property`, `escape_special_chars_roundtrip` (`a.b`, `x#y`, `p/q`, `t~1`, `100%`, space, unicode), `parse_rejects_noncanonical_escape`, `parse_rejects_unknown_kind_and_lang`, `parse_splits_on_first_hash_last_slash`, `ordinal_format_and_leading_zero_rejected`, `reserved_prefix_rejected_as_lang`, `module_path_strips_extensions_and_dts`, `module_path_collision_tsx`, `backslash_paths_normalized`, `nfc_normalization_applied`, `too_long_rejected`, `symbol_key_matches_blake3_prefix_golden_vectors`, `symbol_key_hex_is_32_lowercase`, `symbol_id_of_assembles_from_ir_symbol`.
- **Benchmarks if applicable:** `benches/symbol_key.rs`: `format+parse+key` ≥ 2M ids/s single-thread.
- **Acceptance criteria:** `engine/scripts/cargo.sh test -p review-core --test symbol_id` and `-p analysis-ir identity` pass; `docs/graph-schema/symbol-id.md` examples are executed as doctests; the golden-vector file is unchanged by the PR.
- **Definition of done:** Tests green; grammar document published and linked from ADR-005; CG-001 reserved-prefix test passes; global DoD met.

---

---

### SID-002 — Qualified-name rules for TypeScript

Status: ☐

- **Task ID:** SID-002
- **Title:** Qualified-name rules for TS: nested classes, namespaces, object-literal methods, const arrows, default exports, anonymous functions → enclosing + ordinal, getters/setters, static members
- **Problem:** The qualified name is the heart of identity: two analyzers runs must name the same construct identically, and an edit that does not change what a symbol *is* must not change its name. TypeScript offers many ways to declare callables and containers, several with no name at all.
- **Why it exists:** ADR-005 (`qualified_name` component; line numbers never participate), PRD §20. TSA-003 calls `naming::qualify` while building symbols. SID-003 (ordinals), SID-004/005 (diff, matcher) and every persisted key depend on these rules being frozen and documented.
- **Scope:**
  - The `naming` module in `lang-typescript` with one entry point `qualify(parent: Option<&QualifiedName>, construct: Construct) -> NameDecision`.
  - A rule table (below), applied by TSA-003's visitor for every emitted symbol, recorded in `docs/languages/typescript.md`.
  - Rules for member-name forms (identifier, string-literal key, numeric key, private `#x`, well-known symbols).
- **Explicit non-scope:** Ordinal assignment (SID-003), ID string escaping (SID-001), local variables and block-scoped declarations inside function bodies (not symbols), type-level members of type literals, JS prototype assignment patterns (`Foo.prototype.bar = function`).
- **Files/modules expected to change:** `engine/crates/lang-typescript/src/visit/declarations.rs` (call `naming::qualify`; remove ad-hoc name building), `docs/languages/typescript.md`.
- **New files/modules expected:** `engine/crates/lang-typescript/src/naming.rs`, `tests/naming.rs`, fixtures `fixtures/repositories/ts-basic/src/naming/{nested.ts, namespaces.ts, objects.ts, defaults-anon.ts, defaults-named.ts, defaults-ident.ts, accessors.ts, statics.ts, quoted-keys.ts, symbols.ts, anonymous.ts}`.
- **Dependencies (task IDs):** TSA-003 (symbol table; developed jointly), SID-001 (escaping applied afterwards, names here are raw).
- **Implementation details:**
  - Representation: `QualifiedName = Vec<String>` (raw segments, no escaping). The Module symbol is `["__module__"]`; every other symbol's qualified name is `parent_qn ++ [segment]` where `parent_qn` excludes the Module segment (top-level class `A` → `["A"]`).
  - Rule table:

    | Construct | Segment(s) | Kind | Notes |
    |---|---|---|---|
    | class / interface / enum / type alias / function | declared name | respective | nested classes inside namespaces: `NS.A`; classes in function bodies: not symbols |
    | class member (method, property, accessor, ctor) | member name | method/property/get/set/constructor | `A.m`; constructor is `A.constructor` |
    | static member | same as instance | unchanged | collision with an instance member of the same name and kind resolved by SID-003 (instance gets no ordinal, static `~1`) |
    | getter / setter | property name | `get` / `set` | distinguished by kind, so `A.x/get` and `A.x/set` coexist |
    | enum member | `E.M` | enum_member | |
    | interface member | `I.m` | property/method | |
    | namespace `A.B.C` | three nested symbols `A`, `A.B`, `A.B.C` | namespace | `declare module 'pkg'` → single segment `pkg` (literal content); `declare global` → `global` |
    | const-bound arrow/function expr | declarator name | function | `const handler = () => {}` → `handler/function` |
    | const-bound class expr | declarator name | class | |
    | object-literal const `const o = { a() {}, b: () => {}, c: { d() {} } }` | `o`, `o.a`, `o.b`, `o.c.d` (depth ≤ 2 below `o`) | constant, method/function | `pair` with function value → function; shorthand method → method |
    | default export, named decl | its own name | respective | |
    | default export, anonymous class/function/arrow | `default` | class/function | Module can hold only one default, so no ordinal needed |
    | default export, object literal | `default` (constant), members `default.k` | constant | |
    | default export, identifier (`export default foo`) | no symbol | | `IrExport::DefaultExpr` (TSA-004) |
    | anonymous function under `AnonymousFnPolicy::Emit` | `<anonymous>` under the enclosing symbol's qualified name | function | always ordinal ≥ 1 by SID-003 (`~1`, `~2`, …) |
    | constructor parameter property | param name under the class | property | same qn as a declared field of that name is a conflict → SID-003 |
    | overload signatures | folded into the implementation (no segment) | | TSA-003 |

  - Member name forms: identifier → text; string-literal key `'a-b'` → `a-b`; numeric key `1` → `1`; private `#x` → `#x` (kept so it cannot collide with public `x`); computed `[expr]` → skipped with `ComputedMemberName`, **except** well-known symbols `[Symbol.iterator]` → `@@iterator` (any `Symbol.<ident>`).
  - Names are taken verbatim from source bytes (UTF-8 lossy), NFC-normalized, never trimmed beyond tree-sitter token boundaries; names with `.` in them stay one segment (SID-001 escapes them).
  - Segments never include type parameters, `static`, `async`, accessibility, signatures or decorators (all attributes).
  - Parent for `export default class {}` etc. is the Module; symbols inside the default-exported anonymous class use `default.m`.
  - `qualify` returns `NameDecision::{Named(QualifiedName), Skip(DiagCode), Fold}`, and is pure, so SID-004 can unit-test names without a tree.
- **Data model changes:** None.
- **API/protocol changes:** Rules are a frozen contract (changing one requires an `ANALYZER_VERSION` major bump, which triggers a full rebuild per PRD §24) and are listed in `docs/languages/typescript.md`.
- **Concurrency semantics:** Pure functions.
- **Failure behavior:** Unnameable constructs → `Skip` plus a diagnostic; never an error.
- **Idempotency considerations:** The name depends only on the declaring construct and its ancestors, never on siblings, position or traversal order (property test: shuffling unrelated top-level declarations leaves every other name unchanged).
- **Security considerations:** Names are untrusted text; length cap 256 bytes per segment (longer → skipped with `UnsupportedConstruct`) so IDs stay bounded.
- **Observability additions:** `ir_symbols_skipped_total{reason}`.
- **Tests required** (`tests/naming.rs`):
  - `nested_class_in_namespace`, `namespace_dotted_expands_to_nested`, `declare_module_string_and_global`, `object_literal_methods_depth_two`, `object_literal_depth_three_not_emitted`, `const_arrow_and_function_expression_names`, `default_export_named_class`, `default_export_anonymous_class_function_arrow`, `default_export_object_members`, `default_export_identifier_no_symbol`, `anonymous_emit_policy_names`, `getter_setter_same_name_distinct_kinds`, `static_and_instance_same_name`, `private_hash_member_distinct_from_public`, `quoted_and_numeric_keys`, `well_known_symbol_member`, `computed_member_skipped_with_diagnostic`, `constructor_parameter_property_name`, `overloads_fold_no_segment`, `names_independent_of_sibling_order` (proptest), `overlong_segment_skipped`, `unicode_names_nfc`.
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** `engine/scripts/cargo.sh test -p lang-typescript --test naming` passes; the rule table in `docs/languages/typescript.md` is checked by a test that parses the table and asserts one fixture example per row.
- **Definition of done:** Tests green; rules documented and referenced from ADR-005; golden snapshots updated (TSA-008); global DoD met.

---

---

### SID-003 — Overload and duplicate ordinal determinism

Status: ☐

- **Task ID:** SID-003
- **Title:** Overload/duplicate ordinal assignment: deterministic `~n` suffix for same-(qualified name, kind) collisions within a file
- **Problem:** `(module, qualified name, kind)` is not always unique. Interface declaration merging, duplicate function declarations in conditional blocks, static vs instance members sharing a name, constructor parameter properties colliding with fields, and anonymous functions (`Emit` policy) all produce collisions. The identity must stay unique and must not drift when unrelated code moves.
- **Why it exists:** ADR-005 (`~n` ordinal "used only for overloads or duplicate names in the same scope"), TSA-001 (`IrSymbol.ordinal`, `validate()` requires `(qualified_name, kind, ordinal)` unique), TSA-003 ("ordinals come from SID-003 after the visitor finishes, in a per-file pass"). Wrong ordinals cause spurious add/remove pairs in SID-004.
- **Scope:**
  - A per-file pass `assign_ordinals(&mut [IrSymbol]) -> OrdinalReport` run after TSA-003 and before hashing/IDs.
  - The ordering rule for colliding groups, the "first has none" rule, and the anonymous-function base rule.
  - A `DuplicateSymbol` diagnostic for non-benign collisions.
- **Explicit non-scope:** TS function/method overload *signatures* (folded into one symbol by TSA-003, never ordinals). Ordinals across files (identity includes module path). Matching a shifted ordinal to its predecessor (SID-005).
- **Files/modules expected to change:** `engine/crates/lang-typescript/src/visit/mod.rs` (invoke after the visitor), `engine/crates/analysis-ir/src/validate.rs` (uniqueness check message).
- **New files/modules expected:** `engine/crates/lang-typescript/src/ordinals.rs`, `tests/ordinals.rs`, fixtures `fixtures/repositories/ts-basic/src/ordinals/{merged-interface.ts, duplicate-functions.ts, static-instance.ts, param-property-collision.ts, anonymous-emit.ts, object-duplicate-keys.ts}`.
- **Dependencies (task IDs):** SID-002 (names), TSA-003 (symbols), SID-001 (ordinal grammar `~n`, n ≥ 1).
- **Implementation details:**
  - Group key: `(parent qualified name, qualified name, kind)`. Groups of size 1 get `ordinal = 0` (no suffix).
  - Within a group, members are ordered by the tuple `(is_static, source start byte)` ascending. The first member keeps `ordinal = 0` (no suffix); the next get `1, 2, …`. Rationale: adding a duplicate *after* an existing symbol never changes the existing symbol's identity; instance members keep their plain ID when a static twin appears.
  - Anonymous functions (`AnonymousFnPolicy::Emit` only): the group key is `(enclosing symbol qn, "<anonymous>", function)`; ordinals start at **1** (a bare `<anonymous>` is never emitted) and follow source order within the enclosing symbol.
  - Interface merging (`interface X {}` twice): members of both declarations get qualified names `X.m`; interface symbols themselves collide → ordinals by source order; a member that exists in both declarations gets ordinals through the same rule. Documented limitation: inserting a *new* earlier duplicate shifts later ordinals; SID-005's matcher is responsible for pairing them via `body_hash`.
  - Collisions that are not benign (two same-kind functions with identical names in one scope, duplicate object keys) emit `DiagCode::DuplicateSymbol` (info), but still receive ordinals so identity stays unique.
  - Ordinals are `u16`; > 65,535 collisions in one group → the excess symbols are dropped with an error diagnostic (guards against generated files).
  - The pass is a pure function over the symbol slice: input order is irrelevant (it sorts by source offsets), output is stable across runs, threads and platforms.
- **Data model changes:** None. The ordinal is part of `IrSymbol` and the `SymbolId` (`~n`).
- **API/protocol changes:** `lang_typescript::ordinals::assign_ordinals`; semantics documented in `docs/languages/typescript.md` under "Identity".
- **Concurrency semantics:** Pure per file.
- **Failure behavior:** Overflow → dropped symbols plus an error diagnostic (`DepthLimit`-style, with `Partial` status); otherwise cannot fail.
- **Idempotency considerations:** Running `assign_ordinals` twice yields the same ordinals (test); re-analysis of identical bytes yields identical IDs; formatting changes do not alter source order, so ordinals are unchanged.
- **Security considerations:** The cap bounds memory for adversarial files with millions of duplicate names.
- **Observability additions:** `ir_duplicate_symbols_total{kind}`, `ir_ordinal_overflow_total`.
- **Tests required** (`tests/ordinals.rs`):
  - `unique_symbols_have_no_ordinal`, `duplicate_functions_first_plain_rest_numbered`, `static_and_instance_same_name_instance_plain`, `merged_interfaces_ordinals_by_source_order`, `merged_interface_member_collision`, `param_property_vs_field_collision`, `anonymous_emit_starts_at_one`, `anonymous_ordinals_scoped_per_enclosing_symbol`, `duplicate_object_keys_diagnostic_and_ordinals`, `adding_duplicate_after_keeps_existing_ids`, `unrelated_insertion_does_not_shift_other_groups` (proptest), `reformat_does_not_change_ordinals`, `pass_is_idempotent`, `shuffled_input_same_output`, `validate_accepts_result_unique_identity`, `overflow_guard`.
- **Benchmarks if applicable:** None (linear after sort; asserted on a 10k-symbol synthetic file in a unit test with a time bound).
- **Acceptance criteria:** `engine/scripts/cargo.sh test -p lang-typescript --test ordinals` passes; `validate()` uniqueness check passes for every fixture; IDs for the colliding fixtures are pinned in snapshots.
- **Definition of done:** Tests green; limitation and rule documented; SID-005 test list references the shifted-ordinal case; global DoD met.

---

---

### SID-004 — Per-file symbol diff

Status: ☐

- **Task ID:** SID-004
- **Title:** Per-file symbol diff: `unchanged` / `modified{signature, body, attributes}` / `added` / `removed` keyed by `SymbolId`
- **Problem:** Incremental indexing and PR change mapping need to know, per changed file, exactly which symbols are new, gone, or changed, and in *what way* (signature vs body vs attributes). Comparing by line ranges or whole-file hashes cannot tell a reformat from a behaviour change.
- **Why it exists:** Target-architecture §3.5 step 3 (symbol diff per file: unchanged | modified(signature|body|attrs) | added | removed | renamed(matcher)) and ADR-004; ADR-005 (signature is an attribute: changing parameters is *modified*, not delete+add). Feeds INC (re-link), DIFF/CHG (changed-symbol set), and SID-005 (candidate pool for rename matching).
- **Scope:**
  - `incremental::symbol_diff::diff_units(base: Option<&ParsedUnit>, head: Option<&ParsedUnit>) -> FileSymbolDiff`.
  - `SymbolChange` classification with a `ModifiedFlags` bitset, and a `moved_range` informational flag.
  - Aggregate counters feeding the incremental counters in ADR-004.
- **Explicit non-scope:** Rename/move pairing (SID-005 consumes `added` and `removed` lists produced here), cross-file matching, edge diffing, persistence, fact-level change categories (CHG consumes `SyntaxFact` deltas separately).
- **Files/modules expected to change:** `engine/crates/incremental/src/lib.rs`, `Cargo.toml` (deps: `analysis-ir`, `review-core`).
- **New files/modules expected:** `engine/crates/incremental/src/symbol_diff.rs`, `engine/crates/incremental/src/counters.rs` (shared counter struct stub, completed in INC), `tests/symbol_diff.rs`, fixtures `fixtures/repositories/diff-basic/{base,head}/src/...` (paired files: unchanged, signature-only, body-only, attr-only, combined, added, removed, reordered, reformatted, ordinal-collision).
- **Dependencies (task IDs):** SID-001, SID-003, TSA-007 (hashes), TSA-001. Pure functions; no dependency on storage.
- **Implementation details:**
  - Identity: symbols are keyed by `SymbolId` via `identity::symbol_id_of` (module path is part of the id, so a diff is meaningful for one path; a deleted or created file diffs `Some/None`). A `BTreeMap<SymbolId, &IrSymbol>` per side gives deterministic iteration.
  - Classification for an id present on both sides:
    - `signature_changed = base.signature_hash != head.signature_hash`
    - `body_changed = base.body_hash != head.body_hash`
    - `attributes_changed = base.attr_hash != head.attr_hash` (decorators, export/visibility flags)
    - all false → `Unchanged` (even if line ranges moved, `moved_range = true` for the record)
    - otherwise `Modified(ModifiedFlags)`.
  - Present only in head → `Added`; only in base → `Removed`. For a deleted file every base symbol is `Removed`; for a new file every head symbol is `Added`.
  - Container nuance (from TSA-007): a container's `body_hash` includes placeholders for members, so adding a method yields `Added(method)` plus `Modified{body}` on the class; editing a method body yields `Modified{body}` on the method only.
  - Output: `FileSymbolDiff { path, base_status, head_status, changes: Vec<SymbolChange>, counts: DiffCounts{unchanged, modified_sig, modified_body, modified_attr, added, removed} }`, sorted by `(kind order, SymbolId)`, and `added`/`removed` exposed as slices for SID-005.
  - Parse-quality guard: if either unit is `ParseStatus::Failed`, the diff returns `FileSymbolDiff::unknown(reason)` (the incremental layer then treats every base symbol of the file as affected). If `Partial`, symbols with `has_errors` or `attrs.hash_partial` that differ are reported as `Modified` with `uncertain = true`.
  - The diff never inspects `SyntaxFact`s; it is cheap (O(n log n)) and allocation-light.
- **Data model changes:** None.
- **API/protocol changes:** Public Rust API in `incremental`; `SymbolChange` serialization (serde) used by the INC delta writer and the `review graph diff` CLI.
- **Concurrency semantics:** Pure; callable from rayon workers; inputs are shared references.
- **Failure behavior:** Never errors; degraded inputs produce `unknown` or `uncertain` results with reasons so callers choose conservatively (re-link rather than skip).
- **Idempotency considerations:** Deterministic order; `diff(a, a)` yields all `Unchanged`; `diff(a, b)` and `diff(b, a)` are mirror images (added↔removed, flags symmetric) — property tested.
- **Security considerations:** None beyond input bounds already enforced by the analyzer; no source text is read.
- **Observability additions:** Counters aggregated by the caller into `symbols_{added,removed,modified}_total` (ADR-004), plus `symbol_diff_uncertain_total`; a `symbol_diff` debug event per file with counts only.
- **Tests required** (`tests/symbol_diff.rs`):
  - `identical_units_all_unchanged`, `reformat_is_unchanged_with_moved_range`, `comment_only_change_is_unchanged`, `signature_only_change`, `body_only_change`, `attribute_only_change_decorator`, `combined_flags`, `added_symbol`, `removed_symbol`, `new_file_all_added`, `deleted_file_all_removed`, `member_added_marks_class_body_modified`, `method_edit_does_not_modify_class`, `export_flag_change_is_attribute`, `failed_parse_yields_unknown`, `partial_parse_marks_uncertain`, `mirror_property_proptest`, `ordinal_collision_ids_diffed_independently`, `output_order_deterministic`.
- **Benchmarks if applicable:** `benches/symbol_diff.rs`: diff of two 2,000-symbol units with 5% changes; target < 1 ms.
- **Acceptance criteria:** `engine/scripts/cargo.sh test -p incremental --test symbol_diff` passes; for every paired fixture the expected per-symbol classification in `diff-basic/expected.json` matches exactly.
- **Definition of done:** Tests green; classification table documented in `docs/architecture/incremental.md` section "Symbol diff"; counters wired for INC; global DoD met.

---

---

### SID-005 — Rename/move matcher and symbol_lineage records

Status: ☐

- **Task ID:** SID-005
- **Title:** Rename/move matcher in `incremental::matcher` (body_hash → signature+name → token Jaccard ≥ 0.8) producing `symbol_lineage` records
- **Problem:** Identity includes the module path and qualified name, so a rename or move looks like `removed + added`. Without pairing them, finding history is lost, embeddings are re-created, dedup across runs fails and impact analysis reports a "new" symbol with no callers. Over-eager pairing is just as harmful: it would attach the wrong history to an unrelated function.
- **Why it exists:** ADR-005 (matching rules in order, `symbol_lineage(transition, similarity)`, history and embeddings follow lineage), target-architecture §3.2 and §3.5 step 3, risk R2 ("stable identity breaks under real refactors"; mitigation: lineage matcher + dedicated suite SID-006). On the critical path (master plan high-risk node).
- **Scope:**
  - `matcher::match_symbols(removed: &[SymbolRef], added: &[SymbolRef], cfg: &MatcherConfig) -> MatchResult` operating over the **snapshot-wide** removed and added sets produced by SID-004 across all changed files.
  - Rule ladder, ambiguity policy, container-first ordering, deterministic assignment.
  - `LineageRecord` type and `LineageSink` trait; consumers' helper `follow(lineage, key) -> SymbolKey`.
- **Explicit non-scope:** Persisting the table (GS), consuming lineage in dedup (VER/DED) and Qdrant re-keying (SEM), file-level rename detection from git (DIFF; used here only as an optional *hint*), cross-kind matches (a method never matches a function), semantic-similarity (embedding) matching.
- **Files/modules expected to change:** `engine/crates/incremental/src/lib.rs`, `src/symbol_diff.rs` (expose `SymbolRef { id, key, kind, name, module_path, parent_id, signature_hash, body_hash, body_token_count, shingles }`).
- **New files/modules expected:** `engine/crates/incremental/src/matcher/{mod.rs, rules.rs, assign.rs, lineage.rs}`, `tests/matcher.rs`, `benches/matcher.rs`.
- **Dependencies (task IDs):** SID-004, SID-001, TSA-007 (`body_hash`, `signature_hash`, `ShingleSet`, `jaccard`). DOM-001 for `SymbolKey`.
- **Implementation details:**
  - Types: `MatcherConfig { min_tokens_exact: u32 = 6, min_tokens_fuzzy: u32 = 12, jaccard_min: f32 = 0.8, max_candidates: usize = 64 }`; `MatchRule { ExactBody, SignatureAndName, TokenSimilarity }`; `SymbolTransition { Renamed, Moved, RenamedAndMoved }` (derived from comparing name/qualified name and module path); `LineageRecord { from: SymbolKey, to: SymbolKey, from_id: SymbolId, to_id: SymbolId, transition, rule, similarity: f32, ambiguous: bool }`; `MatchResult { matches: Vec<LineageRecord>, unmatched_added: Vec<SymbolKey>, unmatched_removed: Vec<SymbolKey>, ambiguous: Vec<AmbiguityNote> }`.
  - Candidate pairs exist only for the **same `SymbolKind`** and, for members, after their parents are decided. Process kinds in the order Class/Interface/Enum/Namespace → Function/Constant/Variable/TypeAlias → Method/Getter/Setter/Property/Constructor/EnumMember. Once a container pair is matched, member candidates are restricted to children of the matched removed parent and the matched added parent (this stops copy-pasted small methods from cross-matching between unrelated classes). Members of an *unmatched* container are matched globally but only by rule 1 with `body_token_count ≥ min_tokens_exact`.
  - Rule 1 `ExactBody`: identical `body_hash` and `body_token_count ≥ min_tokens_exact`; similarity 1.0. Rule 2 `SignatureAndName`: identical `signature_hash` **and** same simple name, module path differs (a pure move); similarity = max(jaccard(body), 0.8). Rule 3 `TokenSimilarity`: `jaccard(body_shingles) ≥ 0.8` and `body_token_count ≥ min_tokens_fuzzy`; similarity = the Jaccard value.
  - Assignment per rule (rules applied in order, each over what remains): build candidate edges, sort by `(similarity desc, same_parent_match desc, same_name desc, same_dir_distance asc, from_id asc, to_id asc)`, then greedily take the best edge whose endpoints are both free (a stable greedy 1:1 assignment; no Hungarian needed at this scale). Complexity is bounded by indexing candidates through `body_hash` and a shingle-bucket LSH (first 4 min-hashes) so work is O(n log n + candidates), capped by `max_candidates` per symbol.
  - Ambiguity policy: if a symbol has several equally best candidates after all tie-breakers (same similarity, same name, same directory distance) it stays unmatched and is reported in `ambiguous` (a wrong pairing is worse than none); `LineageRecord.ambiguous` is true when the winner beat another candidate by < 0.02 similarity.
  - Transition classification: name or qualified name differs → `Renamed`; only module path differs → `Moved`; both → `RenamedAndMoved`. A DIFF-supplied file-rename hint (`old_path → new_path`) raises the sort priority of candidates in the renamed file but never creates a match by itself.
  - Splits and merges are expressed as independent 1:1 symbol matches (a file split moves each symbol to a different file; no file-level lineage exists). Many-to-one (two removed functions merged into one added) is not matched (documented).
  - `follow(lineage: &LineageIndex, key) -> SymbolKey` walks chained records across snapshots with a cycle/length guard (≤ 32 hops).
- **Data model changes:** Produces rows for `symbol_lineage(repository_id, from_snapshot_id, to_snapshot_id, from_key, to_key, transition, similarity)` (table owned by GS); extra `rule`/`ambiguous` stored in a `detail jsonb` column added by that migration.
- **API/protocol changes:** `incremental::matcher` public API; transition names `renamed | moved | renamed_moved` are persisted strings (stable).
- **Concurrency semantics:** Pure function over immutable slices; internally single-threaded for determinism (candidate generation may use rayon but results are merged in sorted order).
- **Failure behavior:** Never errors; over-capacity inputs (> 50k removed or added) fall back to rule 1 only and set `MatchResult.degraded = true` plus a counter, so lineage is conservative rather than slow or wrong.
- **Idempotency considerations:** Deterministic for a given input set regardless of input order (sorted before processing); running twice gives identical records; re-running on the same delta never produces different `to_key`s.
- **Security considerations:** Operates on hashes and names only; no source text. Lineage cannot cross repositories (inputs are one repository's snapshot pair); tenant scoping is applied by the persistence layer.
- **Observability additions:** `symbols_renamed_total` (ADR-004 counter), `symbols_moved_total`, `matcher_matches_total{rule}`, `matcher_ambiguous_total`, `matcher_degraded_total`, `matcher_duration_seconds`.
- **Tests required** (`tests/matcher.rs`):
  - `exact_body_rename_method`, `exact_body_move_function_to_other_file`, `signature_and_name_move_with_body_edit`, `token_similarity_rename_with_small_edit`, `below_threshold_not_matched`, `kind_mismatch_never_matches`, `tiny_bodies_not_matched_by_exact_rule`, `container_first_restricts_member_candidates`, `renamed_class_members_follow_via_body_hash`, `ambiguous_identical_candidates_left_unmatched`, `tie_breakers_prefer_same_name_then_directory`, `rule_order_exact_before_fuzzy`, `file_rename_hint_only_prioritizes`, `input_order_independence_proptest`, `greedy_assignment_is_one_to_one`, `transition_classification`, `degraded_mode_rule1_only`, `follow_chain_with_cycle_guard`, `shifted_ordinal_pairing` (SID-003 limitation).
- **Benchmarks if applicable:** `benches/matcher.rs`: 5,000 removed × 5,000 added with 10% true renames; target < 150 ms, and 50k × 50k < 3 s.
- **Acceptance criteria:** `engine/scripts/cargo.sh test -p incremental --test matcher` passes; SID-006 suite passes; on the replayed history of the reference NestJS repository (IDX-006, `#[ignore]`) lineage miss rate on a hand-labelled sample is ≤ 5% and false-match rate ≤ 1%, results written to `target/lineage-quality.json`.
- **Definition of done:** Tests green; rules and thresholds documented in `docs/architecture/incremental.md`; thresholds exposed in `MatcherConfig` and covered by the calibration report; global DoD met.

---

---

### SID-006 — Rename/move test suite on fixtures/repositories/rename-move

Status: ☐

- **Task ID:** SID-006
- **Title:** Rename/move acceptance suite on `fixtures/repositories/rename-move`: rename method, rename class, move file, move class across files, rename + small edit, split file (plus negative cases)
- **Problem:** ADR-005 makes the rename/move tests "required acceptance criteria", and risk R2 names identity under refactoring as a likely failure. Unit tests of the matcher use hand-built symbols; they cannot show that the analyzer → ids → hashes → diff → matcher chain behaves correctly on real source edits.
- **Why it exists:** ADR-005 consequences; master plan risk R2 and the Phase 5 exit gate; the INC oracle (INC-012) and finding-history features rely on lineage correctness. This suite is the regression net for any change to naming (SID-002), ordinals (SID-003), hashing (TSA-007) or matching (SID-005).
- **Scope:**
  - Fixture repository with paired `base/` and `head/` trees per scenario and an `expected.json` per scenario.
  - An end-to-end test harness: analyze both trees with `TypeScriptAnalyzer`, run SID-004 per file, pool removed/added, run SID-005, compare with expectations.
  - Negative and adversarial scenarios (no match is the correct answer).
- **Explicit non-scope:** Git-level rename detection (DIFF), graph/edge effects of lineage (INC), finding-history consumers, performance (SID-005 bench), real-repository replay (IDX-006).
- **Files/modules expected to change:** `engine/crates/incremental/Cargo.toml` (dev-dependency on `lang-typescript`, `serde_json`), `fixtures/build.sh` (include `rename-move`, if fixtures are materialized by script).
- **New files/modules expected:** `fixtures/repositories/rename-move/{README.md, 01-rename-method/, 02-rename-class/, 03-move-file/, 04-move-class-across-files/, 05-rename-plus-small-edit/, 06-split-file/, 07-neg-unrelated-tiny-functions/, 08-neg-ambiguous-copies/, 09-rename-method-with-callers/}/{base/src/..., head/src/..., expected.json}`, `engine/crates/incremental/tests/rename_move.rs`, `tests/support/lineage_harness.rs`.
- **Dependencies (task IDs):** SID-005, SID-004, SID-002, SID-003, TSA-007, TSA-003.
- **Implementation details:**
  - Scenarios (each 20-60 lines of realistic NestJS-style TypeScript so bodies exceed the token minimums):
    - `01-rename-method`: `UsersService.findByEmail` → `UsersService.findOneByEmail`, same body, callers updated. Expect 1 match, `Renamed`, rule `ExactBody`, similarity 1.0; class unchanged; callers `Modified{body}` only.
    - `02-rename-class`: `OrderService` → `OrdersService` (file name unchanged). Expect class `Renamed` (ExactBody on the container hash) and **every member** matched `Renamed` through the parent; no unmatched symbols.
    - `03-move-file`: `src/util/strings.ts` → `src/common/strings.ts`, content identical. Expect every symbol `Moved`, rules `ExactBody` or `SignatureAndName`, `module_path` changes recorded in `from_id`/`to_id`.
    - `04-move-class-across-files`: `PaymentService` moved from `billing.service.ts` into `payments/payment.service.ts` while `billing.service.ts` keeps other classes. Expect `Moved` for the class and members; remaining classes `Unchanged`.
    - `05-rename-plus-small-edit`: method renamed and two lines changed in the body. Expect `Renamed`, rule `TokenSimilarity`, similarity in `[0.8, 1.0)`. A variant `05b` with the body 50% rewritten expects **no** match (added + removed).
    - `06-split-file`: one file with 5 symbols split across 3 files. Expect 5 independent `Moved` matches, 1:1, none ambiguous.
    - `07-neg-unrelated-tiny-functions`: removed `getId() { return this.id }` and added `getName() { return this.name }` in different classes. Expect no match (below token minimum / Jaccard).
    - `08-neg-ambiguous-copies`: three identical 10-token helper functions in three files, one removed and two added. Expect no match and one `ambiguous` note.
    - `09-rename-method-with-callers`: as 01 plus verifying that callers' own IDs and `body_hash` change only where the call text changed (the edit shows up as body modification, not as renames).
  - Harness: `fn run_scenario(dir) -> ScenarioOutcome { diffs, lineage, unmatched }`; assertions compare against `expected.json`: `{ "matches": [{"from": "<SymbolId>", "to": "<SymbolId>", "transition": "...", "rule": "...", "min_similarity": 0.8}], "unmatched_added": [...], "unmatched_removed": [...], "unchanged": [...], "ambiguous": n }`. IDs are written in canonical `SymbolId` form, which also pins SID-001/002 behaviour end to end.
  - The harness also asserts invariants for all scenarios: lineage is one-to-one; every match has the same `SymbolKind`; `from` existed only in base and `to` only in head; results are identical across 3 input orderings and 1 vs 4 threads.
  - Fixture content is synthetic (modelled on a reference NestJS repository, no copied code) and contains no secrets.
- **Data model changes:** None.
- **API/protocol changes:** None. Adds `tests/support/lineage_harness.rs`, reusable by INC-012 and IDX-006.
- **Concurrency semantics:** Tests are independent; scenario directories are read-only.
- **Failure behavior:** A mismatch prints a readable diff (expected vs actual matches, unmatched lists, similarity values) before failing, so threshold regressions are easy to diagnose.
- **Idempotency considerations:** Running the suite repeatedly gives identical outcomes; no temp files besides optional `target/` reports.
- **Security considerations:** None (static synthetic fixtures).
- **Observability additions:** The harness prints a summary table per scenario (rule, similarity, transition); `target/rename-move-report.json` is written for the calibration report used by SID-005's acceptance criteria.
- **Tests required** (`tests/rename_move.rs`): `rename_method_exact_body`, `rename_class_members_follow`, `move_file_all_symbols_moved`, `move_class_across_files`, `rename_plus_small_edit_token_similarity`, `rename_plus_large_edit_is_add_and_remove`, `split_file_one_to_one_moves`, `unrelated_tiny_functions_not_matched`, `ambiguous_copies_left_unmatched`, `callers_of_renamed_method_are_body_modifications`, `lineage_invariants_hold_for_all_scenarios`, `scenario_results_independent_of_input_order_and_threads`, `every_scenario_dir_has_expected_json`.
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** `engine/scripts/cargo.sh test -p incremental --test rename_move` passes with all nine scenarios; changing the Jaccard threshold to 0.5 or 0.95 makes at least one scenario fail (verified once by a documented mutation check in the PR).
- **Definition of done:** Suite green and wired into CI as a required check; README explains how to add a scenario; ADR-005 acceptance criterion marked satisfied; global DoD met.

---
