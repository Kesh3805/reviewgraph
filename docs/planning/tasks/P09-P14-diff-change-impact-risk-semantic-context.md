# Phases 9–14 — Diff engine, change model, impact, risk, semantic, context

**Date:** 2026-10-02 · **Parent:** [MASTER_IMPLEMENTATION_PLAN.md](../MASTER_IMPLEMENTATION_PLAN.md) §9
**Governing docs:** [target-architecture](../../architecture/target-architecture.md) §2, §3.6–§3.9, §7, §8 · ADR-003, ADR-004, ADR-005, ADR-006, ADR-008, ADR-009, ADR-010, ADR-015 · PRD §25–§39, §90–§92, §99, §151.

Status markers: ☐ todo · ◐ in progress · ☑ done (acceptance criteria executed and passed). The global Definition of Done in master plan §10 applies to every task in addition to its own.

---

## 0. Conventions for this file

### 0.1 Crate placement (target-architecture §2, §2.1)

| Concern | Crate / module | May depend on |
|---|---|---|
| git object access (gix) | `engine/crates/repository/src/git/` | review-core, telemetry |
| file diff, hunks, anchors, hunk→symbol, change model, change classes, intent signals | `engine/crates/diff-engine` | repository, analysis-ir, codegraph, review-core |
| impact graph, risk engine, clustering | `engine/crates/impact` (`impact::{graph,risk,cluster}`) | diff-engine, codegraph |
| embedding providers, Qdrant adapter, embedding sync | `engine/crates/semantic` | codegraph, review-core, telemetry |
| candidates, lexical index, ranking, budgets, compression, package hash | `engine/crates/context-engine` | impact, semantic, profile, codegraph |
| anything that calls a model (intent CLASSIFIER tier) or touches PG `stage_outputs` | `engine/crates/pipeline` (composition root) | everything |

Consequences used throughout:
- `diff-engine`, `impact` and `context-engine` have **no model calls**. Optional model refinement (CHG-009) is a trait defined low and implemented in `pipeline`.
- `impact` does not depend on `profile`. Configuration that risk needs (risk paths, generated globs) is passed in as plain data (`RiskPolicy`), mapped from `.review/config.yaml` by `pipeline` (POL-001).
- Every traversal and every selection takes an explicit budget and reports truncation (master plan principle 4).

### 0.2 Upstream task references

Upstream IDs are cited with the capability they provide in parentheses, e.g. `TSA-006 (syntax facts)`. If a sibling task file numbers a capability differently, the named capability governs and the dependency line is corrected when that file lands. Capabilities relied on:

| Capability | Provided by |
|---|---|
| ids, `RepoPath`, `CommitSha`, `SymbolKey`, versions | DOM-001, DOM-003 |
| domain entities `PullRequest`, `ChangedFile`, `ChangedSymbol` shells, `RiskSignal` shell | DOM-004, DOM-005 |
| `CandidateFinding` / evidence types | DOM-006, DOM-007 |
| sqlx migrations (single schema source) | DOM-009 |
| compose services (Qdrant, PG) / fixtures build script | FND-005, FND-006 |
| manifest & migration path detection / test file patterns / rule-doc discovery | INIT-004, INIT-006, INIT-010 |
| generated / vendored detection rules | INIT-008 |
| `LanguageAnalyzer`, `ParsedUnit`, `IrSymbol` ranges, `IrFrameworkFact` | TSA-001 |
| `SyntaxFact` per symbol (calls, conditions, loops, throws/catches, awaits, returns, db writes, transaction wrappers, guard decorators, validation calls) | TSA-006 |
| NestJS routes (NEST-001), guards → `AUTHORIZES` (NEST-002), DI (NEST-003), TypeORM entities/writes/transactions (NEST-004), DTO validation + BullMQ (NEST-005), Jest suites/mocks (NEST-006) | NEST-001..NEST-006 |
| class/interface members | TSA-005 |
| `SymbolId`/`SymbolKey`, `body_hash`, `signature_hash` | SID-001..SID-004 |
| rename/move matcher + `symbol_lineage` | SID-005 |
| node/edge enums, confidence table, linker, synthetic node IDs, `Graph`/`GraphOverlay`, `bounded_bfs` | CG-001..CG-008 (CG-003 confidence, CG-004/005 graph + linker, CG-006 synthetic IDs, CG-007 queries) |
| `GraphStore` load base/head | GS-001, GS-004 |
| incremental head graph + symbol diff + invalidation set | INC-005, INC-007, INC-008, INC-009 |
| `ModelGateway`, tiers, replay adapter | GW-001, GW-003, GW-006 |
| repository profile, conventions, rule docs | PROF-001..PROF-004 |
| precedence resolver (policy > docs > convention > generic) | POL-004 |
| secret pattern set / detector | SEC-003 |
| reviewer model-input schema, reviewer routing | REV-001, REV-002 |
| stage outputs (`stage_outputs`) | PIPE-005 |
| historical signals (post-MVP) | HIST-001..HIST-004 |
| `.review/config.yaml` schema (budgets, risk paths, generated globs) | POL-001 |
| OTel init + metric instruments | OBS-001..OBS-003 |
| per-run budget manager | PIPE-006 |

### 0.3 The golden scenario (used by every phase in this file)

`fixtures/pull-requests/auth-bypass/` encodes PRD §151. It is created by DIFF-007 and extended (expected outputs only) by CHG-008, IMP-008, RISK-004, CTX-010.

```
fixtures/pull-requests/auth-bypass/
  scenario.yaml            # base_ref, head_ref, provider metadata (PR number, base/head SHAs placeholders), description
  base/                    # full base tree (plain files; fixtures/build.sh turns base/ + patch into a 2-commit git repo)
    package.json  tsconfig.json  .review/config.yaml   (risk.paths: "src/auth/**": critical)
    src/auth/auth-provider.interface.ts   interface AuthProvider { authorize(user, resource): Promise<boolean> }
    src/auth/auth.service.ts              @Injectable() class AuthService implements AuthProvider
                                          constructor(private readonly permissionService: PermissionService)
                                          async authorize(user, resource) { return this.permissionService.check(user.id, resource.id); }
    src/auth/permission.service.ts        class PermissionService { check(userId, resourceId) }
    src/admin/admin.service.ts            class AdminService { updateUser(actor, target, dto) { if (!await this.auth.authorize(actor, target)) throw new ForbiddenException(); ... this.userRepo.save(...) } }
    src/users/user.controller.ts          @Controller('users') class UserController { @Put(':id') update(...) → adminService.updateUser }
    src/users/user.entity.ts              @Entity('users') class User
    src/auth/authorize.spec.ts            describe('AuthService') › it('denies without permission') — imports AuthService, mocks PermissionService
    src/reports/report.service.ts         decoy: calls PermissionService.check independently (must NOT appear as impacted by authorize)
    src/util/format.ts                    decoy: unrelated, lexically similar name `authorizeHeader`
  patch.diff               # the §151 change, plus a comment-only edit in src/util/format.ts (low-risk control)
  expected/
    diff.json              # DIFF-007
    change-model.json      # CHG-008
    impact.json            # IMP-008
    clusters.json          # IMP-008
    risk.json              # RISK-004
    context/correctness.json, context/security.json   # CTX-010
```

Expected facts (assertions reused across tasks):
- Changed symbol: `ts:src/auth/auth.service#AuthService.authorize/method`, change kind `modified(body)`.
- Change classes: `call_removed(PermissionService.check)`, `dependency_removed(PermissionService)`, `return_changed`, `condition_changed` (new comparison expression), `authorization_changed`.
- Impact: callers `AdminService.updateUser` (d=1), `UserController.update` (d=2); endpoint `http:PUT /users/:id` (d=3 via `HANDLED_BY`); `IMPLEMENTS → AuthProvider.authorize`; removed callee `PermissionService.check` (base side); test `test:src/auth/authorize.spec.ts#AuthService › denies without permission`.
- Risk: `authorization` signal + path floor `critical` → level `critical`.
- `report.service.ts` and `format.ts#authorizeHeader` never appear in the impact graph; `format.ts` is classified low-risk (comment-only).

---

## Task index

| ID | Title |
|---|---|
| DIFF-001 | Git object access (gix): resolve SHAs, merge base, read blobs at commit, bare mirror support |
| DIFF-002 | Tree diff → ChangedFile {added/modified/deleted/renamed/copied, old_path} with rename threshold |
| DIFF-003 | Hunk computation with imara-diff (old/new ranges), independent of user git config |
| DIFF-004 | Binary/large/generated/vendored file handling |
| DIFF-005 | Provider diff reconciliation + anchorable-line sets per side (for comment placement) |
| DIFF-006 | Hunk→symbol mapping (new side head IR innermost symbol; deletions via base IR) |
| DIFF-007 | Golden diff tests (base+patch → changed files/hunks/symbols) |
| CHG-001 | ChangedSymbol with base/head versions, change kind, ranges |
| CHG-002 | control_flow_changed / condition_changed / loop_changed / return changes |
| CHG-003 | call_added / call_removed / dependency_added / dependency_removed |
| CHG-004 | exception_handling_changed / async_behavior_changed |
| CHG-005 | authorization_changed / validation_removed / database_write_changed / transaction_boundary_changed / api_contract_changed / return_type_changed |
| CHG-006 | ChangedAPI, ChangedDependency (manifest/lockfile diff), ChangedSchema (migrations/entities), ChangedConfiguration, ChangedTest |
| CHG-007 | PullRequestChangeModel assembly (PRD §26) |
| CHG-008 | Change-model golden tests |
| CHG-009 | Intent classification (11 PRD §29 classes; deterministic signals + optional CLASSIFIER tier; never overrides risk) |
| IMP-001 | ImpactGraph model |
| IMP-002 | Callers and callees bounded expansion |
| IMP-003 | Type hierarchy relations |
| IMP-004 | API entrypoint reachability |
| IMP-005 | Test mapping |
| IMP-006 | Config, DB, queue and external API relations |
| IMP-007 | Impact budgets and truncation reporting |
| IMP-008 | Impact golden tests incl. auth-bypass |
| IMP-009 | Change clustering |
| IMP-010 | Cluster risk ranking, budget allocation, unreviewed-region report |
| RISK-001 | RiskSignal model + rule table (18 PRD §37 categories) |
| RISK-002 | Path, config and manifest signals |
| RISK-003 | Framework and graph signals |
| RISK-004 | Risk scoring (0..1, level) |
| RISK-005 | Risk effects |
| RISK-006 | Low-risk change detection with contract-change override |
| SEM-001 | `EmbeddingProvider` trait + `EmbeddingSpace` identity |
| SEM-002 | Providers: openai, voyage, hash |
| SEM-003 | Qdrant REST client via reqwest |
| SEM-004 | Collection bootstrap and versioning |
| SEM-005 | TenantScope-enforced search API |
| SEM-006 | Embedding unit builders |
| SEM-007 | Incremental embedding sync |
| SEM-008 | Integration with INC-008 invalidations |
| SEM-009 | Retrieval quality and latency benchmark |
| CTX-001 | ContextPackage / ContextItem model |
| CTX-002 | Structural candidates from the impact graph |
| CTX-003 | Tests, config, API and rule-doc candidates |
| CTX-004 | Lexical identifier index and candidates |
| CTX-005 | Ranking |
| CTX-006 | Budgeting |
| CTX-007 | Compression |
| CTX-008 | Semantic candidates (fill remaining budget only) |
| CTX-009 | Context cache + deterministic package hash |
| CTX-010 | Context tests (golden packages; budget-never-exceeded property test) |

---

### DIFF-001 — Git object access (gix): resolve SHAs, merge base, read blobs at commit, bare mirror support
Status: ☐

- **Task ID:** DIFF-001
- **Title:** `repository::git::GitRepo` — a config-isolated, read-only gix wrapper for commit resolution, merge base, tree/blob reads and bare mirrors.
- **Problem:** Every downstream stage (diff, incremental, base/head verification) needs file contents and trees at arbitrary commits. Using `git` subprocesses or a checked-out working tree makes results depend on user config, hooks, filters, autocrlf and concurrent checkouts, and is slow for many reads.
- **Why it exists:** Target-architecture §3.6 (diff via gix, "does not shell out, so user git config cannot alter it") and §9 (bare mirror checkouts); PRD §25 inputs (base commit, head commit); MVP exit step 2.
- **Scope:**
  - `GitRepo::open_mirror(path)` / `open(path)` with an isolated gix configuration (no system/global/user config, no includes, no hooks, no filters, no attributes).
  - `resolve_commit`, `commit_exists`, `merge_base`, `tree_of`, `read_blob(commit, path)`, `read_blob_by_oid`, `blob_size`, `list_tree(commit, prefix)`.
  - Thread-local handle management for rayon use.
  - Typed errors incl. shallow/missing objects.
- **Explicit non-scope:** Cloning/fetching/mirror maintenance and credentials (INIT/IDX-004 `repository::checkout`). Tree diffing (INC-001) and rename detection. Blame/log walking beyond merge base.
- **Files/modules expected to change:** `engine/crates/repository/Cargo.toml` (`gix` with `max-performance-safe`, no `blocking-network-client`), `engine/crates/repository/src/lib.rs`.
- **New files/modules expected:** `engine/crates/repository/src/git/{mod.rs, open.rs, resolve.rs, merge_base.rs, blob.rs, error.rs}`, `engine/crates/repository/tests/git_access.rs`.
- **Dependencies:** FND-001, DOM-001 (`CommitSha`, `RepoPath`).
- **Implementation details:**
  ```rust
  pub struct GitRepo { inner: gix::ThreadSafeRepository, limits: ReadLimits }
  pub struct ReadLimits { pub max_blob_bytes: u64 /* 8 MiB */, pub max_tree_entries: u32 /* 200_000 */ }
  impl GitRepo {
      pub fn open(path: &Path, limits: ReadLimits) -> Result<Self, GitError>;       // normal or bare; rejects paths with symlinked .git escaping the root
      pub fn local(&self) -> gix::Repository;                                       // cheap thread-local handle (object cache enabled, 64 MiB)
      pub fn resolve_commit(&self, sha: &CommitSha) -> Result<gix::ObjectId, GitError>;  // only full 40/64-hex ids; refs are resolved by the caller from provider metadata
      pub fn merge_base(&self, a: &CommitSha, b: &CommitSha) -> Result<MergeBase, GitError>;
      pub fn read_blob(&self, commit: &CommitSha, path: &RepoPath) -> Result<Option<Blob>, GitError>;
      pub fn blob_header(&self, oid: &ObjectId) -> Result<ObjectHeader, GitError>;   // size without inflating (loose/packed header)
  }
  pub enum MergeBase { Found { sha: CommitSha, candidates: u8 }, None /* unrelated histories */ }
  ```
  - Isolation: `gix::open::Options::isolated()` plus `config_overrides` for `core.autocrlf=false`, `core.fsmonitor=`, `diff.*` irrelevant; environment (`GIT_DIR`, `GIT_CONFIG_*`, `HOME`) is not consulted (`open::permissions::Environment::none()`); alternates are honoured only inside the mirror root.
  - `merge_base`: gix `merge_bases_many`; multiple bases (criss-cross) → choose deterministically by (committer time desc, oid asc) and report `candidates`; none → `MergeBase::None` and the caller diffs against the base commit directly with a warning.
  - `read_blob` resolves `commit → tree → path` by component walk (O(depth × entry lookups)); returns `None` for missing path, error for non-blob; enforces `max_blob_bytes` before inflating via `blob_header`.
  - Shallow repositories: if a needed parent is absent, `GitError::ShallowBoundary { missing }` so the caller can deepen the mirror; never silently treats the boundary as the root.
  - SHA-1 and SHA-256 object formats both accepted via `CommitSha` shape.
- **Data model changes:** None.
- **API/protocol changes:** None (internal).
- **Concurrency semantics:** `ThreadSafeRepository` shared; each thread calls `local()`; no interior mutation; safe concurrent reads including while the mirror is being fetched (gix re-checks packs on miss; one retry on `ObjectNotFound` after a pack refresh).
- **Failure behavior:** `GitError::{NotARepository, ObjectNotFound(sha), ShallowBoundary, BlobTooLarge{size}, PathNotFound, NotABlob, Corrupt, Io}`. Corrupt/partial mirrors are reported, never repaired here.
- **Idempotency considerations:** Pure reads; deterministic.
- **Security considerations:** Never executes hooks, filters, textconv or external diff drivers; no `.gitattributes`/`.gitmodules` processing; refuses `core.sshCommand`-style config by ignoring config entirely; path inputs are `RepoPath` (no traversal); `safe.directory` ownership checks stay at default (trust reduced) for mirrors owned by the worker user; blob size cap defends memory.
- **Observability additions:** span `git.read_blob` only at debug level (attrs `bytes`); span `git.merge_base` (attrs `candidates`); counter `git_object_reads_total{kind=blob|tree|commit}`; histogram `git_read_blob_duration_seconds`.
- **Tests required:** `resolves_full_sha_and_rejects_short_or_ref`, `merge_base_linear_and_branching`, `merge_base_criss_cross_is_deterministic`, `unrelated_histories_returns_none`, `read_blob_at_commit_matches_content`, `missing_path_returns_none`, `oversize_blob_rejected_without_inflating`, `bare_mirror_with_alternates_reads`, `shallow_boundary_reported`, `isolated_from_global_config` (global config with `core.autocrlf=true`, `url.*.insteadOf`, `include.path` must not change results), `concurrent_reads_from_32_threads`, `sha256_repo_supported`.
- **Benchmarks if applicable:** `git/read_blob_1k_small` ≥ 20k blobs/s warm; `git/resolve_commit` < 50 µs.
- **Acceptance criteria:** All tests pass on fixture repositories built by `fixtures/build.sh`; no `std::process::Command` in the module (grep test).
- **Definition of done:** Global DoD; `docs/operations/git-access.md` stub describing isolation guarantees.

---

### DIFF-002 — Tree diff → ChangedFile {added/modified/deleted/renamed/copied, old_path} with rename threshold
Status: ☐

- **Task ID:** DIFF-002
- **Title:** `diff_engine::files::diff_commits` — three-dot (`merge_base..head`) file-level diff producing `ChangedFile`s with correct status, `old_path`, similarity and ordering, on top of INC-001's tree-diff primitive.
- **Problem:** Providers report changed files in their own terms and with caps (GitHub truncates at 3,000 files, omits patches for large files). The reviewer needs one authoritative local model of what changed from the merge base to head, including renames and copies, independent of provider quirks.
- **Why it exists:** Target-architecture §3.6 (three-dot equivalent `merge_base..head`; `ChangedFile` shape); PRD §25; DOM-005 `ChangedFile` invariants.
- **Scope:**
  - `DiffOptions` (rename threshold 50%, rename limit, copy detection opt-in, path filters).
  - Base selection (merge base from DIFF-001, fallback when none).
  - Mapping `RawTreeChange` → `ChangedFile` honouring DOM-005 invariants.
  - `DiffModel` container (files + commits) that DIFF-003..006 extend.
  - Stable sorting, summary stats.
- **Explicit non-scope:** Hunks/line stats (DIFF-003). Binary/large/generated disposition (DIFF-004). Provider reconciliation (DIFF-005). Symbol mapping (DIFF-006).
- **Files/modules expected to change:** `engine/crates/diff-engine/Cargo.toml` (deps `repository`, `review-core`, `analysis-ir`), `engine/crates/diff-engine/src/lib.rs`, `engine/crates/repository/src/git/tree_changes.rs` (copy option wiring).
- **New files/modules expected:** `engine/crates/diff-engine/src/files.rs`, `engine/crates/diff-engine/src/model.rs`, `engine/crates/diff-engine/tests/files.rs`.
- **Dependencies:** DIFF-001, INC-001 (`tree_changes`), DOM-005 (`ChangedFile`, `FileChangeStatus`).
- **Implementation details:**
  ```rust
  pub struct DiffOptions { pub rename_similarity: f32 /* 0.5 */, pub rename_limit: u32 /* 1000 */, pub detect_copies: bool /* false */,
                           pub copy_similarity: f32 /* 0.9 */, pub include_globs: Vec<Glob>, pub exclude_globs: Vec<Glob> }
  pub struct DiffModel { pub base: CommitSha, pub head: CommitSha, pub merge_base: Option<CommitSha>, pub files: Vec<FileDiff>, pub stats: DiffStats }
  pub struct FileDiff { pub file: ChangedFile, pub base_oid: Option<ObjectId>, pub head_oid: Option<ObjectId>, pub similarity: Option<u8>,
                        pub disposition: FileDisposition /* DIFF-004, default Analyze */, pub lines: Option<LineStats> /* DIFF-003 */, pub hunks: Vec<DiffHunk> /* DIFF-003 */ }
  pub fn diff_commits(repo: &GitRepo, base: &CommitSha, head: &CommitSha, opts: &DiffOptions) -> Result<DiffModel, DiffError>;
  ```
  1. `merge_base(base, head)`; `Found` → old side = merge base tree; `None` → old side = `base` tree and `merge_base=None` (warn). The PR "base" provided by the provider may be ahead of the real fork point; always diffing from the merge base reproduces what the provider's three-dot diff shows.
  2. `tree_changes(old_tree, head_tree, opts)` → map: `Added→Added`, `Deleted→Deleted`, `Modified→Modified`, `Renamed→Renamed{old_path}` (similarity ≥ threshold, content equality = 100), `Copied→Copied{old_path}` only with `detect_copies`; copy candidates are restricted to files *modified* in the same diff (git's default `-C`) to bound cost.
  3. `TypeChanged` file↔symlink → `Deleted + Added`; submodule entries are dropped (counted). Mode-only changes (equal oid) do not appear.
  4. A renamed-and-edited file keeps status `Renamed` and later diffs content against `old_path` (DIFF-003).
  5. Filters apply to *new* path (and old path for deletions); filtered files are removed from `files` and counted in `stats.filtered`.
  6. `ChangedFile::new` enforces DOM-005 invariants; violations indicate a bug (`DiffError::Invariant`).
  7. Output sorted by `path`; ties impossible. Complexity O(changed entries) after gix subtree skipping; rename detection O(R×A) bounded by `rename_limit`, exceeding it degrades to add/delete (flag in stats).
  - Provider-supplied status never overrides local results here; discrepancies are reported by DIFF-005.
- **Data model changes:** None (`pull_request_files` rows, if any, are written by the pipeline from `FileDiff`).
- **API/protocol changes:** `DiffModel` JSON Schema in contracts (consumed by CLI `review diff --json` and the API).
- **Concurrency semantics:** Single call is synchronous; callers use `spawn_blocking`. Pure.
- **Failure behavior:** `DiffError::{Git(GitError), Invariant, TooManyFiles{n, cap}}`; `TooManyFiles` (default cap 20,000) returns a truncated model flagged `stats.truncated` rather than failing, because very large PRs still need a partial review (PRD §91).
- **Idempotency considerations:** Deterministic; same SHAs and options → byte-identical JSON.
- **Security considerations:** Paths validated as `RepoPath`; no content read in this task; no git config influence (DIFF-001).
- **Observability additions:** span `diff_analysis` (attrs `base`, `head`, `files`, `renamed`, `copied`, `filtered`, `truncated`); histogram `diff_files_duration_seconds`; counter `diff_rename_limit_hits_total`.
- **Tests required:** `statuses_for_added_modified_deleted`, `rename_at_threshold_50_percent`, `rename_below_threshold_is_add_delete`, `exact_rename_without_edit`, `copy_detection_off_by_default`, `copy_detected_when_enabled_from_modified_source`, `three_dot_uses_merge_base_not_base_tip` (base branch advanced after fork), `no_merge_base_falls_back_with_flag`, `symlink_and_submodule_skipped`, `filters_exclude_vendor_dir`, `changedfile_invariants_hold` (old_path iff renamed/copied), `output_sorted_and_deterministic`, `matches_git_name_status_for_fixture` (compare with `git diff -M50% --name-status` generated at fixture build time).
- **Benchmarks if applicable:** `diff_commits/1_file_in_100k_tree` < 50 ms warm; `diff_commits/rename_1000_candidates` recorded.
- **Acceptance criteria:** Fixture outputs equal git's `--name-status` for all statuses; golden `auth-bypass` lists exactly `src/auth/auth.service.ts` and `src/util/format.ts` as `Modified`.
- **Definition of done:** Global DoD; `DiffModel` documented.

---

### DIFF-003 — Hunk computation with imara-diff (old/new ranges), independent of user git config
Status: ☐

- **Task ID:** DIFF-003
- **Title:** `diff_engine::hunks` — line-level diff of old/new blob content with imara-diff (histogram), producing unified-style `DiffHunk`s, zero-context changed ranges and line statistics.
- **Problem:** Symbol mapping and comment anchoring need exact old/new line ranges. `git diff` output depends on `diff.algorithm`, `core.autocrlf`, `.gitattributes` drivers, color/pager and the git version; providers use their own diff engines. We need a deterministic in-process diff.
- **Why it exists:** Target-architecture §3.6 (gix + imara-diff, "user git config cannot alter it"); PRD §25/§27; DIFF-006 and DIFF-005 consume the ranges; CHG tasks need changed-line sets.
- **Scope:**
  - `compute_hunks(old, new, HunkOptions) -> HunkSet`.
  - Line splitting, EOL/no-newline-at-EOF handling, token interning, histogram algorithm with indentation-heuristic postprocessing.
  - `LineStats`, zero-context `ChangedRanges`.
  - Filling `FileDiff.hunks`/`lines` for `Analyze` files, reading blobs through DIFF-001 (old side from `old_path` for renames/copies).
- **Explicit non-scope:** Deciding which files to diff (DIFF-004). Word/token-level diff. Provider patch parsing (DIFF-005). Semantic ranges (DIFF-006).
- **Files/modules expected to change:** `engine/crates/diff-engine/Cargo.toml` (`imara-diff` 0.1), `engine/crates/diff-engine/src/lib.rs`, `engine/crates/diff-engine/src/files.rs`.
- **New files/modules expected:** `engine/crates/diff-engine/src/hunks.rs`, `engine/crates/diff-engine/src/lines.rs`, `engine/crates/diff-engine/tests/hunks.rs`.
- **Dependencies:** DIFF-002, DIFF-001, DOM-005 (`Hunk`).
- **Implementation details:**
  ```rust
  pub struct HunkOptions { pub context: u32 /* 3 */, pub algorithm: DiffAlgo /* Histogram */, pub ignore_eol: bool /* false */, pub max_lines: u32 /* 200_000 */ }
  pub struct DiffHunk { pub header: Hunk /* old_start, old_lines, new_start, new_lines (unified-header semantics) */, pub lines: Vec<HunkLine> }
  pub struct HunkLine { pub kind: LineKind /* Context|Add|Del */, pub old_no: Option<u32>, pub new_no: Option<u32>, pub span: Range<u32> /* byte range in the side's buffer */, pub no_eol: bool }
  pub struct HunkSet { pub hunks: Vec<DiffHunk>, pub changed_old: Vec<Range<u32>>, pub changed_new: Vec<Range<u32>> /* zero-context, 1-based inclusive-exclusive lines */, pub stats: LineStats }
  pub struct LineStats { pub additions: u32, pub deletions: u32 }
  pub fn compute_hunks(old: &[u8], new: &[u8], o: &HunkOptions) -> Result<HunkSet, HunkError>;
  ```
  1. Split into lines on `\n` only; a trailing `\r` stays part of the line (so CRLF↔LF changes are real changes unless `ignore_eol`); last line without newline sets `no_eol` and compares unequal to the same text with newline (matches git).
  2. `imara_diff::intern::InternedInput::new(old_lines, new_lines)`, `Diff::compute(Algorithm::Histogram, &input)`, then `diff.postprocess_with_heuristic(&input, IndentHeuristic)` for readable hunks. O(N·D) worst case, near-linear for typical source; inputs above `max_lines` return `HunkError::TooLarge` (DIFF-004 handles by marking the file `TooLarge`).
  3. Convert `Diff::hunks()` (before/after token ranges) into `ChangedRanges` first (these zero-context ranges are authoritative for symbol mapping), then expand with `context` lines and merge hunks whose context overlaps (gap ≤ 2·context) into `DiffHunk`s.
  4. Header semantics follow unified diff: for a pure insertion `old_lines=0` and `old_start` is the line *before* the insertion (0 at file start); same for pure deletion on the new side (DOM-005).
  5. Renamed/copied files diff old content (at `old_path`, base side) against new content; `Added` uses an empty old side; `Deleted` an empty new side.
  - Never reads `diff.*`, `core.*`, attributes, textconv, env vars or the process locale; invalid UTF-8 is fine (bytes), only the line terminator matters. Memory O(file size); interned tokens dropped per file.
  - Parallelism: files diffed in parallel on the rayon pool; per-file results are placed back by index (path order).
- **Data model changes:** None.
- **API/protocol changes:** `DiffHunk`/`LineStats` appear in `DiffModel` JSON (hunk line content omitted by default; `include_lines` flag for the CLI).
- **Concurrency semantics:** Pure per file; shared pool; cancellation checked between files.
- **Failure behavior:** `HunkError::{TooLarge, Blob(GitError)}`; a blob error for one file aborts the diff (stale mirror) as `DiffError`; `TooLarge` is a per-file disposition, not an error.
- **Idempotency considerations:** Deterministic: fixed algorithm, no randomness, no env input.
- **Security considerations:** Bytes never logged; line spans reference buffers rather than copying text into spans/metrics.
- **Observability additions:** span `diff.hunks` (attrs `files`, `hunks`, `additions`, `deletions`); histogram `diff_hunks_duration_seconds`; counter `diff_files_too_large_total`.
- **Tests required:** `simple_modification_range`, `pure_insertion_and_deletion_headers`, `context_merging_adjacent_hunks`, `crlf_vs_lf_is_a_change_unless_ignored`, `no_newline_at_eof_detected`, `empty_old_side_for_added_file`, `rename_with_edit_diffed_against_old_path`, `matches_git_diff_histogram_u3_for_fixture` (zero diffs vs `git diff --histogram -U3` captured at fixture build time), `unaffected_by_user_git_config` (`diff.algorithm=patience`, `core.autocrlf=true`, `diff.noprefix` in a temp global config), `too_large_input_rejected`, `golden_authorize_hunk_ranges`, proptest `apply_hunks_reconstructs_new` (applying hunks to old yields new).
- **Benchmarks if applicable:** `hunks/10k_line_file_small_edit` < 5 ms; `hunks/100k_line_file_scattered_edits` recorded.
- **Acceptance criteria:** The reconstruction property passes 1,000 cases; fixture parity with git; config-independence test green.
- **Definition of done:** Global DoD.

---

### DIFF-004 — Binary/large/generated/vendored file handling
Status: ☐

- **Task ID:** DIFF-004
- **Title:** `diff_engine::disposition` — classify every changed file (`Analyze`, `Binary`, `TooLarge`, `Generated`, `Vendored`, `Minified`, `LockfileSummary`) before hunk computation and attach that disposition to `FileDiff`.
- **Problem:** Diffing and analyzing generated bundles, vendored trees, lockfiles and binaries wastes CPU, produces enormous noisy hunks, inflates context and invites false positives. They must be detected cheaply, listed in the change model for completeness, and excluded from symbol analysis and (by default) from comments.
- **Why it exists:** PRD §93 (generated code), §91 (large PRs), §25; master plan invariant "out-of-diff findings go to the summary, never silently dropped"; coverage must be reported, not hidden.
- **Scope:**
  - `classify_file(meta, head_prefix, rules) -> FileDisposition` with deterministic precedence.
  - Rules: binary (NUL-byte probe), size/line caps, generated markers and globs (INIT-008), vendored directory patterns, minified heuristic, lockfile recognition.
  - Applying dispositions in `diff_commits`: skip hunk computation for non-`Analyze` files while still recording `LineStats` when cheap.
  - Coverage report entries for the change model.
- **Explicit non-scope:** Repository-level generated-code *detection rules and config* (INIT-008, reused). Semantic summarisation of lockfiles (CHG-006). Deciding review policy for these files (REV/POL).
- **Files/modules expected to change:** `engine/crates/diff-engine/src/files.rs`, `engine/crates/diff-engine/src/hunks.rs`, `engine/crates/diff-engine/src/lib.rs`.
- **New files/modules expected:** `engine/crates/diff-engine/src/disposition.rs`, `engine/crates/diff-engine/tests/disposition.rs`, `fixtures/pull-requests/diff-edge-cases/` (binary png, 3 MiB json, minified js, `dist/` bundle, `// @generated` file, `package-lock.json`, `vendor/lib.js`).
- **Dependencies:** DIFF-002, DIFF-003, INIT-008 (`GeneratedRules`), DIFF-001 (`blob_header`).
- **Implementation details:**
  ```rust
  pub enum FileDisposition { Analyze, Binary, TooLarge { bytes: u64, lines: Option<u32> }, Generated { reason: GeneratedReason }, Vendored, Minified, LockfileSummary { ecosystem: Ecosystem } }
  pub enum GeneratedReason { PathGlob, HeaderMarker, ConfigDeclared }
  pub struct DispositionRules { pub max_diff_bytes: u64 /* 1 MiB */, pub max_diff_lines: u32 /* 20_000 */, pub vendored_dirs: Vec<Glob>, pub generated: GeneratedRules, pub lockfiles: Vec<Glob> }
  pub fn classify_file(path: &RepoPath, old: Option<&ObjectHeader>, new: Option<&ObjectHeader>, probe: &[u8] /* first 8 KiB of new (or old) */, rules: &DispositionRules) -> FileDisposition;
  ```
  - Precedence (first match wins): `LockfileSummary` > `Vendored` > `Generated` (config glob, then header marker within the first 1 KiB: `@generated`, `DO NOT EDIT`, `Code generated … DO NOT EDIT`, `<auto-generated>`) > `Binary` (NUL in first 8,000 bytes on either side, git's rule; `.png/.jpg/.pdf/.woff2…` hint only) > `TooLarge` (size from `ObjectHeader` without inflating; line cap checked during `compute_hunks`) > `Minified` (avg line length > 500 or any line > 5,000 bytes in the probe) > `Analyze`.
  - Defaults for vendored: `node_modules/**`, `vendor/**`, `third_party/**`, `dist/**`, `build/**`, `.yarn/**`, `**/*.min.js`, `**/__snapshots__/**` is *not* vendored (tests are reviewed). Repository `.review/config.yaml` can add/remove globs (POL-001 supplies `DispositionRules`).
  - Effects: non-`Analyze` files get empty `hunks`, `ChangedFile.binary = true` only for `Binary`; `LineStats` filled from a cheap line count for `Generated/Vendored/Minified/LockfileSummary` when size < 8 MiB, else `None`. Such files never enter `SymbolMap`/`ChangedSymbol`s (DIFF-006, CHG-001) and are reported in `DiffModel.coverage` with their reason so reviewers and the summary can state what was not analyzed.
  - `Generated` files that are *edited by hand*-looking (marker present but PR also touches the generator input) are still `Generated`; the pipeline's risk rules may flag them separately.
  - Complexity O(1) per file plus one 8 KiB read; no full-blob inflate for oversized files.
- **Data model changes:** None.
- **API/protocol changes:** `FileDisposition` added to `FileDiff` and the DiffModel JSON Schema.
- **Concurrency semantics:** Pure per file, parallel with DIFF-003.
- **Failure behavior:** Probe read failure → `Analyze` is *not* assumed; the file becomes `TooLarge{bytes:0}`-style `Unreadable` coverage entry and a warning (stale mirror surfaces as `DiffError` only if the blob is a tree object miss).
- **Idempotency considerations:** Deterministic classification from content prefix, headers and rules.
- **Security considerations:** Bounded reads (8 KiB probe) prevent decompression bombs; binary content is never decoded or logged; generated-marker matching is a literal substring check (no regex backtracking).
- **Observability additions:** counter `diff_files_total{disposition}`; span attribute `dispositions` on `diff_analysis`.
- **Tests required:**
  - `png_is_binary_both_sides`
  - `nul_byte_after_8k_not_binary`
  - `three_mib_json_too_large`
  - `line_cap_enforced_by_hunk_computation`
  - `generated_header_marker_detected`
  - `generated_glob_from_config`
  - `dist_bundle_vendored`
  - `minified_js_detected_by_line_length`
  - `package_lock_is_lockfile_summary`
  - `precedence_lockfile_over_generated`
  - `non_analyze_files_have_no_hunks_but_are_listed`
  - `coverage_report_lists_reasons`
  - `vendored_override_via_config`
- **Benchmarks if applicable:** `disposition/classify_10k_files` < 100 ms.
- **Acceptance criteria:** Edge-case fixture yields the expected disposition per file (golden `diff-edge-cases/expected/dispositions.json`); no hunks computed for skipped files (counting wrapper).
- **Definition of done:** Global DoD.

---

### DIFF-005 — Provider diff reconciliation + anchorable-line sets per side (for comment placement)
Status: ☐

- **Task ID:** DIFF-005
- **Title:** `diff_engine::anchor` — reconcile the locally computed `DiffModel` with the provider's reported changed files/patches and produce, per file and per side, the exact set of lines on which an inline review comment can be anchored.
- **Problem:** Inline comments are accepted only on lines inside the diff *as the provider computed it* (GitHub: RIGHT side added/context lines, LEFT side deleted/context lines within a hunk). Our local diff can differ in hunk boundaries or file list (provider truncation at 3,000 files, omitted patches for large files, different rename detection). Posting on a non-anchorable line fails the whole review or lands on the wrong line.
- **Why it exists:** Target-architecture §5 (`publisher` posts inline + summary), §3.6; PRD §59/§60 (comment requirements); master plan invariant "out-of-diff findings go to the summary, never silently dropped"; legacy new-side line set (audit) replaced by a typed model.
- **Scope:**
  - `ProviderFileDiff` input type and parsing of provider patch text into hunk line sets.
  - `reconcile()` producing `Reconciled { anchors, discrepancies }`.
  - `AnchorMap` / `LineSet` with `contains`, `nearest_within`, side-aware queries.
  - Policy for choosing the authority per file and for non-anchorable files.
- **Explicit non-scope:** Posting comments or provider API calls (GH-*). Finding→line selection (VER anchor stage, PUB). Multi-line comment range validation beyond start/end membership.
- **Files/modules expected to change:** `engine/crates/diff-engine/src/lib.rs`, `engine/crates/diff-engine/src/model.rs`.
- **New files/modules expected:** `engine/crates/diff-engine/src/anchor/{mod.rs, patch.rs, lineset.rs, reconcile.rs}`, `engine/crates/diff-engine/tests/anchor.rs`.
- **Dependencies:** DIFF-002, DIFF-003, DIFF-004, API-006 (normalized `ProviderChangedFile` persisted with the PR).
- **Implementation details:**
  ```rust
  pub struct ProviderFileDiff { pub path: RepoPath, pub old_path: Option<RepoPath>, pub status: FileChangeStatus, pub additions: u32, pub deletions: u32,
                                pub patch: Option<String> /* None when provider omitted it */, pub truncated_list: bool }
  pub struct LineSet(Vec<Range<u32>>);                       // sorted, disjoint, merged; 1-based [start, end)
  pub struct FileAnchors { pub right: LineSet, pub left: LineSet, pub source: AnchorSource /* ProviderPatch|LocalHunks|None */, pub anchorable: bool }
  pub struct AnchorMap { pub files: BTreeMap<RepoPath, FileAnchors> }
  pub enum Discrepancy { ProviderOnly(RepoPath), LocalOnly(RepoPath), StatusMismatch{path, local, provider}, StatsMismatch{path, local: (u32,u32), provider: (u32,u32)}, HunkBoundaryDiffers(RepoPath), PatchMissing(RepoPath), ProviderListTruncated }
  pub fn reconcile(local: &DiffModel, provider: &[ProviderFileDiff]) -> Reconciled;
  impl FileAnchors { pub fn can_anchor(&self, side: DiffSide, line: u32) -> bool; pub fn nearest_within(&self, side: DiffSide, line: u32, max_dist: u32) -> Option<u32>; }
  ```
  - Authority per file: **provider patch if present** (parse `@@ -a,b +c,d @@` headers and body to collect RIGHT lines = context + added, LEFT lines = context + deleted); else **local hunks with 3-line context** only when the file is in the provider list with `patch=None` *and* additions+deletions are small (< 3,000 lines) — flagged `AnchorSource::LocalHunks`, because the provider will still reject lines it did not compute; else `anchorable=false` and findings on this file go to the summary.
  - Discrepancy detection: file sets compared by path (rename-aware via `old_path`); status and `additions/deletions` compared with local `LineStats`; hunk line sets compared and `HunkBoundaryDiffers` raised (informational: provider wins). `ProviderOnly` means our checkout lacks the file (stale mirror → caller refetches); `ProviderListTruncated` means local is authoritative for analysis but not for anchoring beyond provider's listed files.
  - `LineSet` operations are O(log n) (`partition_point`); building is O(lines in hunks).
  - Anchoring rules are side-aware: comments on deleted lines use LEFT numbers (old file); comments on added/context lines use RIGHT (new file). Out-of-diff or non-anchorable lines expose `nearest_within` so the publisher can choose to move a comment to a nearby anchorable line within ≤ 3 lines of the same symbol, otherwise fall back to the summary.
  - Everything is deterministic and keyed by repo-relative path; for renames both `path` (RIGHT) and `old_path` (LEFT) are recorded.
- **Data model changes:** None (`published_findings.anchor_side/line` already DOM-009).
- **API/protocol changes:** `AnchorMap` serialized into `stage_outputs` and consumed by verification stage 2 and the publisher.
- **Concurrency semantics:** Pure, thread-safe.
- **Failure behavior:** Malformed provider patch for a file → that file's provider source is dropped (`PatchMissing`) and falls back per the policy above; never panics on adversarial patch text (bounded parser: ≤ 200k lines, header numbers validated).
- **Idempotency considerations:** Deterministic; discrepancies sorted.
- **Security considerations:** Provider patch text is untrusted data: parsed structurally only, never interpreted as instructions, never logged; sizes bounded.
- **Observability additions:** span `diff.reconcile` (attrs `files_local`, `files_provider`, `discrepancies`, `anchorable_files`); counters `diff_discrepancies_total{kind}`, `diff_unanchorable_files_total`.
- **Tests required:** `provider_patch_defines_right_and_left_sets`, `local_hunks_used_when_patch_omitted_small_file`, `large_file_without_patch_is_unanchorable`, `hunk_boundary_difference_prefers_provider`, `provider_list_truncation_flagged`, `rename_anchors_left_on_old_path`, `deleted_line_anchors_left_only`, `nearest_within_respects_distance_and_side`, `malformed_patch_does_not_panic` (fuzz via proptest), `lineset_merges_adjacent_ranges`, `golden_authorize_anchor_lines` (the changed `return` line is RIGHT-anchorable, the removed original line LEFT-anchorable).
- **Benchmarks if applicable:** `anchor/reconcile_3000_files` < 100 ms.
- **Acceptance criteria:** Reconciliation on fixtures with provider patches captured from a real GitHub response (sanitized) shows zero unexplained discrepancies; anchor queries match the provider's accepted lines.
- **Definition of done:** Global DoD; anchoring rules documented in `docs/reviewers/anchoring.md`.

---

### DIFF-006 — Hunk→symbol mapping (new side head IR innermost symbol; deletions via base IR)
Status: ☐

- **Task ID:** DIFF-006
- **Title:** `diff_engine::symbol_map` — map each changed line range to the innermost enclosing symbol on the head side, and each deleted/old-side range to the innermost symbol of the base side, producing a `SymbolMap`.
- **Problem:** Line-level diffs ("line 81 changed") are not reviewable units (PRD §27). The review must reason about `AuthService.authorize()`, including when a hunk spans several members, touches only a decorator, deletes a whole method, or sits at module level.
- **Why it exists:** Target-architecture §3.6 (innermost symbol intersecting a changed line on the new side; old side for deletions); critical path (`INC-009 → DIFF-006 → CHG-007`); PRD §27.
- **Scope:**
  - Per-file interval index over symbol ranges for head and base.
  - Mapping of zero-context changed ranges (DIFF-003) to `SymbolHit`s.
  - Module-level and class-header ranges, deleted symbols, renames (old path/key), generated/disposition exclusions.
  - Cosmetic hint (hunk touches only comments/whitespace within a symbol).
- **Explicit non-scope:** Determining *how* a symbol changed (hash/fact comparison: INC-003 and CHG-002..005). Cross-file move detection (SID-005/INC-003). Anchor lines (DIFF-005).
- **Files/modules expected to change:** `engine/crates/diff-engine/src/lib.rs`, `engine/crates/diff-engine/src/model.rs`.
- **New files/modules expected:** `engine/crates/diff-engine/src/symbol_map/{mod.rs, intervals.rs, mapping.rs}`, `engine/crates/diff-engine/tests/symbol_map.rs`.
- **Dependencies:** DIFF-003, DIFF-004, INC-009 (`GraphPair`: `nodes_in_file` with ranges on both sides), SID-001.
- **Implementation details:**
  ```rust
  pub struct SymbolHit { pub key: NodeKey, pub symbol_id: SymbolId, pub kind: NodeKind, pub side: DiffSide /* Head|Base */, pub path: RepoPath,
                         pub ranges: Vec<Range<u32>> /* changed line ranges inside the symbol on that side */, pub hunk_ids: Vec<u32>,
                         pub scope: HitScope /* Body|Header|Decorator|ModuleLevel */ }
  pub struct SymbolMap { pub hits: Vec<SymbolHit> /* sorted by (path, side, key) */, pub unmapped: Vec<UnmappedRange> /* non-code files, unparsable files */ }
  pub fn map_hunks(diff: &DiffModel, pair: &GraphPair, cfg: &MapConfig) -> SymbolMap;
  ```
  1. **Intervals.** For a file, take its nodes (`nodes_in_file`) with ranges, excluding the file node; build `Vec<Seg>` by sweeping symbols sorted by `(start, -end)` with a stack: each segment is a maximal run of lines whose innermost symbol is constant (`O(S log S)`), stored as parallel arrays for binary search.
  2. **New side.** For each `changed_new` range `[a,b)` find via `partition_point` the first segment with `end > a` and scan until `start >= b`: every intersecting innermost symbol becomes a hit with the portion of the range it covers (O(log S + k)). Lines inside a class but outside any member (decorators, `implements` clause, property initialisers without symbols) map to the class (`scope=Header|Decorator`); lines in no symbol map to the file's Module symbol (`ModuleLevel`).
  3. **Old side.** `changed_old` ranges map the same way against the **base** nodes of `old_path` (renames) / `path`. A base symbol key that still exists in head merges with the new-side hit (same `key`, `side=Head`, old ranges attached via `ranges_old`); a key absent in head (and not renamed) becomes a `Base`-side hit = deleted symbol. Renamed symbols follow lineage so a pure rename yields one hit on the new key.
  4. **Whole-symbol deletion/addition.** If every line of a symbol's range is in `changed_*`, scope stays `Body` and the hit is flagged `whole_symbol` for CHG-001's Added/Removed classification.
  5. **Cosmetic hint.** `touches_code` is computed by checking whether any changed line in the range lies outside comment ranges recorded by the analyzer (`IrSymbol.attrs["comment_ranges"]`) — authoritative cosmetic classification still uses `body_hash` equality (INC-003); this hint only filters obviously comment-only hunks early.
  6. Files with disposition ≠ `Analyze`, unsupported languages and `Failed` parse units go to `unmapped` with a reason (never silently dropped; surfaces in `coverage`).
  - Complexity: O(Σ_f (S_f log S_f + changed ranges × (log S_f + k))) — only changed files are indexed; independent of repository size.
- **Data model changes:** None.
- **API/protocol changes:** `SymbolMap` in contracts; CLI `review diff --symbols`.
- **Concurrency semantics:** Per-file independent (rayon); the `GraphPair` is read-only shared.
- **Failure behavior:** Missing base/head node data for a changed file (e.g. parse `Failed`) → entries in `unmapped{reason:ParseFailed}`; function is infallible.
- **Idempotency considerations:** Deterministic order and tie-break (innermost = deepest, then earliest start).
- **Security considerations:** Operates on ranges/keys only.
- **Observability additions:** span `symbol_mapping` (attrs `files`, `hits`, `deleted_symbols`, `module_level`, `unmapped`); counter `symbol_map_unmapped_total{reason}`; histogram `symbol_map_duration_seconds`.
- **Tests required:** `edit_inside_method_maps_to_method_not_class`, `edit_spanning_two_methods_yields_two_hits`, `decorator_edit_maps_to_decorated_symbol_header`, `import_line_maps_to_module_level`, `deleted_method_uses_base_ranges`, `rename_maps_to_new_key_with_lineage`, `whole_symbol_added_flagged`, `nested_function_innermost_wins`, `unparsable_file_goes_to_unmapped`, `ranges_beyond_eof_clamped`, `golden_authorize_maps_to_AuthService_authorize` (head hit on `ts:src/auth/auth.service#AuthService.authorize/method`; the `format.ts` comment hunk maps with `touches_code=false`), proptest `every_changed_line_mapped_exactly_once` (union of hit ranges + unmapped equals changed lines).
- **Benchmarks if applicable:** `symbol_map/200_files_1000_hunks` < 100 ms.
- **Acceptance criteria:** Golden scenario matches `expected/diff.json` symbol section; the coverage property holds over 1,000 random cases.
- **Definition of done:** Global DoD.

---

### DIFF-007 — Golden diff tests (base+patch → changed files/hunks/symbols)
Status: ☐

- **Task ID:** DIFF-007
- **Title:** Create the `fixtures/pull-requests/auth-bypass` scenario (PRD §151) and a set of edge-case scenarios, a deterministic fixture builder, and insta golden tests covering files, hunks, dispositions, anchors and symbol mapping.
- **Problem:** Diff, hunk and symbol-mapping logic regress easily and silently. One shared, versioned golden scenario must anchor every later phase (change model, impact, risk, context) so they all agree on the same facts.
- **Why it exists:** Master plan milestone M4 (golden diff → changed symbols for all fixture PRs incl. auth-bypass); target-architecture §2 fixtures layout and §11 testing strategy (golden tests with insta); PRD §141 diff-mapping acceptance.
- **Scope:**
  - `fixtures/pull-requests/auth-bypass/` (`scenario.yaml`, `base/`, `patch.diff`, `expected/diff.json`), exactly as laid out in the phase conventions: `AuthService.authorize` drops `PermissionService.check` and returns `user.role === 'admin'`; reachable via `UserController.update → AdminService.updateUser → AuthService.authorize`; decoys `report.service.ts` and `util/format.ts#authorizeHeader`; comment-only edit in `src/util/format.ts` as the low-risk control; spec `src/auth/authorize.spec.ts`.
  - Edge scenarios under `fixtures/pull-requests/diff-edge-cases/`: rename+edit, pure rename, deletion, new file, binary, large file, CRLF conversion, generated/vendored/lockfile, symbol move between files.
  - `fixtures/build.sh` support: build a 2-commit bare repo from `base/` + `patch.diff` with fixed identities and dates so SHAs are reproducible; also writes `git diff --histogram -U3` and `--name-status -M50%` references for parity tests.
  - Rust golden tests in `diff-engine/tests/golden_*.rs`.
- **Explicit non-scope:** Change-model/impact/risk/context expected files (CHG-008, IMP-008, RISK-004, CTX-010 add their own `expected/*`). Provider live data.
- **Files/modules expected to change:** `fixtures/build.sh`, `engine/crates/diff-engine/Cargo.toml` (dev-deps `insta`, `serde_json`).
- **New files/modules expected:** `fixtures/pull-requests/auth-bypass/{scenario.yaml, patch.diff, base/**, expected/diff.json, expected/git-diff-u3.patch, expected/name-status.txt}`, `fixtures/pull-requests/diff-edge-cases/**`, `engine/crates/diff-engine/tests/{golden_auth_bypass.rs, golden_edge_cases.rs, support/mod.rs}`, `engine/crates/diff-engine/tests/snapshots/`.
- **Dependencies:** DIFF-001..006, FND-001, SID-001 (expected `SymbolId` syntax), INC-009 (head/base graphs for mapping tests; tests may use `FullIndexer` + INC pipeline on the fixture).
- **Implementation details:**
  - `build.sh <scenario-dir> <out-repo>`: `git init --bare`, populate a temp worktree from `base/`, commit with `GIT_AUTHOR_DATE=GIT_COMMITTER_DATE=2026-01-01T00:00:00Z`, `GIT_AUTHOR_NAME=fixture`, `core.autocrlf=false`, apply `patch.diff` with `git apply --whitespace=nowarn`, commit, write both SHAs to `scenario.resolved.json`. The builder is test tooling and may call `git`; the engine under test never does.
  - `support::Scenario::load(name)` builds (cached in `target/fixtures`), opens the repo via `GitRepo`, runs `diff_commits`, hunks, dispositions, indexes base and head with `FullIndexer` (tests only; INC delta path is covered in INC-013/CHG-008), then `map_hunks`.
  - `expected/diff.json` (canonical, sorted keys) contains: files (`path`, `status`, `old_path`, `disposition`, `additions`, `deletions`), hunks (`old_start/old_lines/new_start/new_lines`), anchor sets (RIGHT/LEFT line ranges), and symbol hits (`ts:src/auth/auth.service#AuthService.authorize/method` scope Body; `format.ts` comment hunk with `touches_code=false`). No SHAs in the golden file (placeholders), so it is stable across builder changes.
  - Golden update flow documented: `cargo insta review`; PRs changing a golden require reviewer note in the PR description.
  - Parity tests compare our hunks to the git-generated reference for fixtures where histogram and git agree; where they legitimately differ (documented per case) the expected file records ours.
- **Data model changes:** None.
- **API/protocol changes:** None.
- **Concurrency semantics:** Builder is idempotent and uses a lock file for parallel test processes; tests are read-only on the built repo.
- **Failure behavior:** Missing `git` binary in the build container → test setup fails with an explicit message (container image includes git).
- **Idempotency considerations:** Fixed identities/dates → identical object ids across machines; rebuild is a no-op when the content hash of the scenario dir matches.
- **Security considerations:** Fixtures contain no real secrets or customer identifiers; a CI lint scans fixture trees for secret patterns and forbidden names (SEC-003 detector).
- **Observability additions:** None (tests assert on spans only where DIFF-00x defined them: `assert_span_exists("diff_analysis")` via the `tracing-test` harness).
- **Tests required:**
  - `golden_auth_bypass_diff_json`
  - `auth_bypass_changed_files_are_exactly_two`
  - `auth_bypass_authorize_hunk_old_and_new_ranges`
  - `auth_bypass_format_ts_is_comment_only`
  - `auth_bypass_anchor_sets_match_git_u3`
  - `auth_bypass_symbol_hit_is_authorize_method`
  - `edge_pure_rename_has_no_hunks`
  - `edge_rename_with_edit_old_path_diffed`
  - `edge_deleted_file_symbols_from_base`
  - `edge_binary_and_large_have_dispositions`
  - `edge_crlf_conversion_reports_every_line`
  - `edge_symbol_move_between_files_two_hits`
  - `fixture_build_is_reproducible`
  - `fixture_tree_has_no_forbidden_strings`
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** All goldens committed and reviewed; later tasks can import `Scenario::load("auth-bypass")` from `diff-engine`'s `testkit` feature; reproducible SHAs verified on two machines (CI matrix).
- **Definition of done:** Global DoD; `fixtures/pull-requests/README.md` documents scenario format and update flow.

---

### CHG-001 — ChangedSymbol with base/head versions, change kind, ranges
Status: ☐

- **Task ID:** CHG-001
- **Title:** `diff_engine::change::symbols` — build the `ChangedSymbol` set by joining the hunk→symbol map (where) with the hash-based symbol diff (what), carrying base and head versions, change kind and ranges.
- **Problem:** Downstream consumers (classifiers, impact, context, verification) need, for each changed symbol, its identity, both versions (signature, body range, hashes, IR handle for facts), how it changed, and whether the change is merely cosmetic. The DOM-005 shell has only one side and one range.
- **Why it exists:** PRD §26 (`ChangedSymbol`), §27; target-architecture §3.6; VER base/head comparison and CTX before/after context need both versions; critical path (`DIFF-006 → CHG-007`).
- **Scope:**
  - Extend `review_core::ChangedSymbol` (DOM-005) with `base`, `head`, `cosmetic`, `hunk_ids`, `classes` (filled later), keeping existing fields (`side`, `range`, `change`).
  - `SymbolVersion` type and `UnitSource` abstraction giving access to base/head `ParsedUnit`s.
  - Join algorithm, cosmetic detection, renamed/moved handling, generated/test flags, ordering.
- **Explicit non-scope:** Change classes (CHG-002..005). APIs/deps/schemas/configs/tests aggregates (CHG-006). Model assembly (CHG-007). Impact.
- **Files/modules expected to change:** `engine/crates/review-core/src/change.rs` (add fields; backward-compatible constructors), `engine/crates/diff-engine/src/lib.rs`.
- **New files/modules expected:** `engine/crates/diff-engine/src/change/{mod.rs, symbols.rs, versions.rs}`, `engine/crates/diff-engine/tests/change_symbols.rs`.
- **Dependencies:** DIFF-006, INC-003 (`SymbolChangeSet`), INC-009 (`GraphPair`), SID-005 (`LineageRecord`), TSA-001 (IR types), DOM-005.
- **Implementation details:**
  ```rust
  pub struct SymbolVersion { pub key: SymbolKey, pub id: SymbolId, pub path: RepoPath, pub kind: NodeKind, pub range: SourceRange, pub body_range: Option<SourceRange>,
                             pub signature: Option<String>, pub signature_hash: Hash128, pub body_hash: Hash128, pub attr_hash: Hash128, pub local: LocalId /* index into the unit */ }
  // review_core::ChangedSymbol after this task (existing fields kept):
  pub struct ChangedSymbol { pub symbol_key: SymbolKey, pub symbol_id: SymbolId, pub path: RepoPath, pub side: DiffSide, pub range: SourceRange, pub change: SymbolChange,
                             pub base: Option<SymbolVersion>, pub head: Option<SymbolVersion>, pub cosmetic: bool, pub hunk_ids: Vec<u32>,
                             pub flags: SymbolFlags /* GENERATED|TEST|EXPORTED|PUBLIC_API|ENTRYPOINT */, pub classes: Vec<ChangeClass> /* CHG-002..005 */ }
  pub trait UnitSource: Send + Sync { fn unit(&self, side: GraphSide, path: &RepoPath) -> Option<Arc<ParsedUnit>>; }
  pub fn build_changed_symbols(map: &SymbolMap, sc: &SymbolChangeSet, pair: &GraphPair, units: &dyn UnitSource, cfg: &ChangeCfg) -> Vec<ChangedSymbol>;
  ```
  1. Seed set = keys in `SymbolMap` hits ∪ keys in `SymbolChangeSet.{added, removed, modified, renamed}` (the symbol diff can find changed symbols that no hunk touched, e.g. a renamed type changing a signature elsewhere does not, but a decorator-only change on a parent may). Hits with `touches_code=false` and unchanged hashes → included only as `cosmetic=true` records (so coverage shows comment-only edits as low-risk, PRD §39).
  2. Per key: `base` from the base graph node + base unit; `head` from the head node + head unit. `change`: not in base → `Added`; not in head and no lineage → `Removed`; lineage record → `Renamed{from, similarity}` (if hashes also differ the body/signature flags are carried in `classes` later); else `Modified{signature, body, attrs}` by comparing the three hashes. A symbol with all hashes equal and only range shift is dropped unless a hunk touched its code lines.
  3. `side`/`range`: Head range for everything except `Removed` (Base). Containers (class) are included only when their own header/attrs changed (decorators, heritage, modifiers) — changed members do not mark the class changed (avoids a class-level false inflation); the class is available via `parent` lookups.
  4. Flags from node attrs: generated (file disposition), test (`NodeFlags::TEST`), `PUBLIC_API` (exported/public visibility), `ENTRYPOINT` (has `HANDLED_BY`/queue consumer edge on head).
  5. Output sorted by `(path, range.start, symbol_id)`; truncated at `cfg.max_symbols` (5,000) with an explicit `truncated` flag returned beside the vector.
  - Complexity O(|seed| × (log node lookup)); unit lookups only for seeds' files.
- **Data model changes:** None. JSON shape of `ChangedSymbol` extended (additive).
- **API/protocol changes:** `ChangedSymbol`, `SymbolVersion` JSON Schema regenerated in contracts (additive fields; schema version of the change model bumps in CHG-007).
- **Concurrency semantics:** Pure over immutable inputs; may parallelize per file with rayon (ordered collect).
- **Failure behavior:** Missing unit for a changed file (parse failed) → symbol still emitted from the graph node with `base/head.local` unavailable and `classes` left empty plus a coverage note; function never panics.
- **Idempotency considerations:** Deterministic ordering; same inputs → same output.
- **Security considerations:** No source text carried (signatures ≤ 512 chars, already bounded by IR).
- **Observability additions:** span `change.symbols` (attrs `seeds`, `changed`, `cosmetic`, `added`, `removed`, `renamed`, `modified`); counter `changed_symbols_total{change}`.
- **Tests required:** `body_edit_is_modified_body`, `signature_edit_has_both_versions`, `added_symbol_has_no_base`, `removed_symbol_has_no_head_and_base_side_range`, `rename_carries_lineage_and_similarity`, `comment_only_edit_is_cosmetic`, `class_not_marked_changed_for_member_edit`, `decorator_edit_marks_class_header_changed`, `generated_file_symbols_flagged_or_excluded`, `ordering_is_deterministic`, `truncation_flag_set_at_cap`, `golden_authorize_has_base_and_head_versions` (different `body_hash`, same `signature_hash`).
- **Benchmarks if applicable:** `change_symbols/5000_symbols` < 200 ms.
- **Acceptance criteria:** Golden scenario yields exactly one non-cosmetic changed symbol, `authorize`, `Modified{body}`; format.ts comment edit appears only as cosmetic.
- **Definition of done:** Global DoD; DOM-005 task note updated (shell now populated).

---

### CHG-002 — control_flow_changed / condition_changed / loop_changed / return changes
Status: ☐

- **Task ID:** CHG-002
- **Title:** `diff_engine::change::classify` foundation and structural classes: fact-diff infrastructure plus `ControlFlowChanged`, `ConditionChanged`, `LoopChanged`, `ReturnChanged`.
- **Problem:** The change classes (PRD §28) must come from a deterministic comparison of what the code *does* (syntax facts) on both sides, not from text diffs and not from a model. A shared fact-diff engine avoids five tasks inventing five comparators.
- **Why it exists:** PRD §28 (`control_flow_changed`, `condition_changed`, `loop_changed`; `loop_changed` is added by target-architecture §3.6); PRD §27 example ("behavior branch modified"); golden scenario expects `return_changed` and `condition_changed`.
- **Scope:**
  - `ChangeClass` enum (all 16 values defined here, populated across CHG-002..005), `ClassifiedChange` with evidence.
  - `FactDiff` (multiset + sequence diff over `SyntaxFact.key`), the `Classifier` trait and registry.
  - The four structural classifiers.
- **Explicit non-scope:** Call/dependency classes (CHG-003), exception/async (CHG-004), authorization/validation/db/transaction/API/return-type (CHG-005). Assembling the model (CHG-007).
- **Files/modules expected to change:** `engine/crates/diff-engine/src/lib.rs`, `engine/crates/review-core/src/change.rs` (`ChangeClass` enum with stable snake_case wire strings).
- **New files/modules expected:** `engine/crates/diff-engine/src/change/{classify.rs, fact_diff.rs}`, `engine/crates/diff-engine/src/change/classes/{control_flow.rs, condition.rs, loops.rs, returns.rs}`, `engine/crates/diff-engine/tests/classify_structural.rs`.
- **Dependencies:** CHG-001, TSA-006 (`SyntaxFact`/`FactKind` per symbol), TSA-001.
- **Implementation details:**
  ```rust
  pub enum ChangeClass { ControlFlowChanged, ConditionChanged, LoopChanged, ReturnChanged, ExceptionHandlingChanged, AsyncBehaviorChanged, CallAdded, CallRemoved,
      DependencyAdded, DependencyRemoved, AuthorizationChanged, ValidationRemoved, DatabaseWriteChanged, TransactionBoundaryChanged, ApiContractChanged, ReturnTypeChanged }
  pub struct ClassifiedChange { pub class: ChangeClass, pub evidence: Vec<FactRef>, pub detail: BTreeMap<String, AttrValue>, pub certainty: Certainty /* Exact|Heuristic */ }
  pub struct FactRef { pub side: GraphSide, pub kind: FactKind, pub key: String, pub range: SourceRange }
  pub struct FactDiff { pub added: Vec<FactRef>, pub removed: Vec<FactRef>, pub kept: u32, pub reordered: bool }
  pub trait Classifier: Send + Sync { fn name(&self) -> &'static str; fn classify(&self, cx: &ClassifyCx<'_>, sym: &ChangedSymbol) -> Vec<ClassifiedChange>; }
  pub fn diff_facts(base: &[SyntaxFact], head: &[SyntaxFact], kinds: &[FactKind]) -> FactDiff;
  ```
  - `diff_facts` filters to `kinds`, then (a) multiset diff on `key` via sorted-merge with counts (O(n log n)), and (b) `reordered` via an LCS over interned key sequences using imara-diff (O(ND)). Positions never participate (facts' `key` is position-free by TSA-001 contract), so line shifts produce no change.
  - **ControlFlowChanged:** the ordered sequence of `{Condition, Loop, Throw, TryCatch, Return}` keys differs *structurally* (insertion/removal/reordering of branch points), excluding pure key renames within a Condition. **ConditionChanged:** pairing Condition facts by LCS alignment; an aligned pair with different normalized key, or an unaligned added/removed Condition → one change with `detail{old, new}` (normalized expression hashes, text only when ≤ 120 chars). **LoopChanged:** Loop facts added/removed/kind-changed (`for_of`↔`map`) or loop condition key changed. **ReturnChanged:** the multiset of Return keys differs (expression shape, count, early returns); `detail` records `added`/`removed` counts.
  - Requirement on TSA-006 (verified when implementing, fix TSA-006 if absent): Condition facts include equality/relational comparisons and `&&`/`||` operands wherever they occur (not only inside `if`), so `return user.role === 'admin'` yields a `Condition`; Return facts carry a normalized expression key with identifiers abstracted only for literals.
  - Classifiers run per symbol only when both a base and a head unit exist (Added/Removed symbols are handled by CHG-003/007 rules: an added symbol is not "condition changed").
  - Complexity O(F log F) per changed symbol, F = facts in that symbol; parallel over symbols.
- **Data model changes:** None.
- **API/protocol changes:** `ChangeClass` and `ClassifiedChange` JSON Schemas in contracts; wire strings are exactly the PRD names in snake_case.
- **Concurrency semantics:** Pure; per-symbol parallelism via rayon with ordered collect.
- **Failure behavior:** A symbol with missing facts (`syntax_facts=false` analyzer config, parse `Failed`) yields no classes and a coverage marker `facts_unavailable`; never guesses from text.
- **Idempotency considerations:** Deterministic outputs and ordering by `(class, evidence order)`.
- **Security considerations:** `detail` stores hashes and short normalized fragments only (≤ 120 chars); no full expressions or string literals that could carry secrets (string literal values are replaced by `"<str>"` in keys by TSA-006).
- **Observability additions:** span `change.classify` with attr `classifier`; counter `change_classes_total{class}`.
- **Tests required:** `condition_literal_change_detected`, `new_comparison_expression_is_condition_changed` (golden), `reorder_of_branches_is_control_flow_changed_only`, `line_shift_changes_nothing`, `loop_kind_change_detected`, `loop_removed`, `return_expression_changed` (golden), `early_return_added_is_control_flow_and_return`, `facts_unavailable_yields_no_classes`, `evidence_refs_point_to_correct_sides`, `golden_authorize_has_condition_and_return_changed`.
- **Benchmarks if applicable:** `classify/structural_5000_symbols` < 300 ms.
- **Acceptance criteria:** Golden scenario symbol carries `condition_changed` and `return_changed` with evidence on the head side; unit tests green.
- **Definition of done:** Global DoD; `docs/reviewers/change-classes.md` defines each class and its evidence.

---

### CHG-003 — call_added / call_removed / dependency_added / dependency_removed
Status: ☐

- **Task ID:** CHG-003
- **Title:** Classifiers for added/removed calls (with resolved targets on the correct graph side) and symbol-level dependency additions/removals.
- **Problem:** The most review-relevant semantic facts are "this method no longer calls X" and "this method now depends on Y" (PRD §27). Matching calls by text breaks when receivers change; matching by graph edges alone loses unresolved calls. We need target-aware comparison that degrades gracefully.
- **Why it exists:** PRD §28 (`call_added`, `call_removed`, `dependency_added`, `dependency_removed`); golden scenario (`call_removed(PermissionService.check)`, `dependency_removed(PermissionService)`); impact (IMP-002 `RemovedCallee` uses `call_removed` targets resolved on base).
- **Scope:**
  - Binding `Call`/`New` facts of a symbol to graph targets on base and head (`BoundCall`).
  - Call multiset diff on `(target key | callee text)`.
  - Symbol-level dependency sets (owning classes/modules/packages of bound callees, injected types) and their diff.
  - Exposing `RemovedCall`/`AddedCall` structures for IMP/VER consumers.
- **Explicit non-scope:** Changes inside file-level imports with no use change (reported by CHG-006 for packages). Authorization semantics of a removed call (CHG-005 consumes these results). Transitive dependencies.
- **Files/modules expected to change:** `engine/crates/diff-engine/src/change/classify.rs` (register), `engine/crates/diff-engine/src/lib.rs`.
- **New files/modules expected:** `engine/crates/diff-engine/src/change/classes/{calls.rs, dependencies.rs}`, `engine/crates/diff-engine/src/change/bind.rs`, `engine/crates/diff-engine/tests/classify_calls.rs`.
- **Dependencies:** CHG-002 (framework), CHG-001, INC-009 (`GraphPair`), CG-007, TSA-006.
- **Implementation details:**
  ```rust
  pub struct BoundCall { pub callee_text: String /* normalized e.g. "this.permissionService.check" */, pub target: Option<NodeKey>, pub target_id: Option<String>,
                         pub owner: Option<NodeKey> /* class/module/package of target */, pub count: u16, pub confidence: Confidence, pub range: SourceRange, pub side: GraphSide }
  pub fn bind_calls(unit: &ParsedUnit, local: LocalId, g: &dyn GraphQuery, side: GraphSide) -> Vec<BoundCall>;
  pub struct CallDelta { pub added: Vec<BoundCall>, pub removed: Vec<BoundCall>, pub retargeted: Vec<(BoundCall, BoundCall)> }
  ```
  1. **Binding:** for each `Call|New` fact on the symbol, candidate edges are `out_edges(symbol, {Calls})` whose target's terminal `name` equals the fact's terminal identifier; with several candidates choose the edge whose `location` lies within `fact.range`; unresolved → `target=None` (kept by `callee_text`). Edge occurrence-collapsing (C5) is handled by counting facts, not edges. Cost O(facts × deg(symbol)).
  2. **Call delta:** key = `target` when bound on both sides else `callee_text`. Added/removed come from a multiset diff; the same `callee_text` bound to a *different* target on head (e.g. receiver injected type changed) is `retargeted`, reported as one removed + one added with `detail.retargeted=true`. Calls to the same target merely reordered are ignored.
  3. `CallAdded`/`CallRemoved` carry `detail{target_id?, callee_text, count}`; removed calls are resolved against the **base** graph and added against **head** so that deleted callees still resolve. Golden: removed `this.permissionService.check` → `ts:src/auth/permission.service#PermissionService.check/method`.
  4. **Dependencies:** `deps(side) = { owner of each bound callee } ∪ { types of constructor-injected params used (DiInjection refs) } ∪ { pkg: nodes for external callees }` restricted to owners ≠ the symbol's own class. `DependencyAdded/Removed` = set diff on owner keys; `detail{owner_id, via: call|di|type}`. A dependency is "removed" only if *no* call/type use of that owner remains in the symbol (golden: `PermissionService`), so replacing one method of a service with another is `call_removed + call_added`, not a dependency change.
  5. Class-level DI changes (constructor parameter removed) are classified on the constructor symbol as `DependencyRemoved{via=di}`.
  - Calls inside nested anonymous callbacks belong to the enclosing symbol (TSA-003 policy), so they are included.
- **Data model changes:** None.
- **API/protocol changes:** `BoundCall` (JSON in the change model evidence).
- **Concurrency semantics:** Pure; parallel per symbol.
- **Failure behavior:** Unresolved targets are normal and compared by text; missing graph node for a bound target → treated unresolved; never fails.
- **Idempotency considerations:** Deterministic order by `(target key | callee_text)`.
- **Security considerations:** Callee text normalized and truncated (≤ 120 chars); no argument values stored.
- **Observability additions:** span `change.classify` attr `classifier=calls|dependencies`; counters `change_calls_unresolved_total{side}`, `change_classes_total{class}`.
- **Tests required:**
  - `removed_call_resolves_on_base_graph`
  - `added_call_resolves_on_head_graph`
  - `same_target_reordered_not_changed`
  - `receiver_change_is_retargeted`
  - `duplicate_calls_count_changes_detected`
  - `unresolved_call_compared_by_text`
  - `dependency_removed_only_when_no_use_remains`
  - `PermissionService`
  - `swap_method_same_owner_is_not_dependency_change`
  - `di_param_removed_is_dependency_removed`
  - `external_package_call_is_package_dependency`
  - `anonymous_callback_calls_attributed_to_enclosing_symbol`
- **Benchmarks if applicable:** Covered by CHG-002 `classify/*`; add `bind_calls/10k_symbols` < 300 ms.
- **Acceptance criteria:** Golden scenario yields `call_removed` and `dependency_removed` on `authorize` with correct target ids and no `call_added` (the role comparison adds no call).
- **Definition of done:** Global DoD; semantics added to `docs/reviewers/change-classes.md`.

---

### CHG-004 — exception_handling_changed / async_behavior_changed
Status: ☐

- **Task ID:** CHG-004
- **Title:** Classifiers for error-handling semantics (try/catch/throw/finally) and asynchronous behaviour (await, async modifier, floating promises, concurrency shape).
- **Problem:** Swallowed exceptions, removed `throw`s, dropped `await`s and newly floating promises are classic correctness regressions that a diff hides. They must be detected from syntax facts on both sides, deterministically.
- **Why it exists:** PRD §28 (`exception_handling_changed`, `async_behavior_changed`); correctness reviewer inputs (REV-C); risk categories later (RISK-001).
- **Scope:**
  - `ExceptionHandlingChanged` from `TryCatch`/`Throw` facts and `THROWS`/`CATCHES` edges.
  - `AsyncBehaviorChanged` from `Await` facts, async/generator modifiers and `Promise` return-type wrapping, including floating-promise and parallelization changes.
  - Subtype `detail` vocabularies.
- **Explicit non-scope:** Judging whether the change is a bug (reviewers). Retry/timeout semantics inside libraries. Return-type changes in general (CHG-005).
- **Files/modules expected to change:** `engine/crates/diff-engine/src/change/classify.rs` (register).
- **New files/modules expected:** `engine/crates/diff-engine/src/change/classes/{exceptions.rs, asyncs.rs}`, `engine/crates/diff-engine/tests/classify_exceptions_async.rs`.
- **Dependencies:** CHG-002, CHG-003 (bound calls to identify what is awaited), TSA-006, INC-009.
- **Implementation details:**
  ```rust
  pub enum ExceptionChange { TryAdded, TryRemoved, CatchAdded, CatchRemoved, CatchTypeChanged, CatchBodyEmptied /* swallow */, FinallyAdded, FinallyRemoved,
                             ThrowAdded, ThrowRemoved, ThrowTypeChanged, RethrowRemoved }
  pub enum AsyncChange { AwaitAdded, AwaitRemoved, AsyncAdded, AsyncRemoved, FloatingPromiseIntroduced, FloatingPromiseResolved, ParallelizationChanged, ThenChainChanged }
  ```
  - **Exceptions:** `diff_facts(kinds=[TryCatch, Throw])`. Each TryCatch fact carries `detail{catch_types, has_finally, catch_empty, rethrows}` (TSA-006). Alignment by LCS on TryCatch keys. `CatchBodyEmptied` when `catch_empty` flips to true or the catch body fact set becomes only a log call (configurable `swallow_calls`, default `console.*`, `logger.*`). Throw facts compared by `(thrown class, enclosing condition key)`: removing a guard `throw` is also surfaced to CHG-005 as input for `validation_removed`.
  - **Async:** `Await` facts carry `detail{callee_key}` (what is awaited). `AwaitRemoved` when a call that was awaited on base is not awaited on head while still present (`FloatingPromiseIntroduced` if the callee's return type text contains `Promise` or the target node has the `async` flag in the graph); `AwaitAdded` symmetrical. `AsyncAdded/Removed` from the `ASYNC` modifier. `ParallelizationChanged`: sequential awaits ↔ `Promise.all/allSettled/race` over the same callees (detected by Call facts whose callee text is `Promise.*` containing awaited sub-calls), or `for … await` ↔ `Promise.all(map)`. `ThenChainChanged` for `.then/.catch/.finally` chain shape changes.
  - Output `ClassifiedChange{class, detail{subtype, callee?}, certainty}`: floating-promise and parallelization are `Heuristic` (type information limited without the semantic provider), the rest `Exact`.
  - Class-level: `ExceptionHandlingChanged` also fires when `THROWS`/`CATCHES` *edges* from the symbol differ between base and head graphs (catches facts missed by text), de-duplicated against fact-based changes.
  - Complexity O(F log F) per symbol.
- **Data model changes:** None.
- **API/protocol changes:** Subtype strings are part of the stable `detail.subtype` vocabulary documented in contracts.
- **Concurrency semantics:** Pure, parallel per symbol.
- **Failure behavior:** Missing `detail` keys from an older analyzer version → fall back to key comparison only (`certainty=Heuristic`); no panics.
- **Idempotency considerations:** Deterministic order by `(class, subtype, evidence range)`.
- **Security considerations:** No exception messages or string literals stored.
- **Observability additions:** counters `change_classes_total{class}`, `change_heuristic_total{classifier}`; span attr `classifier=exceptions|async`.
- **Tests required:**
  - `try_catch_added_detected`
  - `catch_emptied_is_swallow`
  - `catch_type_narrowed`
  - `throw_guard_removed`
  - `finally_removed`
  - `await_removed_on_async_callee_is_floating_promise`
  - `await_removed_on_sync_callee_is_not_floating`
  - `async_modifier_removed`
  - `sequential_awaits_to_promise_all`
  - `for_await_to_map_all`
  - `edge_based_throws_diff_deduplicated`
  - `older_analyzer_without_detail_degrades_to_heuristic`
- **Benchmarks if applicable:** Covered by `classify/*` (CHG-002).
- **Acceptance criteria:** Each subtype has a passing positive and negative test; golden `auth-bypass` yields none of these classes for `authorize`.
- **Definition of done:** Global DoD; subtype vocabulary in `docs/reviewers/change-classes.md`.

---

### CHG-005 — authorization_changed / validation_removed / database_write_changed / transaction_boundary_changed / api_contract_changed / return_type_changed
Status: ☐

- **Task ID:** CHG-005
- **Title:** Domain-aware classifiers built on fact diffs, bound calls and framework edges: authorization, validation, database writes, transaction boundaries, API contract and return type.
- **Problem:** These six classes drive the highest-weight risk signals and decide which reviewers run. They require combining syntax facts, graph edges (`AUTHORIZES`, `VALIDATES`, `WRITES_TABLE`, `HANDLED_BY`) and framework facts, with deliberately conservative, explainable rules, because a false "authorization changed" inflates risk and a miss hides the §151 bug.
- **Why it exists:** PRD §28, §151 (authorization behaviour changed); golden scenario expects `authorization_changed`; RISK-001 category inputs; VER stages 5/6 re-evaluate these predicates on base vs head.
- **Scope:**
  - `AuthorizationPolicy` (identifier/callee patterns, guard decorators, edges) and `AuthorizationChanged` rules.
  - `ValidationRemoved` (validation calls, DTO/pipe decorators, guard clauses).
  - `DatabaseWriteChanged` (`DbWriteLike` facts, `WRITES_TABLE` edges, ORM method set).
  - `TransactionBoundaryChanged` (`TransactionWrapper` facts and write-inside-wrapper membership).
  - `ApiContractChanged` (HTTP handlers and exported public API).
  - `ReturnTypeChanged`.
- **Explicit non-scope:** Whether the change is acceptable (reviewers/verification). Aggregated API/schema artifacts (CHG-006). Risk weights (RISK-*).
- **Files/modules expected to change:** `engine/crates/diff-engine/src/change/classify.rs` (register).
- **New files/modules expected:** `engine/crates/diff-engine/src/change/classes/{authorization.rs, validation.rs, db_write.rs, transaction.rs, api_contract.rs, return_type.rs}`, `engine/crates/diff-engine/src/change/policy.rs`, `engine/crates/diff-engine/tests/classify_domain.rs`.
- **Dependencies:** CHG-002, CHG-003, TSA-006, NEST-002 (`AUTHORIZES`), NEST-004 (ORM/transactions), NEST-005 (DTO validation), POL-001 (patterns from `.review/config.yaml`, defaults used until present).
- **Implementation details:**
  ```rust
  pub struct ClassifierPolicy { pub authz_callee_patterns: Vec<Regex> /* ^(check|authorize|can|has(Permission|Role)|isAllowed|assert(Can|Permission)) */,
      pub authz_identifiers: Vec<Regex> /* role|roles|permission|isAdmin|scope|acl|owner */, pub guard_decorators: Vec<String> /* UseGuards, Roles, Public, Permissions */,
      pub validation_callee_patterns: Vec<Regex>, pub validation_decorators: Vec<String> /* IsString, IsEmail, ValidateNested, Min, Max, … */, pub db_write_methods: Vec<String> /* save, insert, update, delete, remove, upsert, softDelete, query */ }
  ```
  - **AuthorizationChanged** (any of; each yields evidence): (a) `GuardDecorator` facts or `AUTHORIZES` in-edges to the symbol's endpoint differ between sides; (b) a removed/added bound call (CHG-003) whose target has `AUTHORIZES`/`VALIDATES`-style attrs or matches `authz_callee_patterns`; (c) a Condition fact added/removed/changed whose expression mentions `authz_identifiers` (so `user.role === 'admin'` qualifies); (d) a Throw of `Forbidden*/Unauthorized*` added/removed. Golden: (b) removed `PermissionService.check` + (c) added `user.role === 'admin'` → one `AuthorizationChanged` with both evidence items and `detail.direction = weakened|strengthened|unknown` (weakened when authorization calls/guards are removed with no equivalent added; heuristic).
  - **ValidationRemoved:** removed validation call/decorator/pipe, or a guard clause (`Condition` immediately followed by `Throw`) removed. Only removals are classed (additions are benign).
  - **DatabaseWriteChanged:** `DbWriteLike` fact multiset diff keyed by `(entity/table target, method)` with targets from `WRITES_TABLE` edges; added, removed or retargeted writes; raw `query(` with write verbs counted.
  - **TransactionBoundaryChanged:** `TransactionWrapper` facts added/removed, or any `DbWriteLike` fact whose `inside_transaction` detail flips; also wrapper moved to enclose a different set of writes.
  - **ApiContractChanged:** (1) for handlers with a `HANDLED_BY` edge on head or base: HTTP method/path change (endpoint node id differs), parameter decorators (`@Body/@Query/@Param/@Headers`) added/removed/retyped, DTO type changed, `@HttpCode`, response/serializer decorators; (2) for exported public symbols (`PUBLIC_API` flag) a `signature_hash` change with parameter list/optionality/type text differences (`detail.scope = http|exported`, with `breaking = true` for removed/required-added parameters).
  - **ReturnTypeChanged:** normalized declared `return_type` text differs (whitespace-insensitive, `Promise<T>` unwrapped to compare `T`, plus `asyncness` flagged separately); absent annotations compare as unknown (no class).
  - Everything is policy-table driven and unit-tested; patterns can be extended per repository but not disabled by a model.
- **Data model changes:** None.
- **API/protocol changes:** `ClassifierPolicy` section in `.review/config.yaml` schema (POL-001) under `classification:`; defaults compiled in.
- **Concurrency semantics:** Pure; parallel per symbol; regexes precompiled once (`OnceLock`).
- **Failure behavior:** Invalid user regex → config error surfaced at load (POL-001), defaults retained; missing graph data → fact-only rules still apply with `certainty=Heuristic`.
- **Idempotency considerations:** Deterministic ordering and evidence.
- **Security considerations:** Regexes are anchored and linear-time (`regex` crate, no backrefs); config-provided patterns length-limited.
- **Observability additions:** counters `change_classes_total{class}`, `change_authorization_direction_total{direction}`; span attr `classifier=authorization|validation|db_write|transaction|api_contract|return_type`.
- **Tests required:** `golden_authorize_is_authorization_changed_weakened`, `guard_decorator_removed_is_authorization_changed`, `role_condition_added_alone_is_unknown_direction`, `validation_pipe_removed`, `guard_clause_removed_is_validation_removed`, `db_save_added_and_removed`, `write_moved_out_of_transaction`, `transaction_wrapper_removed`, `route_path_changed_is_api_contract_changed`, `dto_property_decorator_changed`, `exported_function_required_param_added_is_breaking`, `return_type_promise_unwrap_equal_is_not_changed`, `return_type_changed`, `custom_pattern_from_config_applies`.
- **Benchmarks if applicable:** Covered by `classify/*`.
- **Acceptance criteria:** Golden symbol carries `authorization_changed`; the decoy `report.service.ts` (not changed) yields nothing; each rule has positive/negative tests.
- **Definition of done:** Global DoD; policy defaults and rules documented in `docs/reviewers/change-classes.md`.

---

### CHG-006 — ChangedAPI, ChangedDependency (manifest/lockfile diff), ChangedSchema (migrations/entities), ChangedConfiguration, ChangedTest
Status: ☐

- **Task ID:** CHG-006
- **Title:** The five aggregate change collectors of the `PullRequestChangeModel`: APIs, dependencies, schemas, configuration, tests.
- **Problem:** Some of the highest-risk changes are not inside symbols: a major version bump in `package.json`, a destructive migration, a new environment variable, a skipped test, a removed endpoint. They need first-class, typed, deterministic records.
- **Why it exists:** PRD §26 (`APIs`, `dependencies`, `schemas`, `configs`, `tests`), §37 risk categories (dependency, migration, config); target-architecture §3.6; IMP-007/IMP-008 consume `ChangedTest`/`ChangedAPI`; RISK-003 consumes manifest/migration signals.
- **Scope:**
  - `Collector` trait and five implementations over `DiffModel`, `GraphPair`, `ChangedSymbol`s and blobs.
  - Manifest parsing (npm/pnpm/yarn lockfiles), semver bump classification.
  - SQL/TypeORM migration statement classification and entity column diff.
  - Config file/env-var change detection and test-change summarisation.
- **Explicit non-scope:** Vulnerability lookups (SEC/SCA out of scope). Executing or validating migrations. Language ecosystems beyond npm in MVP (collector trait leaves room). Risk weighting.
- **Files/modules expected to change:** `engine/crates/diff-engine/src/lib.rs`, `engine/crates/diff-engine/Cargo.toml` (`semver`, `serde_json`, `serde_yaml`, `sqlparser`).
- **New files/modules expected:** `engine/crates/diff-engine/src/change/collect/{mod.rs, apis.rs, deps.rs, schemas.rs, configs.rs, tests.rs}`, `engine/crates/diff-engine/src/change/collect/parsers/{package_json.rs, lockfiles.rs, sql.rs}`, `engine/crates/diff-engine/tests/collectors.rs`.
- **Dependencies:** CHG-001, CHG-005, DIFF-004 (`LockfileSummary`), INIT-004 (manifest detection), INIT-006 (migration/test path conventions), NEST-001/004/006 (routes, ORM facts, Jest facts), INC-009.
- **Implementation details:**
  ```rust
  pub struct ChangedAPI { pub endpoint: String /* http:PUT /users/{} */, pub method: HttpMethod, pub path: String, pub change: ApiChange /* Added|Removed|Modified{route,params,dto,status,auth} */, pub handler: Option<SymbolKey>, pub breaking: bool, pub auth_changed: bool }
  pub struct ChangedDependency { pub ecosystem: Ecosystem, pub name: String, pub scope: DepScope /* Prod|Dev|Peer|Optional|Transitive */, pub change: DepChange /* Added{version}|Removed|Bumped{from,to,kind: Patch|Minor|Major|Prerelease|Downgrade}|SourceChanged */, pub manifest: RepoPath, pub lockfile_only: bool }
  pub struct ChangedSchema { pub kind: SchemaKind /* Migration|Entity */, pub path: RepoPath, pub ops: Vec<SchemaOp>, pub destructive: bool, pub table: Option<String> }
  pub enum SchemaOp { CreateTable, DropTable, AddColumn{nullable, default}, DropColumn, AlterColumnType, SetNotNull, AddIndex, DropIndex, AddConstraint, DropConstraint, RenameColumn, RawSql }
  pub struct ChangedConfiguration { pub path: RepoPath, pub kind: ConfigKind /* Env|AppConfig|Ci|Container|Build|ReviewPolicy */, pub keys_added: Vec<String>, pub keys_removed: Vec<String>, pub keys_changed: Vec<String>, pub env_reads_added: Vec<String>, pub env_reads_removed: Vec<String> }
  pub struct ChangedTest { pub path: RepoPath, pub suites_added: u32, pub cases_added: u32, pub cases_removed: u32, pub cases_modified: u32, pub skipped_added: u32 /* skip|only|todo */, pub assertions_delta: i32, pub mocks_changed: bool, pub targets: Vec<SymbolKey> }
  pub trait Collector { type Out; fn collect(&self, cx: &CollectCx<'_>) -> Vec<Self::Out>; }
  ```
  - **APIs:** compare `ApiEndpoint` nodes reachable from changed handler symbols plus endpoint nodes added/removed in the delta (`delta.nodes_added/removed` filtered by kind); `breaking` when removed endpoint, removed/renamed required param, or narrowed response; `auth_changed` mirrors `AUTHORIZES` edge differences.
  - **Dependencies:** read only manifests/lockfiles present in `DiffModel` (`package.json`, `package-lock.json` v2/v3 `packages`, `pnpm-lock.yaml` `importers`, `yarn.lock` entries) from both commits (≤ 20 MiB each); semver compare with `semver` (ranges compare by minimum satisfying version). Lockfile-only transitive changes are summarized (`Transitive`, `lockfile_only=true`) and capped at 200 entries with the rest counted.
  - **Schemas:** migration files (paths from INIT-006) added/changed are parsed: SQL via `sqlparser` (PostgreSQL dialect) statement classification; TypeORM `queryRunner.query(\`…\`)` strings and `createTable/dropColumn/addColumn` calls via Call facts. `destructive` = drop table/column, column type narrowing, `SET NOT NULL` without default, rename. Entities: diff `OrmColumn`/`OrmRelation` framework facts for changed entity classes, base vs head.
  - **Configs:** classify changed non-source files by path (`.env*`, `config/**`, `docker-compose*`, `Dockerfile`, `.github/workflows/**`, `tsconfig*`, `.review/config.yaml`); parse JSON/YAML/dotenv key sets (values never stored; keys only); `env_reads_*` from `READS_CONFIG` edge differences on `env:NAME` nodes between base and head.
  - **Tests:** test files from convention paths; `TestSuite/TestCase` node diff; `skipped_added` from case attrs (`skip|only|todo`); `assertions_delta` from `expect|assert` Call fact counts; `targets` via `TESTS` edges on head.
  - All collectors are bounded (entries per list ≤ 1,000; excess counted in `truncated`), deterministic and independent; failures isolate per collector.
- **Data model changes:** None (persisted inside the change model artifact, CHG-007).
- **API/protocol changes:** Five types JSON Schema in contracts.
- **Concurrency semantics:** Collectors run in parallel; each is pure over immutable inputs.
- **Failure behavior:** Unparsable manifest/migration → record with `ops=[RawSql]`/`parse_error=true` and a coverage note (never fails the model); oversize files skipped with reason.
- **Idempotency considerations:** Sorted by `(path, name)`; deterministic.
- **Security considerations:** Config values (potential secrets) are never read into memory beyond key extraction; dotenv values are discarded by the parser; blob size caps; SQL parsed, never executed.
- **Observability additions:** span `change.collect` (attr `collector`); counters `changed_dependencies_total{kind}`, `changed_schemas_total{destructive}`, `changed_configs_total{kind}`.
- **Tests required:** `npm_major_bump_classified`, `devdependency_added`, `lockfile_only_transitive_summarized_and_capped`, `pnpm_lock_importers_parsed`, `drop_column_is_destructive`, `add_nullable_column_not_destructive`, `typeorm_migration_query_string_parsed`, `entity_column_type_change_detected`, `env_example_key_added_and_env_read_added`, `ci_workflow_change_classified`, `skipped_test_added_flagged`, `removed_assertions_negative_delta`, `endpoint_removed_is_breaking`, `config_values_never_stored`.
- **Benchmarks if applicable:** `collect/lockfile_20mib` < 1.5 s.
- **Acceptance criteria:** Fixture scenarios (dependency-bump, migration-drop-column, endpoint-removed, test-skip) produce expected records in CHG-008 goldens; golden `auth-bypass` yields `tests` entry for `authorize.spec.ts` only if the spec changed (it does not) and no spurious deps/schemas.
- **Definition of done:** Global DoD.

---

### CHG-007 — PullRequestChangeModel assembly (PRD §26)
Status: ☐

- **Task ID:** CHG-007
- **Title:** `build_change_model` — orchestrate diff, symbol mapping, symbol/class classification and aggregate collectors into one immutable, versioned, hashable `PullRequestChangeModel` (alias `ChangeModel`).
- **Problem:** Impact, risk, context, reviewers and verification each need the same view of "what this PR changes". Without a single assembled artifact with a stable hash, every consumer recomputes pieces differently and caches cannot key on it.
- **Why it exists:** PRD §26; target-architecture §3.6 (`PullRequestChangeModel { files, symbols, apis, dependencies, schemas, configs, tests, risk_signals }`); critical path (`CHG-007 → IMP-002 → CTX-006`); `stage_outputs` cache key needs `input_hash`.
- **Scope:**
  - The model struct, `ChangeSummary`, `Coverage`, versioning and canonical hash.
  - The assembly pipeline and ordering, budgets and truncation reporting.
  - A `ChangeModelInputs` bundle and a `build` function usable from CLI (local) and pipeline.
  - The `risk_view()` projection for the risk engine.
- **Explicit non-scope:** Computing risk signals (RISK-001..; the field is populated by `impact::risk`). Intent classification logic (CHG-009 fills `intent`). Persistence (PIPE-005 stores the artifact).
- **Files/modules expected to change:** `engine/crates/diff-engine/src/lib.rs`, `engine/crates/review-core/src/change.rs` (`RiskSignal` shell already exists).
- **New files/modules expected:** `engine/crates/diff-engine/src/change/model.rs`, `engine/crates/diff-engine/src/change/assemble.rs`, `engine/crates/diff-engine/tests/assemble.rs`.
- **Dependencies:** CHG-001..006, DIFF-002..006, INC-009, DOM-005.
- **Implementation details:**
  ```rust
  pub const CHANGE_MODEL_VERSION: u16 = 1;
  pub struct PullRequestChangeModel { pub schema_version: u16, pub base_sha: CommitSha, pub head_sha: CommitSha, pub merge_base_sha: Option<CommitSha>,
      pub files: Vec<ChangedFileDetail>, pub symbols: Vec<ChangedSymbol>, pub apis: Vec<ChangedAPI>, pub dependencies: Vec<ChangedDependency>, pub schemas: Vec<ChangedSchema>,
      pub configs: Vec<ChangedConfiguration>, pub tests: Vec<ChangedTest>, pub risk_signals: Vec<RiskSignal> /* empty at assembly */, pub intent: Option<IntentAssessment> /* CHG-009 */,
      pub summary: ChangeSummary, pub coverage: Coverage, pub input_hash: Hash256 }
  pub type ChangeModel = PullRequestChangeModel;
  pub struct ChangedFileDetail { pub file: ChangedFile, pub disposition: FileDisposition, pub lines: Option<LineStats>, pub language: Option<Language>, pub is_test: bool, pub symbol_count: u32 }
  pub struct ChangeSummary { pub files_changed: u32, pub symbols_changed: u32, pub classes: BTreeMap<ChangeClass, u32>, pub additions: u32, pub deletions: u32, pub languages: Vec<Language> }
  pub struct Coverage { pub unanalyzed_files: Vec<(RepoPath, UnanalyzedReason)>, pub truncated: Vec<Truncated /* symbols|files|collector */>, pub facts_unavailable: Vec<SymbolKey> }
  pub fn build_change_model(inp: ChangeModelInputs<'_>, cfg: &ChangeCfg) -> Result<ChangeModel, ChangeError>;
  impl ChangeModel { pub fn risk_view(&self) -> RiskInput<'_> /* everything except `intent` */ }
  ```
  - Pipeline order (each stage deterministic): (1) `DiffModel` → `files`; (2) `SymbolMap` → `build_changed_symbols` (CHG-001); (3) run all `Classifier`s per symbol in parallel (CHG-002..005) and merge `classes` sorted by `(ChangeClass as u8, evidence order)`; (4) run the five collectors (CHG-006) in parallel; (5) summary and coverage; (6) `input_hash`.
  - `input_hash = blake3(canonical_json(model without input_hash) ‖ CHANGE_MODEL_VERSION ‖ classifier_policy_hash ‖ analyzer_versions ‖ base_snapshot ‖ head_snapshot)`; canonical JSON follows RFC 8785 with floats rendered fixed-precision (no raw floats hashed).
  - Budgets: `max_symbols=5,000`, `max_files=20,000`; overflow keeps highest-priority entries (non-cosmetic, tests/auth paths first, then path order) and records `Coverage.truncated` — never silently drops (PRD §91).
  - Generated/vendored/binary/minified files are listed in `files` and `coverage.unanalyzed_files` with reasons but contribute no symbols or classes.
  - `risk_view()` returns a projection type that physically lacks the `intent` field, so the risk engine cannot read intent (CHG-009 invariant).
  - Complexity: O(changed files + changed symbols × facts); independent of repository size.
- **Data model changes:** None; persisted by the pipeline as a `stage_outputs` JSON artifact keyed by `input_hash`.
- **API/protocol changes:** `PullRequestChangeModel` JSON Schema published in contracts; review-engine `GET /internal/review-runs/{id}/change-model`; CLI `review diff --json`.
- **Concurrency semantics:** Parallel classification/collectors on rayon, deterministic merge; immutable result, `Send + Sync`.
- **Failure behavior:** Collector or classifier failures are isolated: the affected section is empty and listed in `Coverage.truncated{reason: collector_failed}`; only input-contract violations (missing DiffModel, graph pair for the wrong commits) return `ChangeError`.
- **Idempotency considerations:** Same inputs and versions → byte-identical model and `input_hash` (asserted).
- **Security considerations:** No source text beyond bounded fragments in `detail`; untrusted PR title/body are not part of this model (CHG-009 handles them as signals only).
- **Observability additions:** spans `diff_analysis` (parent) → `change.symbols`, `change.classify`, `change.collect`, `change.assemble`; histogram `change_model_build_duration_seconds`; counters `change_model_truncated_total{kind}`; attrs `files`, `symbols`, `classes`.
- **Tests required:** `assembly_is_deterministic_and_hash_stable`, `hash_changes_when_policy_or_version_changes`, `generated_files_listed_but_no_symbols`, `truncation_recorded_not_silent`, `collector_failure_isolated`, `risk_view_excludes_intent` (compile-fail test via `trybuild`), `summary_counts_match_symbols`, `golden_auth_bypass_model_contains_expected_classes` (see CHG-008), `local_cli_assembly_without_graph_store_uses_units_only`.
- **Benchmarks if applicable:** `change_model/100_files_500_symbols` < 300 ms; `change_model/5000_symbols` < 1.5 s.
- **Acceptance criteria:** Golden model matches CHG-008 expected file; hash stable across thread counts.
- **Definition of done:** Global DoD; `docs/graph-schema/change-model.md` documents every field.

---

### CHG-008 — Change-model golden tests
Status: ☐

- **Task ID:** CHG-008
- **Title:** Golden/insta test suite for the change model over the auth-bypass scenario and a catalogue of focused scenarios, one per change class and collector.
- **Problem:** The change model is the contract for impact, risk, context and reviewers. Regressions in any classifier silently alter every downstream result; only committed golden outputs make such drift visible in review.
- **Why it exists:** Master plan milestone M4 ("golden diff → changed symbols → change classes → impact graph for all fixture PRs, incl. auth-bypass"); target-architecture §11 golden testing; PRD §141 diff-mapping acceptance.
- **Scope:**
  - `fixtures/pull-requests/auth-bypass/expected/change-model.json`.
  - Focused micro-scenarios (each a tiny base + patch) under `fixtures/pull-requests/change-*/`: `change-control-flow`, `change-calls-deps`, `change-exceptions`, `change-async`, `change-authz-validation`, `change-db-transaction`, `change-api-contract`, `change-return-type`, `change-dependency-bump`, `change-migration-drop-column`, `change-config-env`, `change-test-skip`, `change-rename-move`, `change-comment-only`, `change-generated-file`.
  - Positive and negative assertions (what must *not* be classified).
  - Determinism and thread-independence tests.
- **Explicit non-scope:** Impact/risk/context goldens (IMP-008, RISK-004, CTX-010). Quality benchmark corpus (QB). Live provider data.
- **Files/modules expected to change:** `engine/crates/diff-engine/Cargo.toml` (dev-deps), `fixtures/build.sh` (no new behaviour).
- **New files/modules expected:** `fixtures/pull-requests/auth-bypass/expected/change-model.json`, `fixtures/pull-requests/change-*/{scenario.yaml,base/**,patch.diff,expected/change-model.json}`, `engine/crates/diff-engine/tests/golden_change_model.rs`, `engine/crates/diff-engine/tests/golden_change_classes.rs`, `engine/crates/diff-engine/tests/snapshots/`.
- **Dependencies:** CHG-007, DIFF-007 (fixture builder and `Scenario` loader), INC-013 or the in-process INC pipeline (to produce `GraphPair`).
- **Implementation details:**
  - Test harness `support::model_for(scenario) -> ChangeModel`: build fixture repo, full-index base, run the INC pipeline to the head overlay (`INC-001..009`), `diff_commits` + hunks + dispositions, `map_hunks`, `build_change_model`. A second harness path builds the head via a *full* rebuild to assert both produce identical models (`model_equal_for_incremental_and_full_head`).
  - Auth-bypass expected facts (from the phase conventions): exactly one non-cosmetic changed symbol `ts:src/auth/auth.service#AuthService.authorize/method`, `Modified{body}`; classes `call_removed(PermissionService.check)`, `dependency_removed(PermissionService)`, `return_changed`, `condition_changed`, `authorization_changed(weakened)`; no `call_added`, no exception/async/db/transaction/validation classes; `format.ts` appears only as a cosmetic symbol/file entry (comment-only, `touches_code=false`); `apis`, `dependencies`, `schemas`, `configs` empty; `tests` empty because the spec file is unchanged; `coverage` empty; `risk_signals` empty at this stage; `intent` unset (CHG-009 test fills separately).
  - Snapshot format: canonical JSON with sorted keys, SHAs and hashes replaced by placeholders (`<sha>`, `<hash>`), so only semantic drift changes the snapshot. `cargo insta review` workflow documented in `fixtures/pull-requests/README.md`.
  - Each micro-scenario asserts a full expected `symbols[].classes[]` set (not only inclusion) so over-classification fails; negative scenarios (`change-comment-only`, `change-rename-move`) assert *absence* of semantic classes and correct `Renamed` lineage.
  - Mutation guard: a test deliberately removes one classifier from the registry in a debug-only build and asserts the golden test fails (ensures goldens actually cover each class).
- **Data model changes:** None.
- **API/protocol changes:** None.
- **Concurrency semantics:** Tests run the model build with 1 and 8 rayon threads and assert identical JSON and hash.
- **Failure behavior:** Golden mismatch prints a structural diff (path, expected, actual) and the command to review snapshots; missing `git` binary fails fast (container includes it).
- **Idempotency considerations:** Reproducible fixture SHAs (DIFF-007) and canonical JSON make snapshots stable across machines.
- **Security considerations:** Fixture content is synthetic; CI lints fixtures for secrets and forbidden names.
- **Observability additions:** None (assert that the `diff_analysis`/`change.*` spans are emitted once via the test tracing layer).
- **Tests required:**
  - `golden_auth_bypass_change_model`
  - `auth_bypass_exact_class_set_for_authorize`
  - `auth_bypass_comment_only_file_is_cosmetic`
  - `auth_bypass_has_no_decoy_symbols`
  - `model_equal_for_incremental_and_full_head`
  - `golden_control_flow`
  - `golden_calls_and_dependencies`
  - `golden_exceptions`
  - `golden_async`
  - `golden_authz_and_validation`
  - `golden_db_and_transaction`
  - `golden_api_contract`
  - `golden_return_type`
  - `golden_dependency_bump`
  - `golden_migration_drop_column_destructive`
  - `golden_config_env`
  - `golden_test_skip`
  - `golden_rename_move_lineage`
  - `golden_comment_only_is_cosmetic`
  - `golden_generated_file_unanalyzed`
  - `model_hash_identical_across_thread_counts`
  - `removing_a_classifier_breaks_goldens`
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** All goldens committed and reviewed; every `ChangeClass` and every collector has at least one positive and one negative scenario; goldens are identical between incremental and full head construction.
- **Definition of done:** Global DoD; `docs/graph-schema/change-model.md` links each scenario to the behaviour it pins.

---

### CHG-009 — Intent classification (11 PRD §29 classes; deterministic signals + optional CLASSIFIER tier; never overrides risk)
Status: ☐

- **Task ID:** CHG-009
- **Title:** `diff_engine::intent` — a deterministic weighted-signal classifier over the change model into the 11 PRD §29 intents, with an optional model refinement behind a trait, producing an advisory `IntentAssessment` that can never lower risk.
- **Problem:** Reviewers should frame their review by what the change is trying to do (a refactor should preserve behaviour; a bugfix should add a test; a dependency bump should check changelogs). Intent is only an estimate (PRD §29: "never treated as infallible"), and a model-derived label must not be able to talk the system out of a deterministic risk finding (e.g. "test-only" hiding an auth change).
- **Why it exists:** PRD §29; target-architecture §3.6 ("deterministic signals plus an optional CLASSIFIER model call. It never overrides deterministic risk"); reviewer routing (PRD §48) and prompt framing (REV-001) consume it.
- **Scope:**
  - `Intent` enum (11 classes), `IntentSignal`, `IntentAssessment`, signal table and scoring.
  - `IntentRefiner` trait (defined here, implemented in `pipeline` over `ModelGateway` tier CLASSIFIER).
  - Untrusted-text handling for PR title/branch/labels.
  - Invariant enforcement that risk never reads intent.
- **Explicit non-scope:** The gateway call implementation and prompts (`pipeline`, GW-*). Risk scoring (RISK-*). Using intent to skip review (explicitly disallowed).
- **Files/modules expected to change:** `engine/crates/diff-engine/src/lib.rs`, `engine/crates/diff-engine/src/change/model.rs` (fills `intent`).
- **New files/modules expected:** `engine/crates/diff-engine/src/intent/{mod.rs, signals.rs, score.rs, refine.rs}`, `engine/crates/diff-engine/tests/intent.rs`, `fixtures/pull-requests/intent-*/` (one tiny scenario per intent class where not already covered).
- **Dependencies:** CHG-007, CHG-008 (fixtures), GW-001 (only for the implementation in `pipeline`; this crate stays model-free), DOM-005 (`PullRequest` metadata).
- **Implementation details:**
  ```rust
  pub enum Intent { Feature, Bugfix, Refactor, TestOnly, Documentation, Configuration, Dependency, Migration, Performance, Security, GeneratedCode }
  pub struct IntentSignal { pub id: &'static str, pub intent: Intent, pub weight: f32, pub evidence: String /* ≤ 120 chars, no source */ , pub source: SignalSource /* Files|Symbols|Classes|PrMeta */ }
  pub struct IntentAssessment { pub primary: Option<Intent>, pub secondary: Vec<Intent>, pub scores: BTreeMap<Intent, f32>, pub signals: Vec<IntentSignal>,
                                pub confidence: f32, pub source: IntentSource /* Deterministic|Refined{model, prompt_version} */, pub advisory: bool /* always true */ }
  pub trait IntentRefiner: Send + Sync { fn refine(&self, input: &IntentRefineInput) -> Result<IntentRefinement, RefineError>; }
  pub fn classify_intent(m: &ChangeModel, pr: Option<&PrMeta>, refiner: Option<&dyn IntentRefiner>, cfg: &IntentCfg) -> IntentAssessment;
  ```
  - **Deterministic signals (weights in a versioned table):** all non-ignored files are docs (`*.md`, `docs/**`) → `Documentation` (1.0); all are tests/fixtures → `TestOnly` (1.0; requires 100%, partial test changes give only a weak `Feature/Bugfix` co-signal); only manifests/lockfiles → `Dependency`; migration paths or `schemas` non-empty → `Migration`; only config files → `Configuration`; all files generated/vendored → `GeneratedCode`; classes `authorization_changed|validation_removed` → `Security` (0.6 each); symbols all `Modified` with unchanged `signature_hash`, no new/removed symbols, no behavioural classes (only renames/moves/cosmetic or pure body restructuring with equal call/dependency sets) → `Refactor`; added symbols/endpoints/entities → `Feature`; loop/await/db-read/cache-related class changes with no new public API → `Performance` (0.4); small modified-only change plus a test added/modified → `Bugfix` (0.3).
  - **PR metadata (untrusted, low weight ≤ 0.25 total):** Conventional-commit prefix in title (`fix:`, `feat:`, `refactor:`, `perf:`, `docs:`, `chore(deps):`), branch prefixes (`fix/`, `feature/`), labels (`bug`, `security`). Extracted by anchored regex only; the text is never interpolated into anything and never passed to the refiner unredacted; metadata cannot create a `TestOnly`/`Documentation` primary on its own.
  - **Scoring:** per-intent sum of weights, normalized by the max; `primary` set when top score ≥ 0.6 and margin over second ≥ 0.15, else `None` (mixed). `secondary` = intents ≥ 0.4. `confidence` = margin-based in [0,1].
  - **Optional refinement:** invoked only if `refiner.is_some()` and (`primary.is_none()` or `confidence < 0.5`). Input (`IntentRefineInput`) is a deterministic summary: file-class counts, change-class counts, symbol kinds, redacted title — no code, no bodies. A refinement may choose among the top candidates or set `primary`, recorded as `source=Refined`, but is validated (`Intent` must be one of the 11; invalid → ignored). Refinement failures degrade silently to the deterministic result.
  - **Never overrides risk:** `IntentAssessment` is stored in `ChangeModel.intent`; the risk engine consumes `ChangeModel::risk_view()` (CHG-007) which has no intent field, and a workspace test greps `impact::risk` for `intent` identifiers. Reviewers receive intent only as framing text; reviewer *selection* uses risk effects (RISK-005), not intent.
  - Complexity O(files + classes + symbols).
- **Data model changes:** None (inside the change model artifact).
- **API/protocol changes:** `IntentAssessment` in contracts; the `IntentRefiner` input schema is versioned (`intent_refine_v1`).
- **Concurrency semantics:** Pure; the refiner call is made by `pipeline` outside the pure function (the trait object is provided already bound to the gateway) with the standard timeout/budget.
- **Failure behavior:** Any refiner error/timeout/budget refusal → deterministic result retained; invalid metadata ignored.
- **Idempotency considerations:** Deterministic path is byte-stable; refined path is cached by request hash via the gateway (replay provider in tests).
- **Security considerations:** PR title/branch/labels are attacker-controlled: they are matched structurally, length-limited, redacted before leaving the process, and can never raise or lower risk or suppress reviewers; prompt-injection text in a title cannot change a deterministic classification beyond its capped weight.
- **Observability additions:** span `change.intent` (attrs `primary`, `confidence`, `source`); counters `intent_assessments_total{intent,source}`, `intent_refine_calls_total{result}`.
- **Tests required:** `docs_only_is_documentation`, `all_tests_is_test_only`, `mixed_src_and_test_is_not_test_only`, `manifest_only_is_dependency`, `migration_path_is_migration`, `generated_only`, `auth_change_is_security`, `rename_only_is_refactor`, `new_endpoint_is_feature`, `small_fix_with_test_is_bugfix`, `ambiguous_returns_none_primary`, `title_prefix_alone_cannot_make_test_only`, `prompt_injection_in_title_has_capped_effect`, `refiner_invoked_only_when_ambiguous`, `invalid_refiner_output_ignored`, `refiner_error_falls_back`, `risk_view_has_no_intent` (trybuild) and `risk_module_does_not_reference_intent` (grep test), `golden_auth_bypass_intent_is_security_primary`.
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** Each of the 11 classes has a passing fixture; the auth-bypass scenario yields `Security` primary with `Bugfix`/`Feature` not exceeding it; the no-override invariant tests pass.
- **Definition of done:** Global DoD; signal table documented in `docs/reviewers/intent.md` with the "advisory only" rule.

---

### IMP-001 — ImpactGraph model
Status: ☐

- **Task ID:** IMP-001
- **Title:** `ImpactGraph` model: elements with relation, distance, path and min-confidence
- **Problem:** Reviewers, context selection and verification all need the same derived neighbourhood of a change, with *why* each element is included and how trustworthy the path is.
- **Why it exists:** PRD §30–§31; target-arch §3.7 ("each element records `distance`, `path` and `min_confidence`"); VER stage 3 re-checks these paths.
- **Scope:** types, builder API skeleton, path representation, merge rule for elements reached by multiple paths, serialization, and `ImpactBudget` defaults.
- **Explicit non-scope:** the expansion algorithms (IMP-002..006).
- **Files/modules expected to change:** `engine/crates/impact/Cargo.toml`, `impact/src/lib.rs`.
- **New files/modules expected:** `impact/src/graph/mod.rs`, `graph/model.rs`, `graph/path.rs`, `graph/budget.rs`, `tests/model.rs`.
- **Dependencies:** CG-001/CG-002 (node/edge kinds), CG-007 (`bounded_bfs`, `shortest_path`), CHG-007.
- **Implementation details:**
  ```rust
  pub enum Relation { Caller, Callee, RemovedCallee /*base-only*/, Implementation, Interface, Override, OverriddenBy, Subtype, Supertype,
                      RelatedType, Endpoint, Test, Config, EnvVar, DbTable, DbEntity, QueueProducer, QueueConsumer, ExternalApi, Container }
  pub struct PathStep { pub from: NodeKey, pub edge: EdgeKind, pub to: NodeKey, pub confidence: f32, pub graph: GraphSide /*Base|Head*/ }
  pub struct ImpactElement { pub node: NodeKey, pub node_id: String, pub kind: NodeKind, pub relation: Relation,
                             pub distance: u8, pub path: Vec<PathStep> /*seed → node, best path*/, pub min_confidence: f32,
                             pub alt_paths: u16, pub weak: bool /*min_confidence < budget.min_confidence*/ }
  pub struct SymbolImpact { pub seed: SymbolKey, pub elements: Vec<ImpactElement>, pub truncation: Vec<Truncation> }
  pub struct ImpactGraph { pub schema_version: u16, pub symbols: Vec<SymbolImpact>, pub budget: ImpactBudget,
                           pub stats: ImpactStats, pub input_hash: Hash256 }
  pub struct Truncation { pub relation: Relation, pub limit: u32, pub dropped: u32, pub reason: TruncReason /*Limit|Depth|TotalCap*/ }
  ```
  - Best path selection when a node is reached multiple ways: maximize `min_confidence`, then minimize `distance`, then lexicographic on node keys of the path (deterministic). `alt_paths` counts others.
  - `min_confidence` = min over path step confidences (confidence values from CG-003 table).
  - Elements sorted by `(relation order, distance, -min_confidence, node_id)`.
  - Default graph side is Head; `RemovedCallee` and removed-symbol impacts use Base.
  - `input_hash = blake3(change_model.input_hash ‖ head_snapshot ‖ base_snapshot ‖ budget ‖ IMPACT_VERSION)`.
- **Data model changes:** None (stored in `stage_outputs`).
- **API/protocol changes:** `ImpactGraph` JSON Schema in contracts; review-engine `GET /internal/review-runs/{id}/impact` and `POST /internal/impact` (`{snapshot_id, symbol_keys, budget?}` for CLI/UI) — handler in API-013.
- **Concurrency semantics:** Immutable output; built from `Arc<Graph>` reads.
- **Failure behavior:** N/A for the model; builder errors typed `ImpactError::{GraphMissing, SeedNotFound}`.
- **Idempotency considerations:** Deterministic ordering and best-path tie-breaking.
- **Security considerations:** Node IDs reveal paths/symbol names only; tenant-scoped storage.
- **Observability additions:** None here (IMP-002 onwards).
- **Tests required:** `best_path_prefers_higher_min_confidence`, `tie_breaks_by_distance_then_lexicographic`, `elements_sorted_deterministically`, `serde_roundtrip_and_schema`, `input_hash_includes_budget`.
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** tests green; schema published.
- **Definition of done:** global DoD; `docs/graph-schema/impact-graph.md` describes relations and fields.

---

### IMP-002 — Callers and callees bounded expansion
Status: ☐

- **Task ID:** IMP-002
- **Title:** Caller (≤2, transitive if budget) and callee (1) expansion per changed symbol
- **Problem:** "Who calls this and what does it call" is the core reasoning surface; unbounded expansion explodes on utility functions.
- **Why it exists:** Critical path (`CHG-007 → IMP-002 → CTX-006`); PRD §30.
- **Scope:** reverse `CALLS` (callers) up to depth 2, depth 3 only if the per-symbol and per-PR budgets have ≥ 50% remaining after all seeds' depth-2 pass; forward `CALLS` depth 1 on head; base-side callees removed by the change (`RemovedCallee`); DI/interface dispatch hops.
- **Explicit non-scope:** endpoint reachability (IMP-004 has its own deeper bounded search); type hierarchy (IMP-003).
- **Files/modules expected to change:** `impact/src/lib.rs`.
- **New files/modules expected:** `impact/src/graph/calls.rs`, `impact/src/graph/builder.rs` (orchestrates IMP-002..006 per seed), `tests/calls.rs`.
- **Dependencies:** IMP-001, CHG-003 (removed calls), CG-007, INC-009 (head overlay).
- **Implementation details:**
  - Seeds: all `ChangedSymbol`s except cosmetic ones and generated files (still listed with zero elements so coverage is visible). For `Removed` symbols, callers are computed on the **base** graph (who used to call the deleted thing — and, via head graph, whether those callers still exist).
  - Callers: `bounded_bfs(seed, In, [CALLS], max_depth=2, max_nodes=max_callers_per_symbol, min_confidence)`. Interface dispatch: if seed implements/overrides `I.m`, callers of `I.m` are included at the same distance with a `PathStep{edge: IMPLEMENTS}` hop of confidence 0.9 (virtual dispatch), since NestJS DI calls go through interfaces/tokens.
  - Frontier order inside BFS: higher edge confidence first, then node id — deterministic and quality-preserving under truncation.
  - Transitive pass (depth 3): only after all seeds finished depth ≤ 2, and only if `remaining_pr_elements ≥ 0.5 × max_total_elements_pr`; seeds processed in descending risk order (risk from RISK-004 if available, else change class count).
  - Callees: `out_edges(seed, [CALLS])` depth 1 on head; `RemovedCallee` = CHG-003 `call_removed` targets resolved on base, with `graph=Base` steps.
  - Each relation records `Truncation` when its limit was hit.
- **Data model changes:** None.
- **API/protocol changes:** None beyond IMP-001.
- **Concurrency semantics:** Seeds processed in parallel (rayon) for depth ≤ 2 with a shared `AtomicU32` PR element counter (reservation via `fetch_update`, never exceeding the cap); the depth-3 pass is sequential in risk order so results do not depend on thread timing. Results sorted at the end.
- **Failure behavior:** seed missing from graph (degraded parse) → empty `SymbolImpact` with `Truncation{reason: SeedMissing}`; continue.
- **Idempotency considerations:** Deterministic despite parallelism (reservation order cannot affect depth ≤ 2 results because per-PR cap is checked only in pass 2; pass 1 overflow is handled by per-seed caps whose sum is bounded: if `seeds × max_total_elements > max_total_elements_pr`, per-seed caps are reduced to `floor(pr_cap / seeds)` up-front).
- **Security considerations:** None.
- **Observability additions:** span `impact_analysis` (attrs `seeds`, `elements`, `truncated_relations`, `transitive_pass_ran`); histogram `impact_analysis_ms`; counter `impact_truncations_total{relation}`.
- **Tests required:** `auth_bypass_callers_updateuser_d1_controller_d2`, `auth_bypass_removed_callee_permission_check_on_base`, `report_service_decoy_not_a_caller`, `interface_dispatch_callers_included`, `depth3_only_when_budget_remains`, `per_symbol_caller_cap_truncates_and_reports`, `low_confidence_edge_excluded_but_weak_listed` (0.3 `name_ambiguous`), `removed_symbol_callers_from_base_graph`, `parallel_result_equals_sequential` (proptest over random DAGs), `bfs_never_exceeds_budget` (proptest).
- **Benchmarks if applicable:** `benches/impact_calls.rs` on synthetic 1M-symbol graph (PERF-001): 50 seeds, p95 < 50 ms total.
- **Acceptance criteria:** golden expectations for auth-bypass met; proptests 1,000 cases; bench within target.
- **Definition of done:** global DoD.

---

### IMP-003 — Type hierarchy relations
Status: ☐

- **Task ID:** IMP-003
- **Title:** Implementations, interfaces, overrides, sub/supertypes and related types
- **Problem:** Changing `AuthService.authorize` affects the `AuthProvider` contract and any sibling implementations; changing a DTO affects every handler accepting it.
- **Why it exists:** PRD §30–§31 (`IMPLEMENTS → AuthProvider.authorize`).
- **Scope:** `IMPLEMENTS`, `EXTENDS`, `OVERRIDES` both directions at depth 1 (2 for EXTENDS chains), method-level mapping (class implements interface ⇒ method implements interface method by name/arity), and `RelatedType` via `USES_TYPE/ACCEPTS_TYPE/RETURNS_TYPE` for changed type-like symbols.
- **Explicit non-scope:** structural typing inference beyond what the analyzer/linker emits.
- **Files/modules expected to change:** `impact/src/graph/builder.rs`.
- **New files/modules expected:** `impact/src/graph/types.rs`, `tests/types.rs`.
- **Dependencies:** IMP-001, CG-005 (hierarchy edges), TSA-005 (class/interface members).
- **Implementation details:**
  - For a changed method `C.m`: `Interface` = `I.m` where `C IMPLEMENTS I` and `I` declares `m` (edge from linker, or synthesized here with confidence = class-level edge confidence × 0.95); `Override`/`OverriddenBy` from `OVERRIDES`; sibling implementations `D.m` of the same `I.m` included as `Implementation` at distance 2 (path `C.m → I.m ← D.m`).
  - For a changed class/interface/type alias: `Subtype` (reverse EXTENDS/IMPLEMENTS, depth ≤ 2), `Supertype` (forward), `RelatedType` users (reverse `USES_TYPE|ACCEPTS_TYPE|RETURNS_TYPE`, depth 1, cap `max_type_relations`) — a changed DTO thereby pulls in its handlers.
  - Signature change on an interface method ⇒ all implementations at distance 1 (contract break).
- **Data model changes:** None.
- **API/protocol changes:** None.
- **Concurrency semantics:** as IMP-002.
- **Failure behavior:** as IMP-002.
- **Idempotency considerations:** Deterministic.
- **Security considerations:** None.
- **Observability additions:** counter `impact_elements_total{relation}` (shared by IMP-002..006).
- **Tests required:** `auth_bypass_implements_authprovider_authorize`, `sibling_implementation_at_distance_2`, `override_chain`, `interface_signature_change_pulls_all_impls`, `dto_change_pulls_handlers_as_related_type`, `type_relations_capped_and_reported`.
- **Benchmarks if applicable:** included in IMP-008 end-to-end bench.
- **Acceptance criteria:** tests green; auth-bypass shows `Interface: AuthProvider.authorize` with min_confidence ≥ 0.9.
- **Definition of done:** global DoD.

---

### IMP-004 — API entrypoint reachability
Status: ☐

- **Task ID:** IMP-004
- **Title:** Reverse reachability from a changed symbol to `APIEndpoint` nodes (bounded)
- **Problem:** "Does the affected path reach a public endpoint?" is a §151 verification question and a major risk amplifier; it can be farther than caller depth 2.
- **Why it exists:** PRD §30, §151 stage 5; VER stage 3 uses the recorded path.
- **Scope:** reverse search over `CALLS` (+ interface dispatch) to handlers, then `HANDLED_BY`/`ROUTES_TO` to `APIEndpoint`; also `CLICommand`, `JobHandler`/`QueueConsumer`, `Worker` entrypoints as `Endpoint` with `entry_kind`.
- **Explicit non-scope:** guard analysis along the path (VER-007 contradiction search does that); runtime reachability.
- **Files/modules expected to change:** `impact/src/graph/builder.rs`.
- **New files/modules expected:** `impact/src/graph/entrypoints.rs`, `tests/entrypoints.rs`.
- **Dependencies:** IMP-002, NEST-001 (routes → `HANDLED_BY`), NEST-005/BullMQ facts (job handlers), CG-007.
- **Implementation details:**
  - Search: best-first (priority = path min_confidence, then depth) over reverse `CALLS ∪ dispatch` up to `max_endpoint_depth=6`, visiting at most 2,000 nodes per seed (independent of the caller element cap; visited-but-not-emitted nodes are not elements). When a node is a handler (has incoming `HANDLED_BY` from an `APIEndpoint`), emit the endpoint with the full path.
  - Endpoint element attrs: `{method, path, guards: [..] (from endpoint attrs), entry_kind: http|queue|cli|cron|worker}`; guards are *recorded*, not evaluated.
  - Stop at `max_endpoints`; record truncation. Reuse IMP-002 caller results as the first two BFS layers (no recomputation).
- **Data model changes:** None.
- **API/protocol changes:** None.
- **Concurrency semantics:** per seed parallel; visited set per seed.
- **Failure behavior:** visit cap reached → truncation `Depth` with `visited=2000` recorded; continue.
- **Idempotency considerations:** Deterministic priority queue with total order `(Reverse(min_conf), depth, node_id)`.
- **Security considerations:** None.
- **Observability additions:** counter `impact_endpoint_search_visits_total`; span attr `endpoints_found`.
- **Tests required:** `auth_bypass_reaches_put_users_id_at_distance_3`, `queue_consumer_entrypoint_found`, `unreachable_internal_symbol_has_no_endpoint`, `visit_cap_truncates`, `best_path_reported_when_two_routes`, `guards_recorded_on_endpoint`.
- **Benchmarks if applicable:** synthetic graph: 50 seeds, p95 < 80 ms.
- **Acceptance criteria:** auth-bypass endpoint `http:PUT /users/:id` with path `authorize ← updateUser ← UserController.update ← HANDLED_BY`.
- **Definition of done:** global DoD.

---

### IMP-005 — Test mapping
Status: ☐

- **Task ID:** IMP-005
- **Title:** Map tests to production symbols: `TESTS` edges, imports from test files, naming/path conventions, mock references
- **Problem:** Test reviewer and context need "which tests cover this", and "no test covers this" is itself a signal; the reference consumer used only callers-in-test-files.
- **Why it exists:** PRD §99 (signals: direct imports, invocation, naming, mocks, path conventions; coverage/semantic later).
- **Scope:** multi-signal test scorer, `Test` impact elements with `test_signals`, and an `untested` flag per changed symbol.
- **Explicit non-scope:** coverage data ingestion (post-MVP `RUNTIME-001`); semantic similarity test signal (added by CTX-008 only as context candidates, not as mapping).
- **Files/modules expected to change:** `impact/src/graph/builder.rs`.
- **New files/modules expected:** `impact/src/graph/tests_map.rs`, `tests/tests_map.rs`, fixture repo `fixtures/repositories/test-mapping/`.
- **Dependencies:** IMP-002, NEST-006 (Jest suites/cases, `jest.mock`, `Test.createTestingModule` providers), CG-005 (`TESTS` edges), INIT-006 (test file patterns).
- **Implementation details:**
  - Signals per (test case `t`, symbol `s`), each in [0,1]:
    - `invocation` = 1.0 if `t` (or a helper it calls within depth 2) has a `CALLS` path to `s` (reverse BFS from `s` restricted to test-file nodes, depth ≤ 3).
    - `tests_edge` = 1.0 if `TESTS(t → s)` or `TESTS(suite → class of s)` exists.
    - `import` = 0.8 if the test file imports the module of `s` (0.6 if it imports a re-export barrel).
    - `naming` = 0.6 if suite/case name contains `s.name` or its class name (case-insensitive token match), or file name `x.spec.ts`/`x.test.ts`/`x.e2e-spec.ts` matches source `x.ts`.
    - `path` = 0.4 if test lives in the conventional location (`same dir`, `__tests__/`, `test/` mirror path).
    - `mock` = 0.5 if `t` mocks `s`'s class (`jest.mock(path)`, `useValue` provider override) — means "depends on, but does not exercise"; recorded as `relation=Test` with `mocked=true`.
  - `score = 1 − Π(1 − signal)`; include if `score ≥ 0.6`; mocked-only tests are included with `mocked=true` and do not clear the `untested` flag.
  - `untested(s) = no included non-mocked test with score ≥ 0.8`.
  - Cap `max_tests` by score; truncation reported.
  - Also emit for each `ChangedTest` from CHG-006 the reverse: target symbols (helps test reviewer).
- **Data model changes:** None.
- **API/protocol changes:** `ImpactElement.test: Option<TestMapping{score, signals, mocked}>`.
- **Concurrency semantics:** per seed parallel; test-file node set precomputed once per head graph (cached in `Arc`).
- **Failure behavior:** no test facts (no Jest adapter) → naming/path/import signals only; `untested` still computed but with `test_mapping_degraded=true`.
- **Idempotency considerations:** Deterministic.
- **Security considerations:** None.
- **Observability additions:** counter `impact_untested_changed_symbols_total`; histogram `test_mapping_score`.
- **Tests required:** `auth_bypass_authorize_spec_maps_with_invocation_and_naming`, `e2e_spec_covers_controller_via_invocation`, `mock_only_test_marked_mocked_and_untested`, `barrel_import_lower_score`, `path_convention_only_below_threshold`, `untested_flag_when_no_tests`, `changed_test_maps_to_targets`, `no_jest_adapter_degraded_mode`.
- **Benchmarks if applicable:** test node precompute on reference-api < 50 ms.
- **Acceptance criteria:** on `test-mapping` fixture precision ≥ 0.9 and recall ≥ 0.9 against labelled `expected/test-map.json`.
- **Definition of done:** global DoD; signal weights documented in `docs/graph-schema/impact-graph.md`.

---

### IMP-006 — Config, DB, queue and external API relations
Status: ☐

- **Task ID:** IMP-006
- **Title:** Resource relations: config/env, DB tables/entities, queue producers/consumers, external APIs
- **Problem:** A changed method writing a table or producing a job affects consumers that are not callers (PRD §30 "database interactions, configuration dependencies, queue consumers/producers, external API boundaries").
- **Why it exists:** PRD §30; RISK-003 uses these relations; context needs the consumer handler of a produced job.
- **Scope:** depth-1 resource edges from the seed (`READS_CONFIG`, `WRITES_CONFIG`, `READS_TABLE`, `WRITES_TABLE`, `PRODUCES_JOB`, `CONSUMES_JOB`, `PUBLISHES`, `SUBSCRIBES`, `DEPENDS_ON → ExternalAPI/ExternalDependency`), and the **other side** at distance 2 (consumers of a produced queue; other writers/readers of a written table capped at 5; other readers of a changed env var capped at 5).
- **Explicit non-scope:** SQL parsing beyond what NEST-004 emits; external API schema diffing.
- **Files/modules expected to change:** `impact/src/graph/builder.rs`.
- **New files/modules expected:** `impact/src/graph/resources.rs`, `tests/resources.rs`, fixture `fixtures/repositories/queues-db-config/`.
- **Dependencies:** IMP-001, NEST-004 (TypeORM tables), NEST-005 (BullMQ queues), CG-006 (synthetic nodes).
- **Implementation details:**
  - Seed-level: for the seed and its container class's injected repositories (`Repository<User>` DI → `db:public.users` through the entity), emit `DbTable`/`DbEntity`/`QueueProducer`/`Config`/`EnvVar`/`ExternalApi` at distance 1.
  - Other side: queue `queue:X` → `CONSUMES_JOB` handlers at distance 2 (`QueueConsumer`); table → other `WRITES_TABLE` sources (`DbTable` relation with `role=co_writer`); env var → other readers.
  - Removed resource edges (base-only, e.g. a write removed) emitted with `graph=Base` and attr `removed=true`.
- **Data model changes:** None.
- **API/protocol changes:** `ImpactElement.resource: Option<ResourceAttrs{role, removed}>`.
- **Concurrency semantics:** as IMP-002.
- **Failure behavior:** missing adapters → no elements, flag `resource_facts_available=false`.
- **Idempotency considerations:** Deterministic.
- **Security considerations:** env var *names* only; values never present in the graph.
- **Observability additions:** shared `impact_elements_total{relation}`.
- **Tests required:** `produced_job_pulls_consumer_at_distance_2`, `table_write_lists_co_writers_capped`, `env_var_read_added_lists_other_readers`, `removed_write_reported_from_base`, `auth_bypass_admin_updateuser_writes_users_table_not_seed_relation` (only seeds' own resources at d=1; caller's table appears via caller element attrs, not duplicated).
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** fixture expectations met; caps respected and reported.
- **Definition of done:** global DoD.

---

### IMP-007 — Impact budgets and truncation reporting
Status: ☐

- **Task ID:** IMP-007
- **Title:** Configurable impact budgets, risk-scaled, with complete truncation reporting
- **Problem:** Budgets that are fixed or silently applied either blow latency on large PRs or hide relevant impact.
- **Why it exists:** Master plan principle 4; PRD §90 ("maximum graph expansion").
- **Scope:** `ImpactBudget` resolution from defaults → `.review/config.yaml` (`review.budgets.impact.*`) → risk multiplier (RISK-005 `depth` effect) → PIPE-006 run budget clamp; truncation aggregation into `ImpactGraph.stats` and a human-readable summary line for the review summary.
- **Explicit non-scope:** context budgets (CTX-006).
- **Files/modules expected to change:** `impact/src/graph/budget.rs`, `impact/src/graph/builder.rs`.
- **New files/modules expected:** `impact/tests/budget.rs`.
- **Dependencies:** IMP-001..IMP-006, POL-001, RISK-005 (multipliers), PIPE-006.
- **Implementation details:**
  - Resolution: `effective = clamp(default × risk_multiplier, min=default/2, max=run_budget)`; multipliers per risk level: low 0.5, medium 1.0, high 1.5, critical 2.0 applied to element caps; depth caps: `max_caller_depth` = 1 (low), 2 (medium/high), 3 (critical, still subject to the IMP-002 remaining-budget rule).
  - Config validation: caps must be positive and ≤ hard maxima (callers 200, total per PR 20,000) — enforced by POL-001 schema and re-checked here.
  - `ImpactStats { elements_by_relation, truncations: Vec<(SymbolKey, Truncation)>, seeds_without_graph, weak_elements }`.
  - Summary line: `"Impact truncated for 3 of 41 symbols (callers ×2, tests ×1); 12 low-confidence relations omitted."` consumed by GH-008.
- **Data model changes:** None.
- **API/protocol changes:** config keys `review.budgets.impact.{max_callers,max_callees,max_tests,max_endpoints,max_total_elements_pr,min_confidence}` added to POL-001 schema (documented here, schema change owned by POL-001).
- **Concurrency semantics:** Resolution is pure and computed once per run before expansion.
- **Failure behavior:** invalid config → POL-001 rejects at load; if somehow invalid here → fall back to defaults and emit `config_invalid` warning (never unbounded).
- **Idempotency considerations:** Effective budget is part of `ImpactGraph.input_hash`.
- **Security considerations:** Hard maxima protect workers from config-driven resource exhaustion.
- **Observability additions:** span attrs `budget_multiplier`, `effective_total_cap`; counter `impact_budget_exhausted_total`.
- **Tests required:** `low_risk_halves_caps`, `critical_allows_depth3`, `run_budget_clamps_config`, `invalid_config_falls_back_to_defaults`, `every_truncation_reported_in_stats`, `summary_line_format`, `proptest_elements_never_exceed_effective_caps`.
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** proptest 1,000 cases; summary line golden.
- **Definition of done:** global DoD; `.review/config.yaml` reference doc lists the keys.

---

### IMP-008 — Impact golden tests incl. auth-bypass
Status: ☐

- **Task ID:** IMP-008
- **Title:** Golden impact graphs for all PR fixtures, including the PRD §151 auth-bypass
- **Problem:** Impact correctness underpins verification; regressions must be caught.
- **Why it exists:** Milestone M4; risk R5 (context misses the decisive caller).
- **Scope:** `expected/impact.json` and `expected/clusters.json` (after IMP-009) per fixture; precision/recall check of impact elements against labelled expectations; end-to-end bench.
- **Explicit non-scope:** risk expectations (RISK-004).
- **Files/modules expected to change:** none outside tests.
- **New files/modules expected:** `impact/tests/golden.rs`, `fixtures/pull-requests/*/expected/impact.json`, `fixtures/pull-requests/*/expected/impact-labels.yaml` (`must_include`, `must_exclude`), `impact/benches/impact_e2e.rs`.
- **Dependencies:** IMP-001..IMP-007, CHG-008.
- **Implementation details:**
  - auth-bypass `impact-labels.yaml`: `must_include: [Caller AdminService.updateUser d1, Caller UserController.update d2, Endpoint "http:PUT /users/:id", Interface AuthProvider.authorize, RemovedCallee PermissionService.check, Test "test:src/auth/authorize.spec.ts#AuthService › denies without permission"]`, `must_exclude: [ReportService.*, format.ts#authorizeHeader]`.
  - Harness checks labels first (clear failure messages), then the insta snapshot.
- **Data model changes:** None.
- **API/protocol changes:** None.
- **Concurrency semantics:** as DIFF-007.
- **Failure behavior:** as DIFF-007.
- **Idempotency considerations:** Deterministic.
- **Security considerations:** None.
- **Observability additions:** None.
- **Tests required:** `golden_impact_auth_bypass`, `golden_impact_api_contract_break` (all handlers of DTO), `golden_impact_transaction_removed` (callers + table), `golden_impact_pure_rename` (callers re-linked, no removed callees), `golden_impact_large_mixed` (truncation entries), `labels_must_include_and_exclude`.
- **Benchmarks if applicable:** `impact_e2e`: reference-api 10-file PR < 150 ms; synthetic 200-file PR < 1.5 s.
- **Acceptance criteria:** all label checks pass; benches within target.
- **Definition of done:** global DoD; M4 checklist item ticked in master plan.

---

### IMP-009 — Change clustering
Status: ☐

- **Task ID:** IMP-009
- **Title:** Cluster changed symbols into review units (module + connected components over call/type edges; API/entity grouping)
- **Problem:** Large PRs cannot be reviewed as one unit (PRD §91); unrelated changes in one prompt dilute attention, while related changes split across units lose cross-symbol reasoning.
- **Why it exists:** PRD §91–§92; target-arch §3.7.
- **Scope:** deterministic clustering algorithm, cluster model, max-size splitting, grouping of API and DB-entity changes.
- **Explicit non-scope:** semantic-similarity clustering (PRD §92 lists it; deferred, see note) and ranking (IMP-010).
- **Files/modules expected to change:** `impact/src/lib.rs`.
- **New files/modules expected:** `impact/src/cluster/mod.rs`, `cluster/union_find.rs`, `tests/cluster.rs`.
- **Dependencies:** IMP-002, IMP-003, IMP-006, CHG-006.
- **Implementation details:**
  ```rust
  pub struct ChangeCluster { pub id: ClusterId /*blake3 of sorted member keys, 16 hex*/, pub members: Vec<SymbolKey>,
      pub modules: Vec<RepoPath>, pub apis: Vec<String>, pub entities: Vec<String>, pub files: Vec<RepoPath>,
      pub reason: Vec<ClusterReason /*SameModule|CallEdge|TypeEdge|SameApi|SameEntity|SameTestTarget|Singleton*/>, pub size_lines: u32 }
  ```
  - Union-find over non-cosmetic changed symbols. Union `a,b` if any of:
    1. direct `CALLS`/`IMPLEMENTS`/`EXTENDS`/`OVERRIDES`/`USES_TYPE` edge between them (either direction, head graph, confidence ≥ 0.6);
    2. both within the same module directory (nearest `*.module.ts` owner dir or, absent framework modules, same parent directory) **and** connected via a path of length ≤ 2 through any node inside the PR's impact elements;
    3. same `ChangedApi` endpoint (handler + DTO + guard);
    4. same `db:` entity/table via `WRITES_TABLE`/entity class;
    5. changed test and its mapped targets (IMP-005, score ≥ 0.8) join the target's cluster.
  - Same-module alone (without connection) does **not** merge — avoids one giant cluster per module.
  - Size cap: a cluster with > 12 members (correctness budget 8 × 1.5) is split by Louvain-free deterministic bisection: remove the weakest internal edge(s) (lowest confidence, tie by key) until each part ≤ 12 or no edges remain; record `split_from`.
  - Cosmetic and generated symbols go into one `LowRisk` pseudo-cluster per PR (IMP-010 decides).
  - Note on semantic similarity: deferred to post-MVP task `IMP-011 (semantic clustering)`; recorded here per execution protocol rule 5.
- **Data model changes:** None.
- **API/protocol changes:** `ChangeCluster` in contracts; `ImpactGraph.clusters`.
- **Concurrency semantics:** Single-threaded (O(n α(n)) + edge scan over members); deterministic.
- **Failure behavior:** no graph → every symbol singleton cluster grouped by file; flag `clustering_degraded`.
- **Idempotency considerations:** Cluster IDs are content-derived, so re-runs and incremental re-reviews of the same head produce the same IDs (needed for context cache keys).
- **Security considerations:** None.
- **Observability additions:** span `change_clustering` (attrs `clusters`, `max_cluster_size`, `splits`); histogram `cluster_size`.
- **Tests required:** `auth_bypass_single_cluster_one_member`, `caller_and_callee_both_changed_same_cluster`, `same_module_unconnected_stay_separate`, `dto_handler_guard_grouped_by_api`, `entity_and_writer_grouped`, `test_joins_target_cluster`, `oversized_cluster_split_deterministically`, `cluster_id_stable_across_runs`, `cosmetic_into_low_risk_pseudo_cluster`.
- **Benchmarks if applicable:** 500 changed symbols < 20 ms.
- **Acceptance criteria:** tests green; expected `clusters.json` snapshots for all PR fixtures.
- **Definition of done:** global DoD; `IMP-011` deferral row added to the phase table in this file's index.

---

### IMP-010 — Cluster risk ranking, budget allocation, unreviewed-region report
Status: ☐

- **Task ID:** IMP-010
- **Title:** Rank clusters by risk, allocate the review budget in that order, and report unreviewed regions (PRD §91)
- **Problem:** With a finite budget, the critical cluster must be reviewed first and anything skipped must be disclosed — legacy reviews self-reported completeness unreliably.
- **Why it exists:** PRD §91 ("summary should transparently indicate any skipped low-risk regions"), master plan principle 10 (completeness computed, not self-reported).
- **Scope:** cluster scoring, allocation of reviewer runs/tokens from the PIPE-006 run budget, `ReviewPlan` output, `UnreviewedRegion` list with reasons.
- **Explicit non-scope:** reviewer selection per cluster (REV-002 consumes `ReviewPlan`); context per cluster (CTX).
- **Files/modules expected to change:** `impact/src/cluster/mod.rs`.
- **New files/modules expected:** `impact/src/cluster/plan.rs`, `tests/plan.rs`.
- **Dependencies:** IMP-009, RISK-004 (symbol and cluster risk), RISK-005 (effects), PIPE-006 (run budget).
- **Implementation details:**
  - `cluster_score = max(member risk score) + 0.1 × min(1, n_high_signals/3) + 0.05 × endpoint_reachable`, clamped to 1.0; ties by `size_lines` desc then `id`.
  - Allocation: `plan_budget = { model_tokens, model_calls, reviewed_symbols (config max_symbols=100) }`. Iterate clusters by score: assign `estimated_cost(cluster) = Σ_selected_reviewers (reviewer.context_cap(risk) + output_reserve 2,000 tokens)`, calls = reviewers count. Critical/high clusters are always planned (even if over the soft budget, up to the hard run budget); medium planned while budget remains; low and the LowRisk pseudo-cluster are planned only if ≥ 30% budget remains after medium.
  - `ReviewPlan { units: Vec<PlannedUnit{cluster_id, priority, risk_level, reviewers_hint, token_allowance}>, unreviewed: Vec<UnreviewedRegion> }`; `UnreviewedRegion { cluster_id, files, symbols, risk_level, reason: BudgetExhausted|LowRiskSkipped|GeneratedCode|Binary|ParseDegraded }`. DIFF-004 `skipped` files are appended as regions with their class reason.
  - Invariant: every changed file appears either in a planned unit or in `unreviewed` (completeness computed).
  - Summary text for GH-008: `"Reviewed 4 of 6 change clusters (all high/critical). Skipped: 2 low-risk clusters (14 files: formatting, import sorting)."`
- **Data model changes:** None (stage output).
- **API/protocol changes:** `ReviewPlan`, `UnreviewedRegion` in contracts; review detail API exposes them (API-010).
- **Concurrency semantics:** Pure.
- **Failure behavior:** run budget smaller than the first critical cluster → plan it anyway with `over_budget=true` (critical review is never silently skipped) and emit `budget_overrun_planned` warning; PIPE-006 enforces the hard stop at runtime and records `NOT_EXECUTED` (≠ PASS, INV-014) for units it could not run.
- **Idempotency considerations:** Deterministic given inputs.
- **Security considerations:** Security-relevant clusters (authz/validation/secrets signals) are treated as at least `high` for planning regardless of score (cannot be skipped for budget).
- **Observability additions:** span `review_planning` (attrs `clusters_total`, `clusters_planned`, `clusters_skipped`, `over_budget`); counters `review_units_planned_total{risk_level}`, `unreviewed_regions_total{reason}`.
- **Tests required:** `critical_cluster_first`, `low_risk_skipped_when_budget_tight_and_reported`, `every_file_covered_by_plan_or_unreviewed` (proptest), `security_cluster_never_skipped`, `critical_over_budget_still_planned_flagged`, `binary_files_reported_as_unreviewed`, `summary_text_golden`, `auth_bypass_plan_single_critical_unit`.
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** completeness proptest (1,000 cases); auth-bypass plan has one critical unit and `format.ts` listed as `LowRiskSkipped` or planned low depending on budget (fixture budget makes it planned; a second test with tiny budget asserts skipped + reported).
- **Definition of done:** global DoD; invariant test registered (INV-013 completeness computed).

---

---

### RISK-001 — RiskSignal model + rule table (18 PRD §37 categories)
Status: ☐

- **Task ID:** RISK-001
- **Title:** `RiskSignal` model and a versioned rule table covering the 18 PRD §37 categories
- **Problem:** Risk knowledge must live in one inspectable, testable table rather than in prompt checklists (legacy `passes.rs:106-131`, consumer-only).
- **Why it exists:** PRD §37; target-arch §3.7 ("a rule table of signal → weight → effects").
- **Scope:** category enum, signal type, rule table (static, versioned), rule registry API used by RISK-002/003, explanation strings.
- **Explicit non-scope:** detectors (RISK-002/003), scoring (RISK-004), effects (RISK-005).
- **Files/modules expected to change:** `impact/src/lib.rs`.
- **New files/modules expected:** `impact/src/risk/mod.rs`, `risk/signal.rs`, `risk/rules.rs`, `tests/rules.rs`, `docs/reviewers/risk-rules.md` (generated table, checked by a test).
- **Dependencies:** DOM-005 (`RiskSignal` shell), CHG-007.
- **Implementation details:**
  ```rust
  pub enum RiskCategory { Authentication, Authorization, Cryptography, Permissions, Payments, DatabaseMigrations, SchemaChanges,
      TransactionBoundaries, Concurrency, Queues, BackgroundJobs, PublicApiContracts, Serialization, Validation,
      ExternalNetworkCalls, FilesystemAccess, Secrets, DependencyUpdates }
  pub enum SignalSubject { Pr, File(RepoPath), Symbol(SymbolKey), Cluster(ClusterId) }
  pub struct RiskSignal { pub rule_id: &'static str /*e.g. "authz.call_removed"*/, pub category: RiskCategory, pub subject: SignalSubject,
                          pub weight: f32, pub confidence: f32, pub evidence: Vec<SignalEvidence>, pub explanation: String }
  pub struct RiskRule { pub id: &'static str, pub category: RiskCategory, pub weight: f32, pub source: RuleSource /*Path|Change|Framework|Graph|Manifest*/,
                        pub description: &'static str, pub floor_level: Option<RiskLevel> }
  pub const RISK_RULES_VERSION: u32 = 1;
  ```
  - Base weights per category (rule weight defaults to its category weight unless the rule overrides): Authorization 0.70, Secrets 0.70, Authentication 0.60, Cryptography 0.60, Permissions 0.60, Payments 0.65, DatabaseMigrations 0.55, SchemaChanges 0.50, TransactionBoundaries 0.50, PublicApiContracts 0.50, Validation 0.50, Concurrency 0.45, Queues 0.40, BackgroundJobs 0.35, Serialization 0.35, ExternalNetworkCalls 0.35, FilesystemAccess 0.35, DependencyUpdates 0.30.
  - Overrides examples: `authz.call_removed` 0.85 (floor `high`), `authz.guard_removed` 0.85 (floor `high`), `secrets.literal_added` 0.9 (floor `critical`), `migration.destructive` 0.75 (floor `high`), `migration.edited_existing` 0.7, `dep.major_bump` 0.45, `api.breaking_change` 0.6.
  - Signal dedup key: `(rule_id, subject)`; duplicates keep max confidence and merge evidence.
  - `docs/reviewers/risk-rules.md` is generated by `cargo run -p review-cli -- risk rules --markdown`; test `risk_rules_doc_up_to_date` fails if it differs.
- **Data model changes:** None (signals persisted in the change model artifact; `risk_signals` field).
- **API/protocol changes:** `RiskSignal`, `RiskCategory` in contracts.
- **Concurrency semantics:** Static table; `&'static` data.
- **Failure behavior:** N/A.
- **Idempotency considerations:** `RISK_RULES_VERSION` participates in risk input hash.
- **Security considerations:** Explanations contain no code text — only symbol IDs and rule descriptions.
- **Observability additions:** None (counted by RISK-004).
- **Tests required:** `all_18_categories_have_weight`, `weights_in_unit_interval`, `rule_ids_unique`, `dedup_merges_evidence_keeps_max_confidence`, `risk_rules_doc_up_to_date`.
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** tests green; generated doc committed.
- **Definition of done:** global DoD.

---

### RISK-002 — Path, config and manifest signals
Status: ☐

- **Task ID:** RISK-002
- **Title:** Signals from configured risk paths, migrations, manifests/lockfiles and secret-like files/literals
- **Problem:** Some risk is known from location alone (`src/auth/**: critical`, PRD §122) or from file type (migrations, lockfiles, `.env`).
- **Why it exists:** PRD §37, §111 (secrets), §122 (`risk.paths`).
- **Scope:** `RiskPolicy` input (from config), path-glob signals with floor levels, migration/schema signals from CHG-006, dependency signals, secrets detection on added lines (pattern-based, using SEC-003 detector library).
- **Explicit non-scope:** reading `.review/config.yaml` (pipeline maps POL-001 config into `RiskPolicy`); full secret scanning of the repository (SEC-003 at index time).
- **Files/modules expected to change:** `impact/src/risk/mod.rs`.
- **New files/modules expected:** `impact/src/risk/detect_path.rs`, `risk/detect_manifest.rs`, `risk/detect_secrets.rs`, `risk/policy.rs`, `tests/detect_path.rs`.
- **Dependencies:** RISK-001, CHG-006, POL-001 (config schema), SEC-003 (secret pattern set; if not yet landed, a minimal pattern set is vendored here and replaced).
- **Implementation details:**
  ```rust
  pub struct RiskPolicy { pub paths: Vec<(GlobMatcher, RiskLevel)> /*ordered, first match wins*/, pub authorization_symbols: Vec<String>,
                          pub generated_globs: GlobSet, pub payment_paths: GlobSet /*default: **/payment*/**, **/billing/***/ }
  ```
  - `path.configured` signal: changed file matches `risk.paths` → signal with category inferred from glob keywords (`auth`→Authentication, `permission|policy|acl`→Permissions, `payment|billing`→Payments, `migration`→DatabaseMigrations, `crypto|security`→Cryptography, `secret`→Secrets, otherwise PublicApiContracts; an explicit `category:` in the config entry wins), weight `{critical:0.8, high:0.6, medium:0.4, low:0.1}`, and `floor_level` = configured level. Floors apply to the symbols/clusters in that file only.
  - Default path rules (when no config): `**/auth/**|**/security/**` → Authentication 0.5; `**/migrations/**` → DatabaseMigrations; `**/payment*/**|**/billing/**` → Payments; `Dockerfile`, `.github/workflows/*.yml` → DependencyUpdates 0.3 (rule `path.infra`).
  - Manifest: `ChangedDependency` → `dep.added` 0.3, `dep.major_bump` 0.45, `dep.removed` 0.25, prerelease 0.4, lockfile-only change with no manifest change → `dep.lock_drift` 0.2; category DependencyUpdates.
  - Schema: `ChangedSchema` destructive → `migration.destructive` (floor high); `MigrationModified` → `migration.edited_existing`; `EntityChanged` → SchemaChanges 0.5.
  - Secrets: scan **added lines only** of text files (re-read from head blob) with the SEC-003 pattern set (AWS keys, GitHub tokens `gh[pousr]_[A-Za-z0-9]{36,}`, private key headers, generic `(?i)(secret|token|password|api[_-]?key)\s*[:=]\s*['"][^'"]{12,}['"]`, high-entropy ≥ 4.0 bits/char strings ≥ 32 chars in assignments) → `secrets.literal_added` (floor critical); committed `.env*` (non-example) → `secrets.env_file` 0.8.
  - Evidence stores file + line + rule, **never** the matched value (stored as `redacted_preview: "AKIA****"` ≤ 4 visible chars).
- **Data model changes:** None.
- **API/protocol changes:** None beyond config keys owned by POL-001 (`risk.paths`, `risk.authorization_symbols`, `risk.payment_paths`).
- **Concurrency semantics:** Per file parallel; regexes compiled once (`once_cell`) with `regex` crate (linear time, no backtracking).
- **Failure behavior:** invalid glob in config is rejected by POL-001; unreadable blob → skip secrets scan for that file and add `secrets_scan_incomplete` warning (surfaced in summary).
- **Idempotency considerations:** Deterministic; policy hash in risk input hash.
- **Security considerations:** Secret values never leave the detector function: not in signals, logs, spans or the change model (test asserts serialized output lacks the fake secret).
- **Observability additions:** counters `risk_signals_total{category,source=path|manifest|secrets}`, `risk_secret_detections_total{rule}`.
- **Tests required:** `auth_bypass_src_auth_critical_floor`, `first_matching_path_rule_wins`, `default_rules_without_config`, `npm_major_bump_signal`, `lock_drift_signal`, `destructive_migration_floor_high`, `edited_existing_migration`, `fake_aws_key_detected_and_value_not_serialized`, `github_token_pattern`, `example_env_file_not_flagged`, `entropy_detector_ignores_short_strings`.
- **Benchmarks if applicable:** secrets scan of 10k added lines < 20 ms.
- **Acceptance criteria:** tests green; no secret bytes in any serialized artifact (asserted).
- **Definition of done:** global DoD.

---

### RISK-003 — Framework and graph signals
Status: ☐

- **Task ID:** RISK-003
- **Title:** Signals from change classes, framework facts and impact: guards, endpoints, DB writes, queues, transactions, external calls, fs, crypto, serialization
- **Problem:** Most real risk depends on what the changed code *does* and *reaches*, which only the change model + impact graph know.
- **Why it exists:** PRD §37; §151 requires `authorization` from `call_removed` of an authz target.
- **Scope:** detector mapping CHG classes and impact relations to rules; API-surface amplification; concurrency heuristics.
- **Explicit non-scope:** scoring (RISK-004).
- **Files/modules expected to change:** `impact/src/risk/mod.rs`.
- **New files/modules expected:** `impact/src/risk/detect_change.rs`, `risk/detect_graph.rs`, `tests/detect_change.rs`.
- **Dependencies:** RISK-001, CHG-002..CHG-006, IMP-002..IMP-006, NEST-* facts (crypto/fs/http client usage arrives as TSA-006 call facts against `pkg:` targets).
- **Implementation details:**
  - Class-driven rules (subject = symbol, confidence = class confidence):
    | Rule | Trigger | Category | Weight |
    |---|---|---|---|
    | `authz.call_removed` | `authorization_changed` with `authz_call_removed` | Authorization | 0.85 floor high |
    | `authz.direct_role_check` | `direct_role_comparison_added` | Authorization | 0.6 |
    | `authz.guard_removed` / `authz.guard_added` | guard flags | Authorization | 0.85 / 0.4 |
    | `authz.logic_changed` | other `authorization_changed` | Authorization | 0.7 |
    | `validation.removed` | `validation_removed` | Validation | 0.65 |
    | `db.write_changed` | `database_write_changed`, write inside a transaction scope | TransactionBoundaries | 0.45 |
    | `db.write_changed` | `database_write_changed`, write outside a transaction | SchemaChanges (persistent data mutation) | 0.40 |
    | `tx.boundary_changed` | `transaction_boundary_changed` | TransactionBoundaries | 0.60 |
    | `api.breaking_change` | `api_contract_changed`: removed endpoint/field/param, narrowed type | PublicApiContracts | 0.60 |
    | `api.additive_change` | `api_contract_changed`: additive only | PublicApiContracts | 0.35 |
    | `async.floating_promise` | `async_behavior_changed` with `floating_promise` or `await_removed_on_db_write` | Concurrency | 0.50 |
  - Error handling: PRD §37 has no error-handling category. `exception_handling_changed` (including `swallowed`) therefore produces **no risk-score signal**; it influences reviewer routing through the change class (REV-002) instead. This decision is recorded in `docs/reviewers/risk-rules.md`.
  - Graph/framework rules (subject = symbol):
    - `endpoint.reachable`: impact has `Endpoint` with `entry_kind=http` → amplifier signal PublicApiContracts 0.3 (and multiplies other Authorization/Authentication/Validation signals of that symbol by 1.15 in RISK-004).
    - `queue.produce_changed` / `queue.consume_changed`: change in a symbol with `PRODUCES_JOB`/`CONSUMES_JOB` or a `JobHandler` → Queues 0.45 / BackgroundJobs 0.45.
    - `external.call_added`: `call_added` whose target is an `ExternalAPI` or `pkg:npm/{axios,node-fetch,got,undici}`/`HttpService` → ExternalNetworkCalls 0.4.
    - `fs.access_changed`: calls added/removed to `pkg:node/fs*`, `path.join` with request-derived args (fact flag) → FilesystemAccess 0.4.
    - `crypto.changed`: calls to `pkg:node/crypto`, `bcrypt`, `argon2`, `jsonwebtoken`, `jose` added/removed/changed args → Cryptography 0.6; JWT verify removed → Authentication 0.8 floor high.
    - `serialization.changed`: `SERIALIZES/DESERIALIZES` edges changed, `class-transformer` `@Exclude/@Expose` changes, `JSON.parse` on external input added → Serialization 0.4.
    - `auth.authn_changed`: changes in symbols with Passport strategy / `AuthGuard('jwt')` facts → Authentication 0.7.
    - `payments.changed`: symbol in `payment_paths` or calling `pkg:npm/stripe` → Payments 0.65.
    - `concurrency.shared_state`: module-level mutable variable written in a changed symbol, or `Promise.all` over db writes → Concurrency 0.45.
- **Data model changes:** None.
- **API/protocol changes:** None.
- **Concurrency semantics:** Pure per symbol, parallel.
- **Failure behavior:** missing impact (degraded) → class-driven rules still fire; graph rules skipped with flag.
- **Idempotency considerations:** Deterministic.
- **Security considerations:** None beyond RISK-002.
- **Observability additions:** `risk_signals_total{category,source=change|graph}`.
- **Tests required:** `auth_bypass_emits_authz_call_removed_and_direct_role_check_and_endpoint_reachable`, `guard_removed_floor_high`, `validation_removed_signal`, `write_in_tx_vs_outside_category`, `breaking_vs_additive_api_weight`, `floating_promise_concurrency_signal`, `catch_swallow_no_risk_signal_documented`, `queue_producer_change`, `axios_call_added_external`, `jwt_verify_removed_authentication_floor`, `report_service_decoy_no_signal`.
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** tests green; the documented decision on error handling recorded in `docs/reviewers/risk-rules.md`.
- **Definition of done:** global DoD.

---

### RISK-004 — Risk scoring (0..1, level)
Status: ☐

- **Task ID:** RISK-004
- **Title:** Deterministic risk score in [0,1] and level per symbol, cluster and PR
- **Problem:** Effects need a single comparable number and level, with floors from policy, without letting many weak signals masquerade as one strong one.
- **Why it exists:** PRD §37–§38; target-arch §3.7 `RiskAssessment { score 0..1, level, ... }`.
- **Scope:** scoring formula, floors, amplifiers, aggregation, `RiskAssessment` type, golden risk expectations.
- **Explicit non-scope:** effects (RISK-005), low-risk dampening (RISK-006 adjusts inputs before scoring).
- **Files/modules expected to change:** `impact/src/risk/mod.rs`.
- **New files/modules expected:** `impact/src/risk/score.rs`, `tests/score.rs`, `fixtures/pull-requests/*/expected/risk.json`.
- **Dependencies:** RISK-001..RISK-003, RISK-006 (dampening input), IMP-009 (clusters).
- **Implementation details:**
  - Per subject: `contrib_i = clamp(w_i · c_i · a_i, 0, 0.95)` where `a_i` = amplifier (1.15 for Authorization/Validation/Authentication when `endpoint.reachable`; 1.0 otherwise).
  - Within a category take the max contribution (prevents ten weak path signals from stacking); across categories combine by noisy-OR: `score = 1 − Π_categories (1 − max_contrib_category)`.
  - Level thresholds: `low < 0.25 ≤ medium < 0.50 ≤ high < 0.75 ≤ critical`. Floors: `level = max(threshold_level, max floor_level of signals)`; if a floor raises the level, `score = max(score, floor_min_score)` with `{medium:0.25, high:0.5, critical:0.75}`.
  - Cluster score: noisy-OR over member category maxima (same formula over the union); PR score = max over clusters (not noisy-OR across the whole PR — a big PR is not riskier per se), plus `pr_size_signal` 0.1 when > 50 files changed (reported, small).
  - Low-risk dampening (RISK-006): symbols classified low-risk get `c_i × 0.3` for path signals only; change/graph signals are never dampened.
  - Output: `RiskAssessment { score, level, signals (sorted by contribution desc), per_symbol: BTreeMap<SymbolKey, (f32, RiskLevel)>, per_cluster: BTreeMap<ClusterId, (f32, RiskLevel)>, effects /*RISK-005*/, rules_version, input_hash }`.
  - Worked §151 example (confidences 1.0): Authorization max = `authz.call_removed` 0.85 × 1.15 = 0.9775 → clamp 0.95; Authentication = `path.configured` (`src/auth/**: critical`, category inferred Authentication) 0.8 with floor critical; PublicApiContracts = `endpoint.reachable` 0.3. score = 1 − (1 − 0.95)(1 − 0.8)(1 − 0.3) = 1 − 0.05 · 0.2 · 0.7 = 0.993 → level critical (floor also critical).
- **Data model changes:** None.
- **API/protocol changes:** `RiskAssessment` in contracts; review detail API shows it (API-010, WEB risk tab).
- **Concurrency semantics:** Pure.
- **Failure behavior:** no signals → score 0.0, level low (explicitly, not "unknown").
- **Idempotency considerations:** Deterministic; floats rounded to 4 decimals in serialization.
- **Security considerations:** None.
- **Observability additions:** span `risk_classification` (attrs `pr_score`, `pr_level`, `signals`); histogram `risk_score`; counter `risk_level_total{level}`.
- **Tests required:** `auth_bypass_scores_critical_0_993`, `same_category_signals_do_not_stack`, `cross_category_noisy_or`, `floor_raises_level_and_min_score`, `pr_score_is_max_of_clusters`, `large_pr_size_signal_small`, `no_signals_low_zero`, `dampening_never_applies_to_change_signals`, `proptest_score_monotone_in_signal_weight`, `golden_risk_all_fixtures`.
- **Benchmarks if applicable:** None (microseconds).
- **Acceptance criteria:** tests green; `expected/risk.json` for auth-bypass has `level: critical`, `score ≥ 0.95`, top signal `authz.call_removed`.
- **Definition of done:** global DoD; formula documented in `docs/reviewers/risk-rules.md`.

---

### RISK-005 — Risk effects
Status: ☐

- **Task ID:** RISK-005
- **Title:** Map risk level and categories to effects: reviewers, depth, context budget, model tier, verification depth, confidence threshold, test inspection
- **Problem:** Risk is only useful if it changes what the system does (PRD §38).
- **Why it exists:** PRD §38; target-arch §3.7 `effects: { reviewers, depth, context_budget, model_tier, verification_depth }`.
- **Scope:** pure `effects(assessment, config) -> RiskEffects` per cluster and per PR; defaults table; config overrides.
- **Explicit non-scope:** actually routing reviewers (REV-002), running verification (VER), enforcing budgets (PIPE-006/CTX-006).
- **Files/modules expected to change:** `impact/src/risk/mod.rs`.
- **New files/modules expected:** `impact/src/risk/effects.rs`, `tests/effects.rs`.
- **Dependencies:** RISK-004, POL-001 (`review.reviewers`, `confidence.minimum_publish`), ADR-010 (tiers).
- **Implementation details:**
  ```rust
  pub struct RiskEffects { pub reviewers: BTreeSet<ReviewerKind>, pub depth: ReviewDepth /*Light|Standard|Deep*/,
      pub context_budget_multiplier: f32, pub impact_budget_multiplier: f32, pub model_tier: ModelTier,
      pub verification_depth: VerificationDepth /*Structural|Standard|Full*/, pub confidence_threshold_delta: f32,
      pub test_inspection: bool, pub focus_profiles: BTreeSet<FocusProfile /*DatabaseSafety|Authz|...*/> }
  ```
  | Level | reviewers (if enabled in config) | depth | ctx × | impact × | tier | verification | threshold Δ | tests |
  |---|---|---|---|---|---|---|---|---|
  | low | correctness (only if non-cosmetic) | Light | 0.5 | 0.5 | FAST_REASONER | Structural (stages 1–4) | +0.05 | no |
  | medium | correctness | Standard | 1.0 | 1.0 | REVIEW_REASONER | Standard (1–7, contradiction deterministic only) | 0 | if untested |
  | high | correctness + category reviewers | Deep | 1.25 | 1.5 | REVIEW_REASONER | Full (1–8 incl. VERIFIER) | 0 | yes |
  | critical | correctness + security + category reviewers | Deep | 1.5 | 2.0 | REVIEW_REASONER (+ DEEP_REASONER if `deep_reasoner.enabled` and budget) | Full | −0.03 (min floor 0.55 never crossed) | yes |
  - Category reviewers: Authorization/Authentication/Cryptography/Permissions/Secrets/Validation/Payments → `security`; DatabaseMigrations/SchemaChanges/TransactionBoundaries → correctness with `FocusProfile::DatabaseSafety` (gap-analysis §48 decision); Concurrency/Queues/BackgroundJobs → correctness + `performance` (if enabled); PublicApiContracts → `architecture` (if enabled); untested changed symbols → `tests` reviewer (if enabled).
  - Threshold Δ: lower only for critical, never below the 0.55 suppression floor (gap-analysis P); raised for low to reduce noise.
  - Intent is not an input (CHG-009 invariant).
- **Data model changes:** None.
- **API/protocol changes:** `RiskEffects` in contracts.
- **Concurrency semantics:** Pure.
- **Failure behavior:** reviewer disabled in config → removed from set and recorded in `effects.disabled_by_config` (summary shows "security reviewer disabled by configuration").
- **Idempotency considerations:** Deterministic.
- **Security considerations:** A config cannot disable verification stages for critical clusters (verification depth floor Full for Authorization/Secrets signals regardless of config).
- **Observability additions:** span attrs `effects.reviewers`, `effects.tier`, `effects.verification_depth`.
- **Tests required:** `auth_bypass_effects_security_deep_full_verification`, `low_risk_light_structural`, `db_signals_enable_database_safety_focus`, `disabled_reviewer_recorded`, `threshold_never_below_055`, `authz_forces_full_verification_even_if_config_lowers`, `intent_not_an_input_compile_check` (doc-test that `effects` signature has no intent).
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** table-driven test covers every row and category mapping.
- **Definition of done:** global DoD; table mirrored in `docs/reviewers/risk-rules.md`.

---

### RISK-006 — Low-risk change detection with contract-change override
Status: ☐

- **Task ID:** RISK-006
- **Title:** Detect formatting/comment-only, pure rename, generated, import-sort and simple-constant changes, overridden when contracts or behaviour change
- **Problem:** Reviewing formatting churn wastes budget and produces noise; but "nominally simple" changes (renaming an exported function, changing a constant used in an auth check) can break contracts (PRD §39).
- **Why it exists:** PRD §39.
- **Scope:** per-symbol and per-file `LowRiskKind` detection, override rules, outputs consumed by RISK-004 (dampening), IMP-009 (LowRisk pseudo-cluster) and IMP-010 (skip with disclosure).
- **Explicit non-scope:** suppressing findings (VER-011); generated detection rules (INIT-008/DIFF-004).
- **Files/modules expected to change:** `impact/src/risk/mod.rs`.
- **New files/modules expected:** `impact/src/risk/low_risk.rs`, `tests/low_risk.rs`, fixtures under `fixtures/pull-requests/{format-only,pure-rename}` (from CHG-008) plus `import-sort/`, `exported-rename/`, `constant-in-auth/`, `snapshot-update/`.
- **Dependencies:** CHG-001 (cosmetic flag), CHG-005 (api contract), DIFF-004, IMP-002 (callers outside PR), RISK-001.
- **Implementation details:**
  - `LowRiskKind { FormattingOrComments, PureRename, Generated, ImportSort, SimpleConstant, SnapshotUpdate }` detection:
    - FormattingOrComments: symbol `Modified{false,false,false}` (hashes equal) or file hunks all `whitespace_only`/comment-only (normalized token stream equal, from body_hash at file level).
    - PureRename: `Renamed{similarity ≥ 0.95, body_changed:false}` or file rename with similarity 1.0.
    - Generated: file class Generated/Vendored.
    - ImportSort: only unattributed import lines changed and the multiset of import specifiers+bindings is equal.
    - SimpleConstant: a `Constant`/`Variable` initializer literal changed with no other change.
    - SnapshotUpdate: `__snapshots__/**/*.snap` or `*.snap` only.
  - **Overrides** (any → not low-risk, recorded as `override_reason`):
    1. symbol exported/public (`exported` flag or package entrypoint) **and** PureRename with callers outside the PR (impact `Caller` not in changed set) or any `api_contract_changed`;
    2. any CHG class other than none on the symbol;
    3. SimpleConstant whose readers (reverse REFERENCES/READS) include a symbol with Authorization/Validation/Cryptography/Payments signals, or a config/env default;
    4. file matches a `risk.paths` rule at `high|critical` (path floor still applies, low-risk only dampens path weight per RISK-004);
    5. ImportSort that changes side-effect import order (`import './polyfill'` without bindings moved).
  - Output `LowRiskReport { per_symbol: BTreeMap<SymbolKey, LowRiskVerdict>, per_file: BTreeMap<RepoPath, LowRiskVerdict> }`, `LowRiskVerdict { kind: Option<LowRiskKind>, overridden_by: Option<OverrideReason> }`.
- **Data model changes:** None.
- **API/protocol changes:** `LowRiskReport` in contracts (summary disclosure).
- **Concurrency semantics:** Pure.
- **Failure behavior:** degraded parse → never low-risk (conservative).
- **Idempotency considerations:** Deterministic.
- **Security considerations:** Conservative by construction: uncertainty resolves to "not low-risk".
- **Observability additions:** counters `low_risk_symbols_total{kind}`, `low_risk_overrides_total{reason}`.
- **Tests required:** `format_only_all_low_risk`, `comment_only_low_risk`, `auth_bypass_format_ts_comment_low_risk_authorize_not`, `pure_rename_internal_low_risk`, `exported_rename_with_external_callers_overridden`, `import_sort_low_risk`, `side_effect_import_reorder_overridden`, `constant_used_in_auth_overridden`, `snapshot_update_low_risk`, `degraded_parse_never_low_risk`, `critical_path_file_low_risk_still_floored`.
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** fixtures classify as expected; no override case is reported low-risk.
- **Definition of done:** global DoD; PRD §39 mapping documented in `docs/reviewers/risk-rules.md`.

---

---

### SEM-001 — `EmbeddingProvider` trait + `EmbeddingSpace` identity
Status: ☐

- **Task ID:** SEM-001
- **Title:** `EmbeddingProvider` port and `EmbeddingSpace` identity (provider, model, dims, version)
- **Problem:** Vectors from different models are incomparable; mixing them silently corrupts retrieval. Provider code must be swappable and testable offline.
- **Why it exists:** ADR-008 ("vectors from different spaces are never mixed"), target-arch §3.9, ADR-015 (model-derived artifacts record `embedding_space`).
- **Scope:** trait, space identity type with canonical string, input/output types, batching contract, error taxonomy, token/cost accounting hooks.
- **Explicit non-scope:** concrete providers (SEM-002), Qdrant (SEM-003).
- **Files/modules expected to change:** `engine/crates/semantic/Cargo.toml`, `semantic/src/lib.rs`.
- **New files/modules expected:** `semantic/src/embedding/mod.rs`, `embedding/space.rs`, `embedding/error.rs`, `tests/space.rs`.
- **Dependencies:** DOM-001, DOM-003 (version types), OBS-002 (metric instruments).
- **Implementation details:**
  ```rust
  pub struct EmbeddingSpace { pub provider: ProviderName /*openai|voyage|hash*/, pub model: String, pub dims: u16, pub version: u16 /*our re-embed epoch*/ }
  impl EmbeddingSpace { pub fn id(&self) -> String /* "{provider}-{model}-{dims}" sanitized [a-z0-9_-] */;
                        pub fn collection_name(&self) -> String /* "rg_{provider}_{model}_{dims}_v{version}" (ADR-008) */; }
  pub enum InputKind { Document, Query }          // asymmetric models (voyage input_type, e5 prefixes)
  pub struct EmbedRequest<'a> { pub kind: InputKind, pub texts: &'a [String], pub trace: TraceContext }
  pub struct EmbedResponse { pub vectors: Vec<Vec<f32>> /*L2-normalized*/, pub usage_tokens: u32, pub latency_ms: u32 }
  #[async_trait] pub trait EmbeddingProvider: Send + Sync {
      fn space(&self) -> &EmbeddingSpace;
      fn max_batch(&self) -> usize; fn max_input_tokens(&self) -> usize;
      async fn embed(&self, req: EmbedRequest<'_>) -> Result<EmbedResponse, EmbedError>;
  }
  pub enum EmbedError { Transient(String), RateLimited { retry_after_ms: u64 }, Permanent(String), InputTooLong { index: usize }, DimensionMismatch { expected: u16, got: usize } }
  ```
  - Collection naming reconciles ADR-008 (`rg_{provider}_{model}_{dims}_v{n}`) and target-arch §3.9 (`rg_{space}_v{n}`): the space id rendered with `_` separators is the ADR form; this task adopts it and target-arch §3.9 wording is updated to match.
  - All vectors are L2-normalized by the trait wrapper `Normalized<P>`; Qdrant uses `Cosine` distance (normalization makes dot equivalent).
  - `DimensionMismatch` is checked on every response.
- **Data model changes:** None.
- **API/protocol changes:** `semantic::embedding::*` public API.
- **Concurrency semantics:** Providers are `Send + Sync`, shared via `Arc<dyn EmbeddingProvider>`; concurrency limiting lives in SEM-002 adapters (semaphore).
- **Failure behavior:** typed errors; retry policy implemented once in `RetryingProvider<P>` wrapper: Transient/RateLimited retried 3× with jittered exponential backoff (base 250 ms, max 8 s, honours `retry_after`); Permanent/InputTooLong not retried.
- **Idempotency considerations:** Embedding is a pure function of `(space, kind, text)` for deterministic providers; for remote providers results may differ in low bits — irrelevant because points are keyed by content hash, not by vector.
- **Security considerations:** Text passes through `telemetry::redact` (secret patterns) in the wrapper **before** any provider call; texts never logged.
- **Observability additions:** span `embedding_request` (attrs `provider`, `model`, `batch_size`, `input_kind`); histograms `embedding_latency_ms{provider}`, counters `embedding_tokens_total{provider}`, `embedding_errors_total{provider,kind}`.
- **Tests required:** `collection_name_format`, `space_id_sanitizes_model_names` (`text-embedding-3-small` → `text_embedding_3_small`), `normalized_wrapper_unit_length`, `dimension_mismatch_detected`, `retry_on_transient_then_success`, `no_retry_on_permanent`, `redaction_applied_before_provider` (fake provider records inputs).
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** tests green; target-arch §3.9 naming updated in the same change.
- **Definition of done:** global DoD.

---

### SEM-002 — Providers: openai, voyage, hash
Status: ☐

- **Task ID:** SEM-002
- **Title:** Embedding adapters: OpenAI, Voyage, and a deterministic offline `hash` provider (feature hashing)
- **Problem:** Production needs a real embedding model; CI and the offline dev environment (no API keys, gap-analysis Q) need a deterministic provider with meaningful-enough lexical similarity.
- **Why it exists:** ADR-008; master plan risk R8 (no keys in dev).
- **Scope:** three adapters behind `EmbeddingProvider`, config selection, HTTP via `reqwest` (rustls), concurrency semaphore, batching.
- **Explicit non-scope:** self-hosted models (post-MVP), routing via ModelGateway (embeddings are not chat models; ADR-009's gateway is not used, but the same redaction and accounting conventions apply).
- **Files/modules expected to change:** `semantic/Cargo.toml` (`reqwest` with `rustls-tls`, `json`), `semantic/src/lib.rs`.
- **New files/modules expected:** `semantic/src/embedding/{openai,voyage,hash}.rs`, `embedding/config.rs`, `tests/providers_http.rs` (wiremock), `tests/hash_provider.rs`.
- **Dependencies:** SEM-001.
- **Implementation details:**
  - **openai:** `POST https://api.openai.com/v1/embeddings` `{model, input:[...], dimensions?, encoding_format:"float"}`; default model `text-embedding-3-small`, dims 1536 (or configured `dimensions` 512/1024); `max_batch=256`, `max_input_tokens=8191`; `InputKind` ignored. API key from env `OPENAI_API_KEY` or secret manager reference, never from repo config.
  - **voyage:** `POST https://api.voyageai.com/v1/embeddings` `{model, input, input_type: "document"|"query", output_dimension?}`; default `voyage-code-3`, dims 1024; `max_batch=128`; key `VOYAGE_API_KEY`.
  - **hash:** dims 768 (configurable 256..4096). Tokenize: split identifiers on non-alphanumerics, then camelCase/snake_case boundaries, lowercase; features = unigrams + bigrams of sub-tokens + character trigrams of each identifier (weight 0.5). Each feature `f` → index `xxh3_64(f, seed=0x5245_5649_4557) % dims`, sign from bit 63; value += sign × weight × (1 + ln(tf)); L2-normalize. Pure, no I/O, deterministic across platforms (no float reductions dependent on thread order). Space `hash-fh768-768-v1`.
  - Timeouts: connect 5 s, request 30 s. Concurrency: `Semaphore` permits default 4 per provider instance (config `semantic.embedding.concurrency`).
  - Token estimate for batching = `ceil(chars/4)`; texts above `max_input_tokens` are truncated by the unit builders (SEM-006), never here (here → `InputTooLong`).
  - Provider selection: `semantic.embedding.provider: openai|voyage|hash` (default `hash` in dev/test, `voyage` recommended prod); privacy `no_external` forces `hash` (fail-safe, logged).
- **Data model changes:** None.
- **API/protocol changes:** config keys `semantic.embedding.{provider,model,dims,concurrency}` (POL-001 schema / deployment env).
- **Concurrency semantics:** semaphore-bounded parallel batches; responses re-ordered by `index` field from the API before return.
- **Failure behavior:** HTTP 429 → `RateLimited{retry_after}`; 5xx/timeouts → `Transient`; 4xx → `Permanent`; missing key at startup → provider construction fails with a clear error (worker refuses to start semantic sync; reviews proceed without semantic candidates, CTX-008 degrades).
- **Idempotency considerations:** hash provider bit-exact; remote providers idempotent at the point level (SEM-007).
- **Security considerations:** API keys only in memory, sent only to the fixed provider host (no configurable base URL in prod builds except via env for testing, guarded by `cfg(any(test, feature="test-endpoints"))`); `Authorization` header redacted in logs; inputs redacted (SEM-001).
- **Observability additions:** reuse SEM-001 metrics; counter `embedding_batches_total{provider}`.
- **Tests required:** `openai_request_shape_and_order_by_index` (wiremock), `voyage_input_type_query_vs_document`, `http_429_maps_to_rate_limited_with_retry_after`, `http_500_transient`, `http_400_permanent`, `hash_deterministic_bit_exact` (golden vector hash), `hash_similarity_camel_vs_snake` (`authorizeUser` vs `authorize_user` cosine > 0.8), `hash_unrelated_low_similarity` (< 0.2), `privacy_no_external_forces_hash`, `missing_key_fails_construction`.
- **Benchmarks if applicable:** hash provider: 10k symbol texts < 200 ms single thread.
- **Acceptance criteria:** wiremock tests green; hash provider golden stable across x86_64/aarch64 CI (if both runners exist; otherwise x86_64).
- **Definition of done:** global DoD; `docs/operations/local-development.md` mentions `hash` as default.

---

### SEM-003 — Qdrant REST client via reqwest
Status: ☐

- **Task ID:** SEM-003
- **Title:** Minimal typed Qdrant REST client: collections, payload indexes, upsert, delete-by-filter, search, scroll
- **Problem:** We need a small, auditable client whose filter construction we control (for SEM-005), without pulling the gRPC stack.
- **Why it exists:** ADR-008 transport decision (REST via `reqwest`).
- **Scope:** endpoints `GET/PUT /collections/{c}`, `PUT /collections/{c}/index`, `PUT /collections/{c}/points?wait=true`, `POST /collections/{c}/points/delete`, `POST /collections/{c}/points/search` (or `/points/query`), `POST /collections/{c}/points/scroll`, `POST /collections/{c}/points/payload`, `GET /readyz`; typed filter AST.
- **Explicit non-scope:** tenant enforcement (SEM-005 wraps this client; the raw client is `pub(crate)`), snapshots/backups (ops runbook).
- **Files/modules expected to change:** `semantic/src/lib.rs`.
- **New files/modules expected:** `semantic/src/qdrant/{mod.rs,client.rs,filter.rs,types.rs,error.rs}`, `tests/qdrant_http.rs` (wiremock), `tests/qdrant_live.rs` (`#[ignore]` unless `QDRANT_URL` set; run in CI integration job with compose Qdrant).
- **Dependencies:** SEM-001, FND-005 (compose Qdrant service).
- **Implementation details:**
  ```rust
  pub(crate) struct QdrantClient { http: reqwest::Client, base: Url, api_key: Option<SecretString> }
  pub enum Cond { Match { key: &'static str, value: FieldValue }, MatchAny { key: &'static str, values: Vec<FieldValue> },
                  Range { key: &'static str, gte: Option<f64>, lte: Option<f64> } }
  pub struct Filter { pub must: Vec<Cond>, pub should: Vec<Cond>, pub must_not: Vec<Cond> }   // serialized to Qdrant JSON
  pub struct PointUpsert { pub id: Uuid, pub vector: Vec<f32>, pub payload: Payload }
  pub struct ScoredPoint { pub id: Uuid, pub score: f32, pub payload: Payload }
  impl QdrantClient {
    pub async fn ensure_collection(&self, name: &str, dims: u16, hnsw: HnswCfg) -> Result<CollectionState, QdrantError>;
    pub async fn ensure_payload_index(&self, c: &str, field: &str, schema: IndexSchema /*keyword|integer|uuid*/, is_tenant: bool) -> Result<(), QdrantError>;
    pub async fn upsert(&self, c: &str, pts: &[PointUpsert]) -> Result<(), QdrantError>;           // wait=true, batches of 256
    pub async fn delete_by_filter(&self, c: &str, f: &Filter) -> Result<(), QdrantError>;
    pub async fn search(&self, c: &str, v: &[f32], f: &Filter, limit: u32, score_threshold: Option<f32>, hnsw_ef: Option<u32>) -> Result<Vec<ScoredPoint>, QdrantError>;
    pub async fn scroll_ids(&self, c: &str, f: &Filter, with_payload: &[&str], page: u32) -> Result<ScrollPage, QdrantError>;
    pub async fn set_payload(&self, c: &str, ids: &[Uuid], payload: Payload) -> Result<(), QdrantError>;
  }
  ```
  - Filter keys are `&'static str` from a `fields` module (no free-form keys) — reduces typo risk and makes SEM-005's audit simple.
  - HNSW defaults: `m=16, ef_construct=128`, `on_disk_payload=true`; `organization_id` index created with `is_tenant: true` (Qdrant tenant-index optimization) and `repository_id` keyword index.
  - Payload size cap 8 KiB per point (summary text is not stored in payload; only metadata; text lives in PG).
- **Data model changes:** None in PG.
- **API/protocol changes:** None external.
- **Concurrency semantics:** `reqwest::Client` shared (connection pool); upserts with `wait=true` make read-after-write consistent for the sync job.
- **Failure behavior:** `QdrantError::{Unavailable, Timeout, BadRequest{body}, NotFound, Conflict}`; Unavailable/Timeout retried 3× (backoff 200 ms → 3 s); search timeout default 2 s (search failure must never fail a review; CTX-008 degrades).
- **Idempotency considerations:** upsert by deterministic UUID is idempotent; delete-by-filter idempotent.
- **Security considerations:** optional API key header (`api-key`) from env, redacted in logs; TLS when URL is https; client never exposes raw filter-free search publicly (`pub(crate)`).
- **Observability additions:** span `qdrant_request` (attrs `op`, `collection`, `points`, `status`); histogram `qdrant_latency_ms{op}`; counter `qdrant_errors_total{op,kind}`.
- **Tests required:** `filter_serialization_golden`, `upsert_batches_of_256`, `search_request_shape`, `delete_by_filter_shape`, `error_mapping_503_unavailable`, `retry_then_success`, `live_roundtrip_upsert_search_delete` (ignored unless Qdrant available).
- **Benchmarks if applicable:** in SEM-009.
- **Acceptance criteria:** wiremock tests green; live test green in CI integration job.
- **Definition of done:** global DoD.

---

### SEM-004 — Collection bootstrap and versioning
Status: ☐

- **Task ID:** SEM-004
- **Title:** Collection bootstrap (`rg_{space}_v{n}`), payload indexes, version registry and cut-over
- **Problem:** Collections must exist with the right dims and indexes before use, and a model change needs a parallel collection plus a clean cut-over without mixing spaces.
- **Why it exists:** ADR-008 consequences ("new collection and background re-embed, followed by a cut-over").
- **Scope:** idempotent bootstrap at worker start, PG registry of collections and their state, cut-over and retirement commands.
- **Explicit non-scope:** the re-embed job itself (SEM-007 does sync; a full re-embed is SEM-007 over all units).
- **Files/modules expected to change:** `semantic/src/lib.rs`, `engine/apps/review-worker/src/main.rs` (call bootstrap), `engine/apps/review-cli` (subcommand `review semantic collections`).
- **New files/modules expected:** `semantic/src/collections.rs`, `engine/migrations/NNNN_semantic_collections.sql`, `tests/collections.rs`.
- **Dependencies:** SEM-001, SEM-003, DOM-009 (migrations).
- **Implementation details:**
  - Migration:
    ```sql
    CREATE TABLE semantic_collections (
      name text PRIMARY KEY, space_id text NOT NULL, provider text NOT NULL, model text NOT NULL, dims int NOT NULL,
      version int NOT NULL, state text NOT NULL CHECK (state IN ('building','active','retiring','retired')),
      created_at timestamptz NOT NULL DEFAULT now(), activated_at timestamptz, UNIQUE (space_id, version));
    CREATE UNIQUE INDEX one_active_collection ON semantic_collections ((true)) WHERE state = 'active';
    ```
    (global, not tenant-scoped: collections are shared and tenant isolation is by payload filter; RLS not applicable — table holds no tenant data.)
  - Bootstrap: for the configured space, `ensure_collection` (dims check: existing collection with different dims → hard error `SpaceMismatch`), ensure payload indexes: `organization_id (keyword, tenant)`, `repository_id`, `kind`, `language`, `module`, `symbol_key`, `chunk_key`, `file_path`, `content_hash`, `snapshot_ids` (keyword, array), `embedding_version` (integer). Insert registry row `building` if absent.
  - Activation: `review semantic activate <name>` (or automatic when SEM-007 reports full coverage ≥ 99% of active units) flips `building → active` and previous `active → retiring` in one transaction. Searches always target the `active` collection; writers write to `active` **and** `building` during migration (dual-write).
  - Retirement: `retiring` collection deleted after 7 days (`review semantic retire --older-than 7d`).
- **Data model changes:** new table `semantic_collections` (above).
- **API/protocol changes:** CLI subcommands `review semantic collections|activate|retire`.
- **Concurrency semantics:** Bootstrap guarded by PG advisory lock `pg_advisory_lock(hashtext('semantic_bootstrap'))` so concurrent workers do not race index creation; activation is one transaction protected by the partial unique index.
- **Failure behavior:** Qdrant unavailable at startup → worker starts with semantic disabled (`semantic_available=false` gauge), retries bootstrap every 60 s; reviews continue (structural-only context).
- **Idempotency considerations:** Bootstrap is repeatable; index creation tolerates "already exists".
- **Security considerations:** No tenant data in registry.
- **Observability additions:** gauge `semantic_available`; span `semantic_bootstrap`; counter `semantic_collection_transitions_total{to}`.
- **Tests required:** `bootstrap_creates_collection_and_indexes`, `bootstrap_idempotent`, `dims_mismatch_is_hard_error`, `activation_flips_previous_to_retiring`, `only_one_active_enforced_by_index`, `dual_write_during_building`, `qdrant_down_starts_disabled_and_recovers`.
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** integration tests against compose Qdrant + PG green.
- **Definition of done:** global DoD; `docs/operations/semantic-reembed.md` runbook written.

---

### SEM-005 — TenantScope-enforced search API
Status: ☐

- **Task ID:** SEM-005
- **Title:** Tenant-scoped search and write API; a test proves no query can omit the tenant filter
- **Problem:** A missing `organization_id` filter in a vector search is a cross-tenant data leak (risk R9, Critical).
- **Why it exists:** ADR-008 tenant isolation; master plan §13.1; production readiness item "Qdrant isolation/filtering".
- **Scope:** `TenantScope` type, `SemanticIndex` public API (the *only* public way to search/write), filter rendering that always prepends tenant conditions, static and dynamic tests.
- **Explicit non-scope:** PG RLS (SEC-001); org-level authorization of the caller (API layer).
- **Files/modules expected to change:** `semantic/src/lib.rs` (re-exports only `SemanticIndex`, not `QdrantClient`).
- **New files/modules expected:** `semantic/src/index.rs`, `semantic/src/tenant.rs`, `tests/tenant_isolation.rs`, `tests/no_raw_client_exports.rs`.
- **Dependencies:** SEM-003, SEM-004, DOM-001 (`OrganizationId`, `RepositoryId`).
- **Implementation details:**
  ```rust
  pub struct TenantScope { organization_id: OrganizationId, repository_ids: NonEmptyVec<RepositoryId> }   // fields private
  impl TenantScope { pub fn new(org: OrganizationId, repos: NonEmptyVec<RepositoryId>) -> Self; }
  pub struct SemanticQuery { pub vector: QueryVector /*Text(String)|Vector(Vec<f32>)*/, pub kinds: Vec<UnitKind>, pub languages: Vec<Language>,
                             pub snapshot_id: Option<SnapshotId>, pub exclude_symbol_keys: Vec<SymbolKey>, pub limit: u32 /*≤ 100*/, pub min_score: f32 }
  pub struct SemanticIndex { client: QdrantClient, provider: Arc<dyn EmbeddingProvider>, collections: CollectionResolver }
  impl SemanticIndex {
    pub async fn search(&self, scope: &TenantScope, q: &SemanticQuery) -> Result<Vec<SemanticHit>, SemanticError>;
    pub async fn upsert_units(&self, scope: &TenantScope, units: &[EmbeddingUnit]) -> Result<UpsertReport, SemanticError>;
    pub async fn delete_units(&self, scope: &TenantScope, sel: DeleteSelector) -> Result<(), SemanticError>;
  }
  ```
  - Filter rendering: `fn scoped(scope, extra: Filter) -> Filter` always produces `must = [Match(organization_id), MatchAny(repository_id, scope.repos), ...extra.must]`; `extra` can only *add* conditions (type `ExtraFilter` has no way to express `should` over tenant keys and rejects tenant keys at construction).
  - Writes: every point's payload `organization_id`/`repository_id` is set **from the scope**, overriding unit data; a unit whose `repository_id` ∉ scope → `SemanticError::ScopeViolation`.
  - Result post-check: every returned hit's payload org/repo is re-verified against the scope; mismatch → drop + `qdrant_scope_violation_total` + error log (defence in depth).
  - Static guarantee: `QdrantClient` is `pub(crate)`; `tests/no_raw_client_exports.rs` uses `trybuild` compile-fail tests: (1) constructing `SemanticQuery` search without a scope does not compile, (2) `semantic::qdrant` is not accessible outside the crate. Dynamic guarantee: a recording HTTP mock asserts **every** search/scroll/delete request body sent during the full test-suite contains both tenant conditions (`TenantAuditLayer` wraps the client in tests and panics on violation).
- **Data model changes:** None.
- **API/protocol changes:** public `SemanticIndex` API.
- **Concurrency semantics:** `SemanticIndex` is `Send + Sync`, shared via `Arc`.
- **Failure behavior:** `ScopeViolation` is a bug signal: error-level log, metric, request rejected.
- **Idempotency considerations:** as SEM-003.
- **Security considerations:** This *is* the isolation control; SEC-002 adds cross-tenant end-to-end tests (two orgs, identical code, assert zero cross hits).
- **Observability additions:** counter `qdrant_scope_violation_total` (alert on > 0, OBS-008); span `qdrant_search` (attrs `organization_id`, `repositories`, `kinds`, `limit`, `hits`).
- **Tests required:** `every_request_has_org_and_repo_must_filter` (audit layer over all tests), `extra_filter_cannot_override_tenant`, `write_payload_tenant_from_scope`, `unit_outside_scope_rejected`, `result_postcheck_drops_foreign_hits` (mock returns a foreign point), `compile_fail_search_without_scope` (trybuild), `compile_fail_raw_client_access` (trybuild), `two_org_isolation_live` (ignored unless Qdrant).
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** all tests green; the audit layer is enabled for the whole `semantic` test suite and the CTX integration tests.
- **Definition of done:** global DoD; listed in INV suite as the Qdrant isolation invariant.

---

### SEM-006 — Embedding unit builders
Status: ☐

- **Task ID:** SEM-006
- **Title:** Build embedding units: symbol summary text, code chunk, doc, convention
- **Problem:** What we embed determines retrieval quality; it must be deterministic, size-bounded and secret-free, with stable keys for incremental sync.
- **Why it exists:** ADR-008 kinds (`symbol_summary, code_chunk, doc, convention, finding_history`); target-arch §7 semantic summary cache.
- **Scope:** builders for four kinds (`finding_history` is post-MVP HIST-*), text templates, chunking, keys and content hashes, payload construction.
- **Explicit non-scope:** LLM-generated natural-language summaries (optional later via `symbol_summaries` table from GW; MVP uses a deterministic structural "summary text"); embedding calls (SEM-007).
- **Files/modules expected to change:** `semantic/src/lib.rs`.
- **New files/modules expected:** `semantic/src/units/{mod.rs,symbol.rs,chunk.rs,doc.rs,convention.rs}`, `tests/units.rs`, `tests/snapshots/`.
- **Dependencies:** SEM-001, CG-004 (graph nodes + attrs), SID-004 (body_hash), PROF-003 (conventions), INIT-010 (rule docs discovery).
- **Implementation details:**
  ```rust
  pub enum UnitKind { SymbolSummary, CodeChunk, Doc, Convention }
  pub struct EmbeddingUnit { pub kind: UnitKind, pub key: String /*symbol_key | chunk_key*/, pub repository_id: RepositoryId,
      pub language: Option<Language>, pub module: Option<String>, pub file_path: Option<RepoPath>, pub start_line: Option<u32>, pub end_line: Option<u32>,
      pub text: String, pub content_hash: Hash128 /*blake3(kind‖template_version‖text)*/, pub snapshot_id: SnapshotId }
  pub const UNIT_TEMPLATE_VERSION: u16 = 1;
  ```
  - **SymbolSummary** (functions, methods, classes, interfaces; skip private trivial getters < 3 lines and generated files): template
    `"{kind} {qualified_name}\nsignature: {signature}\nmodule: {module_path}\ndecorators: {decorators}\ncalls: {top 10 callee names by confidence}\ncalled by: {top 5 caller names}\ndoc: {first 300 chars of doc comment}"` — names and structure only; truncated to 1,500 chars.
  - **CodeChunk**: body of functions/methods > 3 lines, split into chunks of ≤ 60 lines with 10-line overlap on statement boundaries (line granularity from IR ranges); `chunk_key = blake3(symbol_key‖chunk_ordinal)`; text = signature line + chunk lines; cap 6,000 chars.
  - **Doc**: markdown/ADR/README/rule docs (INIT-010 discovery + `.review/` knowledge sources): split on headings (H1–H3), ≤ 2,000 chars per unit, `chunk_key = blake3(path‖heading_path‖ordinal)`.
  - **Convention**: one unit per inferred/declared convention: `"{rule}\nscope: {scope}\nexamples: {2 sample symbol names}\nconfidence: {c}"`, key = convention id.
  - Secrets: every text passes `telemetry::redact`; units from files with a `secrets` signal at index (SEC-003) are skipped entirely for CodeChunk.
  - Deterministic ordering and hashing; `content_hash` excludes `snapshot_id` (so unchanged content across snapshots hashes equal).
- **Data model changes:** None (unit text is reconstructible; optional cache table is not introduced).
- **API/protocol changes:** None external.
- **Concurrency semantics:** Pure, parallel per file.
- **Failure behavior:** missing body (degraded parse) → summary only, no chunks.
- **Idempotency considerations:** Same IR + graph → same units and hashes; `UNIT_TEMPLATE_VERSION` bump changes all hashes (deliberate re-embed).
- **Security considerations:** redaction + secret-file skip; payload metadata only (text not stored in Qdrant).
- **Observability additions:** counter `embedding_units_built_total{kind}`; histogram `embedding_unit_chars{kind}`.
- **Tests required:** `auth_service_authorize_summary_snapshot`, `trivial_getter_skipped`, `generated_file_skipped`, `chunking_60_lines_overlap_10`, `doc_split_on_headings`, `convention_unit_text`, `content_hash_excludes_snapshot`, `secret_literal_redacted_in_text`, `template_version_changes_hash`.
- **Benchmarks if applicable:** reference-api full unit build < 2 s.
- **Acceptance criteria:** snapshots stable; redaction test green.
- **Definition of done:** global DoD.

---

### SEM-007 — Incremental embedding sync
Status: ☐

- **Task ID:** SEM-007
- **Title:** Incremental embedding sync: content-hash check before embedding, lineage re-key, obsolete deletion
- **Problem:** Re-embedding everything per snapshot is slow and expensive; stale points pollute retrieval.
- **Why it exists:** ADR-008 incremental writes; PRD §21 spirit (work proportional to change).
- **Scope:** `sync(scope, snapshot, units_changed, units_removed, lineage) -> SyncReport`; content-hash lookup; batching; lineage re-key without re-embedding; snapshot membership (`snapshot_ids` payload); obsolete-point garbage collection.
- **Explicit non-scope:** deciding *which* units changed (SEM-008 from INC-008 invalidations); scheduling (PIPE job `semantic-sync` via the `incremental-index` queue follow-up stage).
- **Files/modules expected to change:** `semantic/src/lib.rs`, `pipeline/src/stages/index.rs` (call sync after delta snapshot commit).
- **New files/modules expected:** `semantic/src/sync.rs`, `semantic/src/point_id.rs`, `tests/sync.rs`.
- **Dependencies:** SEM-005, SEM-006, SID-005 (lineage), INC-009 (snapshot ids).
- **Implementation details:**
  - `point_id = uuid_v5(NAMESPACE_RG, "{org}|{repo}|{kind}|{key}|{space_id}")`, `NAMESPACE_RG` a fixed UUID constant in `point_id.rs`.
  - Algorithm per batch of ≤ 256 units:
    1. Fetch existing points by ids (`scroll` with `has_id` filter + tenant scope) → map `id → (content_hash, snapshot_ids)`.
    2. Same `content_hash` → **no embed**; `set_payload` to add the snapshot id to `snapshot_ids` (cap: keep last 50 snapshot ids; older dropped; default-branch head always kept).
    3. Different/missing → embed (provider batch) → upsert with full payload.
    4. Lineage: for `Renamed{from,to}` with unchanged `body_hash`, read old point vector (`with_vector=true`), upsert under the new id with new payload (`symbol_key`, `file_path`), delete old id — zero embedding calls.
  - Obsolete deletion (GC job, nightly + after default-branch delta): delete points whose `snapshot_ids` contain none of {default-branch head snapshot, snapshots of open PRs (from PG `review_runs` with state not terminal)} — implemented as: set-difference computed in PG of live snapshot ids, then `delete_by_filter(must_not: MatchAny(snapshot_ids, live))` scoped per repository.
  - Budget: per-sync embedding token cap (default 2M tokens, config) → remaining units deferred to next run, reported.
  - `SyncReport { embedded, skipped_unchanged, rekeyed, deleted, deferred, tokens }`.
- **Data model changes:** None (Qdrant only; live snapshot set derived from existing PG tables).
- **API/protocol changes:** None.
- **Concurrency semantics:** One sync per `(repository, collection)` at a time via PG advisory lock `hashtext('semsync:'||repo_id)`; batches embedded concurrently up to provider semaphore; upserts idempotent so a crashed sync is safely re-run.
- **Failure behavior:** provider/Qdrant failure mid-sync → completed batches stay; job fails retryable; next run skips completed units by hash. Sync failure never blocks review (semantic is supplementary).
- **Idempotency considerations:** Deterministic point ids + content-hash check ⇒ re-running sync performs zero embeddings (asserted).
- **Security considerations:** scope-enforced writes (SEM-005).
- **Observability additions:** span `semantic_sync` (attrs from `SyncReport`); counters `embedding_points_upserted_total`, `embedding_skipped_unchanged_total`, `embedding_rekeyed_total`, `embedding_points_deleted_total`, `embedding_deferred_total`.
- **Tests required:** `second_sync_embeds_nothing`, `changed_body_reembedded`, `rename_unchanged_body_rekeyed_without_embedding` (fake provider call counter = 0), `snapshot_ids_appended_not_reembedded`, `gc_deletes_points_not_in_live_snapshots`, `gc_keeps_default_branch_head`, `token_budget_defers_and_reports`, `crash_midway_resume_idempotent`, `concurrent_sync_serialized_by_lock`.
- **Benchmarks if applicable:** 1k changed units with hash provider + compose Qdrant < 3 s.
- **Acceptance criteria:** tests green; idempotency asserted.
- **Definition of done:** global DoD.

---

### SEM-008 — Integration with INC-008 invalidations
Status: ☐

- **Task ID:** SEM-008
- **Title:** Drive embedding sync from incremental invalidation sets
- **Problem:** Summary units include caller/callee names, so a change in one symbol can stale its neighbours' summary text; sync must know exactly which units to rebuild without scanning the repository.
- **Why it exists:** target-arch §3.5 step 6 ("invalidation set … → cache keys + embeddings").
- **Scope:** mapping `InvalidationSet` → unit rebuild set and removal set; trigger after delta snapshot commit; full rebuild path on space/template version change.
- **Explicit non-scope:** computing invalidations (INC-008).
- **Files/modules expected to change:** `pipeline/src/stages/index.rs`.
- **New files/modules expected:** `semantic/src/invalidation.rs`, `pipeline/tests/semantic_invalidation.rs`.
- **Dependencies:** SEM-007, INC-008.
- **Implementation details:**
  - Mapping: changed/added symbols → SymbolSummary + CodeChunk units; 1-hop dependents (edge kinds `CALLS`, `IMPLEMENTS`, `EXTENDS`) → SymbolSummary only (their "calls/called by" lines may change; content hash decides if re-embed is needed); removed symbols → delete units (by `symbol_key` filter, all kinds, scoped); renamed → lineage path of SEM-007; changed docs/rule files → Doc units for those paths; profile version change → all Convention units.
  - Triggers: PIPE `incremental-index` job emits a follow-up `semantic-sync` step with `{repository_id, snapshot_id, invalidation_ref}` (ids only, ADR-012). Full re-sync when `UNIT_TEMPLATE_VERSION` or space changes (detected by comparing registry row vs runtime).
  - PR head snapshots: sync only changed/added units of the PR (so PR-local code is searchable for that PR's review); they carry the PR snapshot id and are GC'd when the PR closes.
- **Data model changes:** None.
- **API/protocol changes:** job payload `semantic-sync` added to `packages/contracts` (IDs only).
- **Concurrency semantics:** as SEM-007; the review pipeline does **not** wait for semantic sync (CTX-008 tolerates missing PR units by falling back to base-branch points).
- **Failure behavior:** sync job failure is retried by the queue (max 5); dead job alerts but does not affect reviews.
- **Idempotency considerations:** job idempotency key `semsync:{repo}:{snapshot}:{space}`.
- **Security considerations:** none beyond SEM-005.
- **Observability additions:** counter `semantic_invalidated_units_total{kind}`; span link from `incremental_graph_update` to `semantic_sync`.
- **Tests required:** `one_file_change_resyncs_only_its_units_and_dependents`, `dependent_summary_unchanged_hash_not_reembedded`, `removed_symbol_points_deleted`, `template_version_bump_triggers_full_resync`, `pr_snapshot_units_tagged_and_gced_on_close`, `job_payload_ids_only`.
- **Benchmarks if applicable:** one-file change on reference-api → ≤ 30 units embedded, sync < 1 s with hash provider.
- **Acceptance criteria:** counters prove proportional work (asserted in tests).
- **Definition of done:** global DoD.

---

### SEM-009 — Retrieval quality and latency benchmark
Status: ☐

- **Task ID:** SEM-009
- **Title:** Filtered recall and p95 latency benchmark for Qdrant retrieval
- **Problem:** ADR-008 chose one collection with payload filters; this must be validated (filtered recall can degrade for selective filters), and latency must meet the alert threshold (Qdrant p95 > 500 ms).
- **Why it exists:** ADR-008 ("revisit if filtered recall degrades for one kind"); master plan §12.
- **Scope:** benchmark harness: synthetic multi-tenant corpus, exact (brute-force) ground truth, recall@k under filters, latency percentiles, report.
- **Explicit non-scope:** embedding model quality comparison (EVAL-*); context-level recall (CTX-010 / EVAL).
- **Files/modules expected to change:** None outside benches.
- **New files/modules expected:** `benchmarks/perf/semantic/README.md`, `engine/crates/semantic/benches/qdrant_filtered.rs` (custom harness, not criterion, because it needs a live service), `benchmarks/perf/semantic/reports/.gitkeep`.
- **Dependencies:** SEM-005, SEM-007, PERF-001 (synthetic repo generator).
- **Implementation details:**
  - Corpus: 20 orgs × 5 repos; sizes skewed (one org 1M points, others 10k–100k) using hash-provider vectors from the synthetic repo; kinds mixed (60% chunk, 30% summary, 10% doc).
  - Queries: 500 per scenario, scoped to (a) large org one repo, (b) small org (selective filter, < 1% of points), (c) kind=doc only.
  - Ground truth: brute-force cosine over the filtered subset (computed offline in Rust).
  - Metrics: recall@10 and recall@50, p50/p95/p99 latency at `hnsw_ef` ∈ {64, 128, 256}.
  - Targets: recall@10 ≥ 0.95 for all scenarios at ef=128; p95 < 100 ms at 1M points on the reference VM; any scenario below target → open ADR-008 revisit (per its text).
  - Output JSON + markdown report under `benchmarks/perf/semantic/reports/{date}.md`.
- **Data model changes:** None.
- **API/protocol changes:** None.
- **Concurrency semantics:** load phase parallel upserts; query phase with 1 and 8 concurrent clients.
- **Failure behavior:** benchmark refuses to run without `QDRANT_URL` and an empty `rg_bench_*` collection prefix (never touches real collections).
- **Idempotency considerations:** seeded; reproducible corpus.
- **Security considerations:** uses dedicated `rg_bench_` collections; deletes them at the end.
- **Observability additions:** None (reports).
- **Tests required:** `ground_truth_bruteforce_small_corpus` (unit test of the harness math), `bench_collection_prefix_enforced`.
- **Benchmarks if applicable:** this task.
- **Acceptance criteria:** first report committed; targets met or an ADR-008 follow-up note filed with measurements.
- **Definition of done:** global DoD; OBS alert threshold confirmed against measured p95.

---

---

### CTX-001 — ContextPackage / ContextItem model
Status: ☐

- **Task ID:** CTX-001
- **Title:** `ContextPackage` and `ContextItem` model with source locations, provenance per item and an omitted-with-reason list
- **Problem:** Reviewers must receive structured, traceable context (PRD §89), and evidence must be traceable back to source (PRD §36); legacy agents decided context themselves with no record.
- **Why it exists:** target-arch §3.8 output contract; VER stage 4 re-checks cited ranges; R5 analysis needs to know what was omitted.
- **Scope:** types, item kinds, provenance (which candidate generator and signals), omitted list, package metadata, JSON Schema; conversion to the PRD §89 model input shape (`changedSymbols, changeSummary, graphNeighborhood, tests, repositoryRules, riskSignals, deterministicFindings`).
- **Explicit non-scope:** candidate generation, ranking, budgeting, compression (CTX-002..008).
- **Files/modules expected to change:** `engine/crates/context-engine/Cargo.toml`, `context-engine/src/lib.rs`.
- **New files/modules expected:** `context-engine/src/model/{mod.rs,item.rs,package.rs,omitted.rs}`, `context-engine/src/model/model_input.rs`, `tests/model.rs`.
- **Dependencies:** IMP-001, RISK-004, CHG-007, REV-001 (model input schema; this task produces the data, REV-001 owns the JSON Schema — coordinated, either can land first with a shared fixture).
- **Implementation details:**
  ```rust
  pub enum ItemKind { ChangedSymbol, RelatedSymbol, Test, Config, ApiEndpoint, RuleDoc, Convention, SchemaChange, DependencyChange, ClassSignature }
  pub enum CandidateSource { Structural, TestMap, Config, Api, RuleDoc, Lexical, Semantic }
  pub struct SourceLocation { pub file: RepoPath, pub start_line: u32, pub end_line: u32, pub side: Side, pub commit: CommitSha }
  pub struct ContextItem { pub id: ItemId /*blake3(kind‖node key‖side)[..16]*/, pub kind: ItemKind, pub node: Option<NodeKey>,
      pub title: String /*qualified name / path*/, pub relation: Option<Relation>, pub distance: Option<u8>, pub path: Vec<PathStep>,
      pub sources: BTreeSet<CandidateSource>, pub signals: SignalVector /*CTX-005*/, pub score: f32,
      pub content: ItemContent /*CTX-007*/, pub locations: Vec<SourceLocation>, pub tokens: u32, pub pinned: bool }
  pub enum OmitReason { BudgetTokens, BudgetCount{category}, BelowThreshold{score, threshold}, DistanceRule{distance}, Duplicate{of: ItemId},
                        Generated, Binary, SecretBearing, SemanticFillNotNeeded, LowConfidencePath{min_confidence} }
  pub struct OmittedItem { pub node: Option<NodeKey>, pub title: String, pub kind: ItemKind, pub score: f32, pub reason: OmitReason }
  pub struct ContextPackage { pub schema_version: u16, pub reviewer: ReviewerKind, pub cluster_id: ClusterId, pub items: Vec<ContextItem>,
      pub omitted: Vec<OmittedItem> /*capped at 200, then `omitted_overflow: u32`*/, pub omitted_overflow: u32,
      pub token_estimate: u32, pub budget: ContextBudget, pub risk_signals: Vec<RiskSignal>, pub change_summary: ChangeSummary,
      pub provenance: ContextProvenance /*input hashes, versions, weights version, semantic space or none*/, pub package_hash: Hash256 /*CTX-009*/ }
  ```
  - `pinned` items (distance 0 changed symbols, the class signature of each changed method, the endpoint path of a reached API for security) cannot be dropped by budgeting except by the hard token cap, and then only with `OmitReason::BudgetTokens` recorded.
  - `to_model_input(&self) -> serde_json::Value` maps items into PRD §89 sections; every code excerpt carries `"location": "path:start-end@side"`.
- **Data model changes:** None (stored in `stage_outputs`, target-arch §7 context cache).
- **API/protocol changes:** `ContextPackage` JSON Schema in contracts; review-engine `GET /internal/review-runs/{id}/context/{cluster}/{reviewer}` for explainability (WEB evidence tab).
- **Concurrency semantics:** Immutable value.
- **Failure behavior:** N/A.
- **Idempotency considerations:** Items sorted by `(pinned desc, score desc, id)`; omitted sorted by `(score desc, title)`.
- **Security considerations:** Package contains source excerpts → stored tenant-scoped, never logged; `SecretBearing` items are excluded by construction (CTX-007).
- **Observability additions:** None here.
- **Tests required:** `model_input_has_all_prd_89_sections`, `every_excerpt_has_location`, `omitted_capped_with_overflow_count`, `item_id_stable`, `sorted_order_deterministic`, `schema_roundtrip`.
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** tests green; schema published.
- **Definition of done:** global DoD; `docs/reviewers/context-package.md` describes fields.

---

### CTX-002 — Structural candidates from the impact graph
Status: ☐

- **Task ID:** CTX-002
- **Title:** Structural candidate generation from the cluster's `ImpactGraph` elements
- **Problem:** The decisive caller/callee/implementation must be in the candidate pool before anything else (R5).
- **Why it exists:** target-arch §3.8 step 1; Invariant 10.
- **Scope:** convert impact elements of the cluster seeds into candidates (dedup across seeds keeping min distance/max confidence), add changed symbols as distance-0 pinned items, add class signatures of containers, include base-side `RemovedCallee` versions.
- **Explicit non-scope:** tests/config/API (CTX-003) though they originate from impact too — split for testability.
- **Files/modules expected to change:** `context-engine/src/lib.rs`.
- **New files/modules expected:** `context-engine/src/candidates/{mod.rs,structural.rs}`, `tests/structural.rs`.
- **Dependencies:** CTX-001, IMP-002, IMP-003, IMP-009.
- **Implementation details:**
  ```rust
  pub struct Candidate { pub node: NodeKey, pub kind: ItemKind, pub side: Side, pub sources: BTreeSet<CandidateSource>,
                         pub distance: u8, pub path: Vec<PathStep>, pub min_confidence: f32, pub relation: Option<Relation>, pub raw: RawSignals }
  pub trait CandidateGenerator { fn generate(&self, ctx: &SelectionInput, out: &mut CandidatePool) -> GenStats; }
  pub struct CandidatePool { by_node: BTreeMap<(NodeKey, Side), Candidate> }   // merge on insert
  ```
  - Merge rule: same `(node, side)` → union sources, `distance = min`, keep the path with max `min_confidence`, merge raw signals by max.
  - Changed symbols: `ItemKind::ChangedSymbol`, distance 0, pinned; Modified symbols contribute **both** sides (head body + base body when `body` changed, so the reviewer sees before/after; base marked `side=Base`).
  - Container class signature (`ClassSignature`) pinned for each changed method (cheap: signature + member list).
  - Security reviewer filter (trust-boundary): keep only candidates on a path to an `Endpoint`, or with relation Interface/Implementation of an authz symbol, or with Authorization/Validation/Authentication signals, or `RemovedCallee` (PRD §35 security bullets).
  - Weak elements (`weak=true`) become candidates only if `min_confidence ≥ 0.3` and are tagged for CTX-005 penalty.
- **Data model changes:** None.
- **API/protocol changes:** None.
- **Concurrency semantics:** Pure; per reviewer selection runs in parallel (separate pools).
- **Failure behavior:** cluster without impact (degraded) → only changed symbols + signatures, `provenance.structural_degraded=true`.
- **Idempotency considerations:** BTreeMap pool ⇒ deterministic iteration.
- **Security considerations:** None.
- **Observability additions:** counter `context_candidates_total{source}`.
- **Tests required:** `auth_bypass_structural_has_updateuser_controller_authprovider_permission_check_base`, `changed_symbol_both_sides_pinned`, `class_signature_pinned`, `merge_keeps_min_distance_max_confidence`, `security_filter_trust_boundary_only`, `weak_edge_candidate_tagged`, `decoy_report_service_absent`.
- **Benchmarks if applicable:** in CTX-010.
- **Acceptance criteria:** tests green.
- **Definition of done:** global DoD.

---

### CTX-003 — Tests, config, API and rule-doc candidates
Status: ☐

- **Task ID:** CTX-003
- **Title:** Candidates for tests (IMP-005), configuration/env, API endpoints/DTOs, and repository rule docs/conventions
- **Problem:** Reviewers need the relevant test, the config a change reads, the endpoint contract, and the repository rule that applies — these are not callers/callees.
- **Why it exists:** PRD §33 signals (test/API/configuration relationship), PRD §65 precedence (policy > docs > convention), PRD §89 `repositoryRules`.
- **Scope:** four generators: `TestCandidates`, `ConfigCandidates`, `ApiCandidates`, `RuleDocCandidates`.
- **Explicit non-scope:** evaluating rules (POL-*), convention inference (PROF-*).
- **Files/modules expected to change:** `context-engine/src/candidates/mod.rs`.
- **New files/modules expected:** `candidates/{tests.rs,config.rs,api.rs,rules.rs}`, `tests/other_candidates.rs`.
- **Dependencies:** CTX-002, IMP-004, IMP-005, IMP-006, CHG-006, PROF-004 (conventions with confidence), POL-004 (precedence resolver), INIT-010 (rule docs).
- **Implementation details:**
  - Tests: IMP-005 `Test` elements (score ≥ 0.6) → `ItemKind::Test`, raw `test_relationship = mapping score`, `mocked` tests penalized 0.5×; changed tests in the cluster are pinned.
  - Config: `Config`/`EnvVar` elements and `ChangedConfiguration` in cluster files → `ItemKind::Config` with the *key-level* excerpt (keys + line ranges; values for non-secret config files only, `.env*` never).
  - API: `Endpoint` elements → `ItemKind::ApiEndpoint` with `{method, path, guards, dto types}` and the handler signature; DTO classes reached by `ACCEPTS_TYPE` get `RelatedSymbol` with `api_relationship = 1.0`.
  - Rule docs/conventions: profile entries applicable to the cluster's modules (scope glob match) → `RuleDoc`/`Convention` items; precedence from POL-004 sets raw `rule_priority` (policy 1.0, documented 0.8, inferred 0.6 if confidence ≥ 0.9 and samples ≥ 10 else excluded, generic 0.3).
- **Data model changes:** None.
- **API/protocol changes:** None.
- **Concurrency semantics:** Generators are independent; run sequentially in a fixed order for determinism (cheap).
- **Failure behavior:** missing profile → no rule candidates, recorded in provenance.
- **Idempotency considerations:** Deterministic.
- **Security considerations:** env/secret values excluded; config excerpts pass redaction.
- **Observability additions:** shared `context_candidates_total{source}`.
- **Tests required:** `auth_bypass_authorize_spec_candidate`, `mocked_test_penalized`, `changed_test_pinned`, `env_values_never_included`, `endpoint_candidate_has_guards_and_dto`, `policy_rule_beats_inferred_convention`, `low_sample_convention_excluded`.
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** tests green.
- **Definition of done:** global DoD.

---

### CTX-004 — Lexical identifier index and candidates
Status: ☐

- **Task ID:** CTX-004
- **Title:** Per-snapshot lexical identifier index and lexical candidates from changed identifiers
- **Problem:** Some relevant code is linked only by name (string-keyed DI tokens, event names, config keys, unresolved calls) and is missed by the graph; semantic search is too fuzzy for exact identifiers.
- **Why it exists:** target-arch §3.8 step 3 (lexical before semantic).
- **Scope:** inverted index from identifier tokens to nodes; extraction of changed identifiers from the change model; candidate generation with an IDF-weighted lexical score.
- **Explicit non-scope:** full-text search over source bodies (not stored); fuzzy matching.
- **Files/modules expected to change:** `context-engine/src/lib.rs`.
- **New files/modules expected:** `context-engine/src/lexical/{mod.rs,index.rs,tokenize.rs}`, `candidates/lexical.rs`, `tests/lexical.rs`.
- **Dependencies:** CTX-002, CG-004 (graph node names), TSA-006 (string literal identifiers in facts: DI tokens, event/queue names, config keys), CHG-003.
- **Implementation details:**
  - Index build per snapshot (cached `Arc<LexicalIndex>` in an LRU keyed by snapshot id, capacity 8): postings `HashMap<CompactString, Vec<NodeIx>>` from (a) node simple names, (b) qualified-name segments, (c) literal identifiers in syntax facts (`'PERMISSION_SERVICE'`, `'user.updated'`, `ConfigService.get('X')`), (d) unresolved reference names. Tokens: full identifier + camel/snake sub-tokens of length ≥ 3, lowercased. Stopword list for sub-tokens (`get,set,id,data,value,service,controller,module,dto,impl,handler`).
  - `idf(t) = ln(1 + N / df(t))`; postings with `df > 500` dropped from candidate generation (too common).
  - Changed identifiers: names in CHG-003 added/removed calls (callee text), literal identifiers in added/removed facts, names of changed symbols. Full identifiers weigh 1.0, sub-tokens 0.3.
  - Candidate lexical score `= Σ_t∈match w_t · idf(t) / Σ_t∈query w_t · idf(t)` ∈ [0,1]; keep top 50 with score ≥ 0.25; exclude nodes already in the pool with structural distance ≤ 2 (they just get the `Lexical` source added and the lexical signal set).
  - Build cost O(V + facts); reference-api ~15k symbols → small; synthetic 1M symbols → bounded by benchmark.
- **Data model changes:** None (in-memory).
- **API/protocol changes:** None.
- **Concurrency semantics:** Index built once per snapshot under a `OnceCell` per LRU entry; concurrent readers share `Arc`.
- **Failure behavior:** index build failure (OOM guard: > 5M postings) → lexical disabled for the run, provenance flag.
- **Idempotency considerations:** Deterministic (postings sorted by node key).
- **Security considerations:** Literal identifiers only (no arbitrary string literal values longer than 64 chars; values that look like secrets are skipped via redaction regex).
- **Observability additions:** histogram `lexical_index_build_ms`, gauge `lexical_index_postings`; span `context_selection.lexical`.
- **Tests required:** `di_string_token_links_provider`, `event_name_links_producer_and_consumer`, `common_token_df_cutoff`, `camel_subtokens_lower_weight`, `auth_bypass_lexical_does_not_promote_format_authorizeheader_above_threshold`, `already_structural_gets_source_added`, `index_cached_per_snapshot`.
- **Benchmarks if applicable:** index build on synthetic 1M symbols < 4 s and < 1.5 GB RSS; query < 5 ms.
- **Acceptance criteria:** tests and bench within targets.
- **Definition of done:** global DoD.

---

### CTX-005 — Ranking
Status: ☐

- **Task ID:** CTX-005
- **Title:** Relevance ranking from 10 PRD §33 signals with per-reviewer weights (no weight > 0.35) and the §34 distance heuristic
- **Problem:** Candidates exceed the budget; selection quality decides review quality (high-risk node on the critical path).
- **Why it exists:** PRD §33–§34; risk R5.
- **Scope:** signal computation in [0,1], weight tables per reviewer, distance gating rules, score and gate outcome per candidate, weight-table validation.
- **Explicit non-scope:** learned weights (EVAL may tune the table later; table is versioned config, not code constants only).
- **Files/modules expected to change:** `context-engine/src/lib.rs`.
- **New files/modules expected:** `context-engine/src/rank/{mod.rs,signals.rs,weights.rs,gate.rs}`, `context-engine/weights/v1.yaml`, `tests/rank.rs`.
- **Dependencies:** CTX-002, CTX-003, CTX-004, RISK-004, HIST-* (historical signal = 0 until post-MVP).
- **Implementation details:**
  - Signals (each in [0,1]):
    1. `structural` — relation strength: Caller/Callee/RemovedCallee 1.0, Interface/Implementation/Override 0.9, RelatedType 0.6, resource relations 0.6, Container 0.5, none 0.
    2. `proximity` — §34: d0 1.0, d1 0.8, d2 0.5, d3 0.2, d4+ 0.0.
    3. `execution_path` — 1.0 if on a path seed→Endpoint (IMP-004), 0.6 if on a path to a queue/job entry, else 0; × path min_confidence.
    4. `changed_code_similarity` — Jaccard of identifier sets between candidate and changed symbols' changed lines (from lexical tokens).
    5. `test_relationship` — CTX-003 mapping score.
    6. `api_relationship` — 1.0 endpoint/handler/DTO of a changed API, 0.5 reaching endpoint, else 0.
    7. `config_relationship` — 1.0 config/env read by a changed symbol, 0.5 by a caller.
    8. `semantic_similarity` — Qdrant cosine mapped `(s − 0.5)/0.5` clamped (CTX-008; 0 when absent).
    9. `historical` — 0 in MVP (HIST-* fills: past findings/changes co-occurrence).
    10. `risk_importance` — max risk contribution of the candidate symbol's own signals (RISK-004 per-symbol score).
  - Score: `score = Σ wᵢ · sᵢ · conf_penalty`, `conf_penalty = 0.5 + 0.5 · min_confidence` (path trust), weak edges extra × 0.7.
  - Weights `v1.yaml` (each row sums to 1.0; validated: every w ≤ 0.35, ≥ 0, sum = 1 ± 1e-6):
    | reviewer | struct | prox | exec | sim | test | api | config | sem | hist | risk |
    |---|---|---|---|---|---|---|---|---|---|---|
    | correctness | 0.25 | 0.20 | 0.12 | 0.10 | 0.08 | 0.06 | 0.05 | 0.06 | 0.00 | 0.08 |
    | security | 0.20 | 0.15 | 0.25 | 0.05 | 0.03 | 0.12 | 0.05 | 0.03 | 0.00 | 0.12 |
    | tests | 0.15 | 0.15 | 0.05 | 0.10 | 0.35 | 0.05 | 0.03 | 0.07 | 0.00 | 0.05 |
    | architecture | 0.30 | 0.10 | 0.05 | 0.05 | 0.00 | 0.15 | 0.10 | 0.15 | 0.00 | 0.10 |
    | performance | 0.25 | 0.20 | 0.15 | 0.10 | 0.02 | 0.05 | 0.08 | 0.05 | 0.00 | 0.10 |
    | maintainability | 0.25 | 0.20 | 0.00 | 0.20 | 0.05 | 0.00 | 0.05 | 0.20 | 0.00 | 0.05 |
    With `historical` = 0 in MVP its weight is 0; when HIST lands, a v2 table redistributes.
  - Distance gate (§34): d0 always (pinned); d1 included if score ≥ 0.20; d2 if score ≥ 0.35; d3 only if score ≥ 0.50 **and** ≥ 2 non-proximity signals ≥ 0.5; d4+ excluded (`OmitReason::DistanceRule`) unless `execution_path == 1.0` (an endpoint path) or it is a Test/ApiEndpoint item. Candidates without a graph distance (lexical/semantic only) are treated as d3.
  - "No signal dominates" enforced statically by the ≤ 0.35 validation and dynamically by a test computing, over the golden corpus, that removing any single signal changes the top-20 by ≤ 50% (sanity check, not a hard invariant).
- **Data model changes:** None.
- **API/protocol changes:** weights file versioned (`weights_version` in package provenance); config override `review.context.weights.{reviewer}` allowed but validated by the same rules.
- **Concurrency semantics:** Pure.
- **Failure behavior:** invalid weight override → rejected at config load (POL-001) and defaults used.
- **Idempotency considerations:** Deterministic; floats rounded to 6 decimals before comparison; ties by item id.
- **Security considerations:** None.
- **Observability additions:** histogram `context_candidate_score{reviewer}`; counter `context_gate_rejections_total{distance}`.
- **Tests required:** `weights_rows_sum_to_one_and_max_035`, `override_with_040_rejected`, `distance_gate_table` (one test per distance bucket), `d4_endpoint_path_exception`, `lexical_only_treated_as_d3`, `weak_edge_penalty`, `auth_bypass_security_ranks_controller_and_endpoint_top3`, `auth_bypass_correctness_ranks_updateuser_first_related`, `no_single_signal_dominates_corpus_check`, `ties_broken_by_id`.
- **Benchmarks if applicable:** 2,000 candidates scored < 2 ms.
- **Acceptance criteria:** tests green; golden ranking snapshots for auth-bypass (both reviewers).
- **Definition of done:** global DoD; weights documented in `docs/reviewers/context-package.md`.

---

### CTX-006 — Budgeting
Status: ☐

- **Task ID:** CTX-006
- **Title:** Per-reviewer budgets with greedy selection by score per token
- **Problem:** Context must stop before it becomes unbounded (PRD §35) while keeping the most valuable items.
- **Why it exists:** Critical path (`IMP-002 → CTX-006 → REV-C-002`); PRD §35, §90.
- **Scope:** `ContextBudget` resolution (defaults table above → config → risk multiplier → run budget), category caps, greedy selection, omitted records, never-exceed guarantee.
- **Explicit non-scope:** token-exact counting with provider tokenizers (estimator is deterministic and conservative; GW reports actual usage for calibration).
- **Files/modules expected to change:** `context-engine/src/lib.rs`.
- **New files/modules expected:** `context-engine/src/budget/{mod.rs,resolve.rs,select.rs,tokens.rs}`, `tests/budget.rs`.
- **Dependencies:** CTX-005, CTX-007 (token cost of compressed forms), RISK-005, PIPE-006, POL-001.
- **Implementation details:**
  ```rust
  pub struct ContextBudget { pub max_changed: u16, pub max_related: u16, pub max_tests: u16, pub max_configs: u16, pub max_rule_docs: u16,
                             pub max_tokens: u32, pub reserve_tokens: u32 /*for instructions+schema: 2,500*/ }
  pub fn estimate_tokens(text: &str) -> u32 { (text.chars().count() as u32 + 3) / 4 + 8 }  // +8 per-item framing overhead
  ```
  - Resolution: `max_tokens = min(round(default_cap × ctx_multiplier), config.max_context_tokens, run_remaining_tokens) − reserve_tokens`; counts scaled by multiplier for related/tests only (`ceil`), never below 1 for changed symbols.
  - Selection:
    1. Pinned items first in order (changed symbols by risk desc, then class signatures, then pinned tests/endpoints). If pinned alone exceed `max_tokens`, compress pinned items to their minimum form (CTX-007 level `SignatureOnly` for non-seed context; seeds keep changed hunks + signature) and, if still over, drop lowest-risk changed symbols with `BudgetTokens` (they are reported as unreviewed in the IMP-010 plan update) — the budget is **never** exceeded.
    2. Remaining candidates passing the gate sorted by `score / tokens(compressed_default_form)` desc (ties: score desc, id).
    3. Take while category cap and token budget allow; if the default form does not fit but a cheaper compression level does and `score ≥ 0.5`, take the cheaper form.
    4. Everything not taken → `omitted` with exact reason.
  - Changed symbols beyond `max_changed` in a cluster → cluster is split for this reviewer into sub-packages (ordered by risk) only if IMP-010 allowance permits; else omitted with `BudgetCount{ChangedSymbol}` and propagated to the unreviewed report.
- **Data model changes:** None.
- **API/protocol changes:** config `review.budgets.context.{reviewer}.{...}` (POL-001 schema).
- **Concurrency semantics:** Pure.
- **Failure behavior:** zero budget (run budget exhausted) → returns `ContextError::BudgetExhausted`; pipeline records `NOT_EXECUTED` for that reviewer unit (INV-014).
- **Idempotency considerations:** Deterministic.
- **Security considerations:** Budget caps protect cost; config cannot exceed hard maxima (tokens 150,000 per package).
- **Observability additions:** span `context_selection` (attrs `reviewer`, `cluster_id`, `candidates`, `selected`, `omitted`, `token_estimate`, `max_tokens`); histogram `context_package_tokens{reviewer}`; counter `context_omitted_total{reason}`.
- **Tests required:** `correctness_defaults_8_20_8_4`, `risk_multiplier_scales_tokens_and_related`, `config_max_context_tokens_clamps`, `pinned_first`, `pinned_overflow_compresses_then_drops_lowest_risk_reported`, `greedy_by_score_per_token`, `cheaper_form_used_when_default_does_not_fit`, `category_caps_respected`, `every_unselected_candidate_omitted_with_reason`, `budget_exhausted_error`, `proptest_tokens_never_exceed_max` (in CTX-010 as well).
- **Benchmarks if applicable:** selection over 2,000 candidates < 5 ms.
- **Acceptance criteria:** tests green; never-exceed proptest passes 10,000 cases.
- **Definition of done:** global DoD; critical-path checkpoint.

---

### CTX-007 — Compression
Status: ☐

- **Task ID:** CTX-007
- **Title:** Structured excerpts: signature + changed body + selected related bodies, with preserved source locations
- **Problem:** Whole files waste tokens (PRD §36: "entire 1,500-line service"); excerpts without locations break evidence tracing.
- **Why it exists:** PRD §36; VER stage 4 (cited code exists at cited ranges).
- **Scope:** compression levels per item kind, excerpt extraction from blobs at the right commit, elision markers with line numbers, secret screening, token cost per level for CTX-006.
- **Explicit non-scope:** summarization by model (not in MVP).
- **Files/modules expected to change:** `context-engine/src/lib.rs`.
- **New files/modules expected:** `context-engine/src/compress/{mod.rs,levels.rs,excerpt.rs}`, `tests/compress.rs`, `tests/snapshots/`.
- **Dependencies:** CTX-001, DIFF-001 (`ObjectSource` blob reads), DIFF-003 (hunks), TSA-001 (ranges), SEC-003 (secret patterns).
- **Implementation details:**
  ```rust
  pub enum CompressionLevel { Full /*whole symbol body*/, ChangedHunks /*signature + changed hunks ±3 lines + elisions*/,
                              BodyTrimmed /*signature + first 15 and last 5 lines + lines containing calls to seeds*/, SignatureOnly, Reference /*title + location only*/ }
  pub enum ItemContent { Code { level: CompressionLevel, segments: Vec<Segment> }, Structured(serde_json::Value) /*endpoint, config keys, rule text*/ }
  pub struct Segment { pub location: SourceLocation, pub text: String, pub elided_before: Option<LineRange> }
  ```
  - Defaults: ChangedSymbol → `ChangedHunks` if body > 80 lines else `Full` (both sides when body changed: base `Full|ChangedHunks` + head); RelatedSymbol d1 → `BodyTrimmed` if > 40 lines else `Full`; d2+ → `SignatureOnly` unless score ≥ 0.6 (`BodyTrimmed`); ClassSignature → class header + member signatures only; Test → test case body (`Full` ≤ 60 lines else `BodyTrimmed`); RuleDoc → section text ≤ 1,500 chars; Config → keys with line numbers.
  - `BodyTrimmed` keeps lines that call any seed symbol (from `CALLS` edge locations) — e.g. the `authorize` call line inside `AdminService.updateUser` and the following `throw ForbiddenException` — so the decisive caller logic survives trimming.
  - Elisions rendered as `// … lines 120–187 elided …` inside segments; every segment carries its `SourceLocation` (file, lines, side, commit SHA).
  - Text is read from blobs at the correct commit (head or base) — never from the working tree.
  - Secret screen: any segment matching secret patterns → item dropped with `OmitReason::SecretBearing` (not partially masked: avoids leaking structure of secrets to models).
  - `cost(level)` precomputed for each candidate on demand (lazy, memoized) for CTX-006.
- **Data model changes:** None.
- **API/protocol changes:** None.
- **Concurrency semantics:** Blob reads via `spawn_blocking`; per-item parallel with rayon; memo `DashMap<(ItemId, Level), u32>` per selection.
- **Failure behavior:** blob read failure → item downgraded to `Reference` with a warning; never fails the package.
- **Idempotency considerations:** Deterministic text for given commit and ranges.
- **Security considerations:** secret screening + redaction; package stored tenant-scoped; never logged.
- **Observability additions:** histogram `context_compression_ratio{kind}` (selected tokens / full tokens); counter `context_secret_items_dropped_total`.
- **Tests required:** `auth_bypass_authorize_full_both_sides`, `large_method_changed_hunks_with_elision_markers`, `body_trimmed_keeps_seed_call_lines` (updateUser keeps authorize call + throw), `class_signature_only_members`, `locations_match_blob_lines` (re-read and compare), `secret_bearing_item_dropped`, `blob_failure_downgrades_to_reference`, snapshots per level.
- **Benchmarks if applicable:** compress 200 items < 30 ms (warm blob cache).
- **Acceptance criteria:** `locations_match_blob_lines` passes on all golden packages (this is what VER stage 4 relies on).
- **Definition of done:** global DoD.

---

### CTX-008 — Semantic candidates (fill remaining budget only)
Status: ☐

- **Task ID:** CTX-008
- **Title:** Qdrant semantic candidates used only to fill remaining budget after structural/lexical selection
- **Problem:** Similar code elsewhere (a sibling implementation pattern, a related doc) helps, but semantic retrieval must never displace structural context (Invariant 10).
- **Why it exists:** target-arch §3.8 step 4; ADR-008.
- **Scope:** query construction, tenant-scoped search, mapping hits to candidates, second-pass budgeting restricted to remaining budget, timeouts/degradation.
- **Explicit non-scope:** embedding sync (SEM-007); using semantic hits as impact or evidence.
- **Files/modules expected to change:** `context-engine/src/budget/select.rs` (second pass), `context-engine/src/lib.rs`.
- **New files/modules expected:** `context-engine/src/candidates/semantic.rs`, `tests/semantic_fill.rs`.
- **Dependencies:** CTX-006, SEM-005, SEM-006.
- **Implementation details:**
  - Trigger: after first-pass selection, if `remaining_tokens ≥ 1,500` **and** `related_selected < max_related`. Otherwise record `SemanticFillNotNeeded` in provenance (no query made).
  - Query text: for each seed (top 3 by risk), the SymbolSummary template text of the head version (SEM-006) → embed as `Query`; kinds `[symbol_summary, code_chunk, doc, convention]`; scope = `TenantScope{org, [repo]}`; filter `snapshot_ids ∋ head_snapshot` OR base default-branch snapshot (two queries if the PR snapshot is not yet synced); exclude seeds and already-selected nodes; `limit 20`, `min_score 0.55`.
  - Hits → candidates with source `Semantic`, `semantic_similarity` signal, treated as d3 for gating unless a graph distance exists; ranked with the reviewer weights; selected greedily **only within remaining budget** and at most `max(2, 25% of max_related)` semantic-only items.
  - Invariant check (debug-assert + test): the first-pass selected set is a subset of the final set (semantic can only add).
  - Timeout 800 ms total; on error/timeout → no semantic items, `provenance.semantic=Unavailable{reason}`.
  - Privacy `no_external` with a remote embedding provider → semantic disabled (hash provider space may still be used if that collection exists).
- **Data model changes:** None.
- **API/protocol changes:** None.
- **Concurrency semantics:** async search awaited in the selection task; per reviewer selections for one cluster share one query result (memoized by `(cluster, seed set)`).
- **Failure behavior:** degrade silently to structural-only **with provenance record** (not silent in data).
- **Idempotency considerations:** Semantic results can vary if the index changes; the package hash includes the hit ids and scores, so cache keys reflect actual inputs (CTX-009).
- **Security considerations:** tenant scope mandatory (compile-time via SEM-005); results post-checked.
- **Observability additions:** span `qdrant_search` (child of `context_selection`); counters `context_semantic_fill_total{outcome=added|not_needed|unavailable}`, `context_semantic_items_selected_total`.
- **Tests required:** `semantic_never_removes_structural_items` (proptest), `not_needed_when_budget_full`, `fill_limited_to_25pct_related`, `timeout_degrades_with_provenance`, `excludes_seeds_and_selected`, `tenant_scope_audited` (SEM-005 audit layer on), `auth_bypass_semantic_fill_under_hash_provider_snapshot`.
- **Benchmarks if applicable:** fill step p95 < 150 ms with compose Qdrant at 100k points.
- **Acceptance criteria:** subset invariant proptest passes; degrade test green.
- **Definition of done:** global DoD.

---

### CTX-009 — Context cache + deterministic package hash
Status: ☐

- **Task ID:** CTX-009
- **Title:** Deterministic `package_hash` and a context cache keyed by `(review_run, cluster, reviewer, input_hash)`
- **Problem:** Retries and re-runs must reuse identical context (stage resumability, target-arch §4.1); reproducibility tests (ADR-015) compare package hashes.
- **Why it exists:** target-arch §7 context-package cache; ADR-015 reproducibility test.
- **Scope:** canonical serialization, `input_hash` and `package_hash` definitions, `ContextCache` trait, PG adapter in `pipeline` over `stage_outputs`, cross-run reuse rule.
- **Explicit non-scope:** model response caching (GW-008).
- **Files/modules expected to change:** `context-engine/src/lib.rs`, `pipeline/src/stages/context.rs`.
- **New files/modules expected:** `context-engine/src/hash.rs`, `context-engine/src/cache.rs` (trait + in-memory impl), `pipeline/src/cache/context_pg.rs`, `context-engine/tests/hash.rs`, `pipeline/tests/context_cache.rs`.
- **Dependencies:** CTX-001..CTX-008, PIPE-005 (`stage_outputs`), DOM-009.
- **Implementation details:**
  - `input_hash = blake3(change_model.input_hash ‖ impact.input_hash ‖ risk.input_hash ‖ cluster_id ‖ reviewer ‖ budget ‖ weights_version ‖ CONTEXT_ENGINE_VERSION ‖ profile_version ‖ config_hash ‖ semantic_space_or_none)`.
  - `package_hash = blake3(canonical_json(package without package_hash and without timing fields))`; canonical JSON: sorted keys, floats formatted with 6 decimals, items/omitted in their deterministic order.
  - `trait ContextCache { async fn get(&self, key: &CtxKey) -> Option<ContextPackage>; async fn put(&self, key: &CtxKey, pkg: &ContextPackage) -> Result<()>; }`, `CtxKey { review_run_id, cluster_id, reviewer, input_hash }`.
  - PG adapter: `stage_outputs(review_run_id, stage='context:{reviewer}:{cluster}', input_hash, output jsonb, output_hash, created_at)` with `INSERT ... ON CONFLICT (review_run_id, stage, input_hash) DO NOTHING`; packages > 1 MiB stored compressed in object storage, row holds the ref.
  - Cross-run reuse (superseding run on a new head with identical inputs for a cluster, e.g. only an unrelated file changed): lookup by `(repository_id, stage, input_hash)` index permitted within the same repository (never across tenants); copied into the new run's row.
  - Semantic nondeterminism: semantic hits are part of the package, so `package_hash` reflects them; under the hash provider and a frozen index, reproducibility holds (ADR-015 test).
- **Data model changes:** index `CREATE INDEX stage_outputs_repo_stage_input ON stage_outputs (repository_id, stage, input_hash)` (migration; `stage_outputs.repository_id` column assumed from PIPE-005 — added here if absent).
- **API/protocol changes:** None.
- **Concurrency semantics:** concurrent workers computing the same key: both compute, one insert wins (`DO NOTHING`), both use identical content (deterministic) — no lock needed.
- **Failure behavior:** cache read/write failure → compute without cache, warning metric; never fails the stage.
- **Idempotency considerations:** This task is the idempotency mechanism for the context stage.
- **Security considerations:** cache lookups always include `repository_id` (and RLS on `stage_outputs` by organization); no cross-tenant reuse.
- **Observability additions:** counters `context_cache_hits_total{scope=run|repo}`, `context_cache_misses_total`; span attr `package_hash`.
- **Tests required:** `package_hash_stable_across_runs`, `package_hash_changes_with_weights_version`, `input_hash_covers_all_inputs` (mutate each input → hash differs), `pg_cache_roundtrip`, `concurrent_put_same_key_single_row`, `cross_run_reuse_same_repo_only`, `cache_failure_falls_back_to_compute`, `reproducibility_two_runs_same_hash_under_replay_and_hash_provider`.
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** reproducibility test (ADR-015) passes for auth-bypass.
- **Definition of done:** global DoD.

---

### CTX-010 — Context tests (golden packages; budget-never-exceeded property test)
Status: ☐

- **Task ID:** CTX-010
- **Title:** Golden context packages for PR fixtures and property tests for budgets and invariants
- **Problem:** Context ranking is a top risk (R5); it needs frozen goldens, recall checks on decisive items, and invariant property tests.
- **Why it exists:** master plan §11 (golden + property: "context budget never exceeded"); milestone M5 prerequisite.
- **Scope:** goldens for correctness and security packages of all PR fixtures, decisive-item recall assertions, property tests, latency bench.
- **Explicit non-scope:** review outcomes (REV/QB).
- **Files/modules expected to change:** None outside tests/fixtures.
- **New files/modules expected:** `context-engine/tests/golden.rs`, `context-engine/tests/properties.rs`, `fixtures/pull-requests/*/expected/context/{correctness,security}.json`, `fixtures/pull-requests/*/expected/context-labels.yaml`, `context-engine/benches/context_e2e.rs`.
- **Dependencies:** CTX-001..CTX-009, IMP-008, RISK-004.
- **Implementation details:**
  - `context-labels.yaml` for auth-bypass: correctness `must_include: [AuthService.authorize (head+base), AdminService.updateUser (BodyTrimmed incl. authorize call + throw), PermissionService.check (base, RemovedCallee), AuthProvider.authorize, authorize.spec.ts case]`; security `must_include: [AuthService.authorize, UserController.update, "http:PUT /users/:id" with guards, AdminService.updateUser]`; both `must_exclude: [ReportService.*, format.ts#authorizeHeader]`.
  - Goldens use the hash embedding provider and a frozen local Qdrant snapshot built by the test (or semantic disabled variant `*.nosemantic.json`) — both variants checked.
  - Property tests (proptest, random graphs/candidate sets/budgets): (1) `token_estimate ≤ budget.max_tokens` always; (2) category counts ≤ caps; (3) every candidate is either selected or omitted exactly once; (4) semantic fill never removes first-pass items; (5) pinned seeds present unless `BudgetTokens` omitted; (6) package hash deterministic under candidate input permutation.
- **Data model changes:** None.
- **API/protocol changes:** None.
- **Concurrency semantics:** as other golden suites.
- **Failure behavior:** label failures print the item's rank, score breakdown and omission reason.
- **Idempotency considerations:** Goldens deterministic.
- **Security considerations:** None.
- **Observability additions:** None.
- **Tests required:** `golden_context_auth_bypass_correctness`, `golden_context_auth_bypass_security`, `golden_context_api_contract_break_architecture` (if architecture reviewer enabled in fixture config), `golden_context_transaction_removed_correctness_database_safety`, `labels_must_include_exclude_all_fixtures`, `prop_budget_never_exceeded`, `prop_counts_within_caps`, `prop_selected_xor_omitted`, `prop_semantic_only_adds`, `prop_pinned_present`, `prop_hash_permutation_invariant`.
- **Benchmarks if applicable:** `context_e2e`: auth-bypass package build (warm caches) < 50 ms; synthetic 200-file PR, 30 clusters × 2 reviewers < 2 s total.
- **Acceptance criteria:** all goldens and labels pass; proptests 10,000 cases each; benches within target.
- **Definition of done:** global DoD; M4/M5 checklist items updated in master plan.

---
