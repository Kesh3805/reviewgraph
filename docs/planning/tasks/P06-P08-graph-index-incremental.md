# Phases 6–8 — Persistent CodeGraph, Full Index, Incremental Indexing

**Parent:** [MASTER_IMPLEMENTATION_PLAN.md](../MASTER_IMPLEMENTATION_PLAN.md) §9 (phases 6, 7, 8)
**Governing docs:** [target-architecture §3.1–§3.5, §7, §8](../../architecture/target-architecture.md) · [ADR-003](../../decisions/ADR-003-codegraph-storage-model.md) · [ADR-004](../../decisions/ADR-004-incremental-graph-strategy.md) · [ADR-005](../../decisions/ADR-005-stable-symbol-identity.md) · [ADR-006](../../decisions/ADR-006-language-analyzer-protocol.md) · [ADR-007](../../decisions/ADR-007-tree-sitter-plus-semantic-enrichment.md) · [ADR-014](../../decisions/ADR-014-postgresql-graph-persistence.md) · [ADR-015](../../decisions/ADR-015-repository-snapshot-versioning.md)
**PRD:** §16–§24, §101–§105, §119–§120

Status markers: ☐ todo · ◐ in progress · ☑ done (acceptance criteria executed and passed). The master plan's Global Definition of Done applies to every task in addition to its own.

---

## 0. Conventions and clarifications made by this file

These are decisions taken while detailing the tasks. Each one is a refinement of the target architecture, not a reversal. If an implementer disagrees, fix this section (and add an ADR if architectural) before coding.

| # | Topic | Decision |
|---|---|---|
| C1 | Node-kind count | The PRD §17 list contains **44** names *including* `Repository` (target-architecture §3.3 and the gap analysis say "46 + Repository"; that is a miscount). `NodeKind` has exactly the 44 PRD kinds. A test pins the list. CG-001 updates the count in `docs/graph-schema/`. |
| C2 | Edge-kind count | PRD §18 lists 35 names. `CALLED_BY` and `DEPENDED_ON_BY` (and `TESTED_BY`, §31) are reverse views, so **33 stored `EdgeKind` variants** + a 3-variant `ReverseView` enum. |
| C3 | Framework role refinement | A framework fact may *refine* the graph `NodeKind` of an existing symbol node (e.g. a guard `Class` becomes `Middleware`, a controller `Class` becomes `Controller`, an `@Entity` class becomes `DatabaseEntity`, a `@Process` method becomes `JobHandler`). The `SymbolId` kind segment keeps the IR syntactic kind, so the `SymbolKey` never changes because of refinement. Synthetic nodes exist only for things that are not source symbols (endpoints, queues, tables, env vars, packages, test cases). |
| C4 | DB key encoding | `SymbolKey`/`NodeKey` (128-bit blake3 prefix, ADR-005) is stored as `bytea` with `CHECK (octet_length(x) = 16)`. Hex is the display/string form only. Kind enums are stored as `smallint` with seeded lookup tables (`node_kinds`, `edge_kinds`, `resolved_by_kinds`, `provenance_kinds`) that a test compares with the Rust discriminants. |
| C5 | Edge identity | The logical identity of an edge is `(source_key, kind, target_key)`. Multiple syntactic occurrences collapse into one edge that keeps the first location (min `(line, col)`) and an `occurrences` count. A delta overrides an edge by tombstone + add of the same identity. |
| C6 | `unresolved_refs` scope | Whether a reference resolves depends on the snapshot, not only on the file version. `unresolved_refs` therefore carries `snapshot_id` and follows **per-file replacement** overlay semantics (a delta lists every file it re-linked in `snapshot_files`; for those files its unresolved rows replace the base rows). `file_version_id` stays for provenance. |
| C7 | IR blob cache | The parse cache (target-architecture §7, "PG `file_versions` + object store blob") stores the full `ParsedUnit` as a zstd blob behind an `IrCache` port. Incremental inbound re-linking (INC-006) needs the references of *unchanged* files and reads them from this cache, so it never re-parses them. |
| C8 | CSR layout | Adjacency is a flat CSR (`offsets: Vec<u32>` per node + `edges: Vec<EdgeIx>` sorted by `(kind, other_node_key)`) plus a per-node `u64` kind mask, rather than `Vec<Vec<EdgeIx>>` (a million small allocations) or one offset table per kind (`V × 33` offsets). "Partitioned by edge kind" means the per-node slice is kind-sorted and located by binary search. |
| C9 | Serialization crate | The architecture names bincode + zstd. If `cargo deny check advisories` flags the pinned `bincode` as unmaintained, use `postcard` behind the same `codegraph::codec` module; only the codec byte in the file header changes. |
| C10 | Migrations numbering | Graph migrations use the `01xx` block (`engine/migrations/0100_…`), following whatever prefix convention DOM-009 established (if DOM-009 uses timestamps, keep the order shown here). Every table carries `organization_id` and enables RLS following DOM-009's policy pattern. |
| C11 | Linker version | Confidence values and resolution rules are versioned as `codegraph::LINKER_VERSION` and recorded in `analyzer_versions["linker"]`. A linker version change forces a full **re-link** (from cached IR, no re-parse), not a re-parse. |

#

## Task index

| ID | Title |
|---|---|
| CG-001 | NodeKind enum and synthetic node ID schemes |
| CG-002 | EdgeKind enum, Edge struct and reverse views |
| CG-003 | Confidence table module |
| CG-004 | In-memory Graph builder |
| CG-005 | Linker: resolve IR references to edges |
| CG-006 | Framework-fact mapping to generic nodes and edges |
| CG-007 | GraphQuery trait and basic queries |
| CG-008 | Bounded BFS |
| CG-009 | Shortest path and UI subgraph extraction |
| CG-010 | GraphDelta and GraphOverlay |
| CG-011 | Graph schema version and serialization |
| CG-012 | Consistency validator and graph compare |
| GS-001 | GraphStore trait and shared conformance suite |
| GS-002 | Migration: file_versions, symbols, unresolved_refs |
| GS-003 | Migration: snapshots, snapshot_files, graph_edges, synthetic_nodes, symbol_lineage |
| GS-004 | Postgres GraphStore: write_full and write_delta |
| GS-005 | Postgres GraphStore: load_graph and single-hop SQL neighbors |
| GS-006 | File GraphStore for `.review/graph` |
| GS-007 | Snapshot compaction |
| GS-008 | In-process graph LRU cache |
| IDX-001 | Full index pipeline |
| IDX-002 | Index status, progress and snapshot version fields |
| IDX-003 | Parse diagnostics persistence and tolerance policy |
| IDX-004 | repository-index job consumer |
| IDX-005 | Parse cache keyed by (path, content_hash, analyzer_version) |
| IDX-006 | Index reference-api and parity report |
| INC-001 | Changed-path detection between commits (gix tree diff) incl. renames |
| INC-002 | Hash-skip + reparse-only-changed |
| INC-003 | Apply per-file symbol diffs (SID-004/005) to build the head symbol table |
| INC-004 | Name index with delta support |
| INC-005 | Re-link outgoing refs of changed files |
| INC-006 | Re-link inbound refs of unchanged dependents (reverse index + name-index delta; never full scan) |
| INC-007 | Delta snapshot emission |
| INC-008 | Invalidation set computation (changed ∪ policy 1-hop dependents) + emitted cache keys / embedding invalidations |
| INC-009 | Head graph materialization for a PR (base ⊕ delta overlay in memory) |
| INC-010 | Counters + tests proving unchanged files are not reparsed |
| INC-011 | Full-rebuild trigger evaluation (PRD §24) |
| INC-012 | Oracle property test: incremental == full rebuild under random edit sequences (proptest) |
| INC-013 | incremental-index job consumer |

---

### CG-001 — NodeKind enum and synthetic node ID schemes
Status: ◐
> **Implementation note:** Code and tests are in place: `NodeKind` has the 44 PRD kinds with every discriminant and spelling written out literally in `codegraph/tests/wire_compat.rs`, so a rename or renumbering fails against a table that already exists in `graph-storage/src/kinds.rs`; the synthetic ID schemes (`http:`, `queue:`, `db:`, `env:`, `pkg:`, `test:`, `file:`, `dir:`, `package:`) are pinned in the same file; `cargo test -p codegraph node_` passes (25 unit + 12 integration tests). Still outstanding and outside this crate's lane: `docs/graph-schema/node-kinds.md`.

**Task ID:** CG-001

**Title:** `NodeKind` enum (PRD §17, 44 kinds incl. `Repository`) and deterministic synthetic/structural node ID schemes.

**Problem:** The graph has no node taxonomy. Nodes that are not source symbols (HTTP endpoints, queues, tables, env vars, external packages, test cases, files, directories) need IDs that are stable across commits and identical whether produced by a full or an incremental build.

**Why it exists:** PRD §17 defines the taxonomy. Target-architecture §3.3 defines the synthetic ID formats. The incremental oracle (INC-012) can only pass if every node ID is a pure function of its inputs.

**Scope:**
- `NodeKind` with explicit, stable `#[repr(u8)]` discriminants, `as_str()`/`FromStr` using PRD spelling, `serde` + `schemars` derives.
- `NodeKind::category()` → `NodeCategory { Structural, Type, Callable, Data, Framework, Database, Messaging, Config, Test, External, Build, Governance }`.
- `NodeId` (canonical string) and `NodeKey` (re-export of `review_core::SymbolKey` from SID-001) for all non-symbol nodes.
- Normalizers and constructors for every synthetic scheme.
- Reserved-prefix registry that SID-001 language prefixes must not collide with.

**Explicit non-scope:** extracting nodes from source (TSA/NEST, CG-006); edge kinds (CG-002); kind refinement logic (CG-006 applies it; CG-001 only defines `NodeKind::refinable_from()`).

**Files/modules expected to change:** `engine/crates/codegraph/Cargo.toml`, `engine/crates/codegraph/src/lib.rs` (skeleton from FND-001).

**New files/modules expected:** `engine/crates/codegraph/src/node_kind.rs`, `engine/crates/codegraph/src/node_id.rs`, `engine/crates/codegraph/tests/node_id_golden.rs`, `docs/graph-schema/node-kinds.md`.

**Dependencies:** FND-001 (workspace), SID-001 (`SymbolId`/`SymbolKey`), TSA-001 (IR symbol kinds, for the IR-kind → NodeKind map).

**Implementation details:**
```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[repr(u8)]
pub enum NodeKind {
    Repository = 0, Package = 1, Module = 2, Directory = 3, File = 4,
    Namespace = 10, Class = 11, Interface = 12, Struct = 13, Trait = 14, Enum = 15, TypeAlias = 16,
    Function = 20, Method = 21, Constructor = 22, Property = 23, Field = 24, Parameter = 25,
    Variable = 26, Constant = 27,
    ApiEndpoint = 30, Controller = 31, Handler = 32, Middleware = 33,
    DatabaseEntity = 40, DatabaseTable = 41, DatabaseColumn = 42, Migration = 43,
    Queue = 50, QueueProducer = 51, QueueConsumer = 52, JobHandler = 53,
    Configuration = 60, EnvironmentVariable = 61,
    TestSuite = 70, TestCase = 71, Fixture = 72,
    ExternalDependency = 80, ExternalApi = 81,
    BuildTarget = 90, CliCommand = 91, Worker = 92,
    DocumentationRule = 100, ArchitecturalBoundary = 101,
}
pub const ALL_NODE_KINDS: [NodeKind; 44] = [/* in discriminant order */];
impl NodeKind { pub const fn as_str(self) -> &'static str; /* "APIEndpoint", "CLICommand", ... PRD spelling */ }
```
- Gaps between discriminant blocks leave room for language-specific extensions (PRD §17 last line) without renumbering. Discriminants are never reused; removing a kind requires a `SCHEMA_VERSION` bump (CG-011).
- ID schemes (all produce `NodeId(String)`; `NodeKey = SymbolKey::from_canonical(&id)` — the same blake3-128 function SID-001 uses):

| Kind | Constructor | Format | Normalization |
|---|---|---|---|
| Repository | `NodeId::repository()` | `repo:/` | singleton; the repository is a namespace column (ADR-005) |
| Directory | `NodeId::directory(&RepoPath)` | `dir:{path}` | repo-relative, `/` separators, no trailing `/`, root = `dir:.` |
| File | `NodeId::file(&RepoPath)` | `file:{path}` | repo-relative, extension kept |
| Package (workspace) | `NodeId::workspace_package(&RepoPath)` | `package:{dir}` | workspace package directory |
| ApiEndpoint | `NodeId::http(method, path)` | `http:{METHOD} {normalized_path}` | METHOD upper-cased (`ALL` allowed); path: leading `/`, collapse `//`, strip trailing `/` except root, path params (`:id`, `{id}`, `[id]`, `*`) → `{}`, query string removed, case preserved |
| Queue | `NodeId::queue(name)` | `queue:{name}` | trimmed, case preserved |
| DatabaseTable | `NodeId::table(schema, table)` | `db:{schema}.{table}` | lower-cased (PG folds unquoted identifiers), quoted identifiers keep case, default schema `public` |
| EnvironmentVariable | `NodeId::env(name)` | `env:{NAME}` | exact case, must match `^[A-Za-z_][A-Za-z0-9_]*$` else `NodeIdError::InvalidEnvName` |
| ExternalDependency | `NodeId::package(ecosystem, spec)` | `pkg:{ecosystem}/{name}` | npm: `@scope/name` or `name`; deep imports (`lodash/fp`) reduced to package; `node:` builtins → `pkg:node/{module}`; no version |
| TestSuite / TestCase | `NodeId::test(file, suite_path, name)` | `test:{file}#{suite path} › {name}` | suites joined by ` › ` (U+203A); whitespace collapsed; duplicates in a file get `~{n}` ordinal in source order |

- `RESERVED_PREFIXES: &[&str] = &["repo", "dir", "file", "package", "http", "queue", "db", "env", "pkg", "test"]`. `debug_assert!` plus a unit test assert that no SID-001 language prefix (`ts`, `js`, …) is reserved.
- `NodeKind::from_ir_symbol_kind(IrSymbolKind) -> NodeKind` maps TSA-001's syntactic kinds (enum member → `Constant` with `attrs.enum_member=true`; getter/setter → `Property`).
- `NodeKind::refinable_from(self) -> &'static [NodeKind]` (e.g. `Middleware ← [Class, Function]`, `Controller ← [Class]`, `DatabaseEntity ← [Class]`, `JobHandler ← [Method, Function]`, `QueueConsumer ← [Class]`, `CliCommand ← [Class, Function]`, `Worker ← [Class]`).
- Complexity: every constructor is O(len(input)).

**Data model changes:** None in this task (GS-003 seeds `node_kinds` from `ALL_NODE_KINDS`).

**API/protocol changes:** `NodeKind` and `NodeId` are exported to `packages/contracts` JSON Schema via `schemars` (string enum with PRD spelling).

**Concurrency semantics:** Pure functions and `Copy` types; `Send + Sync`.

**Failure behavior:** Constructors that can receive invalid input return `Result<NodeId, NodeIdError>` (`EmptyName`, `InvalidEnvName`, `InvalidHttpMethod`, `PathOutsideRepository`). Never panic.

**Idempotency considerations:** Normalization is idempotent: `normalize(normalize(x)) == normalize(x)` (property-tested).

**Security considerations:** Paths must be `RepoPath` (validated repo-relative, no `..`, from the `repository` crate), so IDs never embed absolute host paths. Env-var nodes record the variable *name* only, never a value.

**Observability additions:** None (pure types).

**Tests required:**
- `node_kind_list_matches_prd_17` (exact 44 names, order and spelling).
- `node_kind_discriminants_are_unique_and_stable` (insta snapshot of `(name, u8)` pairs).
- `node_kind_roundtrip_str_and_serde`.
- `http_id_normalizes_params_and_slashes` (`/users/:id/` and `/users//users/{userId}` cases).
- `db_id_lowercases_unquoted_and_defaults_public`.
- `pkg_id_reduces_deep_imports_and_scopes`.
- `test_id_joins_suite_path_and_orders_duplicates`.
- `env_id_rejects_invalid_names`.
- `reserved_prefixes_do_not_collide_with_language_prefixes`.
- proptest `normalization_is_idempotent` for http/db/pkg/test.
- `node_key_equals_blake3_prefix_of_id` (cross-check with SID-001).

**Benchmarks:** None.

**Acceptance criteria:**
- `engine/scripts/cargo.sh test -p codegraph node_` passes.
- `ALL_NODE_KINDS.len() == 44` and the golden snapshot is committed.
- `docs/graph-schema/node-kinds.md` lists every kind, its discriminant, category, refinement sources and ID scheme, and records clarification C1.

**Definition of done:** Global DoD; enum exported via schemars into `packages/contracts`; node-kind doc merged.

---

---

### CG-002 — EdgeKind enum, Edge struct and reverse views
Status: ◐
> **Implementation note:** Code and tests are in place: 33 stored `EdgeKind` values plus the three `ReverseView` projections, and `wire_compat.rs` asserts that `CALLED_BY`, `DEPENDED_ON_BY` and `TESTED_BY` do not parse back into a stored kind (clarification C2), alongside the frozen `EdgeFlags` bits and `EdgeIdentity`. Still outstanding and outside this crate's lane: `docs/graph-schema/edge-kinds.md`.

**Task ID:** CG-002

**Title:** `EdgeKind` (PRD §18, 33 stored kinds), `Edge` with confidence/resolved_by/provenance/location, and non-stored reverse views `CALLED_BY`, `DEPENDED_ON_BY`, `TESTED_BY`.

**Problem:** Edges need one typed representation that every producer (linker, framework mapper, type checker) and every consumer (queries, storage, verification) shares, with uncertainty recorded on each edge (PRD §19, Invariant 7).

**Why it exists:** Target-architecture §3.3: reverse relations are views over the reverse index and are never stored, to avoid double writes and drift. Gap analysis §P resolved `TESTED_BY` as a view.

**Scope:**
- `EdgeKind` (33 variants, stable discriminants), `ReverseView` (3 variants) and the `EdgeSelector` that maps a view to `(EdgeKind, Direction::In)`.
- `EdgeKindSet` (u64 bitset) with `const` constructors and named groups.
- `Edge` (owned, for builders/deltas) and `EdgeIdentity`.
- `ResolvedBy`, `Provenance`, `EdgeFlags`, `Location`, `Confidence` types (values from CG-003).
- Direction conventions and the allowed endpoint-kind matrix used by the validator.

**Explicit non-scope:** confidence values (CG-003); producing edges (CG-005/006); storage encoding (GS-003).

**Files/modules expected to change:** `engine/crates/codegraph/src/lib.rs`.

**New files/modules expected:** `engine/crates/codegraph/src/edge.rs`, `engine/crates/codegraph/src/edge_kind.rs`, `engine/crates/codegraph/src/schema_rules.rs`, `docs/graph-schema/edge-kinds.md`.

**Dependencies:** CG-001.

**Implementation details:**
```rust
#[repr(u8)]
pub enum EdgeKind {
    Contains = 0, Declares = 1, Imports = 2, Exports = 3, Calls = 4, Reads = 5, Writes = 6,
    Implements = 7, Extends = 8, Overrides = 9, References = 10,
    UsesType = 11, ReturnsType = 12, AcceptsType = 13, RoutesTo = 14, HandledBy = 15,
    Tests = 16, Covers = 17, ProducesJob = 18, ConsumesJob = 19,
    ReadsConfig = 20, WritesConfig = 21, ReadsTable = 22, WritesTable = 23, DependsOn = 24,
    Throws = 25, Catches = 26, Serializes = 27, Deserializes = 28,
    Validates = 29, Authorizes = 30, Publishes = 31, Subscribes = 32,
}
pub enum ReverseView { CalledBy, DependedOnBy, TestedBy }
impl ReverseView { pub const fn underlying(self) -> EdgeKind { /* Calls | DependsOn | Tests */ } }
pub struct EdgeKindSet(u64);   // const ALL, STRUCTURAL, CALL_LIKE, TYPE_REL, FRAMEWORK, DATA
pub struct Confidence(u16);    // permille 0..=1000; from_f32 rounds; as_f32
pub struct Location { pub file: RepoPathId /* interned */, pub line: u32, pub col: u32 }
bitflags! { pub struct EdgeFlags: u8 { const INSTANTIATES=1; const DECORATOR=2; const TYPE_ONLY=4;
                                       const DYNAMIC=8; const MAPS_TABLE=16; const GLOBAL_SCOPE=32; } }
pub struct Edge {
    pub kind: EdgeKind, pub source: NodeKey, pub target: NodeKey,
    pub confidence: Confidence, pub resolved_by: ResolvedBy, pub provenance: Provenance,
    pub flags: EdgeFlags, pub location: Option<Location>, pub occurrences: u32,
    pub origin_file: Option<RepoPathId>,      // file whose content produced the edge (ownership for re-link)
}
pub struct EdgeIdentity { pub source: NodeKey, pub kind: EdgeKind, pub target: NodeKey }  // C5
pub enum Provenance { Analyzer = 0, Framework = 1, Linker = 2, TypeChecker = 3, Heuristic = 4, Policy = 5 }
```
- `ResolvedBy` variants: `Structural, Import, ThisMember, DiConstructor, TypeAnnotation, NameUnique, NameAmbiguous, Framework, TypeChecker, Heuristic` (stable `#[repr(u8)]`).
- Direction conventions (documented in `edge-kinds.md`, enforced by `schema_rules`): `CALLS` caller→callee; `CONTAINS` parent→child (Directory→File, File→top-level symbol, Class→member); `DECLARES` File→symbol declared at top level; `IMPORTS` File→File or File→ExternalDependency; `EXPORTS` File→Symbol; `EXTENDS`/`IMPLEMENTS` sub→super; `OVERRIDES` subMethod→superMethod; `HANDLED_BY` ApiEndpoint→handler (Method/Function); `ROUTES_TO` ApiEndpoint→Controller; `AUTHORIZES` Middleware→ApiEndpoint; `PRODUCES_JOB` producer→Queue; `CONSUMES_JOB` JobHandler→Queue; `READS_TABLE`/`WRITES_TABLE` callable→DatabaseTable; `READS_CONFIG` callable→EnvironmentVariable|Configuration; `TESTS` TestCase→symbol under test; `DEPENDS_ON` File|Package→Package|ExternalDependency.
- `schema_rules::allowed(kind) -> &'static [(NodeCategory, NodeCategory)]`. Violations are validator **warnings** (CG-012), never build errors, so language extensions remain possible.
- `Edge::identity()` and `Ord for Edge` (by `(source, kind, target)`) give a total deterministic order.
- `Edge::merge_occurrence(&mut self, other)`: keeps min location, max confidence (with its `resolved_by`), sums occurrences. Used when one file has several call sites to the same target.

**Data model changes:** None here (GS-003 seeds `edge_kinds`, `resolved_by_kinds`, `provenance_kinds`).

**API/protocol changes:** `EdgeKind`, `ReverseView`, `ResolvedBy`, `Provenance` exported to contracts with PRD spelling (`CALLS`, `CALLED_BY`, …). The API accepts reverse-view names in kind filters and translates them with `EdgeSelector::parse`.

**Concurrency semantics:** Plain data, `Send + Sync`, `Copy` where possible.

**Failure behavior:** `Confidence::try_from_f32` rejects NaN/out-of-range with `ConfidenceError`; `EdgeSelector::parse` returns `UnknownEdgeKind(String)`.

**Idempotency considerations:** `merge_occurrence` is commutative and associative (property-tested), so the merge result does not depend on reference order.

**Security considerations:** None beyond CG-001 (locations reference repo paths only).

**Observability additions:** None.

**Tests required:**
- `edge_kind_list_matches_prd_18_minus_reverse_views` (33 names + 3 views = PRD 35 + `TESTED_BY`).
- `edge_kind_discriminants_snapshot`.
- `reverse_view_maps_to_underlying_kind_in_direction`.
- `edge_kind_set_ops_and_groups`.
- `confidence_rounding_permille` (0.95 → 950; 0.3 → 300).
- proptest `merge_occurrence_commutative_associative`.
- `edge_ord_is_total_and_matches_identity_order`.
- `schema_rules_cover_every_edge_kind`.

**Benchmarks:** None.

**Acceptance criteria:** Tests pass; no `CalledBy`/`DependedOnBy`/`TestedBy` variant exists in `EdgeKind` (asserted by test); `docs/graph-schema/edge-kinds.md` documents direction, allowed endpoints and reverse views.

**Definition of done:** Global DoD; contracts schema regenerated; edge-kind doc merged.

---

---

### CG-003 — Confidence table module
Status: ☑
> **Implementation note:** Done. `confidence_of` is an exhaustive `const fn`, so a new `ResolvedBy` variant does not compile until a value is chosen for it; `derived` takes the weaker of rule and input and is idempotent; the whole table is frozen in `codegraph/tests/wire_compat.rs` together with the exact `permille / 1000` round-trip storage performs. The only `from_permille` callers outside this module are in `graph-storage`, which keeps a documented mirror of the persisted values in its own lane.

**Task ID:** CG-003

**Title:** `codegraph::confidence` — the single source of truth for `resolved_by → confidence`.

**Problem:** Confidence values must not be scattered as literals across the linker, framework mapper and semantic provider; otherwise calibration (QB/PERF benchmarks) cannot change them consistently.

**Why it exists:** Target-architecture §3.1 table; ADR-007 ("all values live in one table, `codegraph::confidence`"); PRD §19.

**Scope:**
- A `const` table, a lookup fn, a combination rule for derived edges, and the `LINKER_VERSION` constant (C11).
- A clippy-enforced ban on `Confidence::from_permille` literals outside this module.

**Explicit non-scope:** calibration itself (QB/PERF tasks); runtime-configurable overrides (deliberately not supported: values change only with a `LINKER_VERSION` bump).

**Files/modules expected to change:** `engine/crates/codegraph/src/lib.rs`, `engine/crates/codegraph/src/edge.rs` (make `Confidence::from_permille` `pub(crate)`).

**New files/modules expected:** `engine/crates/codegraph/src/confidence.rs`, `engine/crates/codegraph/tests/confidence_table.rs`.

**Dependencies:** CG-002.

**Implementation details:**
```rust
pub const LINKER_VERSION: &str = "1.0.0";   // bump minor when values change; major when rules change
pub const fn confidence_of(r: ResolvedBy) -> Confidence {
    match r {
        ResolvedBy::Structural => Confidence(1000), ResolvedBy::TypeChecker => Confidence(1000),
        ResolvedBy::Import => Confidence(950),     ResolvedBy::ThisMember => Confidence(950),
        ResolvedBy::Framework => Confidence(900),  ResolvedBy::DiConstructor => Confidence(850),
        ResolvedBy::TypeAnnotation => Confidence(800), ResolvedBy::NameUnique => Confidence(600),
        ResolvedBy::Heuristic => Confidence(500),  ResolvedBy::NameAmbiguous => Confidence(300),
    }
}
/// Derived edges (e.g. TESTS from a test's resolved calls, HANDLED_BY from a route fact on a
/// resolved method) take the weaker of the derivation and its input.
pub const fn derived(rule: ResolvedBy, input: Confidence) -> Confidence { min(confidence_of(rule), input) }
pub const TABLE: [(ResolvedBy, Confidence); 10] = [/* generated from confidence_of for docs/tests */];
```
- `confidence_of` is exhaustive (`match` with no wildcard) so adding a `ResolvedBy` variant fails compilation until a value is chosen.
- `docs/graph-schema/edge-kinds.md` includes the table, generated by a test that compares the doc's table with `TABLE` (`confidence_doc_matches_table`).
- `Confidence::from_permille` becomes `pub(crate)` and only `confidence.rs` and the codec call it; external crates can only obtain confidences via `confidence_of`/`derived` or by deserializing stored edges.

**Data model changes:** None.

**API/protocol changes:** None (the API exposes edge confidences as floats).

**Concurrency semantics:** `const fn`, no state.

**Failure behavior:** Not applicable (total function).

**Idempotency considerations:** `derived` is idempotent: `derived(r, derived(r, c)) == derived(r, c)`.

**Security considerations:** None.

**Observability additions:** `LINKER_VERSION` is recorded in `snapshots.analyzer_versions["linker"]` (IDX-002) and as span attribute `linker_version` on `graph.link`.

**Tests required:**
- `table_matches_target_architecture_3_1` (import .95, this_member .95, di_constructor .85, type_annotation .8, name_unique .6, name_ambiguous .3, framework .9, type_checker 1.0).
- `ordering_is_monotonic` (type_checker ≥ import ≥ … ≥ name_ambiguous).
- `derived_takes_minimum`.
- `confidence_doc_matches_table`.

**Benchmarks:** None.

**Acceptance criteria:** Tests pass; `rg "from_permille" engine/crates --glob '!**/codegraph/src/{confidence,codec}.rs'` returns no hits outside tests.

**Definition of done:** Global DoD; doc table generated and checked.

---

---

### CG-004 — In-memory Graph builder
Status: ◐
> **Implementation note:** Code and tests are in place: `tests/graph_build.rs` covers canonical ordering, the property that building from the same input multiset in any order yields an identical graph, dangling-edge rejection and the reverse-index invariants (9 tests, passing at `PROPTEST_CASES=1000`). Still outstanding and outside this crate's lane: the `benchmarks/perf/README.md` baseline row and the memory target or its PERF deviation.

**Task ID:** CG-004

**Title:** Immutable, `Arc`-shared `codegraph::Graph` with interned nodes and CSR forward/reverse adjacency partitioned by edge kind.

**Problem:** Traversals over a 1M-symbol / multi-million-edge graph must be fast and memory-bounded (PRD §120, ADR-014: traversals run in memory, not recursive SQL).

**Why it exists:** Target-architecture §3.3 "In-memory representation"; every query, impact computation and verification stage reads this structure.

**Scope:**
- `GraphBuilder` (mutable, accepts nodes/edges/files/unresolved refs in any order) → `Graph` (immutable).
- Interning: node table, string interner, file table.
- Forward and reverse CSR with per-node kind masks.
- Per-file node ranges and per-file edge ownership index (needed by INC-005/006).
- Unresolved-reference storage with a by-name index.
- `heap_size_bytes()` for the cache (GS-008).

**Explicit non-scope:** resolution (CG-005); query API surface (CG-007); overlays (CG-010); serialization (CG-011).

**Files/modules expected to change:** `engine/crates/codegraph/src/lib.rs`, `engine/crates/codegraph/Cargo.toml` (add `hashbrown`/`ahash` or `rustc-hash`, `smallvec`).

**New files/modules expected:** `engine/crates/codegraph/src/graph/{mod.rs, builder.rs, csr.rs, interner.rs, node.rs, file.rs, unresolved.rs}`, `engine/crates/codegraph/benches/graph_build.rs`, `engine/crates/codegraph/src/testkit.rs` (feature `testkit`: synthetic graph generator used by benches and other crates' tests).

**Dependencies:** CG-001, CG-002, CG-003, SID-001.

**Implementation details:**
```rust
#[derive(Copy, Clone, Eq, PartialEq, Hash, Ord, PartialOrd)] pub struct NodeIx(u32);
#[derive(Copy, Clone, Eq, PartialEq, Hash, Ord, PartialOrd)] pub struct EdgeIx(u32);
#[derive(Copy, Clone, Eq, PartialEq, Hash)] pub struct StrId(u32);
#[derive(Copy, Clone, Eq, PartialEq, Hash)] pub struct FileIx(u32);

pub struct NodeData {                // ~56 bytes
    pub key: NodeKey, pub kind: NodeKind, pub id: StrId, pub name: StrId, pub qualified_name: StrId,
    pub file: Option<FileIx>, pub range: Option<SourceRange>,      // SourceRange{start_line,start_col,end_line,end_col}: u32s
    pub attrs: NodeAttrs,
}
pub struct NodeAttrs { pub visibility: Visibility, pub flags: NodeFlags /* exported|generated|test|abstract|static|async|enum_member */,
                       pub signature: Option<StrId>, pub body_hash: Option<Hash128>, pub signature_hash: Option<Hash128>,
                       pub parent: Option<NodeKey>, pub extra: Option<Box<[(StrId, StrId)]>> }
pub struct EdgeData {                // 28 bytes
    pub source: NodeIx, pub target: NodeIx, pub kind: EdgeKind, pub resolved_by: ResolvedBy,
    pub provenance: Provenance, pub flags: EdgeFlags, pub confidence: Confidence,
    pub origin_file: Option<FileIx>, pub line: u32, pub col: u32, pub occurrences: u32,
}
pub struct FileEntry { pub path: StrId, pub file_version_id: Option<i64>, pub content_hash: Hash256,
                       pub language: Language, pub nodes: Range<u32>, pub edges_owned: Range<u32> /* into owned_edges */ }
struct Csr { offsets: Vec<u32> /* len V+1 */, edges: Vec<EdgeIx>, kind_mask: Vec<u64> /* len V */ }
pub struct Graph {
    schema_version: u32, nodes: Vec<NodeData>, by_key: HashMap<NodeKey, NodeIx>,
    edges: Vec<EdgeData>, fwd: Csr, rev: Csr, owned_edges: Vec<EdgeIx>,
    files: Vec<FileEntry>, by_path: HashMap<StrId, FileIx>, strings: Interner,
    unresolved: Vec<UnresolvedRef>, unresolved_by_name: HashMap<StrId, Vec<u32>>, stats: GraphStats,
}
impl GraphBuilder {
    pub fn new(schema_version: u32) -> Self;
    pub fn add_file(&mut self, f: FileInput) -> Result<(), GraphBuildError>;
    pub fn add_node(&mut self, n: NodeInput) -> Result<(), GraphBuildError>;   // dup key with different id => KeyCollision
    pub fn add_edge(&mut self, e: Edge);                                         // dup identity => merge_occurrence
    pub fn add_unresolved(&mut self, r: UnresolvedRef);
    pub fn build(self) -> Result<Graph, GraphBuildError>;
}
```
- **Deterministic layout.** Nodes are sorted by `(file path, key)`; nodes without a file (synthetic, repository) go last, sorted by key. Each file therefore owns a contiguous `nodes` range. Edges are sorted by `(source key, kind, target key)`.
- **CSR construction.** Count out-degree and in-degree (O(E)), prefix-sum into `offsets` (O(V)), scatter `EdgeIx` (O(E)). Within each node slice, edges are already in `(kind, other key)` order because the global edge order is `(source, kind, target)`; the reverse CSR is sorted per slice by `(kind, source key)` with a counting sort on kind (33 buckets), then a stable sort by key inside each kind run (slices are small). `kind_mask[v]` has bit `k` set iff `v` has an edge of kind `k` in that direction.
- **Complexity.** `build()` is O(V log V + E log E) for the global sorts and O(V + E) for everything else. Input that is already sorted (the linker's output) makes the sorts linear in practice (`sort_unstable` on presorted data).
- **Dangling edges.** `build()` returns `GraphBuildError::DanglingEdge { identity }` if an edge endpoint is not a node. The linker never emits these; deltas are checked by CG-012.
- `edges_owned_by(file) -> &[EdgeIx]`: every edge with `origin_file == Some(f)`, used for per-file replacement in incremental updates.
- `unresolved_by_name` indexes `UnresolvedRef.name` for INC-006 lookups.
- `heap_size_bytes()`: sum of `Vec::capacity * size_of`, interner bytes and hash-map estimates; accurate within ±15% (tested against a counting allocator in the bench).
- Memory target at 1M nodes / 5M edges: ≤ 1.2 GB (≈56 MB nodes, 140 MB edges, 2×(4 MB + 20 MB) CSR, 8 MB masks ×2, by_key map ~40 MB, strings ~150 MB).
- `Graph` is `Send + Sync` and shared as `Arc<Graph>`; there is no interior mutability.

**Data model changes:** None.

**API/protocol changes:** None (internal crate API).

**Concurrency semantics:** Builder is single-threaded (`&mut self`). The linker may build per-file edge vectors in parallel and feed them in file order. `Graph` is immutable and freely shared across tokio tasks and rayon threads.

**Failure behavior:** `GraphBuildError::{KeyCollision{key, id_a, id_b}, DanglingEdge{identity}, TooManyNodes /* > u32::MAX-1 */, SchemaVersionMismatch}`. A key collision (blake3-128) is reported, never silently merged.

**Idempotency considerations:** Building from the same multiset of inputs in any order yields a byte-identical `Graph` (asserted via CG-011 serialization hash).

**Security considerations:** No I/O. Index arithmetic uses checked conversions (`u32::try_from`) so a pathological repository returns `TooManyNodes` instead of wrapping.

**Observability additions:** span `graph.build` (attributes `nodes`, `edges`, `files`, `unresolved`, `heap_bytes`); histogram `graph_build_duration_seconds`; gauge `graph_heap_bytes` (recorded by callers that keep the graph).

**Tests required:**
- `build_is_order_independent` (proptest: shuffled inputs → identical serialized bytes).
- `csr_forward_reverse_symmetry` (every forward edge appears once in reverse).
- `kind_mask_matches_slices`.
- `file_node_ranges_are_contiguous_and_complete`.
- `edges_owned_by_file_partitions_owned_edges`.
- `duplicate_edge_identity_merges_occurrences`.
- `key_collision_is_reported` (forced via test-only constructor).
- `dangling_edge_is_rejected`.
- `heap_size_estimate_within_15_percent`.

**Benchmarks:** `graph_build/{10k,100k,1m}_nodes` (criterion, via `testkit::synthetic(nodes, avg_degree=5, seed)`); records time and `heap_size_bytes`. Targets: 1M nodes / 5M edges build < 4 s single-threaded in the engine container; memory ≤ 1.2 GB.

**Acceptance criteria:** Tests pass; benchmark numbers recorded in `benchmarks/perf/README.md` baseline table; memory target met or a deviation recorded with a PERF task ID.

**Definition of done:** Global DoD; `testkit` feature documented; benchmark baseline committed.

---

---

### CG-005 — Linker: resolve IR references to edges
Status: ◐
> **Implementation note:** Code and tests are in place: `tests/linker_fixture.rs` builds a nine-file TypeScript-shaped repository in memory covering the whole cascade (import through a barrel, `export *`, a re-export cycle, `this.x()`, constructor injection, unique and ambiguous names inside and outside the fan-out limit, an external package, inheritance with an override), asserts the precision/recall thresholds against a hand-labelled expectation, and commits an insta golden for review. Still outstanding and outside this crate's lane: `fixtures/repositories/graph-linker/README.md` and the golden reviewed against that real fixture, plus the benchmark row.

**Task ID:** CG-005

**Title:** `codegraph::linker` — resolve `IrReference`s across files into typed edges using a `ModuleResolver`, per-file symbol tables and a repository name index; deterministic tie-breaking; unresolved refs retained.

**Problem:** Per-file IR (TSA) has local symbols and textual references. The graph needs cross-file `CALLS`/`IMPORTS`/`EXTENDS`/`USES_TYPE`… edges with calibrated confidence, and the result must be identical whether computed for the whole repository or for a subset of files (incremental, INC-005/006).

**Why it exists:** Target-architecture §3.1 (linker consumes ParsedUnits + ModuleResolver); ADR-004 (deterministic resolution, name index supporting deltas); risk R1 (edge precision).

**Scope:**
- `SymbolTable` (per-file local id → key; per-file export table incl. re-exports; class member tables; constructor-parameter types).
- `NameIndex` (top-level and member name → sorted candidate keys).
- The resolution cascade, structural edges, `OVERRIDES` post-pass, external package edges.
- `UnresolvedRef` with reasons.
- A per-file entry point (`link_file`) that the incremental crate reuses, plus `link_all`.
- A new fixture repository with hand-labelled expected edges.

**Explicit non-scope:** framework facts (CG-006); the TypeScript semantic provider (ADR-007, later task); name-index deltas (INC-004); `ModuleResolver` implementation (TSA-009).

**Files/modules expected to change:** `engine/crates/codegraph/src/lib.rs`, `engine/crates/codegraph/Cargo.toml` (depend on `analysis-ir`, `rayon`).

**New files/modules expected:** `engine/crates/codegraph/src/linker/{mod.rs, symbol_table.rs, name_index.rs, resolve.rs, structural.rs, overrides.rs, unresolved.rs}`, `engine/crates/codegraph/tests/linker_fixture.rs`, `fixtures/repositories/graph-linker/` (≈20 TS files: imports/re-exports/barrels, default exports, `this.x()`, DI constructor params, type annotations, unique/ambiguous names, inheritance + overrides, external packages, a circular re-export) with `expected-edges.json`.

**Dependencies:** CG-004, TSA-001 (IR types, `ModuleResolver` trait), TSA-009 (resolver impl used in fixture tests).

**Implementation details:**
```rust
pub struct LinkInput<'a> { pub units: &'a [Arc<ParsedUnit>], pub resolver: &'a dyn ModuleResolver, pub cfg: &'a LinkConfig }
pub struct LinkConfig { pub max_ambiguous_fanout: u8 /* 3 */, pub max_reexport_depth: u8 /* 8 */ }
pub struct SymbolTable { files: HashMap<RepoPath, FileSymbols> }
pub struct FileSymbols { pub file_key: NodeKey, pub locals: Vec<NodeKey> /* by IrSymbol.local_id */,
                         pub exports: BTreeMap<SmolStr, ExportTarget>, pub members: HashMap<NodeKey, BTreeMap<SmolStr, NodeKey>>,
                         pub ctor_params: HashMap<NodeKey /*class*/, BTreeMap<SmolStr /*field*/, TypeRef>>, pub supers: HashMap<NodeKey, Vec<TypeRef>> }
pub enum ExportTarget { Local(NodeKey), ReExport { specifier: SmolStr, name: SmolStr }, StarFrom(SmolStr) }
pub struct NameIndex { top: HashMap<SmolStr, SmallVec<[NodeKey; 2]>>, members: HashMap<SmolStr, SmallVec<[NodeKey; 4]>> } // values sorted
pub struct FileLinkResult { pub path: RepoPath, pub edges: Vec<Edge>, pub unresolved: Vec<UnresolvedRef>, pub deps: ResolutionDeps }
pub struct ResolutionDeps { pub names: BTreeSet<SmolStr>, pub files: BTreeSet<RepoPath> }   // what this file's resolution consulted
pub struct UnresolvedRef { pub file: RepoPath, pub ordinal: u32, pub from: Option<NodeKey>, pub name: SmolStr,
                           pub kind: IrRefKind, pub import_specifier: Option<SmolStr>, pub location: Location,
                           pub reason: UnresolvedReason, pub candidate_count: u16 }
pub enum UnresolvedReason { External, NotFound, Ambiguous, ResolverError, ReexportCycle, DepthExceeded }
impl Linker {
    pub fn build_tables(units: &[Arc<ParsedUnit>]) -> (SymbolTable, NameIndex);              // O(S)
    pub fn link_file(unit: &ParsedUnit, t: &SymbolTable, n: &dyn NameLookup, r: &dyn ModuleResolver, c: &LinkConfig) -> FileLinkResult;
    pub fn link_all(input: LinkInput) -> LinkOutput;                                        // rayon over files, ordered collect
}
pub trait NameLookup { fn top(&self, name: &str) -> &[NodeKey]; fn members(&self, name: &str) -> &[NodeKey]; } // impl by NameIndex and INC-004 overlay
```
- **Resolution cascade** for each `IrReference` (first success wins; each step records `ResolvedBy`):
  1. `Import`: the reference name is bound by an `IrImport` → `resolver.resolve(file, specifier)` → target file → `exports[name]` → follow `ReExport`/`StarFrom` chains (cycle detection with a visited set; `max_reexport_depth`) → `Local(key)`. A bare/package specifier resolves to `pkg:` → emit `IMPORTS File→ExternalDependency` and `DEPENDS_ON`, and record the member reference as `UnresolvedReason::External`.
  2. `ThisMember`: `receiver_hint == This` → enclosing class member table, then superclasses resolved via `supers` (walk ≤ 8 levels, nearest wins).
  3. `DiConstructor`: `receiver_hint == ThisField(f)` where `f` is a constructor parameter with type `T` → resolve `T` (import/local) → member lookup on `T` and its supers.
  4. `TypeAnnotation`: receiver is a local/param with an annotated type → same as 3.
  5. `NameUnique`: `NameLookup` returns exactly one candidate of a compatible kind class (callables for calls, types for type refs).
  6. `NameAmbiguous`: 2..=`max_ambiguous_fanout` candidates → one edge per candidate (confidence 0.3 each, flag `DYNAMIC`); more → `UnresolvedReason::Ambiguous` with `candidate_count`.
  7. Otherwise `NotFound`.
- **Kind mapping:** `call → CALLS`; `new → CALLS` + `INSTANTIATES` (target = class, or its constructor if present); `decorator → REFERENCES` + `DECORATOR`; `type → USES_TYPE` (`RETURNS_TYPE`/`ACCEPTS_TYPE` when the IR position says so); `extends → EXTENDS`; `implements → IMPLEMENTS`; `throw new X → THROWS`; `catch (e: X) → CATCHES`; plain identifier read → `REFERENCES`.
- **Structural edges** (`ResolvedBy::Structural`, provenance `Analyzer`): `Repository CONTAINS Directory`, `Directory CONTAINS File|Directory`, `File CONTAINS/DECLARES` top-level symbols, `Class CONTAINS` members, `File EXPORTS` exported symbols, `File IMPORTS File` for every resolved relative import.
- **OVERRIDES post-pass:** for each `EXTENDS`/`IMPLEMENTS` edge, a member of the subtype with the same name and kind class as a member of the supertype gets `OVERRIDES` (confidence = min of the EXTENDS edge and `Structural`).
- **Determinism:** candidates sorted by `NodeKey` bytes; files processed in path order; per-file results concatenated in path order; `HashMap` is never iterated to produce output (only `BTreeMap`/sorted vecs).
- **Self-containment for incremental use:** `link_file` reads only `SymbolTable`, `NameLookup` and the resolver; `deps` records every name looked up via `NameLookup` and every file whose export table was consulted. `link_all == concat(link_file(f) for f in files)` is asserted by test.
- **Complexity:** tables O(S); linking O(R · (log C + d_reexport)) where R = references, C = candidates per name; parallel over files with rayon.

**Data model changes:** None (persistence of `unresolved` via GS-002/GS-004).

**API/protocol changes:** None.

**Concurrency semantics:** `link_all` uses `par_iter` over files with read-only shared tables; results collected in input order (`collect::<Vec<_>>()` on an indexed parallel iterator preserves order). Must be called from `spawn_blocking` when used inside tokio (IDX-001).

**Failure behavior:** Resolver errors for one specifier become `UnresolvedReason::ResolverError` on the affected references only; linking never fails as a whole. Re-export cycles → `ReexportCycle`. Panics inside `link_file` are not caught here (no panicking code paths; clippy `unwrap_used` deny).

**Idempotency considerations:** Pure function of `(units, resolver config, LinkConfig, LINKER_VERSION)`.

**Security considerations:** The resolver must not resolve outside the repository root (TSA-009 contract); the linker additionally rejects targets whose path is not in `SymbolTable` (treated as `NotFound`). No file I/O in the linker.

**Observability additions:** span `graph.link` (attributes `files`, `references`, `edges`, `unresolved`, `linker_version`); counters `linker_edges_total{resolved_by}`, `linker_unresolved_total{reason}`; histogram `graph_link_duration_seconds`.

**Tests required:**
- `fixture_expected_edges_precision_recall` (graph-linker fixture: precision ≥ 0.95 and recall ≥ 0.90 on hand-labelled `CALLS`/`USES_TYPE`/`EXTENDS`/`IMPLEMENTS`).
- `import_through_barrel_and_star_reexport`.
- `reexport_cycle_is_unresolved_not_infinite`.
- `this_member_resolves_to_nearest_super`.
- `di_constructor_param_type_resolves_member`.
- `ambiguous_name_fans_out_up_to_limit`.
- `ambiguous_over_limit_is_unresolved_with_count`.
- `external_package_creates_pkg_node_and_unresolved_external`.
- `overrides_post_pass`.
- `link_all_equals_concat_link_file`.
- `link_is_deterministic_across_thread_counts` (1 vs 8 rayon threads, identical output).
- `resolution_deps_record_consulted_names_and_files`.
- insta golden `graph_linker_fixture_edges.snap`.

**Benchmarks:** `linker/link_all_{1k,10k}_files` on `testkit` synthetic IR; target ≥ 50k references/s/core.

**Acceptance criteria:** Tests pass; precision/recall thresholds met on the fixture; golden snapshot reviewed; benchmark recorded.

**Definition of done:** Global DoD; fixture committed with its labelling notes (`fixtures/repositories/graph-linker/README.md`).

---

---

### CG-006 — Framework-fact mapping to generic nodes and edges
Status: ◐
> **Implementation note:** Code and tests are in place: `tests/framework_mapping.rs` drives `FrameworkMapper` from hand-built facts (endpoints, controllers, guards, queues, entities, tests, config) so the attribute contract is what is under test rather than any adapter, and a source scan asserts that no framework identifier leaked into the crate. Still outstanding and outside this crate's lane: `docs/graph-schema/framework-facts.md` and the NestJS fixture snapshot owned by `lang-typescript`.

**Task ID:** CG-006

**Title:** Map `IrFrameworkFact`s to generic graph constructs: `APIEndpoint`/`HANDLED_BY`/`ROUTES_TO`, `Middleware`/`AUTHORIZES`, `Queue`/`PRODUCES_JOB`/`CONSUMES_JOB`, `DatabaseTable`/`READS_TABLE`/`WRITES_TABLE`, `TestCase`/`TESTS`, `EnvironmentVariable`/`READS_CONFIG`, plus kind refinement (C3).

**Problem:** NestJS/TypeORM/BullMQ/Jest knowledge must stay in `lang-typescript` (target-architecture §2.1 rule), but the graph must still contain endpoints, guards, queues, tables, tests and config reads so that impact (IMP) and verification (VER) can reason about them.

**Why it exists:** ADR-006 ("the linker maps framework facts onto generic node and edge kinds"); PRD §17/§18 framework node and edge kinds; the auth-bypass golden scenario (§151) depends on `AUTHORIZES` and `HANDLED_BY`.

**Scope:**
- `FrameworkMapper` consuming generic fact categories (`route`, `controller`, `guard`, `queue_producer`, `queue_consumer`, `entity`, `db_access`, `env_read`, `test_suite`, `test_case`, `module`, `provider`), with an attribute contract per category.
- Synthetic node creation using CG-001 IDs, and node-kind refinement.
- Per-file versus global facts, so incremental updates can re-evaluate global ones.

**Explicit non-scope:** detecting the facts (NEST-001..006); test mapping by path convention (IMP-005); `Configuration` file nodes (later task); Express/other frameworks.

**Files/modules expected to change:** `engine/crates/codegraph/src/linker/mod.rs` (invoke mapper after reference resolution).

**New files/modules expected:** `engine/crates/codegraph/src/framework/{mod.rs, contract.rs, http.rs, auth.rs, queue.rs, db.rs, test.rs, config.rs}`, `engine/crates/codegraph/tests/framework_mapping.rs`, `docs/graph-schema/framework-facts.md` (the attribute contract that analyzers must emit).

**Dependencies:** CG-005, TSA-001 (`IrFrameworkFact`). The NestJS fixture from NEST-001..006 is used for end-to-end assertions (soft dependency: unit tests use hand-built facts).

**Implementation details:**
```rust
pub struct FrameworkMapper;
pub struct FactContext<'a> { pub unit: &'a ParsedUnit, pub tables: &'a SymbolTable, pub file_edges: &'a [Edge] /* resolved edges of this file */ }
pub struct FrameworkOutput { pub nodes: Vec<SyntheticNode>, pub edges: Vec<Edge>, pub refinements: Vec<(NodeKey, NodeKind)>,
                             pub global_facts: Vec<GlobalFact>, pub issues: Vec<FactIssue> }
pub enum GlobalFact { GlobalGuard { guard: NodeKey }, GlobalPrefix { prefix: SmolStr } }
impl FrameworkMapper {
    pub fn map_file(ctx: &FactContext) -> FrameworkOutput;                                   // per-file, deterministic
    pub fn apply_globals(globals: &[GlobalFact], endpoints: &[(NodeKey, &SyntheticNode)]) -> Vec<Edge>; // O(G × endpoints)
}
```
- **Attribute contract** (`contract.rs`, validated; violations become `FactIssue` diagnostics, not errors):

| Category | Required attrs | Graph output |
|---|---|---|
| `controller` | `symbol` | refine Class → `Controller` |
| `route` | `symbol` (handler), `method`, `path` (full, prefixes applied by adapter) | node `http:{METHOD} {path}` kind `ApiEndpoint`; `HANDLED_BY` endpoint→handler; `ROUTES_TO` endpoint→controller class (if `controller` attr); handler refined → `Handler` only when it is a Function (methods stay `Method`) |
| `guard` | `symbol` (guard class/fn), `targets` (handler or controller symbols) or `scope=global` | refine → `Middleware`; `AUTHORIZES` guard→each endpoint handled by a target (controller-level guards fan out to all its endpoints; `GLOBAL_SCOPE` flag for global) |
| `queue_producer` | `symbol` (calling fn/method), `queue`, `job?` | node `queue:{name}`; `PRODUCES_JOB` symbol→queue; job names collected in queue node `attrs.jobs` (sorted set) |
| `queue_consumer` | `symbol` (processor class or handler method), `queue`, `job?` | refine class → `QueueConsumer`, method → `JobHandler`; `CONSUMES_JOB` handler→queue |
| `entity` | `symbol`, `table`, `schema?` | refine → `DatabaseEntity`; node `db:{schema}.{table}`; `REFERENCES`+`MAPS_TABLE` entity→table |
| `db_access` | `symbol` (caller), `entity` (type name or symbol), `op` = read\|write | resolve entity → its table via the entity fact (through `SymbolTable`); `READS_TABLE`/`WRITES_TABLE` caller→table |
| `env_read` | `symbol`, `name` | node `env:{NAME}`; `READS_CONFIG` symbol→env |
| `test_suite`/`test_case` | `suite_path`, `name`, `range` | nodes `test:{file}#…` kinds `TestSuite`/`TestCase`; `CONTAINS` file→suite→case; `TESTS` case→each non-test symbol called inside its range (from `file_edges`), confidence `derived(Framework, call_edge.confidence)` |

- Edges get `Provenance::Framework` and `ResolvedBy::Framework`, except derived edges (TESTS, db access through a resolved call) which use `confidence::derived`.
- Synthetic nodes are owned by no file; their existence is reference-counted by incident edges (a node with zero incident edges is dropped at build time, and removed by INC-007 deltas).
- Global facts (APP_GUARD-style guards, global route prefixes) are returned separately and applied after all files are mapped (`apply_globals`), so INC-005 can re-run them cheaply whenever the endpoint set changes.
- No NestJS identifiers (`@Controller`, `UseGuards`, …) appear anywhere in `codegraph` (enforced by a test that greps the crate sources).
- Complexity: O(facts + edges of the file) per file; globals O(G × E_p) where E_p = endpoints.

**Data model changes:** None (synthetic nodes persisted via GS-003 `synthetic_nodes`).

**API/protocol changes:** `docs/graph-schema/framework-facts.md` becomes the contract that `lang-typescript` adapters (and future languages) must emit; the TS adapter tasks reference it.

**Concurrency semantics:** `map_file` is pure and runs inside the linker's parallel per-file loop; `apply_globals` runs once, single-threaded.

**Failure behavior:** Missing or invalid attrs → `FactIssue { code, file, range }` recorded as parse diagnostics (IDX-003) and the fact is skipped. An `entity` reference that cannot be resolved → no table edge, `FactIssue::UnresolvedEntity`.

**Idempotency considerations:** Deterministic output (facts processed in source order; job-name sets sorted; endpoints sorted by key for global fan-out).

**Security considerations:** `env_read` records names only; values are never read (the analyzer must not evaluate `.env` files). Route paths come from source literals and are normalized; no network access.

**Observability additions:** counters `framework_facts_total{category}`, `framework_fact_issues_total{code}`; span attribute `framework_nodes` on `graph.link`.

**Tests required:**
- `route_fact_creates_endpoint_handled_by_and_routes_to`.
- `controller_level_guard_authorizes_all_its_endpoints`.
- `global_guard_applies_to_endpoints_added_later` (apply_globals over an extended endpoint set).
- `queue_producer_and_consumer_share_queue_node`.
- `entity_and_db_access_create_table_edges_with_ops`.
- `env_read_creates_env_node_without_value`.
- `test_case_tests_edges_derive_confidence_from_calls`.
- `refinement_keeps_symbol_key`.
- `invalid_fact_attrs_produce_issue_not_error`.
- `codegraph_contains_no_framework_identifiers`.
- End-to-end (NestJS fixture): `nest_fixture_framework_subgraph` insta snapshot.

**Benchmarks:** None separate (covered by linker benchmark).

**Acceptance criteria:** Tests pass; the NestJS fixture snapshot shows every route as `ApiEndpoint` with exactly one `HANDLED_BY`, guarded routes have `AUTHORIZES`, and the auth-bypass fixture base graph contains the guard→endpoint edge the §151 scenario depends on.

**Definition of done:** Global DoD; `framework-facts.md` merged and linked from `docs/graph-schema/edge-kinds.md`.

---

---

### CG-007 — GraphQuery trait and basic queries
Status: ◐
> **Implementation note:** Code and tests are in place: `GraphQuery` is object-safe and implemented for both `Graph` and `GraphOverlay`, and `let _: &dyn GraphQuery = &graph;` compiles in the tests, which is the acceptance shape downstream crates consume. Still outstanding and outside this crate's lane: the benchmark row.

**Task ID:** CG-007

**Title:** `GraphQuery` trait: node lookup, out/in edges, filtered neighbors (edge kinds incl. reverse views, `min_confidence`), implemented by `Graph` (and later `GraphOverlay`).

**Problem:** Consumers (impact, context, verification, the review-engine API) must query base graphs and PR head overlays through one interface, without knowing which they hold.

**Why it exists:** Target-architecture §3.3 "Queries"; PRD §102 (node lookup, neighbor and reverse-edge traversal, filtered traversal).

**Scope:**
- The `GraphQuery` trait (dyn-compatible, visitor style so no allocation per call).
- Borrowed views `NodeRef`/`EdgeRef`.
- `Direction`, `EdgeFilter`, a `GraphQueryExt` extension trait with collecting helpers.
- The `Graph` implementation.

**Explicit non-scope:** BFS/paths (CG-008/009); overlay implementation (CG-010); SQL neighbors (GS-005).

**Files/modules expected to change:** `engine/crates/codegraph/src/lib.rs`, `engine/crates/codegraph/src/graph/mod.rs`.

**New files/modules expected:** `engine/crates/codegraph/src/query/{mod.rs, filter.rs, views.rs}`, `engine/crates/codegraph/tests/query_basic.rs`.

**Dependencies:** CG-004.

**Implementation details:**
```rust
pub enum Direction { Out, In, Both }
pub struct EdgeFilter { pub kinds: EdgeKindSet, pub min_confidence: Confidence }
impl EdgeFilter { pub fn from_selectors(sel: &[EdgeSelector]) -> (EdgeFilter /*out*/, EdgeFilter /*in, from views*/); }
pub struct NodeRef<'g> { pub key: NodeKey, pub kind: NodeKind, pub id: &'g str, pub name: &'g str,
                         pub qualified_name: &'g str, pub file: Option<&'g str>, pub range: Option<SourceRange>, pub attrs: &'g NodeAttrs }
pub struct EdgeRef<'g> { pub kind: EdgeKind, pub source: NodeKey, pub target: NodeKey, pub confidence: Confidence,
                         pub resolved_by: ResolvedBy, pub provenance: Provenance, pub flags: EdgeFlags,
                         pub location: Option<(&'g str, u32, u32)>, pub occurrences: u32, pub origin_file: Option<&'g str> }
pub trait GraphQuery: Send + Sync {
    fn schema_version(&self) -> u32;
    fn node(&self, key: NodeKey) -> Option<NodeRef<'_>>;
    fn node_by_id(&self, id: &str) -> Option<NodeRef<'_>>;               // hashes id -> key
    fn for_each_edge(&self, key: NodeKey, dir: Direction, f: &EdgeFilter,
                     visit: &mut dyn FnMut(EdgeRef<'_>) -> ControlFlow<()>);
    fn degree(&self, key: NodeKey, dir: Direction, f: &EdgeFilter) -> usize;
    fn node_count(&self) -> usize; fn edge_count(&self) -> usize;
    fn for_each_node(&self, visit: &mut dyn FnMut(NodeRef<'_>));         // canonical key order
    fn nodes_in_file(&self, path: &str, visit: &mut dyn FnMut(NodeRef<'_>));
    fn edges_owned_by(&self, path: &str, visit: &mut dyn FnMut(EdgeRef<'_>));
    fn unresolved_named(&self, name: &str, visit: &mut dyn FnMut(&UnresolvedRef));
    fn file(&self, path: &str) -> Option<FileView<'_>>;                  // path, content_hash, language, file_version_id
}
pub trait GraphQueryExt: GraphQuery {   // blanket impl for all T: GraphQuery + ?Sized
    fn out_edges(&self, k: NodeKey, kinds: EdgeKindSet) -> Vec<EdgeRef<'_>>;
    fn in_edges(&self, k: NodeKey, kinds: EdgeKindSet) -> Vec<EdgeRef<'_>>;
    fn neighbors(&self, k: NodeKey, dir: Direction, kinds: EdgeKindSet, min_conf: Confidence) -> Vec<NodeKey>;
    fn view(&self, k: NodeKey, v: ReverseView) -> Vec<EdgeRef<'_>>;    // CALLED_BY etc.
}
```
- `Graph::for_each_edge`: `by_key` lookup O(1); kind-mask test (skip the node in O(1) if `mask & kinds == 0`); for each requested kind, `partition_point` on the kind-sorted slice finds the run (O(log d) per kind); then confidence filter. Total O(|kinds| · log d + matched).
- Iteration order is deterministic: by kind discriminant, then by the other endpoint's key. `Both` yields `Out` then `In`.
- `neighbors` de-duplicates (a node reachable by two kinds appears once, sorted by key).
- Reverse views translate to `(underlying kind, In)` via `EdgeFilter::from_selectors`.

**Data model changes:** None.

**API/protocol changes:** None directly; review-engine endpoints (API tasks) call this trait.

**Concurrency semantics:** Read-only `&self`; `Send + Sync`; callable concurrently from many tasks.

**Failure behavior:** Unknown keys yield `None`/no visits, never errors. A visitor returning `ControlFlow::Break` stops iteration.

**Idempotency considerations:** Pure reads.

**Security considerations:** None in-crate (tenant scoping is done by whoever selects the snapshot).

**Observability additions:** None per call (hot path). Callers wrap batches in their own spans.

**Tests required:**
- `out_and_in_edges_respect_kind_filter`.
- `min_confidence_filters_edges`.
- `reverse_views_equal_in_edges_of_underlying_kind`.
- `iteration_order_is_kind_then_key`.
- `neighbors_dedupes_and_sorts`.
- `visitor_break_stops_early`.
- `unknown_key_is_empty`.
- `node_by_id_matches_node_by_key`.
- `graph_query_is_object_safe`.
- proptest `query_matches_naive_edge_scan` (random graph; compare with an O(E) filter over all edges).

**Benchmarks:** `query/neighbors_p50_p95` on the 1M-node synthetic graph; target p95 < 5 µs for degree ≤ 50.

**Acceptance criteria:** Tests pass; benchmark recorded; `let _: &dyn GraphQuery = &graph;` compiles.

**Definition of done:** Global DoD.

---

---

### CG-008 — Bounded BFS
Status: ◐
> **Implementation note:** Code and tests are in place: `tests/bfs_props.rs` passes at `PROPTEST_CASES=1000` (and as a 20000-case soak), and truncation is reported explicitly rather than inferred - including the case where the budget is consumed exactly, which the property caught. The two-hop hub walk is measured by `benches/bfs.rs` against the documented p95 < 2 ms target; still outstanding and outside this crate's lane: the recorded baseline row.

**Task ID:** CG-008

**Title:** `bounded_bfs(seeds, direction, kinds, max_depth, max_nodes, min_confidence)` returning visits with parent pointers, reconstructable paths and an explicit `truncated` flag.

**Problem:** Impact analysis needs multi-hop reachability (callers ≤ 2, endpoint reachability) without unbounded work on hub nodes.

**Why it exists:** Target-architecture §3.3 ("every traversal takes an explicit budget and returns `truncated: bool`"); master-plan principle 4 (truncation reported, never silent); the legacy the reference consumer `getImpact` ignored confidence (audit §4.4).

**Scope:** `TraversalSpec`, `TraversalResult`, the algorithm over any `&dyn GraphQuery`, path reconstruction, path-confidence aggregation, and budget property tests.

**Explicit non-scope:** shortest path between two nodes (CG-009); impact semantics such as per-kind depth policies (IMP tasks compose several BFS calls).

**Files/modules expected to change:** `engine/crates/codegraph/src/query/mod.rs`.

**New files/modules expected:** `engine/crates/codegraph/src/query/bfs.rs`, `engine/crates/codegraph/tests/bfs_props.rs`, `engine/crates/codegraph/benches/bfs.rs`.

**Dependencies:** CG-007.

**Implementation details:**
```rust
pub struct TraversalSpec { pub seeds: Vec<NodeKey>, pub direction: Direction, pub kinds: EdgeKindSet,
                           pub max_depth: u8, pub max_nodes: u32, pub max_edges_examined: u32 /* default 50 × max_nodes */,
                           pub min_confidence: Confidence }
pub struct Visit { pub key: NodeKey, pub depth: u8, pub parent: Option<u32> /* index into visits */,
                   pub via: Option<EdgeStep>, pub path_confidence: Confidence /* min along path */ }
pub struct EdgeStep { pub kind: EdgeKind, pub source: NodeKey, pub target: NodeKey, pub confidence: Confidence, pub resolved_by: ResolvedBy }
pub struct TraversalResult { pub visits: Vec<Visit>, pub truncated: bool, pub truncation: Option<Truncation>,
                             pub frontier_at_max_depth: u32, pub edges_examined: u32 }
pub enum Truncation { MaxNodes, MaxEdgesExamined }
pub fn bounded_bfs(g: &dyn GraphQuery, spec: &TraversalSpec) -> Result<TraversalResult, TraversalError>;
impl TraversalResult { pub fn path_to(&self, key: NodeKey) -> Option<Vec<EdgeStep>>; }
```
- Seeds are de-duplicated and sorted; unknown seeds are dropped (`TraversalError::NoValidSeeds` only if *all* are unknown). Seeds have depth 0 and count toward `max_nodes`.
- FIFO BFS with a `HashMap<NodeKey, u32>` visited map. Expansion uses `for_each_edge` (deterministic order), so the first discovery of a node is a BFS-shortest path with deterministic tie-breaking.
- Stops when the queue empties, `visits.len() == max_nodes` (`Truncation::MaxNodes`) or `edges_examined == max_edges_examined` (`Truncation::MaxEdgesExamined`).
- Nodes discovered at `max_depth` are recorded but not expanded. `frontier_at_max_depth` counts those with at least one qualifying edge (O(1) via `degree` + kind mask). Reaching `max_depth` is the caller's request, **not** truncation.
- `path_confidence = min(parent.path_confidence, edge.confidence)`; seeds have 1.0.
- `Direction::Both` expands out then in; a node reached both ways keeps its first discovery.
- Complexity: O(N + X), N ≤ `max_nodes`, X ≤ `max_edges_examined`; memory O(N).
- Validation: `max_depth ≤ 16`, `max_nodes ≤ 1_000_000` (`TraversalError::BudgetTooLarge`).

**Data model changes:** None.

**API/protocol changes:** `TraversalSpec`/`TraversalResult` derive `Serialize`/`JsonSchema` for the review-engine API and contracts.

**Concurrency semantics:** Pure over `&dyn GraphQuery`; many BFS calls may run in parallel on the same `Arc<Graph>`.

**Failure behavior:** `TraversalError::{NoValidSeeds, BudgetTooLarge}`; never panics on hub nodes.

**Idempotency considerations:** Deterministic: same graph + spec → identical result (asserted).

**Security considerations:** Budget caps prevent API callers from triggering unbounded work (the API layer additionally clamps).

**Observability additions:** None in the hot path; `TraversalResult.edges_examined` lets callers emit `graph_traversal_edges_examined` histograms (IMP tasks).

**Tests required:**
- proptest `bfs_never_exceeds_max_nodes_or_edges_examined` (random graphs and budgets).
- proptest `bfs_depths_are_shortest` (vs unbounded reference BFS on small graphs).
- `max_depth_is_not_truncation`.
- `truncated_set_when_max_nodes_hit`.
- `path_to_reconstructs_edges_in_order`.
- `path_confidence_is_min_along_path`.
- `min_confidence_prunes_low_edges`.
- `deterministic_across_runs`.
- `both_direction_expansion_order`.

**Benchmarks:** `bfs/depth2_hub_node`, `bfs/depth3_max500` on the 1M synthetic graph; target p95 < 2 ms for `max_nodes = 500`.

**Acceptance criteria:** Property tests pass with `PROPTEST_CASES=1000`; benchmark recorded.

**Definition of done:** Global DoD.

---

---

### CG-009 — Shortest path and UI subgraph extraction
Status: ◐
> **Implementation note:** Code and tests are in place: `tests/path_subgraph.rs` covers minimum hops on a diamond, the difference between "no path" and "gave up", the reverse direction, the `SUBGRAPH_MAX_NODES` cap and explicit truncation, with a committed insta golden; `benches/milestones.rs` measures both at 1k and 100k nodes. Still outstanding and outside this crate's lane: the generated JSON Schemas and the benchmark row.

**Task ID:** CG-009

**Title:** `shortest_path(a, b, kinds, max_depth)` (BFS with parent pointers) and `subgraph(seeds, depth, max_nodes ≤ 500)` for the graph explorer.

**Problem:** Verification must check claims like "A reaches endpoint E" (VER graph evidence), the CLI needs `review graph path` (PRD §84), and the UI needs bounded server-side subgraphs (target-architecture §6, ≤ 500 nodes per view).

**Why it exists:** PRD §102 (path search); the external codegraph has no two-symbol path query (audit §4.3).

**Scope:** `shortest_path` with direction and kind filter; `subgraph` producing an induced, serializable subgraph; both budgeted with `truncated`.

**Explicit non-scope:** k-shortest or weighted paths; layout (frontend); authorization (API).

**Files/modules expected to change:** `engine/crates/codegraph/src/query/mod.rs`.

**New files/modules expected:** `engine/crates/codegraph/src/query/{path.rs, subgraph.rs}`, `engine/crates/codegraph/tests/path_subgraph.rs`.

**Dependencies:** CG-008.

**Implementation details:**
```rust
pub struct PathSpec { pub from: NodeKey, pub to: NodeKey, pub direction: Direction /* Out or In */, pub kinds: EdgeKindSet,
                      pub max_depth: u8, pub max_nodes: u32, pub min_confidence: Confidence }
pub struct GraphPath { pub steps: Vec<EdgeStep>, pub min_confidence: Confidence }
pub struct PathResult { pub path: Option<GraphPath>, pub truncated: bool, pub nodes_visited: u32 }
pub fn shortest_path(g: &dyn GraphQuery, s: &PathSpec) -> Result<PathResult, TraversalError>;

pub const SUBGRAPH_MAX_NODES: u32 = 500;
pub struct SubgraphSpec { pub seeds: Vec<NodeKey>, pub depth: u8, pub kinds: EdgeKindSet, pub direction: Direction,
                          pub max_nodes: u32 /* clamped to SUBGRAPH_MAX_NODES */, pub min_confidence: Confidence }
pub struct Subgraph { pub nodes: Vec<SubgraphNode>, pub edges: Vec<SubgraphEdge>, pub truncated: bool, pub seeds: Vec<NodeKey> }
pub fn subgraph(g: &dyn GraphQuery, s: &SubgraphSpec) -> Result<Subgraph, TraversalError>;
```
- `shortest_path`: unidirectional BFS from `from` with early exit when `to` is discovered; reuses the `bounded_bfs` core with a target check. O(N + X) within budget. `from == to` → empty path. `truncated` is true only if the budget ran out before the target was found; "not found within `max_depth`" is `path: None, truncated: false`.
- Bidirectional BFS is out of scope here; if p95 at depth 6 exceeds 5 ms, open a PERF follow-up.
- `subgraph`: `bounded_bfs` collects nodes (budget `max_nodes`), then emits the **induced** edges among collected nodes for the requested kinds (`for_each_edge(Out)` per node + membership check: O(Σ deg)). Nodes sorted by key, edges by identity. `SubgraphNode { key, id, kind, name, qualified_name, file, range, depth }`; `SubgraphEdge { kind, source, target, confidence, resolved_by }`.
- Keys serialize as 32-char hex (`serde(with = "hex128")`).

**Data model changes:** None.

**API/protocol changes:** `Subgraph`, `GraphPath`, `PathResult` JSON Schemas exported to `packages/contracts` (consumed by the API graph proxy and the `apps/web` Cytoscape explorer).

**Concurrency semantics:** Pure, read-only.

**Failure behavior:** Unknown `from`/`to` → `TraversalError::UnknownNode(key)`. `max_nodes` above the clamp is clamped; `truncated` reflects the clamp if it bites.

**Idempotency considerations:** Deterministic output for the same inputs.

**Security considerations:** The 500-node clamp is enforced in the engine, not only the UI, so API callers cannot request huge payloads.

**Observability additions:** None in-crate (review-engine handlers add spans `graph_api.path`, `graph_api.subgraph`).

**Tests required:**
- `shortest_path_finds_minimum_hops`.
- `shortest_path_respects_kinds_and_direction`.
- `path_not_found_within_depth_is_not_truncated`.
- `path_truncated_when_budget_exhausted`.
- `subgraph_is_induced_and_sorted`.
- `subgraph_clamps_to_500_nodes`.
- `subgraph_serialization_golden` (insta JSON on the graph-linker fixture).
- proptest `shortest_path_length_equals_bfs_depth`.

**Benchmarks:** `path/depth6_random_pairs` on the 1M synthetic graph (p50/p95 recorded).

**Acceptance criteria:** Tests pass; JSON Schemas generated; benchmark recorded.

**Definition of done:** Global DoD; contracts regenerated.

---

---

### CG-010 — GraphDelta and GraphOverlay
Status: ◐
> **Implementation note:** Code and tests are in place: `tests/overlay_equivalence.rs` proves the central property - for every node and both directions the overlay's `for_each_edge` yields exactly what flattening the overlay and reading the flattened graph yields - by proptest over random bases and random deltas, at `PROPTEST_CASES=1000`. Those tests found three real defects, now fixed: an override could never be expressed (the tombstone also suppressed its own replacement), `flatten` could emit a dangling edge when a delta removed an endpoint without removing the edge, and the delta validator skipped exactly the surviving edges it was meant to report. Still outstanding and outside this crate's lane: recording the query-overhead target or its PERF deviation; `benches/milestones.rs` measures it here (a merged `degree` read over a 50k-node base with a 200-node delta is below timer resolution).

**Task ID:** CG-010

**Title:** `GraphDelta` (added/removed nodes, edges, files, unresolved refs, lineage) and `GraphOverlay { base: Arc<Graph>, … }` implementing `GraphQuery`, plus `flatten()`.

**Problem:** A PR head must be queryable without copying the base graph (target-architecture §3.3: "a PR never copies the base").

**Why it exists:** ADR-003 (in-memory overlay for PR heads); PRD §104 (base + delta).

**Scope:**
- `GraphDelta`, the single in-memory change representation shared by incremental (INC-007), storage (GS-004/005) and compaction (GS-007).
- Overlay index structures and the `GraphQuery` implementation with merged deterministic ordering.
- `flatten() -> Graph`.
- A local delta-validity check limited to touched nodes.

**Explicit non-scope:** computing deltas (INC-005..007); persistence (GS-004); nested overlays (an overlay's base is always a materialized `Graph`; delta chains are materialized by GS-005).

**Files/modules expected to change:** `engine/crates/codegraph/src/lib.rs`.

**New files/modules expected:** `engine/crates/codegraph/src/delta.rs`, `engine/crates/codegraph/src/overlay.rs`, `engine/crates/codegraph/tests/overlay_equivalence.rs`.

**Dependencies:** CG-004, CG-007.

**Implementation details:**
```rust
pub struct GraphDelta {
    pub base_schema_version: u32,
    pub files: Vec<FileChange>,                       // sorted by path
    pub nodes_added: Vec<NodeInput>,                  // includes nodes whose data changed (same key replaces)
    pub nodes_removed: Vec<NodeKey>,
    pub edges_added: Vec<Edge>,                       // includes overrides (tombstone + add, C5)
    pub edges_removed: Vec<EdgeIdentity>,             // tombstones
    pub unresolved_replaced: Vec<(RepoPath, Vec<UnresolvedRef>)>,  // per-file replacement (C6)
    pub lineage: Vec<LineageRecord>,                  // SID-005 transitions
}
pub struct FileChange { pub path: RepoPath, pub change: FileChangeKind /* Added|Modified|Deleted|Renamed{from}|Relinked */,
                        pub file_version_id: Option<i64>, pub content_hash: Option<Hash256>, pub language: Option<Language> }
pub struct GraphOverlay {
    base: Arc<Graph>, delta: Arc<GraphDelta>,
    added_nodes: HashMap<NodeKey, NodeData>, removed_nodes: HashSet<NodeKey>,
    added_out: HashMap<NodeKey, Vec<u32>>, added_in: HashMap<NodeKey, Vec<u32>>,   // into delta.edges_added, sorted (kind, other key)
    removed_edges: HashSet<EdgeIdentity>, strings: Interner /* overlay-local */,
    file_overrides: HashMap<SmolStr, FileOverride>, unresolved_by_name: HashMap<SmolStr, Vec<(u32, u32)>>,
}
impl GraphOverlay {
    pub fn new(base: Arc<Graph>, delta: Arc<GraphDelta>) -> Result<Self, OverlayError>;
    pub fn flatten(&self) -> Result<Graph, GraphBuildError>;
    pub fn delta(&self) -> &GraphDelta; pub fn base(&self) -> &Arc<Graph>;
}
pub fn validate_delta_local(base: &Graph, d: &GraphDelta) -> Vec<ValidationIssue>;   // O(|Δ| · deg)
```
- **Semantics.** A node exists iff `(in base ∧ ∉ removed_nodes) ∨ ∈ added_nodes` (added replaces node data). An edge exists iff `(in base ∧ identity ∉ removed_edges ∧ both endpoints exist) ∨ ∈ edges_added`. `new()` rejects an added identity that already exists in base without a tombstone (`OverlayError::ImplicitOverride`), keeping deltas explicit.
- **File and unresolved overlay.** Files in `delta.files` replace base file entries (Deleted removes); `unresolved_replaced` replaces the base unresolved list of each listed path.
- **Query merge.** `for_each_edge` walks the base slice (filtered by `removed_edges` and removed endpoints; the set lookup is skipped when it is empty) and the added slice, merging both sorted streams by `(kind, other key)`, so ordering equals the flattened graph.
- **Cost.** Construction O(|Δ| log |Δ|); queries O(base cost + added degree); memory O(|Δ|).
- `flatten()`: streams base items filtered by the overlay plus added items into `GraphBuilder` → O(V + E). Used by compaction (GS-007) and by the oracle (INC-012).
- `validate_delta_local`: added edges' endpoints exist; no surviving base edge points at a removed node (base `for_each_edge(Both)` for each removed node); a `Deleted` file contributes no `nodes_added`.

**Data model changes:** None (GS-003 tables mirror these fields).

**API/protocol changes:** None.

**Concurrency semantics:** Immutable after `new()`; `Send + Sync`; shares the base `Arc<Graph>` with the LRU cache (GS-008).

**Failure behavior:** `OverlayError::{SchemaVersionMismatch, ImplicitOverride(identity)}`. Tombstones for items already absent from base are harmless: counted (`graph_overlay_noop_tombstones_total`) and ignored.

**Idempotency considerations:** The same delta on the same base yields the same overlay; `flatten` output is byte-identical across runs.

**Security considerations:** None.

**Observability additions:** span `graph.overlay.build` (attrs `nodes_added`, `nodes_removed`, `edges_added`, `edges_removed`); histogram `graph_overlay_build_duration_seconds`; counter `graph_overlay_noop_tombstones_total`.

**Tests required:**
- proptest `overlay_queries_equal_flattened_graph_queries` (random base + random valid delta; compare `for_each_edge` for every node in both directions).
- `removed_node_hides_incident_base_edges`.
- `override_requires_tombstone`.
- `added_node_replaces_base_node_data`.
- `merged_iteration_order_matches_flatten`.
- `deleted_file_hides_its_unresolved_refs`.
- `validate_delta_local_detects_dangling_added_edge`.
- `validate_delta_local_detects_surviving_edge_to_removed_node`.
- `empty_delta_overlay_equals_base`.

**Benchmarks:** `overlay/build_1k_changes_on_1m_base`, `overlay/neighbors_vs_base` (overhead target < 30% at p95).

**Acceptance criteria:** Equivalence property passes with 1,000 cases; overhead target met or recorded with a PERF task ID.

**Definition of done:** Global DoD.

---

---

### CG-011 — Graph schema version and serialization
Status: ◐
> **Implementation note:** Code and tests are in place, with one deviation. `tests/codec_roundtrip.rs` pins the 52-byte header field by field, proves that a graph and a delta round-trip and re-encode byte-identically, and rejects every corruption a record can carry: wrong magic, an unknown format/codec/payload, a non-zero reserved byte, a flipped payload or hash byte, a truncated record, a schema the caller does not speak, and an oversized payload refused before it is inflated. **Deviation:** `cargo deny check advisories` fails on `bincode 1.3.3` (`RUSTSEC-2025-0141`, unmaintained by its own maintainers), and that is the codec crate this acceptance criterion names. Replacing it needs a workspace dependency in `engine/Cargo.toml` and a baseline entry in `engine/deny.toml`, neither of which belongs to this crate's lane, so it needs a workspace-level decision. The other two advisories in that run (`quick-xml`) reach the tree through `repository`, not through this crate. Still outstanding and outside this crate's lane: `docs/graph-schema/versioning.md` and the benchmark row.

**Task ID:** CG-011

**Title:** `codegraph::SCHEMA_VERSION` and a versioned bincode+zstd codec for `Graph` and `GraphDelta`.

**Problem:** Persisted graphs must be self-describing so that an incompatible reader fails loudly and triggers a rebuild (PRD §18 "the graph schema must be versioned", §24).

**Why it exists:** ADR-015 (`graph_schema_version` is `codegraph::SCHEMA_VERSION`); target-architecture §3.4 (`.review/graph/snapshots/{id}.bin.zst`).

**Scope:** the constant and bump policy; a header format; wire representations (`GraphWire`, `DeltaWire`); encode/decode with an integrity hash and size limits; a determinism test.

**Explicit non-scope:** file layout and manifest (GS-006); Postgres encoding (GS-004).

**Files/modules expected to change:** `engine/crates/codegraph/src/lib.rs`, `engine/crates/codegraph/Cargo.toml` (`bincode` 2 with `serde`, or `postcard` per C9; `zstd`; `blake3`).

**New files/modules expected:** `engine/crates/codegraph/src/codec.rs`, `engine/crates/codegraph/src/schema.rs`, `engine/crates/codegraph/tests/codec_roundtrip.rs`, `docs/graph-schema/versioning.md`.

**Dependencies:** CG-004, CG-010.

**Implementation details:**
```rust
pub const SCHEMA_VERSION: u32 = 1;
pub const MAGIC: [u8; 4] = *b"RGGR";
struct Header { magic: [u8; 4], format: u8 /* 1 */, codec: u8 /* 1 = bincode2, 2 = postcard */,
                payload: u8 /* 1 = Graph, 2 = Delta */, reserved: u8, schema_version: u32 /* LE */,
                uncompressed_len: u64 /* LE */, blake3: [u8; 32] /* of uncompressed payload */ }   // 52 bytes, written field by field
pub fn encode_graph(g: &Graph, w: &mut impl Write) -> Result<EncodeStats, CodecError>;
pub fn decode_graph(r: &mut impl Read, limits: DecodeLimits) -> Result<Graph, CodecError>;
pub fn encode_delta(d: &GraphDelta, w: &mut impl Write) -> Result<EncodeStats, CodecError>;
pub fn decode_delta(r: &mut impl Read, limits: DecodeLimits) -> Result<GraphDelta, CodecError>;
pub struct DecodeLimits { pub max_uncompressed_bytes: u64 /* default 8 GiB */, pub expected_schema: u32 }
```
- The wire form stores the string table, files, nodes, edges and unresolved refs in canonical sorted order (not the CSR). `decode_graph` rebuilds indices through `GraphBuilder` in O(V + E) on presorted input, so the format does not depend on in-memory layout.
- zstd level 3, streaming encoder with frame checksum enabled, 1 MiB buffered writer. Because the header needs the payload hash and length, encoding serializes to a temp buffer/file first (the file store writes to a temp file anyway).
- **Bump policy** (`versioning.md`): bump `SCHEMA_VERSION` when an enum discriminant changes meaning, a variant is removed, a wire field is added or removed, or edge-identity semantics change. A new variant at an unused discriminant does not require a schema bump, but does require a `LINKER_VERSION` bump if the linker starts emitting it.
- A different `schema_version` returns `CodecError::SchemaMismatch { found, expected }`; INC-011 turns that into a full rebuild.

**Data model changes:** None (the integer is stored in `snapshots.graph_schema_version`, GS-003).

**API/protocol changes:** File format documented in `docs/graph-schema/versioning.md`.

**Concurrency semantics:** Synchronous, CPU-bound; callers use `spawn_blocking`.

**Failure behavior:** `CodecError::{BadMagic, UnsupportedFormat(u8), UnsupportedCodec(u8), SchemaMismatch{..}, TooLarge, IntegrityMismatch, Truncated, Decode(String)}`. No partial graphs are returned.

**Idempotency considerations:** Encoding is deterministic: the same graph produces identical bytes (this is the order-independence oracle used in CG-004).

**Security considerations:** Local snapshot files may be tampered with. Decoding enforces `max_uncompressed_bytes` (zip-bomb guard: the decoder is wrapped in `Read::take(limit + 1)`), verifies the blake3 hash before building, and applies codec collection-size limits (`bincode::config::standard().with_limit::<N>()` or postcard bounded reads).

**Observability additions:** spans `graph.codec.encode`, `graph.codec.decode` (attrs `bytes_compressed`, `bytes_uncompressed`, `payload`); histogram `graph_codec_duration_seconds{op}`.

**Tests required:**
- `graph_roundtrip_is_lossless` (via CG-012 `compare`).
- `delta_roundtrip_is_lossless`.
- `encoding_is_deterministic`.
- `schema_mismatch_is_rejected`.
- `bad_magic_and_truncation_are_rejected`.
- `integrity_mismatch_is_rejected` (flip one byte).
- `decompression_limit_enforced`.
- `header_layout_golden` (insta hex dump of the header for a tiny graph).

**Benchmarks:** `codec/encode_decode_1m_nodes` (time and size; targets: decode + rebuild < 5 s, compressed size < 300 MB).

**Acceptance criteria:** Tests pass; versioning doc merged; benchmark recorded; `cargo deny check advisories` clean for the chosen codec crate.

**Definition of done:** Global DoD.

---

---

### CG-012 — Consistency validator and graph compare
Status: ◐
> **Implementation note:** Code and tests are in place: `tests/validate_compare.rs` covers a healthy graph, the schema-version error, the kind-rule and orphan-synthetic warnings that must not fail a snapshot, symmetric comparison, per-field node and edge diffs, the strict/lenient distinction, a bounded render, and the three delta-validator cases. That last group found a real defect: the surviving-edge check tested its condition inverted and so reported nothing. Still outstanding and outside this crate's lane: INC-012's use of `compare(...).is_empty()` and `render(50)`, which lives with the incremental indexer.

**Task ID:** CG-012

**Title:** `validate(graph)` (dangling edges, duplicate keys, reverse-index symmetry, schema version, schema-rule warnings) and `compare(a, b)` producing a structured diff report.

**Problem:** Incremental results must be provably equal to a full rebuild (risk R3), and production needs a cheap check that can mark a snapshot `inconsistent` and force a rebuild (ADR-004).

**Why it exists:** ADR-004 ("a consistency validator is the oracle for incremental updates"); PRD §24 ("graph corruption detected" is a rebuild trigger).

**Scope:** `ValidationReport` with error/warning issues; `compare` over any two `GraphQuery` implementors (overlay vs full works without flattening); a bounded human-readable rendering.

**Explicit non-scope:** scheduling production validation (INC-013); deciding to rebuild (INC-011).

**Files/modules expected to change:** `engine/crates/codegraph/src/lib.rs`.

**New files/modules expected:** `engine/crates/codegraph/src/validate.rs`, `engine/crates/codegraph/src/compare.rs`, `engine/crates/codegraph/tests/validate_compare.rs`.

**Dependencies:** CG-004, CG-010.

**Implementation details:**
```rust
pub enum IssueCode { DanglingEdge, DuplicateNodeKey, ReverseIndexAsymmetry, SchemaVersionMismatch,
                     KindRuleViolation /* warning */, OrphanSyntheticNode /* warning */, NodeOutsideFileRange, ConfidenceOutOfRange }
pub struct ValidationIssue { pub code: IssueCode, pub severity: Severity, pub subject: String /* key/identity hex */, pub detail: String }
pub struct ValidationReport { pub errors: Vec<ValidationIssue>, pub warnings: Vec<ValidationIssue>, pub checked_nodes: u64, pub checked_edges: u64 }
pub fn validate(g: &Graph, expected_schema: u32) -> ValidationReport;               // O(V + E)
pub struct GraphDiffReport {
    pub nodes_only_a: Vec<NodeKey>, pub nodes_only_b: Vec<NodeKey>, pub nodes_differ: Vec<(NodeKey, Vec<&'static str>)>,
    pub edges_only_a: Vec<EdgeIdentity>, pub edges_only_b: Vec<EdgeIdentity>, pub edges_differ: Vec<(EdgeIdentity, Vec<&'static str>)>,
    pub unresolved_only_a: Vec<(String, u32)>, pub unresolved_only_b: Vec<(String, u32)>,
}
pub struct CompareOptions { pub ignore_locations: bool /* false */, pub ignore_file_version_ids: bool /* true */ }
pub fn compare(a: &dyn GraphQuery, b: &dyn GraphQuery, o: &CompareOptions) -> GraphDiffReport;   // O(V + E) sorted merge
impl GraphDiffReport { pub fn is_empty(&self) -> bool; pub fn render(&self, max_items: usize) -> String; }
```
- `validate` checks: every edge endpoint is a node (error); `by_key.len() == nodes.len()` (error); every forward CSR entry has exactly one reverse entry (error; O(E) with a per-edge counter); `schema_version == expected` (error); confidence in range (error); `schema_rules::allowed` (warning); synthetic nodes with zero incident edges (warning).
- `compare` walks nodes of both graphs in key order (`for_each_node` sorted merge). For nodes in both it compares `kind, id, name, qualified_name, file, range, attrs` (differing field names are listed). Edges are compared per source node via `for_each_edge(Out, ALL)` in canonical order, fields `confidence, resolved_by, provenance, flags, location, occurrences`. Unresolved refs are compared per file by `(ordinal, name, reason)`. `file_version_id` is ignored by default because full and incremental builds may reference different but content-identical rows.

**Data model changes:** None (INC-011/013 set `snapshots.status = 'inconsistent'`).

**API/protocol changes:** None.

**Concurrency semantics:** Read-only; can run in a background `spawn_blocking` task while the graph is in use.

**Failure behavior:** Never fails; always returns a report. `render` truncates output; the report itself keeps all items.

**Idempotency considerations:** Deterministic report ordering.

**Security considerations:** `render` prints keys, IDs and paths only, never source text, so it is safe for CI logs.

**Observability additions:** span `graph.validate`; counters `graph_consistency_checks_total{result=ok|error}`, `graph_consistency_issues_total{code}`.

**Tests required:**
- `valid_graph_has_no_errors`.
- `dangling_edge_detected` (via a test-only unchecked builder).
- `reverse_asymmetry_detected`.
- `schema_mismatch_detected`.
- `orphan_synthetic_node_warns`.
- `compare_identical_graphs_is_empty`.
- `compare_reports_node_field_differences`.
- `compare_reports_edge_confidence_difference`.
- `compare_reports_unresolved_differences`.
- proptest `compare_overlay_vs_flatten_is_empty`.
- `render_truncates_to_max_items`.

**Benchmarks:** `validate/1m_nodes` and `compare/1m_nodes` (target < 3 s each).

**Acceptance criteria:** Tests pass; INC-012 uses `compare(...).is_empty()` and prints `render(50)` on failure.

**Definition of done:** Global DoD.

---

---

### GS-001 — GraphStore trait and shared conformance suite
Status: ☑

> **Implementation note:** `graph-storage` mirrors the codegraph taxonomy and edge model in `kinds.rs`/`model.rs` instead of depending on `codegraph`, so the persisted discriminants are pinned on both sides (`codegraph/tests/wire_compat.rs` and the `*_seed_matches_rust_enum` migration tests). `StoreError::Backend` carries a boxed `std::error::Error` rather than `anyhow::Error`, because the workspace dependency rules (FND-004/DOM-002) forbid `anyhow` in library crates; classification still downcasts to `sqlx::Error`. The `integration` feature implies `conformance`, so CI's integration job runs the suite against `MemGraphStore`.

**Task ID:** GS-001

**Title:** The `GraphStore` port (`create_snapshot`, `write_full`, `write_delta`, `load_graph`, `load_delta`, status CAS, file-version upserts, single-hop `neighbors`) and a conformance test suite that runs unchanged against the Postgres and file adapters.

**Problem:** Two adapters (Postgres for SaaS, file for the CLI) must behave identically, or local and server reviews will diverge.

**Why it exists:** PRD §11 (GraphStore port), §102 requirements; ADR-003 consequences (trait shape); target-architecture §3.4 ("the same conformance test suite runs against both").

**Scope:**
- Trait, request/response types, error type.
- `SnapshotMeta`, `SnapshotStatus` state machine.
- An in-memory reference adapter (`MemGraphStore`) used to validate the suite itself.
- The conformance suite as a public `testkit` module with a harness trait.

**Explicit non-scope:** the Postgres adapter (GS-004/005); the file adapter (GS-006); compaction (GS-007); caching (GS-008).

**Files/modules expected to change:** `engine/crates/graph-storage/Cargo.toml` (deps: `codegraph`, `review-core`, `async-trait`, `thiserror`, `uuid`), `engine/crates/graph-storage/src/lib.rs`.

**New files/modules expected:** `engine/crates/graph-storage/src/{port.rs, types.rs, error.rs, status.rs, mem.rs}`, `engine/crates/graph-storage/src/conformance/{mod.rs, roundtrip.rs, delta.rs, status.rs, neighbors.rs, concurrency.rs}` (behind feature `conformance`), `engine/crates/graph-storage/tests/conformance_mem.rs`.

**Dependencies:** CG-010, CG-011, DOM-009 (repository/organization ids and `review-core` id types).

**Implementation details:**
```rust
#[async_trait::async_trait]
pub trait GraphStore: Send + Sync {
    async fn create_snapshot(&self, req: NewSnapshot) -> Result<SnapshotMeta, StoreError>;          // status = Pending
    async fn transition(&self, id: SnapshotId, from: SnapshotStatus, to: SnapshotStatus,
                        error: Option<&str>) -> Result<bool, StoreError>;                          // CAS; false if `from` did not match
    async fn upsert_file_versions(&self, scope: &RepoScope, files: &[FileVersionInput]) -> Result<Vec<FileVersionRef>, StoreError>;
    async fn lookup_file_versions(&self, scope: &RepoScope, keys: &[FileVersionKey]) -> Result<Vec<Option<FileVersionRef>>, StoreError>;
    async fn write_full(&self, id: SnapshotId, g: &Graph) -> Result<WriteStats, StoreError>;        // requires status Persisting
    async fn write_delta(&self, id: SnapshotId, d: &GraphDelta) -> Result<WriteStats, StoreError>;  // requires status Persisting
    async fn load_graph(&self, id: SnapshotId) -> Result<Graph, StoreError>;                        // materializes base ⊕ chain; requires Ready
    async fn load_delta(&self, id: SnapshotId) -> Result<GraphDelta, StoreError>;                   // the delta rows of one snapshot only
    async fn snapshot(&self, id: SnapshotId) -> Result<Option<SnapshotMeta>, StoreError>;
    async fn find_ready(&self, scope: &RepoScope, q: SnapshotQuery) -> Result<Option<SnapshotMeta>, StoreError>; // by commit/fingerprint/kind
    async fn chain(&self, id: SnapshotId) -> Result<Vec<SnapshotMeta>, StoreError>;                 // [full, delta1, …, id]
    async fn neighbors(&self, id: SnapshotId, key: NodeKey, dir: Direction, kinds: EdgeKindSet,
                       min_confidence: Confidence, limit: u32) -> Result<NeighborPage, StoreError>;
    async fn nodes(&self, id: SnapshotId, keys: &[NodeKey]) -> Result<Vec<Option<StoredNode>>, StoreError>;
}
pub enum SnapshotStatus { Pending, Indexing, Persisting, Ready, Failed, Inconsistent }
pub struct NewSnapshot { pub scope: RepoScope /* organization_id + repository_id */, pub commit_sha: CommitSha, pub kind: SnapshotKind,
                         pub base: Option<SnapshotId>, pub purpose: SnapshotPurpose, pub versions: SnapshotVersions /* IDX-002 */ }
pub struct SnapshotMeta { pub id, pub scope, pub commit_sha, pub kind, pub base, pub chain_depth: u16, pub status,
                          pub versions: SnapshotVersions, pub stats: SnapshotStats, pub created_at, pub completed_at }
pub enum StoreError { NotFound(SnapshotId), InvalidStatus { id, expected, found }, SchemaMismatch { found, expected },
                      ChainBroken { id }, Conflict(String), Integrity(String), Backend(#[source] anyhow::Error) }
```
- Allowed transitions (`status.rs`, single table checked by both adapters): `Pending→Indexing→Persisting→Ready`; `Pending|Indexing|Persisting→Failed`; `Ready→Inconsistent`. Everything else returns `false` from `transition` (never an error), so retries are safe.
- Readers only see `Ready` snapshots: `load_graph`/`neighbors` on a non-ready snapshot → `InvalidStatus`.
- `write_delta` requires the delta's base snapshot to be `Ready` and to belong to the same repository.
- **Conformance suite:** `pub trait Harness { async fn fresh(&self) -> Arc<dyn GraphStore>; fn name(&self) -> &str; }` and `pub async fn run_all(h: &dyn Harness)`, which runs every case below and reports all failures (not first-fail). Adapters call it from one integration test each. Fixtures come from `codegraph::testkit` (deterministic small graphs and deltas).

**Data model changes:** None (types only).

**API/protocol changes:** New internal port; `SnapshotMeta` derives `JsonSchema` for review-engine status endpoints.

**Concurrency semantics:** All methods take `&self` and are callable concurrently. The contract: concurrent `transition` calls with the same `from` → exactly one returns `true`; concurrent `upsert_file_versions` for the same key → one row, both callers get the same id.

**Failure behavior:** Typed `StoreError`; adapters must never return a partially written snapshot as `Ready`. A failed `write_*` leaves the snapshot in `Persisting` (the caller transitions it to `Failed`).

**Idempotency considerations:** `upsert_file_versions` is idempotent by key; `transition` is CAS; `write_full`/`write_delta` on a snapshot that is already `Ready` → `InvalidStatus` (callers create a new snapshot instead of rewriting one).

**Security considerations:** Every method takes or resolves a `RepoScope`; adapters must filter on `organization_id` and `repository_id`. A snapshot id from another repository → `NotFound` (no existence oracle). The suite includes cross-tenant cases.

**Observability additions:** The trait defines span names adapters must use: `graph_store.write_full`, `graph_store.write_delta`, `graph_store.load_graph`, `graph_store.neighbors` (attr `adapter = pg|file|mem`).

**Tests required (conformance cases, each a named fn in the suite):**
- `write_full_then_load_roundtrip`.
- `write_delta_then_load_equals_overlay_flatten`.
- `three_level_delta_chain_materializes`.
- `tombstone_removes_edge`.
- `edge_override_in_delta`.
- `deleted_file_removes_nodes_and_unresolved`.
- `relinked_file_replaces_unresolved_rows`.
- `status_cas_only_one_winner` (10 concurrent transitions).
- `illegal_transition_returns_false`.
- `load_non_ready_snapshot_errors`.
- `write_to_ready_snapshot_errors`.
- `delta_on_non_ready_base_rejected`.
- `neighbors_match_in_memory_query` (every node of the fixture, both directions, with kind and confidence filters).
- `nodes_lookup_matches_graph`.
- `upsert_file_versions_is_idempotent_under_concurrency`.
- `cross_repository_snapshot_is_not_found`.
- `find_ready_by_commit_and_fingerprint`.
- `conformance_mem` (the suite passes against `MemGraphStore`).

**Benchmarks:** None (adapter benchmarks in GS-004/005).

**Acceptance criteria:** `engine/scripts/cargo.sh test -p graph-storage --features conformance` passes against `MemGraphStore`; the suite is reusable from other crates' integration tests.

**Definition of done:** Global DoD; the trait is documented in `docs/graph-schema/storage.md` (new) with the status diagram.

---

---

### GS-002 — Migration: file_versions, symbols, unresolved_refs
Status: ☑

**Task ID:** GS-002

**Title:** sqlx migration creating the content-addressed per-file tables `file_versions`, `symbols` and the snapshot-scoped `unresolved_refs` (C6), with PRD §103 indexes and RLS.

**Problem:** Per-file intelligence must be stored once per `(repository, path, content_hash, analyzer_version)` and shared by every snapshot (ADR-003); without these tables nothing persists.

**Why it exists:** Target-architecture §3.4 schema; PRD §103 indexes; ADR-014 (migrations are the only schema source).

**Scope:** DDL, indexes, RLS policies, kind lookup seeds needed by `symbols.kind`, migration up-test.

**Explicit non-scope:** snapshot tables (GS-003; the `unresolved_refs.snapshot_id` FK is added there); IR blob column (IDX-005); diagnostics table (IDX-003).

**Files/modules expected to change:** None existing besides the migration directory.

**New files/modules expected:** `engine/migrations/0100_graph_file_versions.sql`, `engine/crates/graph-storage/tests/migrations_graph.rs`.

**Dependencies:** DOM-009 (initial migrations: `organizations`, `repositories`, RLS helper pattern), SID-001 (key width).

**Implementation details:**
```sql
CREATE TABLE node_kinds (id smallint PRIMARY KEY, name text NOT NULL UNIQUE);   -- seeded from codegraph::ALL_NODE_KINDS
INSERT INTO node_kinds (id, name) VALUES (0,'Repository'), (1,'Package'), /* … all 44 … */ (101,'ArchitecturalBoundary');

CREATE TABLE file_versions (
  id                bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  organization_id   uuid   NOT NULL REFERENCES organizations(id),
  repository_id     uuid   NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
  path              text   NOT NULL CHECK (path <> '' AND path !~ '(^/|\.\./|^\.\.$)'),
  content_hash      bytea  NOT NULL CHECK (octet_length(content_hash) = 32),        -- blake3-256 of bytes
  language          text   NOT NULL,
  analyzer_version  text   NOT NULL,
  parse_status      text   NOT NULL CHECK (parse_status IN ('ok','partial','failed','skipped')),
  size_bytes        integer NOT NULL CHECK (size_bytes >= 0),
  symbol_count      integer NOT NULL DEFAULT 0,
  diagnostic_count  integer NOT NULL DEFAULT 0,
  created_at        timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT file_versions_content_key UNIQUE (repository_id, path, content_hash, analyzer_version)
);
CREATE INDEX file_versions_repo_path ON file_versions (repository_id, path);                    -- PRD §103 (repository_id, file_id)

CREATE TABLE symbols (
  file_version_id   bigint NOT NULL REFERENCES file_versions(id) ON DELETE CASCADE,
  organization_id   uuid   NOT NULL,
  repository_id     uuid   NOT NULL,
  symbol_key        bytea  NOT NULL CHECK (octet_length(symbol_key) = 16),
  symbol_id         text   NOT NULL,
  kind              smallint NOT NULL REFERENCES node_kinds(id),
  name              text   NOT NULL,
  qualified_name    text   NOT NULL,
  signature         text,
  start_line integer NOT NULL, start_col integer NOT NULL, end_line integer NOT NULL, end_col integer NOT NULL,
  body_hash         bytea  CHECK (body_hash IS NULL OR octet_length(body_hash) = 16),
  signature_hash    bytea  CHECK (signature_hash IS NULL OR octet_length(signature_hash) = 16),
  parent_key        bytea  CHECK (parent_key IS NULL OR octet_length(parent_key) = 16),
  visibility        smallint NOT NULL DEFAULT 0,
  is_exported       boolean NOT NULL DEFAULT false,
  is_generated      boolean NOT NULL DEFAULT false,
  attrs             jsonb   NOT NULL DEFAULT '{}'::jsonb,
  PRIMARY KEY (file_version_id, symbol_key)
);
CREATE INDEX symbols_repo_key  ON symbols (repository_id, symbol_key);          -- PRD §103 (repository_id, symbol_id)
CREATE INDEX symbols_repo_name ON symbols (repository_id, name);                -- name lookups (API search, INC-004 cold path)

CREATE TABLE unresolved_refs (
  snapshot_id       uuid   NOT NULL,                                            -- FK added in GS-003 (C6)
  organization_id   uuid   NOT NULL,
  repository_id     uuid   NOT NULL,
  file_version_id   bigint NOT NULL REFERENCES file_versions(id) ON DELETE CASCADE,
  ordinal           integer NOT NULL,                                           -- index in ParsedUnit.references
  from_symbol_key   bytea  CHECK (from_symbol_key IS NULL OR octet_length(from_symbol_key) = 16),
  name              text   NOT NULL,
  ref_kind          smallint NOT NULL,
  import_specifier  text,
  reason            smallint NOT NULL,                                          -- UnresolvedReason discriminant
  candidate_count   smallint NOT NULL DEFAULT 0,
  line integer NOT NULL, col integer NOT NULL,
  PRIMARY KEY (snapshot_id, file_version_id, ordinal)
);
CREATE INDEX unresolved_refs_name ON unresolved_refs (snapshot_id, name);
CREATE INDEX unresolved_refs_spec ON unresolved_refs (snapshot_id, import_specifier) WHERE import_specifier IS NOT NULL;

ALTER TABLE file_versions ENABLE ROW LEVEL SECURITY;  -- + policy per DOM-009 pattern on organization_id
ALTER TABLE symbols ENABLE ROW LEVEL SECURITY;
ALTER TABLE unresolved_refs ENABLE ROW LEVEL SECURITY;
```
- `unresolved_refs` has no `removed` column: per-file replacement (C6) means a delta writes the complete unresolved set of each file it lists in `snapshot_files`, and readers ignore base rows for those files.
- `symbols.organization_id`/`repository_id` are denormalized so that RLS and the PRD §103 index work without joining `file_versions`. A trigger is not used; the writer (GS-004) fills them, and a test asserts they match the parent row.
- Sizing note (reference-api: ~15k symbols; target 1M symbols): `symbols` row ≈ 250 B → ~250 MB per million symbols per distinct file version set.

**Data model changes:** New tables `node_kinds`, `file_versions`, `symbols`, `unresolved_refs`.

**API/protocol changes:** None.

**Concurrency semantics:** The `UNIQUE (repository_id, path, content_hash, analyzer_version)` constraint is the concurrency guard for content-addressed inserts (GS-004 uses `ON CONFLICT DO NOTHING` + re-select).

**Failure behavior:** Migration runs in one transaction (sqlx default for PG); failure leaves no partial schema.

**Idempotency considerations:** sqlx records applied migrations; the migration is never edited after merge (expand/contract for later changes).

**Security considerations:** RLS enabled on all three tables with the DOM-009 `app.organization_id` policy; `path` CHECK rejects absolute paths and `..` segments.

**Observability additions:** None.

**Tests required:**
- `migrations_apply_on_empty_db` (`#[sqlx::test(migrations = "../../migrations")]`).
- `node_kinds_seed_matches_rust_enum`.
- `file_version_unique_key_enforced`.
- `symbol_key_length_check_enforced`.
- `path_check_rejects_traversal`.
- `rls_hides_other_org_rows` (set `app.organization_id`, select as the app role).
- `explain_symbols_repo_key_uses_index` (EXPLAIN on a seeded table shows `symbols_repo_key`).

**Benchmarks:** None (GS-004 measures insert throughput).

**Acceptance criteria:** Migrations apply cleanly in `docker-compose.test.yml` Postgres 16; all tests pass; `engine/migrations` remains the only DDL source (no DDL strings in Rust).

**Definition of done:** Global DoD; schema documented in `docs/graph-schema/storage.md`.

---

---

### GS-003 — Migration: snapshots, snapshot_files, graph_edges, synthetic_nodes, symbol_lineage
Status: ☑

**Task ID:** GS-003

**Title:** sqlx migration for snapshot-level tables with the PRD §103 indexes, kind lookup seeds, and the deferred `unresolved_refs.snapshot_id` FK.

**Problem:** Resolved edges, synthetic nodes, file membership and lineage belong to snapshots (ADR-003) and need indexes that make single-hop lookups fast (ADR-014: neighbour p95 < 20 ms).

**Why it exists:** Target-architecture §3.4; ADR-015 (version columns); PRD §103/§104.

**Scope:** DDL, indexes, seeds for `edge_kinds`, `resolved_by_kinds`, `provenance_kinds`, RLS, FK addition, migration tests.

**Explicit non-scope:** progress table (IDX-002); invalidations table (INC-008); partitioning (deferred: PERF task triggers it if `graph_edges` exceeds 200M rows or vacuum lag is measured).

**Files/modules expected to change:** None existing.

**New files/modules expected:** `engine/migrations/0101_graph_snapshots.sql`, additions to `engine/crates/graph-storage/tests/migrations_graph.rs`.

**Dependencies:** GS-002, CG-002 (discriminants), CG-003 (`ResolvedBy`).

**Implementation details:**
```sql
CREATE TABLE edge_kinds        (id smallint PRIMARY KEY, name text NOT NULL UNIQUE);  -- 33 rows, PRD spelling
CREATE TABLE resolved_by_kinds (id smallint PRIMARY KEY, name text NOT NULL UNIQUE);
CREATE TABLE provenance_kinds  (id smallint PRIMARY KEY, name text NOT NULL UNIQUE);

CREATE TABLE snapshots (
  id                   uuid PRIMARY KEY,
  organization_id      uuid NOT NULL REFERENCES organizations(id),
  repository_id        uuid NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
  commit_sha           text NOT NULL CHECK (commit_sha ~ '^[0-9a-f]{40}([0-9a-f]{24})?$'),
  kind                 text NOT NULL CHECK (kind IN ('full','delta')),
  base_snapshot_id     uuid REFERENCES snapshots(id),
  chain_depth          smallint NOT NULL DEFAULT 0,
  purpose              text NOT NULL CHECK (purpose IN ('default_branch','pull_request','local','compaction')),
  status               text NOT NULL CHECK (status IN ('pending','indexing','persisting','ready','failed','inconsistent')),
  graph_schema_version integer NOT NULL,
  analyzer_versions    jsonb NOT NULL,             -- {"typescript":"0.3.1","linker":"1.0.0"}
  config_hash          bytea NOT NULL CHECK (octet_length(config_hash) = 32),
  config_components    jsonb NOT NULL DEFAULT '{}'::jsonb,   -- {"source_roots":hex,"tsconfig":hex,"generated":hex,"rules":hex}
  fingerprint          bytea NOT NULL CHECK (octet_length(fingerprint) = 32),
  stats                jsonb NOT NULL DEFAULT '{}'::jsonb,
  error                text,
  created_at           timestamptz NOT NULL DEFAULT now(),
  updated_at           timestamptz NOT NULL DEFAULT now(),
  completed_at         timestamptz,
  CONSTRAINT snapshots_kind_base CHECK ((kind = 'full') = (base_snapshot_id IS NULL)),
  CONSTRAINT snapshots_depth CHECK ((kind = 'full' AND chain_depth = 0) OR (kind = 'delta' AND chain_depth >= 1))
);
CREATE INDEX snapshots_repo_commit ON snapshots (repository_id, commit_sha, status);
CREATE INDEX snapshots_base        ON snapshots (base_snapshot_id) WHERE base_snapshot_id IS NOT NULL;
CREATE UNIQUE INDEX snapshots_ready_full  ON snapshots (repository_id, fingerprint) WHERE status = 'ready' AND kind = 'full';
CREATE UNIQUE INDEX snapshots_ready_delta ON snapshots (repository_id, fingerprint, base_snapshot_id) WHERE status = 'ready' AND kind = 'delta';

CREATE TABLE snapshot_files (
  snapshot_id     uuid NOT NULL REFERENCES snapshots(id) ON DELETE CASCADE,
  organization_id uuid NOT NULL,
  path            text NOT NULL,
  file_version_id bigint REFERENCES file_versions(id),        -- NULL = deleted in this delta
  change          text NOT NULL CHECK (change IN ('present','added','modified','deleted','renamed','relinked')),
  old_path        text,
  PRIMARY KEY (snapshot_id, path)
);
CREATE INDEX snapshot_files_fv ON snapshot_files (file_version_id) WHERE file_version_id IS NOT NULL;

CREATE TABLE graph_edges (
  snapshot_id     uuid NOT NULL REFERENCES snapshots(id) ON DELETE CASCADE,
  organization_id uuid NOT NULL,
  source_key      bytea NOT NULL CHECK (octet_length(source_key) = 16),
  kind            smallint NOT NULL REFERENCES edge_kinds(id),
  target_key      bytea NOT NULL CHECK (octet_length(target_key) = 16),
  confidence      real NOT NULL CHECK (confidence >= 0 AND confidence <= 1),
  resolved_by     smallint NOT NULL REFERENCES resolved_by_kinds(id),
  provenance      smallint NOT NULL REFERENCES provenance_kinds(id),
  flags           smallint NOT NULL DEFAULT 0,
  occurrences     integer NOT NULL DEFAULT 1,
  origin_path     text,                                        -- owning file (re-link unit)
  file_version_id bigint REFERENCES file_versions(id),
  line integer, col integer,
  removed         boolean NOT NULL DEFAULT false,              -- delta tombstone
  PRIMARY KEY (snapshot_id, source_key, kind, target_key)      -- PRD §103 (…, source_node_id, edge_type)
);
CREATE INDEX graph_edges_target ON graph_edges (snapshot_id, target_key, kind);          -- PRD §103 (…, target_node_id, edge_type)
CREATE INDEX graph_edges_origin ON graph_edges (snapshot_id, origin_path) WHERE origin_path IS NOT NULL;

CREATE TABLE synthetic_nodes (
  snapshot_id uuid NOT NULL REFERENCES snapshots(id) ON DELETE CASCADE, organization_id uuid NOT NULL,
  node_key bytea NOT NULL CHECK (octet_length(node_key) = 16), node_id text NOT NULL,
  kind smallint NOT NULL REFERENCES node_kinds(id), attrs jsonb NOT NULL DEFAULT '{}'::jsonb,
  removed boolean NOT NULL DEFAULT false,
  PRIMARY KEY (snapshot_id, node_key)
);

CREATE TABLE symbol_lineage (
  id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  organization_id uuid NOT NULL, repository_id uuid NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
  from_snapshot_id uuid NOT NULL REFERENCES snapshots(id), to_snapshot_id uuid NOT NULL REFERENCES snapshots(id) ON DELETE CASCADE,
  from_key bytea NOT NULL CHECK (octet_length(from_key) = 16), to_key bytea NOT NULL CHECK (octet_length(to_key) = 16),
  transition text NOT NULL CHECK (transition IN ('renamed','moved','renamed_moved','signature_changed_moved')),
  similarity real NOT NULL CHECK (similarity >= 0 AND similarity <= 1),
  UNIQUE (to_snapshot_id, from_key, to_key)
);
CREATE INDEX symbol_lineage_from ON symbol_lineage (repository_id, from_key);
CREATE INDEX symbol_lineage_to   ON symbol_lineage (repository_id, to_key);

ALTER TABLE unresolved_refs ADD CONSTRAINT unresolved_refs_snapshot_fk
  FOREIGN KEY (snapshot_id) REFERENCES snapshots(id) ON DELETE CASCADE;
-- RLS enabled on snapshots, snapshot_files, graph_edges, synthetic_nodes, symbol_lineage (DOM-009 pattern)
```
- **PRD §103 index mapping:** the PRD's `(repository_id, …)` prefixes become `(snapshot_id, …)` because a snapshot belongs to exactly one repository and every graph query is snapshot-scoped; `symbols_repo_key` (GS-002) covers `(repository_id, symbol_id)`; `file_versions_repo_path` covers `(repository_id, file_id)`.
- `graph_edges` omits `repository_id` (derivable from the snapshot) to save 16 B/row on the largest table; RLS uses `organization_id`.
- Full snapshots list every path in `snapshot_files` with `change='present'`; deltas list only changed and re-linked paths.
- The transition list is aligned with SID-005's `SymbolTransition` variants; if SID-005 defines different names, use them and update the CHECK in the same change.

**Data model changes:** New tables `edge_kinds`, `resolved_by_kinds`, `provenance_kinds`, `snapshots`, `snapshot_files`, `graph_edges`, `synthetic_nodes`, `symbol_lineage`; FK on `unresolved_refs`.

**API/protocol changes:** None.

**Concurrency semantics:** Partial unique indexes on ready snapshots make "two workers indexed the same fingerprint" detectable at the final `pending→ready` transition (the loser gets a unique violation, marks itself `failed` with `error='duplicate'`, and callers use the winner).

**Failure behavior:** Transactional migration; no partial schema.

**Idempotency considerations:** As GS-002.

**Security considerations:** RLS on all tenant tables; `commit_sha` CHECK prevents arbitrary strings in a column that ends up in logs and URLs.

**Observability additions:** None.

**Tests required:**
- `edge_kinds_seed_matches_rust_enum`, `resolved_by_seed_matches_rust_enum`, `provenance_seed_matches_rust_enum`.
- `full_snapshot_must_not_have_base` / `delta_must_have_base_and_depth`.
- `ready_fingerprint_unique_for_full`.
- `ready_delta_unique_per_base`.
- `edge_pk_rejects_duplicate_identity`.
- `explain_neighbors_uses_pk_and_target_index`.
- `rls_hides_other_org_snapshots`.
- `cascade_delete_snapshot_removes_children`.

**Benchmarks:** None here (GS-005 measures SQL neighbour latency).

**Acceptance criteria:** Migration applies after GS-002; tests pass; EXPLAIN plans for `source_key` and `target_key` lookups are index scans.

**Definition of done:** Global DoD; `docs/graph-schema/storage.md` extended with the ER diagram and index mapping table.

---

---

### GS-004 — Postgres GraphStore: write_full and write_delta
Status: ☐

**Task ID:** GS-004

**Title:** `PgGraphStore` write path: content-addressed file-version upserts, batched COPY/UNNEST inserts of snapshot rows in a single transaction, and snapshot status transitions.

**Problem:** A full snapshot of a large repository is millions of rows; row-by-row inserts would take minutes. Deltas must be small and atomic.

**Why it exists:** ADR-003/ADR-014; PRD §102 (batch upsert, incremental edge replacement); critical path (`GS-004 → GS-005 → IDX-001`).

**Scope:** `PgGraphStore::new(pool)`; `create_snapshot`, `transition`, `upsert_file_versions` (+ symbols), `lookup_file_versions`, `write_full`, `write_delta`; row encoders; conformance harness for the write half.

**Explicit non-scope:** reads (GS-005); compaction (GS-007); IR blobs (IDX-005); job handling (IDX-004).

**Files/modules expected to change:** `engine/crates/graph-storage/Cargo.toml` (`sqlx` 0.8 with `postgres`, `runtime-tokio`, `tls-rustls`, `uuid`, `json`, `time`), `engine/crates/graph-storage/src/lib.rs`.

**New files/modules expected:** `engine/crates/graph-storage/src/pg/{mod.rs, write.rs, copy.rs, rows.rs, file_versions.rs, status.rs}`, `engine/crates/graph-storage/tests/pg_conformance.rs`, `engine/crates/graph-storage/benches/pg_write.rs`.

**Dependencies:** GS-001, GS-002, GS-003.

**Implementation details:**
```rust
pub struct PgGraphStore { pool: PgPool, cfg: PgStoreConfig }
pub struct PgStoreConfig { pub copy_chunk_rows: usize /* 50_000 */, pub statement_timeout: Duration /* 10 min for writes */ }
```
- **Tenant context:** every transaction starts with `SELECT set_config('app.organization_id', $1, true)` (transaction-local) so RLS applies.
- **`upsert_file_versions`** (one transaction per batch of ≤ 1,000 files): `INSERT INTO file_versions (...) SELECT * FROM UNNEST($1::text[], $2::bytea[], ...) ON CONFLICT ON CONSTRAINT file_versions_content_key DO NOTHING RETURNING id, path, content_hash, analyzer_version`; then a re-select of conflicting keys by the same tuple arrays. Symbols for **newly inserted** rows are COPYed in the same transaction, so a visible `file_versions` row always has its complete symbol set (invariant tested). Rows that already existed are not rewritten.
- **`write_full(id, g)`** in one transaction, guarded by `SELECT status FROM snapshots WHERE id=$1 FOR UPDATE` = `persisting`:
  1. `COPY snapshot_files (snapshot_id, organization_id, path, file_version_id, change) FROM STDIN (FORMAT binary)` — via `PgConnection::copy_in_raw`, streaming chunks of `copy_chunk_rows` encoded by `copy.rs` (PG binary COPY format: header, per-row field count + length-prefixed fields, trailer).
  2. `COPY graph_edges` (all edges, `removed=false`), `COPY synthetic_nodes`, `COPY unresolved_refs`.
  3. `UPDATE snapshots SET stats=$2, updated_at=now() WHERE id=$1`.
  Memory is O(chunk), not O(E): rows are encoded while iterating the graph.
- **`write_delta(id, d)`** in one transaction: verify base is `ready` and same repository; insert `snapshot_files` for every `FileChange` (`deleted` → `file_version_id NULL`; `relinked` → base file_version_id); `graph_edges` rows for `edges_added` (`removed=false`) and `edges_removed` (`removed=true`, other columns from the tombstone, confidence 0); `synthetic_nodes` added/removed likewise; `unresolved_refs` for every file in `unresolved_replaced`; `symbol_lineage` rows. Small deltas (< 5,000 rows total) use UNNEST inserts; larger ones use COPY (same code path as full).
- Status: callers do `transition(Indexing→Persisting)`, `write_*`, `transition(Persisting→Ready)`; `transition` is `UPDATE snapshots SET status=$3, error=$4, updated_at=now(), completed_at = CASE WHEN $3 IN ('ready','failed') THEN now() END WHERE id=$1 AND status=$2` and returns `rows_affected == 1`. A unique violation on the ready partial indexes maps to `StoreError::Conflict("duplicate fingerprint")`.
- `chain_depth` for a delta = base depth + 1, computed inside `create_snapshot`.
- Complexity: O(V + E) row encoding; one round trip per COPY chunk.

**Data model changes:** None (uses GS-002/003 tables).

**API/protocol changes:** None.

**Concurrency semantics:** Writes for different snapshots run in parallel on separate pool connections. Concurrent upserts of the same file version resolve through `ON CONFLICT`. The `FOR UPDATE` on the snapshot row serializes duplicate writers for the same snapshot id (the second sees `InvalidStatus` after the first commits `ready`).

**Failure behavior:** Any error rolls back the whole write transaction; the snapshot stays `persisting` and the caller transitions it to `failed` (IDX-001/INC-013). sqlx errors are mapped: serialization/deadlock (`40001`, `40P01`) → `StoreError::Backend` flagged retryable; unique violations → `Conflict`; statement timeout → `Backend` retryable.

**Idempotency considerations:** File-version upserts are idempotent. Writing the same snapshot twice is rejected by status; job retries create a fresh snapshot id (old `failed` ones are garbage-collected by retention, SEC-007).

**Security considerations:** No string-built SQL: all statements are static with bind parameters or COPY of encoded binary rows. RLS context set per transaction. Source text is never written (only names, signatures, hashes, ranges).

**Observability additions:** spans `graph_store.write_full`, `graph_store.write_delta`, `graph_store.upsert_file_versions` (attrs `rows_edges`, `rows_files`, `rows_symbols`, `bytes_copied`, `adapter=pg`); histograms `graph_store_write_duration_seconds{kind}`, counter `graph_store_rows_written_total{table}`.

**Tests required:**
- `pg_conformance_suite` (runs GS-001 `run_all` with a `#[sqlx::test]` harness; write-related cases must pass now, read cases after GS-005).
- `file_version_row_implies_complete_symbols` (kill the transaction mid-COPY via a failing row; assert no orphan file_versions).
- `concurrent_upsert_same_file_version_returns_same_id`.
- `write_full_rolls_back_on_error`.
- `delta_requires_ready_base`.
- `copy_binary_encoder_roundtrip` (encode rows, COPY, select back, compare).
- `duplicate_ready_fingerprint_maps_to_conflict`.
- `rls_context_set_for_writes` (write as app role without context fails).

**Benchmarks:** `pg_write/full_1m_edges` and `pg_write/delta_1k_edges` against compose Postgres; targets: ≥ 200k edge rows/s for full writes; delta of 1k rows < 100 ms p95.

**Acceptance criteria:** Write half of the conformance suite passes on Postgres 16; benchmark numbers recorded; no `format!`-built SQL in `pg/` (grep check in test).

**Definition of done:** Global DoD; benchmark baseline committed.

---

---

### GS-005 — Postgres GraphStore: load_graph and single-hop SQL neighbors
Status: ☐

**Task ID:** GS-005

**Title:** `PgGraphStore` read path: `load_graph(snapshot)` materializing `base full ⊕ delta chain`, `load_delta`, `chain`, `find_ready`, and indexed single-hop `neighbors`/`nodes` queries for the API.

**Problem:** Workers need the full in-memory graph of a base snapshot; the API needs fast neighbour lookups without loading a graph (ADR-014).

**Why it exists:** ADR-003 (readers materialize base ⊕ deltas); ADR-014 (single-hop SQL for the API; p95 < 20 ms gate); critical path.

**Scope:** chain resolution, streaming load into `GraphBuilder` with delta override sets, node reconstruction from `symbols` + `synthetic_nodes`, SQL neighbours with chain-aware dedup, conformance read cases.

**Explicit non-scope:** caching (GS-008); compaction (GS-007); multi-hop SQL (forbidden by ADR-014).

**Files/modules expected to change:** `engine/crates/graph-storage/src/pg/mod.rs`.

**New files/modules expected:** `engine/crates/graph-storage/src/pg/{read.rs, chain.rs, neighbors.rs}`, `engine/crates/graph-storage/benches/pg_read.rs`.

**Dependencies:** GS-004, CG-007.

**Implementation details:**
- **Chain:** iterative walk (`SELECT … FROM snapshots WHERE id=$1`, then follow `base_snapshot_id`) up to `chain_depth + 1` rows; ≤ 21 round trips worst case, cached per call. Missing or non-ready ancestor → `StoreError::ChainBroken`. (A recursive CTE over the *metadata* table is acceptable as an optimization; ADR-014 forbids recursive SQL for graph traversal, not for this.)
- **Load algorithm** for chain `[F, D1, …, Dn]`:
  1. Load all delta rows first (they are small): `snapshot_files`, `graph_edges`, `synthetic_nodes`, `unresolved_refs` for `D1..Dn`, applied in order into in-memory override maps: `path → Option<file_version_id>` (last writer wins), `EdgeIdentity → Option<EdgeRow>` (tombstone = `None`), `node_key → Option<SyntheticRow>`, `path → Vec<UnresolvedRow>`.
  2. Stream the base `F` rows with server-side cursors (`fetch` stream, 10k rows/batch): `snapshot_files` (skip overridden paths), `graph_edges` (skip identities present in the override map), `synthetic_nodes` (skip overridden), `unresolved_refs` (skip overridden files).
  3. Symbols: `SELECT s.* FROM symbols s WHERE s.file_version_id = ANY($1)` over the final file-version id set, batched by 5,000 ids, streaming into `GraphBuilder::add_node` with `NodeKind` from `kind`.
  4. Add override survivors; `build()`.
  Cost: O(E_F + Δ) time, O(Δ) extra memory; no full-base hash map.
- **`load_delta(id)`** returns the rows of one snapshot as `GraphDelta`, including symbols of added/modified file versions as `nodes_added` and keys of nodes from deleted/replaced file versions as `nodes_removed` (computed from the base file versions' `symbols`).
- **`neighbors(id, key, dir, kinds, min_conf, limit)`**:
```sql
WITH chain(snapshot_id, pos) AS (SELECT * FROM unnest($1::uuid[]) WITH ORDINALITY)
SELECT * FROM (
  SELECT DISTINCT ON (e.kind, e.target_key) e.kind, e.target_key, e.confidence, e.resolved_by, e.provenance,
         e.flags, e.origin_path, e.line, e.col, e.removed
  FROM graph_edges e JOIN chain c USING (snapshot_id)
  WHERE e.source_key = $2 AND e.kind = ANY($3::int2[])
  ORDER BY e.kind, e.target_key, c.pos DESC) latest
WHERE NOT latest.removed AND latest.confidence >= $4
ORDER BY kind, target_key LIMIT $5;
```
  (the `In` direction is symmetric on `target_key` using `graph_edges_target`). Results are then filtered for endpoints that exist in the snapshot (a node removed by a later delta has tombstoned edges, so this is a safety net only). `NeighborPage { edges, next_cursor }` paginates with `(kind, other_key) > ($cursor)`.
- **`nodes(id, keys)`**: symbol nodes via `symbols_repo_key` joined to the snapshot's effective file-version set (computed with the same chain override logic over `snapshot_files` filtered by the candidate file_version_ids); synthetic nodes via `synthetic_nodes` with chain dedup.

**Data model changes:** None.

**API/protocol changes:** None (review-engine graph endpoints use `neighbors`/`nodes`; API tasks define the HTTP shape).

**Concurrency semantics:** Reads use `REPEATABLE READ READ ONLY` transactions so a load sees one consistent view even while compaction or new deltas commit. Many loads can run concurrently; GS-008 single-flights duplicate loads of the same snapshot.

**Failure behavior:** `ChainBroken`, `InvalidStatus` (non-ready), `SchemaMismatch` (snapshot `graph_schema_version` ≠ `SCHEMA_VERSION` → caller rebuilds, INC-011), retryable `Backend` for connection errors. A partially streamed load is discarded.

**Idempotency considerations:** Pure reads.

**Security considerations:** Scope check first: the snapshot must belong to the caller's `RepoScope` (`WHERE id=$1 AND repository_id=$2 AND organization_id=$3`), otherwise `NotFound`. RLS also applies. `limit` clamped to 1,000.

**Observability additions:** spans `graph_store.load_graph` (attrs `chain_depth`, `nodes`, `edges`, `delta_rows`), `graph_store.neighbors`; histograms `graph_load_duration_seconds`, `graph_store_neighbors_duration_seconds{direction}`.

**Tests required:**
- `pg_conformance_suite` (complete suite now passes on PG).
- `load_chain_of_20_deltas_equals_flattened_overlays`.
- `neighbors_returns_latest_version_in_chain`.
- `neighbors_hides_tombstoned_edges`.
- `neighbors_pagination_is_stable`.
- `load_rejects_schema_mismatch`.
- `load_with_broken_chain_errors`.
- `repeatable_read_isolates_concurrent_delta_commit`.

**Benchmarks:** `pg_read/load_full_1m_nodes` (target < 30 s, ADR-014 threshold), `pg_read/neighbors_p95` on a 1M-symbol synthetic snapshot with a 20-delta chain (target < 20 ms p95, ADR-014 gate).

**Acceptance criteria:** Full conformance suite green on PG; both benchmarks recorded; if either ADR-014 threshold is exceeded, a PERF task is opened and ADR-014's re-evaluation clause is noted.

**Definition of done:** Global DoD; benchmark baseline committed.

---

---

### GS-006 — File GraphStore for `.review/graph`
Status: ☐

**Task ID:** GS-006

**Title:** `FileGraphStore`: `.review/graph/snapshots/{id}.bin.zst` + `manifest.json`, atomic temp-file + rename writes, advisory file lock, passing the shared conformance suite.

**Problem:** The local CLI (`review init`, `review diff`) must persist graphs without Postgres (ADR-014: SQLite per repo rejected; file adapter instead).

**Why it exists:** Target-architecture §3.4; PRD §14 (`.review/` persistent layout); gap analysis §B (GS-006 file adapter).

**Scope:** directory layout, manifest schema, write/read of full graphs and deltas (CG-011 codec), file-version bookkeeping, lock, integrity checks, conformance harness.

**Explicit non-scope:** `.review/config.yaml` and `repository.json` (INIT-011); garbage collection beyond a simple retention count; IR cache files (IDX-005 uses its own `FsIrCache`).

**Files/modules expected to change:** `engine/crates/graph-storage/src/lib.rs`, `engine/crates/graph-storage/Cargo.toml` (`fd-lock` or `fs4`, `tempfile`, `serde_json`).

**New files/modules expected:** `engine/crates/graph-storage/src/file/{mod.rs, manifest.rs, atomic.rs, lock.rs}`, `engine/crates/graph-storage/tests/file_conformance.rs`.

**Dependencies:** GS-001, CG-011.

**Implementation details:**
```
.review/graph/
  manifest.json            { "format": 1, "repository_key": "<hex>", "snapshots": [SnapshotEntry…], "heads": {"<ref or HEAD>": "<id>"},
                             "file_versions": {"<path>\u0000<hash>\u0000<analyzer>": <local id>} }
  snapshots/{uuid}.bin.zst CG-011 container (payload Graph for full, Delta for delta)
  .lock                    advisory lock file
```
- `SnapshotEntry { id, kind, base, commit_sha, status, chain_depth, versions, stats, file, bytes, blake3, created_at }`.
- **Atomic write:** create `{name}.tmp-{pid}-{rand}` in the same directory → write → `sync_all()` → `std::fs::rename` to the final name (atomic replace on POSIX; on Windows Rust uses `MoveFileExW` with `MOVEFILE_REPLACE_EXISTING`) → on Unix, fsync the directory. The manifest is rewritten the same way after the snapshot file is durable, so a crash leaves either the old manifest or the new one, never a manifest pointing at a missing file.
- **Lock:** exclusive lock on `.lock` for any mutation (create/transition/write); shared lock for reads. Lock acquisition timeout 30 s → `StoreError::Conflict("graph store locked")`.
- **Status:** stored in the manifest; `transition` is CAS under the exclusive lock.
- `load_graph`: read chain from the manifest; decode full + deltas (verifying `bytes` and `blake3` from the manifest before decode); apply via `GraphOverlay::flatten` sequentially (O(V + E) per level; chains are compacted at 20 by GS-007).
- `neighbors`: load (via a small internal 1-entry cache) and query in memory.
- Retention: keep the latest 10 ready snapshots plus everything referenced by `heads` and by chains of kept snapshots; delete unreferenced files after manifest commit (best effort).
- Snapshot ids are `Uuid` values; file names are generated from them, never from user input.

**Data model changes:** New on-disk format (`format: 1`), documented in `docs/graph-schema/storage.md`.

**API/protocol changes:** None.

**Concurrency semantics:** Multi-process safe through the advisory lock (two CLI invocations on the same repo serialize mutations). Within a process, `FileGraphStore` holds a `tokio::sync::Mutex` around manifest mutations and performs file I/O in `spawn_blocking`.

**Failure behavior:** Corrupt manifest (JSON error) → `StoreError::Integrity` with a hint to run `review graph rebuild` (INC-011 maps it to a rebuild). Hash mismatch of a snapshot file → `Integrity`, the snapshot is marked `inconsistent`. Leftover `*.tmp-*` files are removed on open.

**Idempotency considerations:** Re-running a write after a crash before manifest commit simply overwrites the orphaned file; the manifest is the source of truth.

**Security considerations:** All paths are derived from the store root + UUID file names (no traversal). Decode limits from CG-011 apply (local files are untrusted input). Files are created with mode 0600 on Unix. No source text is stored.

**Observability additions:** same span names as GS-001 with `adapter=file`; counter `graph_store_file_integrity_errors_total`.

**Tests required:**
- `file_conformance_suite` (GS-001 `run_all` with a temp-dir harness).
- `crash_before_manifest_commit_keeps_old_state` (inject failure between file rename and manifest rename).
- `tmp_files_cleaned_on_open`.
- `hash_mismatch_marks_inconsistent`.
- `two_processes_serialize_writes` (spawn a child process holding the lock).
- `retention_keeps_heads_and_chains`.
- `windows_rename_replaces_existing` (runs on the CI Windows MSVC job).

**Benchmarks:** `file_store/write_load_100k_nodes` (record time and size).

**Acceptance criteria:** Conformance suite passes against the file adapter on Linux and on the Windows CI runner; crash test passes.

**Definition of done:** Global DoD; on-disk format documented.

---

---

### GS-007 — Snapshot compaction
Status: ☐

**Task ID:** GS-007

**Title:** Compact a delta chain into a new full snapshot when it exceeds 20 deltas or 10% edge churn.

**Problem:** Long delta chains make loads slower and SQL neighbour queries scan more rows (target-architecture §3.4).

**Why it exists:** ADR-003 ("a new full snapshot is written once a chain passes 20 deltas or 10% edge churn").

**Scope:** policy evaluation (`should_compact`), the compaction operation for both adapters, recording provenance, metrics; invocation points (called by INC-013 after a default-branch delta becomes ready, and by `review graph compact` later).

**Explicit non-scope:** deleting old snapshots (retention, SEC-007); compacting PR-head deltas (they are leaves and never compacted).

**Files/modules expected to change:** `engine/crates/graph-storage/src/lib.rs`, `engine/crates/graph-storage/src/port.rs` (add `compact` default method built on the trait primitives).

**New files/modules expected:** `engine/crates/graph-storage/src/compaction.rs`, `engine/crates/graph-storage/tests/compaction.rs`.

**Dependencies:** GS-005, GS-006, CG-010.

**Implementation details:**
```rust
pub struct CompactionPolicy { pub max_chain_depth: u16 /* 20 */, pub max_churn_ratio: f32 /* 0.10 */ }
pub enum CompactionDecision { Keep, Compact { reason: CompactionReason } }
pub enum CompactionReason { ChainDepth(u16), Churn(f32) }
pub fn should_compact(chain: &[SnapshotMeta], p: &CompactionPolicy) -> CompactionDecision;   // O(chain)
pub async fn compact(store: &dyn GraphStore, head: SnapshotId) -> Result<SnapshotMeta, StoreError>;
```
- Churn = `Σ_{deltas} (edges_added + edges_removed) / base_full.stats.edges` using `stats` recorded at write time (no row counting at decision time).
- `compact`: `load_graph(head)` (materialized), `create_snapshot(kind=full, purpose=compaction, commit=head.commit, versions=head.versions)`, `transition` to `persisting`, `write_full`, `transition` to `ready`. `stats.compacted_from = head` and `stats.replaced_chain = [ids]`. Subsequent default-branch deltas choose the newest ready full snapshot for the same commit as their base (INC-013 base selection prefers `full` over `delta` when both exist for the base commit).
- The partial unique index `snapshots_ready_full (repository_id, fingerprint)` makes concurrent compactions of the same head safe: the loser gets `Conflict`, marks itself failed, and the winner is used.

**Data model changes:** None (uses `purpose='compaction'` and `stats` fields).

**API/protocol changes:** None.

**Concurrency semantics:** Compaction reads under `REPEATABLE READ` and writes a new snapshot; it never mutates existing snapshots, so readers of the old chain are unaffected. Concurrent compactions resolve via the unique index.

**Failure behavior:** Any failure leaves the chain usable; the new snapshot becomes `failed`. Compaction is retried at the next evaluation point.

**Idempotency considerations:** Compacting the same head twice yields one ready full snapshot (unique index); a second call finds it with `find_ready` and returns it without rewriting.

**Security considerations:** Same tenant scoping as GS-004/005.

**Observability additions:** span `graph_store.compact` (attrs `chain_depth`, `churn`, `reason`); counter `graph_compactions_total{reason, result}`; gauge `graph_snapshot_chain_depth` recorded per repository at each delta write.

**Tests required:**
- `policy_triggers_at_depth_21`.
- `policy_triggers_at_churn_over_10_percent`.
- `compacted_full_equals_chain_materialization` (compare via CG-012).
- `concurrent_compaction_yields_single_full`.
- `compaction_failure_keeps_chain_readable`.
- `next_delta_bases_on_compacted_full`.
- runs for both PG and file adapters.

**Benchmarks:** `compaction/1m_edges_chain_20` (record duration).

**Acceptance criteria:** Tests pass on both adapters; compacted graph is identical to the chain's materialization.

**Definition of done:** Global DoD.

---

---

### GS-008 — In-process graph LRU cache
Status: ☐

**Task ID:** GS-008

**Title:** Memory-bounded, single-flight `GraphCache` of `Arc<Graph>` keyed by snapshot id.

**Problem:** Loading a large graph takes seconds; several PRs on the same base must not each load it (target-architecture §7: graph cache = PG + in-process LRU).

**Why it exists:** ADR-014 (graph loaded once per snapshot and kept in an LRU); PRD §118 latency targets.

**Scope:** cache type with byte-weighted eviction, coalesced concurrent loads, explicit eviction for `inconsistent` snapshots, metrics.

**Explicit non-scope:** caching overlays (PR heads are cheap to rebuild from a cached base + `load_delta`); cross-process caching (no graphs in Redis, target-architecture §7 rule).

**Files/modules expected to change:** `engine/crates/graph-storage/Cargo.toml` (`moka` with `future` feature).

**New files/modules expected:** `engine/crates/graph-storage/src/cache.rs`, `engine/crates/graph-storage/tests/cache.rs`.

**Dependencies:** CG-004 (`heap_size_bytes`), GS-001.

**Implementation details:**
```rust
pub struct GraphCache { inner: moka::future::Cache<SnapshotId, CachedGraph>, store: Arc<dyn GraphStore> }
#[derive(Clone)] pub struct CachedGraph { pub scope: RepoScope, pub graph: Arc<Graph> }
pub struct GraphCacheConfig { pub max_bytes: u64 /* default 40% of cgroup memory limit, min 512 MiB */ }
impl GraphCache {
    pub fn new(store: Arc<dyn GraphStore>, cfg: GraphCacheConfig) -> Self;
    pub async fn get(&self, scope: &RepoScope, id: SnapshotId) -> Result<Arc<Graph>, Arc<StoreError>>; // single-flight via try_get_with
    pub async fn insert(&self, scope: &RepoScope, id: SnapshotId, g: Arc<Graph>);                       // after a fresh full index
    pub async fn evict(&self, id: SnapshotId);
    pub fn stats(&self) -> CacheStats;
}
```
- `moka` weigher: `|_, c| (c.graph.heap_size_bytes() / 1024).min(u32::MAX as usize) as u32` with `max_capacity = max_bytes / 1024` (weights in KiB to fit `u32`).
- `try_get_with` coalesces concurrent misses for the same key into one `store.load_graph` call; errors are not cached.
- A single graph larger than `max_bytes` is returned to the caller but not retained (moka rejects over-capacity entries); this is counted.
- Snapshots are immutable, so there is no TTL; `evict` is called when a snapshot is marked `inconsistent`.
- The memory limit is read from the cgroup (`/sys/fs/cgroup/memory.max`) when present, otherwise from config.

**Data model changes:** None.

**API/protocol changes:** None.

**Concurrency semantics:** Fully concurrent; one in-flight load per key; `Arc<Graph>` handed out remains valid after eviction (memory is freed when the last user drops it, so the effective peak can exceed `max_bytes` by the in-use set — documented).

**Failure behavior:** Load errors propagate to every coalesced waiter as `Arc<StoreError>`; the next call retries.

**Idempotency considerations:** `get` is idempotent; `insert` of an existing key replaces it with an identical graph.

**Security considerations:** Cache keys are snapshot ids that were already authorized by `RepoScope` at load time; `get` takes a `RepoScope` and checks `cached.scope == *scope` before returning (mismatch → `StoreError::NotFound`), so a cached graph is never returned to another tenant.

**Observability additions:** counters `graph_cache_hits_total`, `graph_cache_misses_total`, `graph_cache_evictions_total`, `graph_cache_oversize_total`; gauge `graph_cache_bytes`; span `graph_cache.load` on misses.

**Tests required:**
- `concurrent_gets_load_once` (counting store, 50 concurrent gets).
- `eviction_respects_byte_budget`.
- `oversize_graph_not_retained`.
- `errors_are_not_cached`.
- `evict_removes_entry`.
- `scope_mismatch_is_rejected`.

**Benchmarks:** None (load cost measured by GS-005).

**Acceptance criteria:** Tests pass; metrics visible in a local OpenObserve run of IDX-004.

**Definition of done:** Global DoD.

---

---

### IDX-001 — Full index pipeline
Status: ☐

**Task ID:** IDX-001

**Title:** `pipeline::index::full_index`: walk → parallel parse (rayon inside `spawn_blocking`) → link → build graph → validate → persist a full snapshot; plus a dev driver `review-worker index-local`.

**Problem:** No component turns a repository at a commit into a persisted graph. Everything downstream (incremental, diff, impact) needs a base snapshot.

**Why it exists:** Target-architecture §3.1–§3.4; milestone M2 (full index on fixture repos and reference-api); critical path (`GS-005 → IDX-001 → INC-005`).

**Scope:**
- `FullIndexer` composing `repository` (walk, blob read), `analysis-ir` analyzers, `codegraph` linker/builder/validator and a `GraphStore`.
- Bounded-memory streaming of file contents.
- A dedicated rayon pool sized by config.
- Persisting file versions as they are parsed.
- The `index-local` dev subcommand (file or PG store) used by tests, IDX-006 and benchmarks.

**Explicit non-scope:** status/progress reporting (IDX-002); diagnostics persistence and tolerance thresholds (IDX-003); job consumption (IDX-004); parse cache lookups (IDX-005, which plugs into the parse step); checkout/clone (repository crate, INIT tasks).

**Files/modules expected to change:** `engine/crates/pipeline/Cargo.toml`, `engine/crates/pipeline/src/lib.rs`, `engine/apps/review-worker/src/main.rs` (subcommand).

**New files/modules expected:** `engine/crates/pipeline/src/index/{mod.rs, full.rs, source.rs, parse.rs, config.rs}`, `engine/apps/review-worker/src/cmd/index_local.rs`, `engine/crates/pipeline/tests/full_index_fixtures.rs`.

**Dependencies:** CG-005, CG-006, CG-012, GS-004, GS-006, INIT-001 (repository discovery/walk), INIT-008 (generated-code detection).

**Implementation details:**
```rust
pub struct FullIndexRequest { pub scope: RepoScope, pub commit: CommitSha, pub source: SourceSpec /* GitTree{repo_path, commit} | WorkingTree{root} */,
                              pub purpose: SnapshotPurpose, pub config: IndexConfig }
pub struct IndexConfig { pub parse_threads: usize /* available_parallelism - 1, min 1 */, pub max_file_bytes: u64 /* 1 MiB */,
                         pub persist_batch: usize /* 500 files */, pub parse_timeout: Duration /* 5 s per file */ }
pub struct FullIndexOutcome { pub snapshot: SnapshotMeta, pub graph: Arc<Graph>, pub stats: IndexStats }
pub struct FullIndexer { store: Arc<dyn GraphStore>, analyzers: AnalyzerRegistry, resolver_factory: Arc<dyn ModuleResolverFactory>,
                         parse_cache: Option<Arc<dyn ParseCache>> /* IDX-005 */, pool: Arc<rayon::ThreadPool> }
impl FullIndexer { pub async fn run(&self, req: FullIndexRequest, progress: &dyn ProgressSink /* IDX-002 */) -> Result<FullIndexOutcome, IndexError>; }
```
1. **Create snapshot** `kind=full`, status `pending→indexing` (versions from IDX-002).
2. **Walk** (`repository::walk::files(source, rules)`): `.gitignore`, `.review/config.yaml` exclusions, generated globs (INIT-008, files kept but flagged `is_generated`), size limit (larger files → `skipped`), binary detection (NUL in the first 8 KiB → skipped), symlinks never followed. Output sorted `Vec<WalkEntry { path, blob_oid?, size }>` — O(F log F).
3. **Read + parse** in `tokio::task::spawn_blocking(move || pool.install(|| …))`: `entries.par_chunks(64).flat_map_iter(...)` with `map_init` giving each worker thread its own analyzer session (tree-sitter parsers are not `Sync`). Blob bytes are read per file inside the worker (gix object lookup with a per-thread `gix::Repository` handle from `ThreadSafeRepository::to_thread_local()`), hashed with blake3, parsed, and dropped. Results are sent over a bounded `crossbeam_channel` (capacity 2 × persist_batch) to the async side.
4. **Persist file versions** on the async side as batches arrive (`upsert_file_versions`) — overlaps I/O with parsing. `ParsedUnit`s are kept as `Arc<ParsedUnit>` for linking (memory note below).
5. **Link** in `spawn_blocking`: `Linker::build_tables` + `link_all` + `FrameworkMapper` (CG-005/006) + `apply_globals`.
6. **Build** `GraphBuilder` from file entries, symbol nodes, synthetic nodes, edges and unresolved refs; `validate()` (CG-012) — errors fail the index (`IndexError::Inconsistent`); warnings are recorded in stats.
7. **Persist** `indexing→persisting`, `write_full`, `persisting→ready`.
8. Return `Arc<Graph>` so the caller can seed the cache (GS-008).
- **Memory:** ParsedUnits dominate (est. 5–20 KB each). At 100k files that is ≤ 2 GB; acceptable for the MVP and measured by PERF. Bodies/sources are never retained. If PERF shows pressure, the follow-up is to drop references after `link_file` and keep only symbols (the IR cache from IDX-005 allows re-reading).
- **Complexity:** O(total bytes) parse, O(S + R) link, O(V + E) build/persist.
- **`index-local`**: `review-worker index-local --repo <path> [--commit <sha>|--working-tree] --store file --state-dir <dir> [--json-stats <file>]` (or `--store pg --database-url …`). `--state-dir` defaults to `<repo>/.review/graph` but can point anywhere (IDX-006 uses a directory outside the read-only repository).

**Data model changes:** None (uses GS tables).

**API/protocol changes:** New dev CLI subcommand on `review-worker` (documented in `docs/operations/local-development.md` by the DEV tasks; this task adds a section stub).

**Concurrency semantics:** CPU work on a dedicated rayon pool (never the tokio runtime threads); async side handles persistence. Back-pressure via the bounded channel. One index run per `(repository, commit)` is enforced by the caller (IDX-004 lock); the indexer itself is re-entrant for different repositories.

**Failure behavior:** File-level failures are recorded and tolerated (IDX-003 policy). Fatal errors (store failure, walk failure, inconsistent graph, cancellation) transition the snapshot to `failed` with a redacted error message and return `IndexError`. Cancellation via a `CancellationToken` checked between chunks and phases.

**Idempotency considerations:** File-version writes are idempotent; a re-run creates a new snapshot id; if a ready snapshot with the same fingerprint exists, `run` returns it without indexing (checked before step 1 via `find_ready`).

**Security considerations:** Reads only inside the checkout root; symlinks not followed; file size cap; no source text in logs or errors (paths and codes only); analyzer panics are contained per file (IDX-003).

**Observability additions:** span `repository_index` (target-architecture §8 name) with children `index.walk`, `index.parse`, `graph.link`, `graph.build`, `graph.validate`, `graph_store.write_full`; attributes `repository_id`, `organization_id`, `commit_sha`, `files`, `symbols`, `edges`; histograms `index_duration_seconds{phase}`; counters `index_files_total{status=ok|partial|failed|skipped}`, `files_parsed_total{language}`.

**Tests required:**
- `full_index_graph_linker_fixture_golden` (insta snapshot of node/edge counts by kind + sample of edges).
- `full_index_nest_fixture_golden`.
- `full_index_is_deterministic_across_thread_counts` (1 vs 4 threads → identical CG-011 bytes).
- `full_index_file_and_pg_stores_produce_identical_graphs`.
- `existing_ready_fingerprint_short_circuits`.
- `cancellation_marks_snapshot_failed`.
- `oversized_and_binary_files_are_skipped`.
- `symlink_outside_repo_not_followed`.
- `store_failure_marks_snapshot_failed`.

**Benchmarks:** `index/full_synthetic_10k_files` (via PERF-001 generator when available; until then `codegraph::testkit` IR); record files/s and peak RSS.

**Acceptance criteria:** Fixture goldens committed and reviewed; determinism test passes; `review-worker index-local` indexes the fixture repos with both stores.

**Definition of done:** Global DoD; local-development doc section added.

---

---

### IDX-002 — Index status, progress and snapshot version fields
Status: ☐

**Task ID:** IDX-002

**Title:** Populate `graph_schema_version`, `analyzer_versions`, `config_hash`, `config_components` and `fingerprint` on every snapshot, and expose index progress through an `index_runs` table.

**Problem:** Without recorded versions, stale analysis is undetectable (PRD §105) and rebuild triggers (INC-011) cannot be evaluated. Operators and the UI need progress for long initial indexes.

**Why it exists:** ADR-015 (provenance and fingerprint); PRD §15, §105; target-architecture §5 Repository Intelligence screen (index status).

**Scope:** `SnapshotVersions` computation; component config hashes; fingerprint via INIT-012; `ProgressSink` trait with a PG implementation (throttled) and a no-op/CLI implementation; review-engine read endpoint for status.

**Explicit non-scope:** UI (WEB tasks); rebuild decisions (INC-011).

**Files/modules expected to change:** `engine/crates/pipeline/src/index/full.rs`, `engine/crates/graph-storage/src/types.rs`, `engine/apps/review-engine/src/routes/mod.rs`.

**New files/modules expected:** `engine/crates/pipeline/src/index/{versions.rs, progress.rs}`, `engine/migrations/0102_index_runs.sql`, `engine/apps/review-engine/src/routes/index_status.rs`, `engine/crates/pipeline/tests/index_versions.rs`.

**Dependencies:** IDX-001, INIT-012 (fingerprint function), GS-003.

**Implementation details:**
```rust
pub struct SnapshotVersions { pub graph_schema_version: u32, pub analyzer_versions: BTreeMap<String, String> /* language → semver, plus "linker" */,
                              pub parser_versions: BTreeMap<String, String> /* "tree-sitter", "tree-sitter-typescript" */,
                              pub config_hash: Hash256, pub config_components: ConfigComponents, pub profile_version: Option<String>,
                              pub fingerprint: Hash256 }
pub struct ConfigComponents { pub source_roots: Hash256, pub tsconfig: Hash256, pub generated_globs: Hash256, pub review_rules: Hash256 }
pub fn compute_versions(scope: &RepoScope, commit: &CommitSha, analyzers: &AnalyzerRegistry, cfg: &ResolvedRepoConfig) -> SnapshotVersions;
#[async_trait] pub trait ProgressSink: Send + Sync { async fn phase(&self, p: IndexPhase); async fn files(&self, done: u64, total: u64); async fn finish(&self, r: &IndexStats); }
pub enum IndexPhase { Walking, Parsing, Linking, Building, Persisting, Done, Failed }
```
- Each component hash is blake3 over a canonical serialization (sorted keys, normalized paths) of: source roots and workspace globs; the set of tsconfig files (path + normalized JSON of `compilerOptions.{baseUrl,paths,rootDirs,moduleResolution}` and `extends` chain); generated globs; review rules. `config_hash = blake3(source_roots ‖ tsconfig ‖ generated_globs ‖ review_rules)`.
- `fingerprint` = INIT-012 / ADR-015 formula over `repository_id ‖ commit_sha ‖ analyzer_versions ‖ graph_schema_version ‖ config_hash ‖ parser_versions ‖ profile_version`, serialized canonically.
```sql
CREATE TABLE index_runs (
  id uuid PRIMARY KEY, organization_id uuid NOT NULL, repository_id uuid NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
  snapshot_id uuid NOT NULL REFERENCES snapshots(id) ON DELETE CASCADE, job_id uuid,
  kind text NOT NULL CHECK (kind IN ('full','incremental','compaction')),
  phase text NOT NULL, files_total integer, files_done integer NOT NULL DEFAULT 0,
  started_at timestamptz NOT NULL DEFAULT now(), updated_at timestamptz NOT NULL DEFAULT now(), finished_at timestamptz,
  stats jsonb NOT NULL DEFAULT '{}'::jsonb, error text
);
CREATE INDEX index_runs_repo_started ON index_runs (repository_id, started_at DESC);
CREATE UNIQUE INDEX index_runs_snapshot ON index_runs (snapshot_id);
ALTER TABLE index_runs ENABLE ROW LEVEL SECURITY;
```
- `PgProgressSink` writes at most once per second or per 1,000 files (whichever comes later), plus every phase change, so progress never adds measurable DB load.
- review-engine `GET /internal/repositories/{repository_id}/index-status` → latest `index_runs` row + snapshot versions + fingerprint (service-auth; API proxies with tenant scope).

**Data model changes:** New table `index_runs`; snapshot version columns populated (columns created in GS-003).

**API/protocol changes:** New internal endpoint; `IndexStatus` JSON Schema in `packages/contracts`.

**Concurrency semantics:** Progress writes are fire-and-forget on a separate pool connection; failures are logged and ignored (progress is advisory).

**Failure behavior:** Version computation failures (e.g. unreadable tsconfig) produce a deterministic hash of the error marker plus a diagnostic, not a crash, so a broken tsconfig still indexes.

**Idempotency considerations:** `compute_versions` is pure; the same inputs give the same fingerprint (tested), which drives short-circuiting in IDX-001.

**Security considerations:** Config hashing reads only config files inside the repository; no file contents are stored (hashes only). Error text is redacted.

**Observability additions:** gauge `index_progress_ratio{repository_id}`; span attribute `fingerprint` (hex) on `repository_index`.

**Tests required:**
- `fingerprint_stable_for_same_inputs`.
- `fingerprint_changes_with_each_component` (commit, analyzer, schema, config, parser, profile).
- `tsconfig_whitespace_change_does_not_change_hash`.
- `tsconfig_paths_change_changes_tsconfig_component_only`.
- `progress_sink_throttles_writes`.
- `index_status_endpoint_returns_latest_run`.
- `snapshot_records_linker_version`.

**Benchmarks:** None.

**Acceptance criteria:** Every snapshot written by IDX-001 has non-empty version fields; `index_runs` reflects phases during a fixture index; endpoint test passes.

**Definition of done:** Global DoD; contracts regenerated.

---

---

### IDX-003 — Parse diagnostics persistence and tolerance policy
Status: ☐

**Task ID:** IDX-003

**Title:** Persist per-file parse diagnostics and apply an explicit tolerance policy: file-level failures never fail the index unless configured thresholds are exceeded.

**Problem:** Real repositories contain files that do not parse, time out or crash an analyzer. One bad file must not block indexing, but silent degradation must also be visible.

**Why it exists:** ADR-006 ("parse errors are tolerated", partial symbols still emitted); master-plan principle 4 (no silent truncation); PRD §24 (corruption detection) needs a clear distinction between tolerated and fatal failures.

**Scope:** classification of per-file outcomes; panic containment; per-file timeout; diagnostics table; thresholds; stats; CLI/JSON reporting.

**Explicit non-scope:** fixing analyzer bugs; deciding reviewer behaviour for unparsed files (REV-002 routing treats them as "not analyzed").

**Files/modules expected to change:** `engine/crates/pipeline/src/index/parse.rs`, `engine/crates/pipeline/src/index/full.rs`, `engine/crates/graph-storage/src/pg/file_versions.rs`.

**New files/modules expected:** `engine/crates/pipeline/src/index/tolerance.rs`, `engine/migrations/0103_parse_diagnostics.sql`, `engine/crates/pipeline/tests/parse_tolerance.rs`, `fixtures/repositories/broken-files/` (syntax errors, a 2 MiB file, a binary file, an analyzer-panic trigger file used with a test analyzer).

**Dependencies:** IDX-001, GS-002.

**Implementation details:**
```rust
pub enum FileOutcome { Ok(Arc<ParsedUnit>), Partial(Arc<ParsedUnit>) /* error nodes present */,
                       Failed { reason: FailReason }, Skipped { reason: SkipReason } }
pub enum FailReason { AnalyzerError(String /* redacted code */), Panic, Timeout, Utf8 }
pub enum SkipReason { TooLarge, Binary, Unsupported, Excluded }
pub struct TolerancePolicy { pub max_failed_ratio: f32 /* 0.05 */, pub max_failed_files: Option<u32>, pub min_parsed_files: u32 /* 1 */ }
```
- Each analyzer call is wrapped in `std::panic::catch_unwind(AssertUnwindSafe(..))` → `Failed{Panic}`; the panic message is not stored (could contain source), only the file path and analyzer version.
- Timeout: the analyzer receives a deadline in `AnalyzerConfig`; the TS analyzer uses tree-sitter's cancellation (progress callback / cancellation flag of the pinned tree-sitter 0.25 API — confirm the exact API name against the crate docs at implementation time) → `Failed{Timeout}`.
- `Partial` units are linked normally (their symbols exist); `Failed` files get a `file_versions` row with `parse_status='failed'` and a `File` node only (so the file still appears in the graph and diff mapping can report "unanalyzed").
- **Policy:** after parsing, `failed / (ok + partial + failed)` > `max_failed_ratio` or `failed > max_failed_files` or `ok + partial < min_parsed_files` → `IndexError::ToleranceExceeded { failed, total }` and the snapshot fails. Values come from `.review/config.yaml` `index.tolerance.*` with these defaults.
```sql
CREATE TABLE parse_diagnostics (
  file_version_id bigint NOT NULL REFERENCES file_versions(id) ON DELETE CASCADE,
  organization_id uuid NOT NULL,
  ordinal smallint NOT NULL,
  severity smallint NOT NULL,        -- 0 info, 1 warning, 2 error
  code text NOT NULL,                -- e.g. 'syntax_error', 'missing_node', 'fact_invalid_attrs', 'timeout', 'panic'
  line integer, col integer, end_line integer, end_col integer,
  PRIMARY KEY (file_version_id, ordinal)
);
ALTER TABLE parse_diagnostics ENABLE ROW LEVEL SECURITY;
```
- At most 50 diagnostics per file version are stored (`diagnostic_count` on `file_versions` keeps the true count). No message text column: codes + ranges only.
- `IndexStats.files_by_outcome` and the top 20 failing paths go into `snapshots.stats` and `index_runs.stats`.

**Data model changes:** New table `parse_diagnostics`.

**API/protocol changes:** `IndexStatus` (IDX-002) gains `files_by_outcome` and `failed_paths_sample`.

**Concurrency semantics:** Panic containment is per rayon task; a panic does not poison shared state because analyzers keep only thread-local sessions (the session is recreated after a panic).

**Failure behavior:** As described; tolerance exceeded is a clear terminal failure with counts. Diagnostics write failures fail the index (they are part of the file-version transaction).

**Idempotency considerations:** Diagnostics are content-addressed with the file version (written only when the file version row is new).

**Security considerations:** No source snippets or panic payloads persisted or logged; paths only.

**Observability additions:** counters `parse_failures_total{reason, language}`, `parse_diagnostics_total{code}`; span event `index.tolerance_exceeded`.

**Tests required:**
- `syntax_error_file_is_partial_and_linked`.
- `analyzer_panic_is_contained_and_file_failed`.
- `timeout_marks_file_failed`.
- `binary_and_oversized_are_skipped`.
- `failed_file_has_file_node_only`.
- `tolerance_exceeded_fails_index_with_counts`.
- `diagnostics_capped_at_50`.
- `no_source_text_in_diagnostics` (assert schema has no message column and stats contain no file content).

**Benchmarks:** None.

**Acceptance criteria:** `broken-files` fixture indexes successfully with the expected outcome counts; lowering `max_failed_ratio` to 0 makes the same index fail with `ToleranceExceeded`.

**Definition of done:** Global DoD; `.review/config.yaml` keys documented in the config reference.

---

---

### IDX-004 — repository-index job consumer
Status: ☐

**Task ID:** IDX-004

**Title:** `review-worker` consumer for the `repository-index` queue: checkout, full index, progress, lease heartbeat, idempotent completion.

**Problem:** The control plane enqueues initial and rebuild indexes (`POST /repositories/:id/initialize`, `review graph rebuild`); something must execute them reliably.

**Why it exists:** Target-architecture §5 (queues: `repository-index`; payloads carry IDs only); ADR-012 (PG queue, at-least-once); PRD §75–§76.

**Scope:** payload contract, consumer handler, per-repository mutual exclusion, checkout via the credential broker, call into `FullIndexer`, cache seeding, job result, compaction-kind jobs (GS-007) routed through the same queue.

**Explicit non-scope:** the queue client and reaper (PIPE-001); enqueueing from NestJS (API tasks); incremental jobs (INC-013).

**Files/modules expected to change:** `engine/apps/review-worker/src/main.rs`, `engine/apps/review-worker/src/consumers/mod.rs`.

**New files/modules expected:** `engine/apps/review-worker/src/consumers/repository_index.rs`, `packages/contracts/schemas/jobs/repository-index.schema.json` (generated), `engine/apps/review-worker/tests/repository_index_consumer.rs`.

**Dependencies:** IDX-001, IDX-002, PIPE-001 (job queue client, lease/heartbeat API).

**Implementation details:**
```rust
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct RepositoryIndexJob { pub repository_id: Uuid, pub organization_id: Uuid, pub commit_sha: CommitSha,
                                pub reason: IndexReason /* initial | rebuild_requested | rebuild_trigger | compaction | consistency_check */,
                                pub compact_snapshot_id: Option<Uuid>, pub requested_by: Option<Uuid> }
// idempotency_key = "repository-index:{repository_id}:{commit_sha}:{reason}" (compaction: "compact:{snapshot_id}")
```
1. Claim (PIPE-001), restore `traceparent`, open span `repository_index`.
2. Acquire a per-repository session advisory lock on a dedicated connection: `pg_try_advisory_lock(hashtextextended('repo-index:' || repository_id, 0))`. Not acquired → reschedule the job (`run_after = now() + 30 s`, attempts not incremented) and return.
3. For `reason=compaction` → `graph_storage::compact(compact_snapshot_id)` and finish.
4. Compute versions/fingerprint (IDX-002); `find_ready` with the fingerprint → if found and `reason != rebuild_*`, complete the job with that snapshot id (idempotent).
5. Checkout: `repository::checkout::ensure_mirror(repository_id, credentials)` where credentials come from `POST /internal/repositories/:id/clone-credentials` (short-lived token kept in memory only); index from the mirror's tree at `commit_sha` (no working-tree checkout needed: `SourceSpec::GitTree`).
6. `FullIndexer::run` with a `PgProgressSink` and a `CancellationToken` tied to job cancellation and SIGTERM.
7. Seed `GraphCache` with the returned graph; write job result `{ snapshot_id }`; complete.
- Heartbeat: extend the lease every `lease/3` while the index runs.

**Data model changes:** None (uses `jobs`, `snapshots`, `index_runs`).

**API/protocol changes:** `repository-index` payload schema published in `packages/contracts`; job result shape `{ snapshot_id: uuid }`.

**Concurrency semantics:** At most one index per repository at a time across all workers (advisory lock); other repositories proceed in parallel. Duplicate deliveries are harmless (fingerprint short-circuit + idempotency key). The advisory lock is released on connection close if the worker dies.

**Failure behavior:** Retryable errors (store backend, checkout network) → job fails with backoff (PIPE-001 policy, `max_attempts=3`); non-retryable (`ToleranceExceeded`, `Inconsistent`, invalid payload) → job `dead` immediately with `last_error` (redacted). The snapshot is `failed` in both cases. SIGTERM: stop at the next chunk boundary, mark snapshot `failed` with `error='cancelled'`, release the lease (PIPE-010 behaviour).

**Idempotency considerations:** Idempotency key prevents duplicate enqueue; fingerprint short-circuit prevents duplicate work for re-delivered jobs.

**Security considerations:** Clone tokens are never logged or written to disk (git credential passed via in-memory helper); payload contains IDs only; the worker verifies that the snapshot and repository belong to `organization_id` before writing.

**Observability additions:** span `repository_index` (root of the job, with `job_id`); counters `index_jobs_total{reason, result}`; histogram `index_job_duration_seconds{reason}`; counter `index_jobs_rescheduled_total{cause="locked"}`.

**Tests required:**
- `consumer_indexes_fixture_and_completes_job` (integration with compose PG and a local bare repo).
- `duplicate_delivery_reuses_ready_snapshot`.
- `second_worker_reschedules_when_repo_locked`.
- `tolerance_failure_goes_dead_without_retry`.
- `transient_store_error_retries`.
- `sigterm_cancels_and_releases_lease`.
- `compaction_job_routes_to_compact`.
- `payload_schema_matches_contracts`.

**Benchmarks:** None (index benchmarks in IDX-001/PERF).

**Acceptance criteria:** Enqueue a job with SQL in the test environment → snapshot `ready`, `index_runs.phase='Done'`, job `succeeded` with the snapshot id; all tests pass.

**Definition of done:** Global DoD; queue documented in `docs/operations/queues.md` (section for `repository-index`).

---

---

### IDX-005 — Parse cache keyed by (path, content_hash, analyzer_version)
Status: ☐

**Task ID:** IDX-005

**Title:** `ParseCache` port: look up `file_versions` before parsing; store and load full `ParsedUnit` blobs through an `IrCache` (object store in SaaS, filesystem in CLI).

**Problem:** Re-indexing (rebuilds, new full snapshots, linker-version bumps) and incremental inbound re-linking (INC-006, C7) must not re-parse files whose content and analyzer version are unchanged.

**Why it exists:** Target-architecture §7 (AST/IR cache keyed `(path, content_hash, analyzer_version)` in PG `file_versions` + object store blob); ADR-006 (analyzers are pure, so results are cacheable).

**Scope:** `ParseCache` trait and implementation over `GraphStore::lookup_file_versions` + `IrCache`; `IrCache` port with `ObjectStoreIrCache` (S3-compatible, MinIO/GCS) and `FsIrCache`; IR codec; integration into IDX-001's parse step; migration adding the blob key column.

**Explicit non-scope:** cache eviction/retention policy (SEC-007); semantic summary caches (SEM tasks).

**Files/modules expected to change:** `engine/crates/pipeline/src/index/parse.rs`, `engine/crates/analysis-ir/src/lib.rs` (serde derives present per TSA-001; add `IR_FORMAT_VERSION`), `engine/crates/graph-storage/src/pg/file_versions.rs`.

**New files/modules expected:** `engine/crates/pipeline/src/index/parse_cache.rs`, `engine/crates/analysis-ir/src/codec.rs`, `engine/crates/graph-storage/src/ir_cache/{mod.rs, object_store.rs, fs.rs, mem.rs}`, `engine/migrations/0104_file_versions_ir.sql`, `engine/crates/pipeline/tests/parse_cache.rs`.

**Dependencies:** IDX-001, GS-002, TSA-001 (serializable IR).

**Implementation details:**
```rust
pub struct FileVersionKey { pub path: RepoPath, pub content_hash: Hash256, pub analyzer_version: String }
#[async_trait] pub trait IrCache: Send + Sync {
    async fn get(&self, scope: &RepoScope, key: &FileVersionKey) -> Result<Option<Bytes>, IrCacheError>;
    async fn put(&self, scope: &RepoScope, key: &FileVersionKey, blob: Bytes) -> Result<IrObjectKey, IrCacheError>;
}
#[async_trait] pub trait ParseCache: Send + Sync {
    async fn lookup_many(&self, scope: &RepoScope, keys: &[FileVersionKey]) -> Result<Vec<CacheLookup>, ParseCacheError>;
    async fn store(&self, scope: &RepoScope, unit: &ParsedUnit, fv: &FileVersionRef) -> Result<(), ParseCacheError>;
}
pub enum CacheLookup { Hit(Arc<ParsedUnit>, FileVersionRef), Miss(MissReason /* NoRow | BlobMissing | DecodeError | IrFormatChanged */) }
```
```sql
ALTER TABLE file_versions ADD COLUMN ir_object_key text, ADD COLUMN ir_format smallint;
```
- **Object key:** `ir/{organization_id}/{repository_id}/{content_hash_hex}/{analyzer_version}/{blake3(path)_hex16}.bin.zst` (tenant-prefixed per master-plan security strategy §13.1). `FsIrCache` uses `.review/cache/ir/` with the same relative layout minus the tenant prefix.
- **IR codec:** `zstd(bincode|postcard(ParsedUnit))` with a small header `{magic "RGIR", ir_format: u16, analyzer_version}`; decoding a different `IR_FORMAT_VERSION` → `Miss(IrFormatChanged)`.
- **Parse step integration (IDX-001):** after hashing file bytes, keys are batched (1,000) → `lookup_many` → hits skip the analyzer entirely; misses are parsed, then `store` writes the blob and sets `ir_object_key`. Lookups use one SQL query per batch: `SELECT … FROM file_versions WHERE repository_id=$1 AND analyzer_version=$2 AND (path, content_hash) IN (SELECT * FROM UNNEST($3::text[], $4::bytea[]))`.
- Blob writes are best-effort: a failed `put` logs, counts and leaves `ir_object_key` NULL (the next run parses again); it never fails the index.
- `file_versions` rows written by IDX-001 without a blob are still valid for symbols/edges; only re-linking needs the blob, and INC-006 falls back to parsing (counted as `files_reparsed_for_relink_total`) when it is missing.

**Data model changes:** `file_versions.ir_object_key`, `file_versions.ir_format`.

**API/protocol changes:** None.

**Concurrency semantics:** Lookups and puts run concurrently with parsing (bounded `buffer_unordered(16)` for blob fetches). Two workers writing the same blob write identical content to the same key (last writer wins, idempotent).

**Failure behavior:** Object-store unavailability degrades to "parse everything" with a warning metric; DB lookup failure fails the index (it is the same database as everything else).

**Idempotency considerations:** Content-addressed keys; identical inputs produce identical blobs.

**Security considerations:** Tenant-prefixed keys; the object-store client uses credentials from env/secret manager; blobs contain IR (names, ranges, hashes) — no raw source. Decode uses size limits (max 64 MiB per blob).

**Observability additions:** counters `parse_cache_hits_total`, `parse_cache_misses_total{reason}`, `ir_cache_put_failures_total`; histogram `ir_cache_get_duration_seconds`.

**Tests required:**
- `second_full_index_parses_zero_files` (counting analyzer; same commit, new snapshot via rebuild reason).
- `analyzer_version_bump_misses_cache_for_that_language_only`.
- `blob_missing_falls_back_to_parse`.
- `ir_format_change_is_a_miss`.
- `object_store_down_degrades_gracefully` (wiremock S3 returning 503).
- `object_keys_are_tenant_prefixed`.
- `fs_ir_cache_roundtrip`.

**Benchmarks:** `parse_cache/lookup_10k_keys` (target < 500 ms against compose PG); `parse_cache/full_reindex_hit_rate` recorded on the fixture.

**Acceptance criteria:** Re-indexing a fixture with unchanged analyzers reports `files_parsed_total = 0` and `parse_cache_hits_total = files`.

**Definition of done:** Global DoD.

---

---

### IDX-006 — Index reference-api and parity report
Status: ☐

**Task ID:** IDX-006

**Title:** Index `C:\Users\user\Desktop\reference\reference-api` (read-only) and produce a parity report against its external `.codegraph` (14,981 nodes / 53,403 edges; kinds per audit §4.2).

**Problem:** Fixture repos do not prove the analyzer + linker work on a real NestJS codebase. The external codegraph is the only available reference (milestone M2 exit criterion: "parity report vs external codegraph").

**Why it exists:** Master plan §12 (real repository benchmark, parity in IDX-006); risk R1 early signal (edge precision); audit §4.2 counts.

**Scope:** read-only mount, indexing via `index-local` with an out-of-repo state dir, a read-only extractor for the external SQLite DB, a kind-mapping table, count comparison, sampled edge agreement, report files, and recorded thresholds.

**Explicit non-scope:** modifying reference-api in any way (no `.review/` written there); matching the external tool exactly (different models); quality benchmarks (QB tasks).

**Files/modules expected to change:** None existing.

**New files/modules expected:** `benchmarks/parity/reference-api/README.md`, `benchmarks/parity/reference-api/kind-mapping.json`, `benchmarks/parity/extract_codegraph_counts.py` (Python stdlib `sqlite3`, opened with `file:…?mode=ro&immutable=1` URI), `benchmarks/parity/compare.py`, `benchmarks/parity/run-reference.sh`, `benchmarks/parity/reference-api/report.md`, `benchmarks/parity/reference-api/report.json`.

**Dependencies:** IDX-001, IDX-003 (and NEST-001..006 for meaningful framework counts).

**Implementation details:**
- `run-reference.sh`: `docker run --rm -v "C:\Users\user\Desktop\reference\reference-api:/src/reference-api:ro" -v "<scratch>:/out" <engine image> review-worker index-local --repo /src/reference-api --working-tree --store file --state-dir /out/graph --json-stats /out/stats.json` (working tree because the external index is of the working tree; the commit SHA is recorded). Then `review-worker graph-export --state-dir /out/graph --format jsonl` (small dev subcommand added here: dumps nodes/edges with kind, id, file, qualified name) for comparison.
- `extract_codegraph_counts.py` reads `nodes`, `edges`, `files`, `unresolved_refs` tables of `reference-api/.codegraph/codegraph.db` read-only and writes counts by kind plus a sample of `(kind, source file, source qualified name, target file, target qualified name, confidence)` for `calls` edges.
- **Kind mapping** (`kind-mapping.json`): external `file → File`; `class → Class|Controller|Middleware|DatabaseEntity|QueueConsumer` (sum of refined kinds); `interface → Interface`; `method → Method|Handler|JobHandler`; `function → Function|Handler`; `property → Property|Field`; `constant → Constant (enum_member=false)`; `enum_member → Constant (enum_member=true)`; `enum → Enum`; `type_alias → TypeAlias`; `variable → Variable`; `route → ApiEndpoint`; `import` (6,740 nodes) → compared with our `IMPORTS` edge count (we model imports as edges, not nodes). Edges: `calls → CALLS (excluding INSTANTIATES)`, `instantiates → CALLS+INSTANTIATES`, `contains → CONTAINS`, `imports → IMPORTS`, `references → REFERENCES|USES_TYPE|ACCEPTS_TYPE|RETURNS_TYPE`, `decorates → REFERENCES+DECORATOR`, `extends → EXTENDS`, `implements → IMPLEMENTS`.
- **Qualified-name normalization** for sampled edge matching: external `Class::member` → `Class.member`; file paths normalized to repo-relative `/`.
- **Report contents:** counts table (ours vs external vs ratio per mapped kind); our extra kinds (HANDLED_BY, AUTHORIZES, queues, tables, env vars, tests); edge agreement for `calls`: of external edges with confidence ≥ 0.7, the % we also have (recall proxy), and of ours with confidence ≥ 0.85, the % external has (precision proxy), each with 20 disagreement examples (paths and names only); unresolved breakdown by reason vs external 75,121; index duration, peak RSS, files by outcome.
- **Thresholds** (recorded as pass/flag, not CI gates): files within ±2% of 1,028 analyzed files; Class/Interface/Method/Function/Enum each within ±10%; `route` vs ApiEndpoint within ±5% of 120; `calls` recall proxy ≥ 0.80; failed files ≤ 1%. Any flag gets a written explanation in `report.md` (e.g. a deliberate modelling difference) or a follow-up task ID.

**Data model changes:** None.

**API/protocol changes:** Dev subcommand `review-worker graph-export` (JSONL).

**Concurrency semantics:** Single run; not part of CI (reference-api is not in the repository). A nightly/manual job re-runs it when the analyzer changes.

**Failure behavior:** If the external DB is missing or locked, the extractor exits non-zero with a clear message; the report records "external reference unavailable" rather than inventing numbers.

**Idempotency considerations:** Re-running overwrites `report.*` deterministically for the same reference commit and analyzer versions (both recorded in the report header).

**Security considerations:** reference-api is mounted read-only and the external DB is opened with `mode=ro&immutable=1`; no reference source text is copied into the report (paths, names, counts only); the report is reviewed before commit to ensure no secrets or business data appear (names only).

**Observability additions:** The run exports its trace to local OpenObserve when available (span `repository_index` with `repository_id=reference-api-local`).

**Tests required:**
- `compare_py_kind_mapping_covers_all_external_kinds` (pytest-free: `python -m unittest benchmarks/parity/test_compare.py`).
- `extractor_opens_db_read_only` (run against a temp copy and assert no WAL/SHM files created).
- `graph_export_jsonl_roundtrip` (Rust test on a fixture).

**Benchmarks:** reference-api full index duration and peak RSS recorded in the report and in `benchmarks/perf/README.md`.

**Acceptance criteria:** `report.md` and `report.json` committed with all sections filled; every flagged threshold has an explanation or a follow-up task ID; `git -C C:\Users\user\Desktop\reference\reference-api status --porcelain` is unchanged by the run (checked and noted in the report).

**Definition of done:** Global DoD; M2 parity item checked in the master plan.

---

---

### INC-001 — Changed-path detection between commits (gix tree diff) incl. renames
Status: ◐

> **Implementation note:** The gix tree-diff primitive (`repository::git::tree_changes`: `diff_trees`, `tree_changes`, rename/copy detection) was built with DIFF-002 and is covered by `diff-engine/tests/files.rs`. Still missing: `incremental::changeset::detect_changes` on top of it and this task's own tests.

- **Task ID:** INC-001
- **Title:** `incremental::changeset` — path-level change detection between two commits using a gix tree diff with rename tracking; no blob content is read.
- **Problem:** Incremental indexing (ADR-004 step 1) must learn *which paths* differ between the base snapshot's commit and the head commit in time proportional to the changed subtrees, not to the repository size. Shelling out to `git diff --name-status` would depend on user git config and is banned (target-architecture §3.6).
- **Why it exists:** Target-architecture §3.5 step 1 (`changed paths P (from git tree diff)`); PRD §22 (per-file change detection); the primitive is reused by DIFF-002 so the incremental indexer and the diff engine can never disagree about which files changed.
- **Scope:**
  - `repository::git::tree_changes(repo, base, head, opts) -> Vec<RawTreeChange>` built on `gix::object::tree::diff` with `Rewrites` enabled.
  - `incremental::changeset::detect_changes` wrapping it into a `ChangeSet` of `PathChange`s with deterministic order.
  - Filtering of non-indexable entries (submodules, symlinks, non-blob modes) with counters.
  - Handling of a PR whose base is *not* an ancestor (diff is taken from the base commit's tree, not the merge base: the graph snapshot being updated is for the base commit).
- **Explicit non-scope:** Reading blobs or hashing (INC-002). Hunks (DIFF-003). Provider reconciliation (DIFF-005). Copy detection (DIFF-002 option only). Deciding whether the diff is "too big" (INC-011).
- **Files/modules expected to change:** `engine/crates/repository/src/git/mod.rs` (export), `engine/crates/incremental/Cargo.toml` (deps `repository`, `review-core`, `gix`), `engine/crates/incremental/src/lib.rs`.
- **New files/modules expected:** `engine/crates/repository/src/git/tree_changes.rs`, `engine/crates/incremental/src/changeset.rs`, `engine/crates/incremental/tests/changeset.rs`.
- **Dependencies:** FND-001, INIT-001 (`RepoPath`, walk rules), DIFF-001 (`GitRepo` handle, config-isolated open).
- **Implementation details:**
  ```rust
  pub struct TreeDiffOptions { pub rename_similarity: f32 /* 0.5 */, pub rename_limit: u32 /* 1000 candidates */, pub track_copies: bool /* false */ }
  pub struct RawTreeChange { pub path: RepoPath, pub old_path: Option<RepoPath>, pub kind: RawKind /* Added|Deleted|Modified|Renamed|Copied|TypeChanged */,
                             pub old_oid: Option<gix::ObjectId>, pub new_oid: Option<gix::ObjectId>, pub old_mode: EntryMode, pub new_mode: EntryMode, pub similarity: Option<u8> }
  pub struct ChangeSet { pub base: CommitSha, pub head: CommitSha, pub entries: Vec<PathChange> /* sorted by path */, pub stats: ChangeSetStats }
  pub struct PathChange { pub path: RepoPath, pub kind: PathChangeKind /* Added|Modified|Deleted|Renamed */, pub renamed_from: Option<RepoPath>,
                          pub base_oid: Option<ObjectId>, pub head_oid: Option<ObjectId>, pub similarity: Option<u8> }
  pub struct ChangeSetStats { pub entries: u32, pub renamed: u32, pub skipped_submodule: u32, pub skipped_symlink: u32, pub rename_limit_hit: bool }
  ```
  - Algorithm: peel both commits to trees; call `base_tree.changes()?.for_each_to_obtain_tree_with_cache(&head_tree, …)` with `Rewrites { copies: None, percentage: Some(0.5), limit: 1000, track_empty: false }`. gix skips identical subtree object ids, so cost is O(changed subtrees + their entries), not O(files).
  - A rename is reported as `Renamed { path = new, renamed_from = old }`. The graph treats it as `Deleted(old) + Added(new)` plus the hint consumed by the SID-005 matcher and by the file-move lineage (INC-003). A rename with 100% similarity and unchanged content still produces both file entries because `path` is part of file/symbol identity (ADR-005 `module_path`).
  - `TypeChanged` (file ↔ symlink) → `Deleted` + `Added` when the new side is indexable, else `Deleted` only. Submodule (`160000`) and symlink (`120000`) entries are dropped and counted, never followed.
  - Mode-only change (exec bit) with equal oids: not emitted by gix (oid equal); mode is ignored for indexing.
  - Output is sorted by `RepoPath` bytes; duplicates are impossible (checked by `debug_assert!`).
  - Rename-limit overflow degrades to Add + Delete pairs and sets `rename_limit_hit`; correctness is unaffected because INC-003's matcher re-derives renames from symbol bodies.
- **Data model changes:** None.
- **API/protocol changes:** Internal crate API only. `ChangeSet` is `Serialize` for `stage_outputs` debugging.
- **Concurrency semantics:** Synchronous, CPU/IO-bound; caller wraps in `spawn_blocking`. Uses a thread-local `gix::Repository` from `ThreadSafeRepository` (DIFF-001); never shares one handle across threads.
- **Failure behavior:** `ChangeSetError::{CommitNotFound(CommitSha), ObjectCorrupt, ShallowBoundary, Io}`. A missing head object is retryable (the worker fetches then retries, INC-013); it never yields an empty change set.
- **Idempotency considerations:** Pure function of two commit ids and options; same inputs give byte-identical `ChangeSet`.
- **Security considerations:** Paths come from tree entries and are validated into `RepoPath` (no `..`, no absolute, no NUL, UTF-8 only; non-UTF-8 paths are skipped and counted). Git config of the host/user is ignored by the isolated open in DIFF-001.
- **Observability additions:** span `incremental.changeset` (attrs `base`, `head`, `entries`, `renamed`); histogram `changeset_detect_duration_seconds`; counter `changeset_skipped_entries_total{reason=submodule|symlink|non_utf8}`.
- **Tests required:** `added_modified_deleted_detected`, `rename_with_edits_reports_renamed_from`, `identical_subtrees_not_visited` (counting tree-object reads on a 5k-file repo with one changed file: reads ≤ depth × fan-out), `rename_limit_degrades_to_add_delete`, `submodule_and_symlink_skipped`, `type_change_file_to_symlink`, `output_sorted_and_deterministic`, `unaffected_by_user_git_config` (set `diff.renames=false` in a temp global config), `golden_auth_bypass_changeset` (exactly `src/auth/auth.service.ts` and the comment-only `src/util/format.ts`).
- **Benchmarks if applicable:** `changeset/1_file_in_100k_tree` (synthetic repo built by PERF-001 or a scripted gix repo): target < 50 ms warm.
- **Acceptance criteria:** All tests pass; the 100k-file benchmark stays flat as file count grows with a fixed change; golden changeset matches.
- **Definition of done:** Global DoD; DIFF-002 consumes `tree_changes` without a second tree-diff implementation.

---

### INC-002 — Hash-skip + reparse-only-changed
Status: ☐

- **Task ID:** INC-002
- **Title:** `incremental::reparse` — for each changed path, hash the head blob, skip it when the content hash equals the base file's, otherwise obtain a `ParsedUnit` (parse cache first, analyzer second) and report counters.
- **Problem:** A tree-diff entry does not always mean new content (reverted edits across merge commits, rename-only, mode changes surfaced by providers). Parsing is the expensive step, and parsing any file whose `(path, content_hash, analyzer_version)` is already known violates ADR-004 and PRD §22.
- **Why it exists:** Target-architecture §3.5 steps 1–2; milestone M3 ("counters prove no unchanged reparse"); IDX-005 cache keys.
- **Scope:**
  - `BlobSource` abstraction (git commit tree or in-memory map for tests).
  - Per-path hashing (blake3 of raw bytes), hash comparison with the base graph's `FileView.content_hash`.
  - Parse-cache lookup (`ParseCache::lookup_many`), analyzer invocation on miss, `ParseCache::store`, `upsert_file_versions` for new versions.
  - Size / binary / generated gating consistent with IDX-001.
  - Result type consumed by INC-003/005.
- **Explicit non-scope:** Symbol diffing (INC-003). Linking (INC-005/006). Counter export plumbing (INC-010 owns the names; this task increments them). Reading IR of *unchanged* files for inbound re-link (INC-006).
- **Files/modules expected to change:** `engine/crates/incremental/src/lib.rs`, `engine/crates/incremental/Cargo.toml` (deps `analysis-ir`, `graph-storage`, `rayon`, `blake3`, `tokio`).
- **New files/modules expected:** `engine/crates/incremental/src/reparse.rs`, `engine/crates/incremental/src/blob_source.rs`, `engine/crates/incremental/tests/reparse.rs`.
- **Dependencies:** INC-001, IDX-005 (`ParseCache`), IDX-001 (`IndexConfig`, analyzer registry), GS-001, CG-007 (`GraphQuery::file`).
- **Implementation details:**
  ```rust
  pub trait BlobSource: Send + Sync { fn read(&self, path: &RepoPath, oid: Option<&ObjectId>, max_bytes: u64) -> Result<BlobRead, BlobError>; }
  pub enum BlobRead { Bytes(Vec<u8>), TooLarge(u64), Binary }
  pub enum FileOutcome {
      Unchanged { path: RepoPath, content_hash: Hash256 },                         // hash == base hash; no parse
      Reparsed { path: RepoPath, unit: Arc<ParsedUnit>, file_version: FileVersionRef, cache_hit: bool, base_hash: Option<Hash256> },
      Deleted { path: RepoPath },
      Skipped { path: RepoPath, reason: SkipReason /* TooLarge|Binary|Unsupported|NotUtf8 */, content_hash: Option<Hash256> },
  }
  pub struct ReparseResult { pub outcomes: Vec<FileOutcome> /* sorted by path */, pub counters: ReparseCounters }
  pub async fn reparse_changed(cx: &ReparseCx<'_>, changes: &ChangeSet) -> Result<ReparseResult, IncError>;
  ```
  1. For every non-deleted `PathChange`, read the head blob (bounded by `max_file_bytes`) via `BlobSource` in a rayon pool inside `spawn_blocking`; hash with blake3 (O(bytes of changed files)).
  2. Look up the base file with `base.file(path)`; for `Renamed`, compare against the *new* path only (a renamed file is a new path: `base.file(new)` is `None`, so it is reparsed unless the cache already holds `(new_path, hash, analyzer_version)`).
  3. Equal hash → `Unchanged` (increments `files_skipped_unchanged_total`). This also covers the analyzer-version-neutral case where only mode/EOL-irrelevant metadata changed.
  4. Otherwise batch keys (≤1,000) → `lookup_many`; hits yield `Reparsed{cache_hit:true}` without calling the analyzer (counted `files_reparsed_total{source=cache}`); misses are parsed with a per-thread analyzer session, validated (`analysis_ir::validate` in debug), stored, then `upsert_file_versions`.
  5. Deleted paths yield `Deleted` and never touch the blob source.
  - `Failed` parse status is not an error: the unit is kept with its diagnostics and IDX-003's tolerance policy applies at INC-013 level.
  - Complexity: O(Σ bytes of changed files) hashing + parse; O(k) cache lookups for k changed paths; independent of repository size.
- **Data model changes:** None (writes `file_versions` rows through `GraphStore::upsert_file_versions`).
- **API/protocol changes:** None.
- **Concurrency semantics:** CPU work on the shared rayon pool (never tokio worker threads); persistence on the async side; cancellation token checked between chunks of 64 files. Output order is by path regardless of completion order.
- **Failure behavior:** A blob read error for one path → `IncError::BlobRead{path}` fails the whole update (a partial head graph is worse than none); oversize/binary files become `Skipped`. Object-store cache failure degrades to parsing (IDX-005 behaviour).
- **Idempotency considerations:** Same inputs → same outcomes; `upsert_file_versions` is idempotent by key.
- **Security considerations:** Reads only blobs listed by the tree diff from the object database; no working-tree access; contents are never logged (paths and hashes only).
- **Observability additions:** span `incremental.reparse` (attrs `changed`, `unchanged`, `reparsed`, `cache_hits`); counters `files_reparsed_total{source=analyzer|cache}`, `files_skipped_unchanged_total`; histogram `incremental_reparse_duration_seconds`.
- **Tests required:** `identical_content_is_skipped_without_analyzer_call` (counting analyzer == 0), `modified_file_is_parsed_once`, `revert_to_base_content_is_skipped`, `rename_only_is_reparsed_as_new_path_unless_cached`, `cache_hit_skips_analyzer`, `deleted_file_not_read`, `oversize_and_binary_skipped`, `unchanged_files_in_tree_never_read` (blob source records reads; only changeset paths appear), `partial_parse_unit_is_kept`, `outcomes_sorted_by_path`.
- **Benchmarks if applicable:** `reparse/10_changed_files_of_50k` — wall time must not depend on the 50k.
- **Acceptance criteria:** Counting-analyzer test shows exactly |changed with different hash| analyzer calls; blob-source read log is a subset of the changeset.
- **Definition of done:** Global DoD.

---

### INC-003 — Apply per-file symbol diffs (SID-004/005) to build the head symbol table
Status: ☐

- **Task ID:** INC-003
- **Title:** `incremental::symbols` — classify symbols of each reparsed file as unchanged/modified/added/removed, run the cross-file rename/move matcher, and produce a head `SymbolTables` view (changed files from fresh IR, everything else lazily from the base).
- **Problem:** The linker (CG-005) resolves against a `SymbolTable`. For the head commit we cannot rebuild tables for all files, and node identity (ADR-005) must survive renames/moves so that unchanged callers keep their edges and finding history follows lineage.
- **Why it exists:** Target-architecture §3.5 step 3; ADR-005 matcher rules; PRD §22 (unchanged/modified/added/removed/renamed). Output feeds counters (`symbols_*_total`), the delta (INC-007), invalidation (INC-008) and the change model (CHG-001).
- **Scope:**
  - `diff_file_symbols(base_nodes, head_unit)` → `FileSymbolDiff`.
  - Aggregation across all changed files, then one global call to `matcher::match_symbols` (SID-005) over removed × added.
  - `HeadSymbolTables`: overlay of fresh `FileSymbols` for changed files over a lazy base view.
  - `SymbolChangeSet` (the artifact other crates consume).
- **Explicit non-scope:** The matcher algorithm itself (SID-005). Name-index maintenance (INC-004). Edge computation (INC-005/006).
- **Files/modules expected to change:** `engine/crates/incremental/src/lib.rs`, `engine/crates/codegraph/src/linker/symbol_table.rs` (expose `FileSymbols::from_unit(&ParsedUnit)` already used by `build_tables`).
- **New files/modules expected:** `engine/crates/incremental/src/symbols/{mod.rs, diff.rs, head_tables.rs}`, `engine/crates/incremental/tests/symbols.rs`.
- **Dependencies:** INC-002, SID-004 (`body_hash`, `signature_hash`, `attr_hash`), SID-005 (`matcher`, `SymbolTransition`, `LineageRecord`), CG-005 (`SymbolTable`, `FileSymbols`), CG-007 (`nodes_in_file`).
- **Implementation details:**
  ```rust
  pub enum SymbolStatus { Unchanged, Modified { signature: bool, body: bool, attrs: bool, moved_range: bool }, Added, Removed }
  pub struct FileSymbolDiff { pub path: RepoPath, pub per_symbol: Vec<(NodeKey, SymbolStatus)>, pub base_only: Vec<NodeKey>, pub head_only: Vec<LocalId> }
  pub struct SymbolChangeSet {
      pub added: Vec<NodeKey>, pub removed: Vec<NodeKey>, pub modified: Vec<(NodeKey, ModifiedFlags)>,
      pub renamed: Vec<LineageRecord> /* from, to, transition, similarity */, pub unchanged_in_changed_files: u32 }
  pub struct HeadSymbolTables { changed: SymbolTable, removed_files: BTreeSet<RepoPath>, base: Arc<dyn BaseSymbolSource> }
  pub trait BaseSymbolSource: Send + Sync { fn file_symbols(&self, path: &RepoPath) -> Option<Arc<FileSymbols>>; }   // lazy, memoized
  ```
  - Per file: head symbols get `SymbolKey` from SID-001 (`SymbolId` from qualified name + kind + ordinal). Base side comes from `base.nodes_in_file(path)` (NodeAttrs hold `body_hash`, `signature_hash`, `attr_hash` in `extra`). Merge-join by key (both sorted): equal key → compare the three hashes; node range change alone sets `moved_range` (counts as *unchanged* for `symbols_modified_total` but still replaces node data in the delta).
  - Removed×added candidates from *all* changed files (and from `Deleted`/`Renamed` file entries) go to `match_symbols` once, so a symbol moved between files is paired. Pairs become `renamed` lineage and are removed from `removed`/`added` lists for counting, but both node keys still exist in the delta (old removed, new added).
  - `BaseSymbolSource` is backed by the IR cache (IDX-005): on first access of an unchanged file's `FileSymbols` (needed when a changed file imports from it) it loads the blob keyed by the base `file_version_id` and calls `FileSymbols::from_unit`; on a missing blob it parses from git (counted `files_reparsed_for_relink_total`, IDX-005). Memoized in a `DashMap`.
  - `HeadSymbolTables::file_symbols(path)`: removed → `None`; changed → fresh table; else base lazily. Lookup is O(1) amortized; total memory O(changed files + consulted files).
  - Complexity: per-file merge O(S_f); matcher O(R×A) bounded by SID-005's candidate caps; no work for unchanged files.
- **Data model changes:** None (lineage persisted by INC-007 via `symbol_lineage`).
- **API/protocol changes:** None.
- **Concurrency semantics:** Per-file diff runs in parallel (rayon); the matcher runs once, single-threaded, over sorted inputs for determinism. `HeadSymbolTables` is `Send + Sync`; lazy loads are memoized with once-cell semantics.
- **Failure behavior:** IR-cache miss falls back to parsing; a key collision from SID-001 aborts the update with `IncError::KeyCollision` and forces a full rebuild (INC-011). Matcher ambiguity never fails: ties resolve deterministically (lowest key) and are counted `rename_ambiguous_total`.
- **Idempotency considerations:** Deterministic: sorted inputs, stable tie-breaks.
- **Security considerations:** Only IR (names, hashes, ranges) is handled; no source retained.
- **Observability additions:** span `incremental.symbol_diff` (attrs `files`, `added`, `removed`, `modified`, `renamed`); counters `symbols_{added,removed,modified,renamed}_total` (owned by INC-010, incremented here); `rename_ambiguous_total`.
- **Tests required:** `body_edit_is_modified_body_only`, `signature_edit_is_modified_not_add_delete`, `comment_edit_leaves_symbol_unchanged` (hash ignores comments), `line_shift_is_moved_range_not_modified`, `rename_within_file_detected_by_body_hash`, `move_across_files_detected`, `rename_threshold_0_8_boundary`, `deleted_file_removes_all_symbols`, `base_table_lazy_load_only_for_consulted_files`, `golden_authorize_modified_body` (auth-bypass: exactly one `Modified{body}`).
- **Benchmarks if applicable:** `symbols/diff_20_files_of_1m_base`: independent of base size.
- **Acceptance criteria:** Golden scenario yields `modified=1` (+ cosmetic file adds none); rename/move suite from SID-006 passes through this path.
- **Definition of done:** Global DoD.

---

### INC-004 — Name index with delta support
Status: ☐

- **Task ID:** INC-004
- **Title:** `incremental::name_index` — a base `NameIndex` cached with the base graph, an `OverlayNameIndex` implementing `NameLookup` for the head, and a `NameDelta` that lists exactly the names whose candidate sets changed.
- **Problem:** Resolution by name (`NameUnique` / `NameAmbiguous`) in *unchanged* files can change when a symbol is added, removed or renamed elsewhere: a unique name becomes ambiguous, an unresolved reference becomes resolvable. Finding those files without scanning needs a name index that can be diffed (ADR-004 "linker must expose a name index that supports deltas").
- **Why it exists:** ADR-004 step 4 and Consequences; INC-006 consumes `NameDelta::affected_names()` instead of scanning references.
- **Scope:**
  - `BaseIndexes { name_index: NameIndex, member_index }` built from a `Graph` and cached next to it.
  - `NameDelta` computed from `SymbolChangeSet` (INC-003).
  - `OverlayNameIndex: NameLookup` (merged, sorted, deduplicated candidates).
  - Threshold-crossing detection (1↔2, ≤fanout↔>fanout).
- **Explicit non-scope:** The cascade itself (CG-005 owns `NameLookup` use). Inbound re-link (INC-006). Persisting the index (rebuilt from the graph on load).
- **Files/modules expected to change:** `engine/crates/incremental/src/lib.rs`, `engine/crates/codegraph/src/linker/name_index.rs` (add `NameIndex::from_graph`), `engine/crates/graph-storage/src/cache.rs` (GS-008 entry holds `Arc<BaseIndexes>`).
- **New files/modules expected:** `engine/crates/incremental/src/name_index.rs`, `engine/crates/incremental/tests/name_index.rs`.
- **Dependencies:** CG-005 (`NameIndex`, `NameLookup`), INC-003, GS-008, CG-007.
- **Implementation details:**
  ```rust
  pub struct BaseIndexes { pub names: NameIndex, pub snapshot: SnapshotId }
  impl BaseIndexes { pub fn build(g: &Graph) -> Self }                      // O(V): one pass over callable/type/member nodes
  pub struct NameDelta { added: BTreeMap<SmolStr, BTreeSet<NodeKey>>, removed: BTreeMap<SmolStr, BTreeSet<NodeKey>>,
                         member_added: BTreeMap<SmolStr, BTreeSet<NodeKey>>, member_removed: BTreeMap<SmolStr, BTreeSet<NodeKey>> }
  impl NameDelta { pub fn from_changes(base: &BaseIndexes, cs: &SymbolChangeSet, head_nodes: &dyn Fn(&NodeKey) -> Option<NameEntry>) -> Self;
                   pub fn affected_names(&self, cfg: &LinkConfig) -> AffectedNames }
  pub struct AffectedNames { pub top: BTreeSet<SmolStr>, pub members: BTreeSet<SmolStr> }
  pub struct OverlayNameIndex<'a> { base: &'a NameIndex, merged: HashMap<SmolStr, Box<[NodeKey]>>, merged_members: HashMap<SmolStr, Box<[NodeKey]>> }
  impl NameLookup for OverlayNameIndex<'_> { fn top(&self, n: &str) -> &[NodeKey]; fn members(&self, n: &str) -> &[NodeKey]; }
  ```
  - `BaseIndexes::build` runs once per base snapshot load and is cached with the `Graph` in the GS-008 LRU; it piggybacks on the O(V+E) graph load that is paid anyway and is *not* part of per-PR marginal cost. Memory ≈ 40 B per indexed name.
  - `NameDelta` is built only from added/removed/renamed keys (a rename = removed old name + added new name). Modified-in-place symbols do not touch the index (name and kind unchanged).
  - `affected_names`: a name is affected when its candidate set differs between base and head **and** the difference can change a resolution: set non-empty on either side. Names that cross `max_ambiguous_fanout` or flip between 1 and ≥2 candidates are flagged `threshold_crossed` (used by INC-006 to also revisit edges of the *remaining* candidates, since their `NameUnique` edges became `NameAmbiguous`).
  - `OverlayNameIndex::new` precomputes merged slices only for affected names (sorted by `NodeKey` bytes, deduplicated, ADR-004 deterministic ordering) and delegates every other lookup to the base slice with zero allocation. Cost O(|Δ| log |Δ|).
  - Compatible-kind filtering stays in the linker: the index stores `(NodeKey, kind class)` so `callables`/`types` filters need no node lookup.
- **Data model changes:** None.
- **API/protocol changes:** None.
- **Concurrency semantics:** `BaseIndexes` is immutable and `Arc`-shared across jobs on the same worker; the overlay is built per update and borrowed immutably by rayon workers.
- **Failure behavior:** Cannot fail. A base graph built by an older linker version cannot reach here (INC-011 forces re-link/rebuild first).
- **Idempotency considerations:** Pure function of `(base, SymbolChangeSet)`; identical input → identical `AffectedNames` and merged slices.
- **Security considerations:** None (names only).
- **Observability additions:** span `incremental.name_delta` (attrs `names_added`, `names_removed`, `affected_top`, `affected_members`, `threshold_crossed`); histogram `name_index_build_duration_seconds`.
- **Tests required:** `overlay_equals_rebuilt_index_for_every_name` (proptest: random base + random symbol changes; overlay lookups equal `NameIndex::from_graph(flattened head)`), `unique_to_ambiguous_flagged_threshold_crossed`, `removal_of_only_candidate_makes_name_empty`, `rename_affects_old_and_new_names`, `unaffected_name_returns_base_slice_pointer_equal`, `candidates_sorted_and_deduplicated`, `member_index_delta_tracked_separately`.
- **Benchmarks if applicable:** `name_overlay/lookup_vs_base` (overhead < 20% p95); `name_index/build_1m_nodes` recorded.
- **Acceptance criteria:** Proptest passes 1,000 cases; the golden scenario produces an empty `AffectedNames` for `authorize` (modified only, so no resolution change).
- **Definition of done:** Global DoD.

---

### INC-005 — Re-link outgoing refs of changed files
Status: ☐

- **Task ID:** INC-005
- **Title:** `incremental::relink_out` — re-resolve every reference *from* each changed file against the head symbol tables and name overlay, rebuild its structural and framework edges, and diff the result against the base edges the file owned.
- **Problem:** Edges are owned by the file whose content produced them (`Edge.origin_file`). When a file changes, all of its outgoing edges may change; edges it no longer produces must be tombstoned and new ones added, without touching other files' edges.
- **Why it exists:** Target-architecture §3.5 step 4 (first half); ADR-004; critical-path node (master plan §7: `INC-005 → INC-006 → INC-009`).
- **Scope:**
  - Run `Linker::link_file` for each reparsed unit with `HeadSymbolTables` + `OverlayNameIndex` + `ModuleResolver`.
  - Structural edges for the file (CONTAINS/DECLARES/EXPORTS/IMPORTS), `FrameworkMapper::map_file` edges and synthetic nodes, `OVERRIDES` post-pass restricted to touched classes.
  - Re-run `apply_globals` only when the set of endpoint synthetic nodes changed.
  - Compute the per-file edge diff: `EdgeChange { added, removed, relocated }`.
  - Replace the file's `UnresolvedRef` list.
- **Explicit non-scope:** Edges owned by unchanged files (INC-006). Emitting the delta (INC-007). Linker resolution rules (CG-005).
- **Files/modules expected to change:** `engine/crates/incremental/src/lib.rs`, `engine/crates/codegraph/src/linker/overrides.rs` (expose `overrides_for_classes`).
- **New files/modules expected:** `engine/crates/incremental/src/relink_out.rs`, `engine/crates/incremental/tests/relink_out.rs`.
- **Dependencies:** INC-003, INC-004, CG-005, CG-006 (`FrameworkMapper`, `apply_globals`), CG-007, TSA-009 (`ModuleResolver` impl).
- **Implementation details:**
  ```rust
  pub struct OutgoingResult { pub per_file: Vec<FileEdgeChange>, pub synthetic_added: Vec<SyntheticNode>, pub touched_synthetic: BTreeSet<NodeKey>,
                              pub unresolved_replaced: Vec<(RepoPath, Vec<UnresolvedRef>)>, pub deps: BTreeMap<RepoPath, ResolutionDeps> }
  pub struct FileEdgeChange { pub path: RepoPath, pub added: Vec<Edge>, pub removed: Vec<EdgeIdentity>, pub relocated: Vec<Edge> /* same identity, new location/occurrences/confidence */ }
  pub fn relink_changed_files(cx: &RelinkCx<'_>, units: &[Arc<ParsedUnit>], deleted: &[RepoPath]) -> OutgoingResult;
  ```
  1. For each changed unit (path order, rayon `par_iter().collect::<Vec<_>>()` preserving order): `link_file(unit, &head_tables, &name_overlay, resolver, cfg)` → `FileLinkResult`; add structural + framework edges; merge duplicate identities with `Edge::merge_occurrence`.
  2. Base edges owned by the file: `base.edges_owned_by(path)` (O(owned edges)). Merge-join both sorted streams by identity: only-in-base → `removed`; only-in-new → `added`; both and any of `{confidence, resolved_by, provenance, flags, location, occurrences}` differ → `relocated` (delta override = tombstone + add per C5); equal → dropped (no-op).
  3. Deleted files: every owned edge is `removed`; unresolved refs list is replaced by empty.
  4. Edges whose *source* is a changed file's symbol but whose `origin_file` is another file (e.g. `OVERRIDES` from a subtype in file A to a supertype in B) are owned by the file that contains the source; the post-pass is recomputed for touched classes only and diffed the same way.
  5. Synthetic nodes: the mapper's output is compared with base synthetic nodes referenced by the file's old edges; refcount bookkeeping is finalized in INC-007 (`touched_synthetic`).
  6. `apply_globals` re-run for guards/route prefixes only if any endpoint node was added/removed or a `GlobalFact` came from a changed file; the diff is attributed to the file owning the global fact.
  - Determinism: candidate lists sorted by key, file order by path, `BTreeMap` for any collected output (CG-005 rules).
  - Complexity: O(Σ refs of changed files × (log C + d_reexport)); independent of repository size.
- **Data model changes:** None.
- **API/protocol changes:** None.
- **Concurrency semantics:** Read-only on shared tables; parallel per file; results reordered to path order. Called from `spawn_blocking`.
- **Failure behavior:** A resolver error affects only that reference (`UnresolvedReason::ResolverError`); the function itself is infallible. A panic-free contract (clippy `unwrap_used` deny) as in CG-005.
- **Idempotency considerations:** Pure; identical inputs → identical output. Re-running after a crash is safe because nothing is persisted here.
- **Security considerations:** Resolver may not escape the repository root (TSA-009); resolved targets must exist in the head tables else `NotFound`.
- **Observability additions:** span `incremental.relink_out` (attrs `files`, `refs`, `edges_added`, `edges_removed`, `edges_relocated`, `unresolved`); counters `edges_{added,removed}_total{phase="outgoing"}` (INC-010 names), `edges_relocated_total`.
- **Tests required:** `body_edit_keeps_call_edges_identical_except_locations`, `removed_call_tombstones_edge` (golden: `AuthService.authorize → PermissionService.check` removed, `DI` edge from the class unchanged), `added_import_creates_imports_and_calls_edges`, `deleted_file_tombstones_all_owned_edges`, `framework_route_edge_updated_when_decorator_changes`, `global_guard_reapplied_when_endpoint_added`, `unresolved_replaced_per_file`, `link_changed_equals_full_link_restricted_to_those_files` (property on the linker fixture).
- **Benchmarks if applicable:** `relink_out/10_files` independent of 1M base.
- **Acceptance criteria:** For the golden scenario the outgoing diff contains the removed `CALLS` edge to `PermissionService.check` and no other removals; property test equals CG-005 `link_all` output restricted to changed files.
- **Definition of done:** Global DoD.

---

### INC-006 — Re-link inbound refs of unchanged dependents (reverse index + name-index delta; never full scan)
Status: ☐

- **Task ID:** INC-006
- **Title:** `incremental::relink_in` — find unchanged files whose references may now resolve differently, using only the base reverse adjacency and `NameDelta`, then re-link just those files from cached IR.
- **Problem:** An edge `A.f → B.g` is owned by A. If B.g is removed/renamed, or a new `g` makes the name ambiguous, or a previously unresolved reference in A can now resolve, A's edges change although A did not. A full scan of all references is O(references) and forbidden (ADR-004, PRD §119); skipping it breaks incremental == full rebuild (risk R3).
- **Why it exists:** ADR-004 step 4 (second half); highest-risk task on the critical path (master plan §7); PRD §23 (dependency-aware invalidation).
- **Scope:**
  - Candidate discovery via four lookups (below), each O(result).
  - Loading IR of candidate files from the IR cache (`files_reparsed_for_relink_total` only on blob miss).
  - Re-link, edge diff and unresolved-ref replacement for candidates.
  - Escalation signal when candidates exceed a bound.
- **Explicit non-scope:** Re-resolving every reference of the repository. Changing resolution rules. Cascading beyond one round (symbol tables of unchanged files do not change, so no fixpoint is needed).
- **Files/modules expected to change:** `engine/crates/incremental/src/lib.rs`, `engine/crates/graph-storage/src/ir_cache/mod.rs` (batch get helper).
- **New files/modules expected:** `engine/crates/incremental/src/relink_in.rs`, `engine/crates/incremental/tests/relink_in.rs`.
- **Dependencies:** INC-005, INC-004, CG-007 (`for_each_edge`, `unresolved_named`, `nodes_in_file`), IDX-005 (IR cache), CG-005.
- **Implementation details:**
  ```rust
  pub enum InboundReason { TargetRemoved, TargetRenamed, TargetSignatureOrExportChanged, NameNowAmbiguous, NameNowResolvable, MemberShadowed }
  pub struct InboundCandidates { pub files: BTreeMap<RepoPath, BTreeSet<InboundReason>>, pub escalate: Option<EscalationReason> }
  pub fn find_inbound_candidates(base: &dyn GraphQuery, changes: &SymbolChangeSet, affected: &AffectedNames, changed_files: &BTreeSet<RepoPath>, cfg: &InboundConfig) -> InboundCandidates;
  pub fn relink_inbound(cx: &RelinkCx<'_>, cands: &InboundCandidates) -> Result<OutgoingResult, IncError>;
  ```
  1. **Removed/renamed targets:** for each removed or renamed key `k` (base graph): `for_each_edge(k, In, ALL)` → `edge.origin_file` is a candidate unless it is itself a changed file (already handled by INC-005). Cost O(in-degree).
  2. **Changed exports/signatures:** for each modified symbol whose `signature` or `EXPORTED` modifier changed, in-edges of kinds `{Imports, Exports, UsesType, ReturnsType, AcceptsType, Overrides, Implements}` give candidates (type-level consumers).
  3. **Names that became resolvable:** for each name in `affected.top ∪ affected.members`, `base.unresolved_named(name)` lists unresolved refs (any file) → candidate `NameNowResolvable`.
  4. **Names that became ambiguous or gained a closer candidate:** for each affected name flagged `threshold_crossed` or with an added candidate, in-edges of the *existing* candidates filtered to `resolved_by ∈ {NameUnique, NameAmbiguous, ThisMember, DiConstructor}` and non-changed origin → `NameNowAmbiguous` / `MemberShadowed`.
  - Candidate files are re-linked **whole** (simplest correct unit); IR comes from `IrCache::get` by the base `file_version_id` in batches (`buffer_unordered(16)`). The re-link uses the same `HeadSymbolTables`/overlay as INC-005, then the same per-file edge diff (identical results are dropped as no-ops, so over-approximating candidates is safe; only *missing* a candidate is a bug).
  - **Bound:** `InboundConfig { max_files: min(2_000, 5% of base files), max_edges_scanned: 1_000_000 }`. Exceeding it returns `escalate = Some(TooManyInboundFiles)`; INC-011/013 then switch to a full rebuild because the work is no longer proportional to the change.
  - Barrel/re-export chains need no special case: consumers resolved *through* a barrel have edges whose target is the final symbol, so (1)/(2) catch them when it changes.
  - Complexity: O(Σ in-degree of changed symbols + |unresolved by affected names| + Σ refs of candidate files).
- **Data model changes:** None. (Relies on `unresolved_refs` per snapshot, C6, and the in-memory by-name index from CG-004.)
- **API/protocol changes:** None.
- **Concurrency semantics:** Candidate discovery is single-threaded over immutable base; IR loading is async/bounded; re-linking is parallel per file (as INC-005).
- **Failure behavior:** IR blob missing → parse from git (counted); parse failure of an unchanged file at base is impossible unless the object is gone → `IncError::BaseObjectMissing` → rebuild. Escalation is a normal outcome, not an error.
- **Idempotency considerations:** Same inputs → same candidate set and result; over-approximation only adds no-op diffs.
- **Security considerations:** IR cache keys tenant-prefixed (IDX-005); no source read except the git fallback.
- **Observability additions:** span `incremental.relink_in` (attrs `candidates`, `by_reason`, `files_relinked`, `edges_added`, `edges_removed`, `escalated`); counters `inbound_candidate_files_total{reason}`, `inbound_escalations_total`, `files_reparsed_for_relink_total`.
- **Tests required:** `removed_callee_retargets_unchanged_caller`, `renamed_method_updates_caller_edge_via_lineage`, `new_duplicate_name_turns_unique_edge_ambiguous`, `new_symbol_resolves_previously_unresolved_reference`, `closer_super_member_shadows_inherited_target`, `barrel_consumer_follows_changed_export`, `unrelated_file_is_never_loaded` (IR-cache read log excludes decoy `report.service.ts`), `escalates_when_candidates_exceed_bound`, `no_full_reference_scan` (instrumented graph: `unresolved_named` and `for_each_edge` call counts bounded by affected sets), `inbound_result_equals_full_link_for_candidates`.
- **Benchmarks if applicable:** `relink_in/rename_with_500_callers_in_1m_graph` — latency dominated by 500 files, not 1M.
- **Acceptance criteria:** Oracle (INC-012) has zero mismatches for rename/move/ambiguity generators; instrumentation test proves no full scan.
- **Definition of done:** Global DoD; reasons table documented in `docs/graph-schema/incremental.md`.

---

### INC-007 — Delta snapshot emission
Status: ☐

- **Task ID:** INC-007
- **Title:** `incremental::delta` — assemble a validated `GraphDelta` from file outcomes, symbol changes and edge changes, then persist it as a `delta` snapshot (`snapshot_files`, tombstones, lineage).
- **Problem:** The results of INC-002..006 are scattered change records. Storage and overlays need one explicit, minimal, internally consistent `GraphDelta` (CG-010) and a snapshot row chained to the base (ADR-003).
- **Why it exists:** Target-architecture §3.5 step 5; ADR-003 (delta = changed paths + added edges + tombstones); PRD §104.
- **Scope:**
  - Node changes (file nodes, symbol nodes incl. replaced data, directory nodes, synthetic nodes by refcount).
  - Edge changes merged from INC-005/006 (override = tombstone + add).
  - `FileChange` list including `Relinked` files and `Renamed{from}`.
  - `LineageRecord`s, per-file `unresolved_replaced`.
  - `validate_delta_local`, snapshot creation, `write_delta`, status transitions, idempotent short-circuit.
- **Explicit non-scope:** Deciding the base or chain compaction (INC-013, GS-007). Loading/overlaying (INC-009). Computing invalidations (INC-008).
- **Files/modules expected to change:** `engine/crates/incremental/src/lib.rs`.
- **New files/modules expected:** `engine/crates/incremental/src/delta.rs`, `engine/crates/incremental/tests/delta.rs`.
- **Dependencies:** INC-006, GS-004, CG-010, CG-012 (`validate_delta_local`), IDX-002 (`SnapshotVersions`).
- **Implementation details:**
  ```rust
  pub struct DeltaInputs<'a> { pub base: &'a Graph, pub files: &'a [FileOutcome], pub symbols: &'a SymbolChangeSet, pub head_units: &'a [Arc<ParsedUnit>],
                               pub outgoing: &'a OutgoingResult, pub inbound: &'a OutgoingResult }
  pub fn build_delta(inp: DeltaInputs<'_>) -> Result<GraphDelta, IncError>;
  pub async fn persist_delta(store: &dyn GraphStore, req: PersistDelta) -> Result<SnapshotMeta, IncError>;
  ```
  1. **Nodes:** for each reparsed unit emit `NodeInput` for every head symbol whose data differs from the base node (any field, including range); removed keys → `nodes_removed`; file node and `Directory` chain nodes added when a new directory appears, removed when a directory loses its last file (checked via base `CONTAINS` out-degree minus removals).
  2. **Synthetic nodes:** for every key in `touched_synthetic`: `incident = base_degree(k) − removed_incident + added_incident`; `0` → `nodes_removed`; newly referenced → `nodes_added` (CG-006 refcount rule).
  3. **Edges:** union of changes from `outgoing` and `inbound`, grouped by owning file; relocations become tombstone + add of the same identity; a tombstone whose target node is also removed is kept (harmless, CG-010 counts no-ops). Edges with an endpoint removed by this delta but owned by an unchanged file must already appear as `removed` (INC-006) — `validate_delta_local` asserts it.
  4. **Files:** `Added|Modified|Deleted|Renamed{from}` from outcomes; `Relinked` for inbound-only files (carry base `file_version_id`, no new content). `Unchanged` outcomes emit nothing.
  5. **Lineage:** `SymbolChangeSet.renamed` → `LineageRecord { from, to, transition, similarity }`.
  6. All vectors sorted (`files` by path, edges by `(source, kind, target)`, nodes by key). `base_schema_version = base.schema_version()`.
  7. `validate_delta_local(base, &delta)` → any issue is `IncError::InvalidDelta` (a bug, not data).
  8. Persist: `create_snapshot(kind=Delta, base=Some(base_snapshot), purpose, versions)` → `Pending→Indexing→Persisting`, `write_delta`, `Persisting→Ready`. Before creating, `find_ready(scope, fingerprint+base)` returns the existing snapshot (idempotent retry).
  - Complexity: O(|Δ| log |Δ| + Σ deg of removed nodes for validation).
- **Data model changes:** None (rows per GS-003: `snapshot_files`, `graph_edges` with `removed=true`, `synthetic_nodes`, `symbol_lineage`, `unresolved_refs`).
- **API/protocol changes:** None.
- **Concurrency semantics:** `build_delta` is pure and single-threaded; persistence is one store transaction. Two workers producing the same `(base, head, fingerprint)` race at the `ready` partial unique index; the loser marks its snapshot `failed(duplicate)` and returns the winner's.
- **Failure behavior:** Store errors leave the snapshot `Persisting`, then the caller transitions to `Failed` (GS-001 contract); retryable errors propagate to the job layer (INC-013). An invalid delta fails before any write.
- **Idempotency considerations:** Same inputs → byte-identical delta (hash asserted via CG-011 `encode_delta`); retry reuses the ready snapshot.
- **Security considerations:** Writes go through `RepoScope`-scoped store calls; no source text in any row.
- **Observability additions:** span `incremental.delta_emit` (attrs `nodes_added`, `nodes_removed`, `edges_added`, `edges_removed`, `files`, `lineage`); `graph_store.write_delta` from GS-004; histogram `delta_size_rows`; counter `delta_snapshots_total{result=created|reused|duplicate}`.
- **Tests required:** `delta_is_minimal_for_single_body_edit` (only changed file's nodes + owned edges), `override_is_tombstone_plus_add`, `empty_directory_removed`, `synthetic_node_dropped_when_last_edge_removed`, `unchanged_outcomes_emit_nothing`, `delta_sorted_and_deterministic`, `invalid_delta_rejected_before_write`, `retry_returns_existing_ready_snapshot`, `overlay_of_delta_equals_flatten_of_full_rebuild_on_golden`.
- **Benchmarks if applicable:** `delta/emit_and_write_1k_edges` target < 100 ms p95 (GS-004 write target).
- **Acceptance criteria:** Golden scenario delta contains one removed `CALLS` edge, replaced `authorize` node, no node/edge outside `auth.service.ts`/`format.ts` ownership.
- **Definition of done:** Global DoD.

---

### INC-008 — Invalidation set computation (changed ∪ policy 1-hop dependents) + emitted cache keys / embedding invalidations
Status: ☐

- **Task ID:** INC-008
- **Title:** `incremental::invalidate` — compute which derived data is stale after a delta, by a per-change-kind edge policy, and emit precise cache keys and embedding invalidations; persist them as `graph_invalidations`.
- **Problem:** Summaries, context packages, embeddings and verification results are cached by symbol-derived keys. Too little invalidation yields stale review context; too much (file- or module-level) defeats incrementality (PRD §23: "must not automatically invalidate unrelated modules").
- **Why it exists:** Target-architecture §3.5 step 6 and §7 cache table; ADR-004 step 6; SEM tasks need a precise list of embeddings to refresh; CTX/VER caches key on input hashes that include these symbols.
- **Scope:**
  - `InvalidationPolicy` table (change kind → edge kinds/directions, depth fixed at 1).
  - `InvalidationSet` computation over base (for removed) and head overlay (for the rest).
  - Cache-key and embedding-invalidation emission.
  - Migration for `graph_invalidations`; persistence and a reader for consumers.
- **Explicit non-scope:** Executing invalidations (deleting Qdrant points, evicting PG rows: SEM-/PIPE- consumers). Transitive (>1 hop) invalidation (explicitly rejected; context assembly handles distance). Policy configurability through `.review/config.yaml` (POL tasks may override later; defaults here).
- **Files/modules expected to change:** `engine/crates/incremental/src/lib.rs`.
- **New files/modules expected:** `engine/crates/incremental/src/invalidate.rs`, `engine/migrations/0105_graph_invalidations.sql`, `engine/crates/incremental/tests/invalidate.rs`, `engine/crates/graph-storage/src/pg/invalidations.rs`.
- **Dependencies:** INC-007, CG-007, CG-010, GS-003, SID-005.
- **Implementation details:**
  ```rust
  pub enum Reason { Changed, Added, Removed, Renamed, DependentOfSignature, DependentOfBody, DependentOfRemoval, TestOfChanged, ContainerOfChanged }
  pub struct InvalidationSet { pub changed: BTreeSet<NodeKey>, pub dependents: BTreeMap<NodeKey, BTreeSet<Reason>>,
                               pub cache: Vec<CacheInvalidation>, pub embeddings: Vec<EmbeddingInvalidation> }
  pub struct CacheInvalidation { pub layer: CacheLayer /* SemanticSummary|ContextPackage|VerificationResult|ImpactIndex */, pub symbol_key: NodeKey, pub stale_body_hash: Option<Hash128>, pub reason: Reason }
  pub struct EmbeddingInvalidation { pub symbol_key: NodeKey, pub kinds: Vec<EmbeddingKind /* SymbolSummary|CodeChunk */>, pub action: EmbAction /* Reembed|Delete|Rekey{to: NodeKey} */ }
  pub struct InvalidationPolicy { pub rules: Vec<(ChangeSignature, Vec<(EdgeKind, Direction)>)> }
  pub fn compute(head: &dyn GraphQuery, base: &dyn GraphQuery, sc: &SymbolChangeSet, pol: &InvalidationPolicy) -> InvalidationSet;
  ```
  - Default policy (1 hop only): **body modified** → `Calls In` (callers' context caches), `Tests In`, container `Contains In` (class summary); **signature or export modified** → body rules + `Implements/Overrides` both directions + `UsesType/AcceptsType/ReturnsType In`; **removed** (looked up on base) → every kind In (callers lose a callee) ; **renamed** → removed rules for the old key plus `Rekey` for embeddings and cache entries; **added** → container only. Cosmetic-only (`moved_range`) → nothing but position-sensitive context caches of that symbol.
  - Caches keyed by body hash (semantic summary: `(symbol_key, body_hash, summarizer_version, model)`) emit `stale_body_hash` so only that entry is dropped; changed symbols emit `SymbolSummary + CodeChunk` re-embed; dependents emit `SymbolSummary` only when their summary input includes callee signatures (`Reembed` limited to signature-change reasons).
  - Dependents are drawn from the head overlay for survivors and from the base for removed/renamed targets. Unchanged modules with no edge into a changed symbol are never visited (`for_each_edge` is called only for changed keys: O(Σ degree of changed symbols)).
  - Persistence: `graph_invalidations(snapshot_id, organization_id, symbol_key bytea(16), layer, reason, stale_body_hash bytea NULL, embedding_action text NULL, PRIMARY KEY(snapshot_id, symbol_key, layer, reason))` with RLS; written in the same transaction family as the delta (after `Ready`, idempotent upsert).
- **Data model changes:** New table `graph_invalidations` (migration 0105), index on `(snapshot_id)`; retention follows snapshot (cascade).
- **API/protocol changes:** `InvalidationSet` JSON in job result; SEM embedding-sync job payload carries `snapshot_id` only and reads the rows.
- **Concurrency semantics:** Pure computation over immutable graphs; persistence is a batch upsert; consumers read-only.
- **Failure behavior:** Persistence failure fails the job retryably (the delta is already `Ready`; invalidations are re-computed from the stored delta on retry, deterministic). Missing base node for a removed key is logged and skipped.
- **Idempotency considerations:** Primary key makes re-insert a no-op; computation is deterministic.
- **Security considerations:** Keys/hashes only; tenant column + RLS; no source.
- **Observability additions:** span `incremental.invalidate` (attrs `changed`, `dependents`, `cache`, `embeddings`); counter `graph_invalidations_total{reason}` and `embedding_invalidations_total{action}`; histogram `invalidation_compute_duration_seconds`.
- **Tests required:** `body_change_invalidates_callers_and_tests_only` (golden: `AdminService.updateUser`, `authorize.spec` case; not `report.service`, not `UserController.update`), `signature_change_adds_type_consumers`, `removal_uses_base_edges`, `rename_emits_rekey`, `no_transitive_invalidation`, `cosmetic_change_minimal`, `unrelated_module_untouched` (edge-visit counter), `persist_is_idempotent`, `rls_isolates_invalidations`.
- **Benchmarks if applicable:** `invalidate/1k_changed_symbols` < 50 ms.
- **Acceptance criteria:** Golden set equals `{authorize}` ∪ `{AdminService.updateUser, authorize.spec case, AuthService (container)}` exactly; decoys absent.
- **Definition of done:** Global DoD; `docs/graph-schema/incremental.md` documents the policy table.

---

### INC-009 — Head graph materialization for a PR (base ⊕ delta overlay in memory)
Status: ☐

- **Task ID:** INC-009
- **Title:** `incremental::head` — produce a queryable `GraphPair { base, head }` for a PR from a delta snapshot (or an in-memory delta) without copying the base graph.
- **Problem:** Impact, context and verification read both Graph(base) and Graph(head) (ADR-011 base/head comparison). Loading two full graphs per PR is O(repository); the head must be an overlay over the already-cached base.
- **Why it exists:** Target-architecture §3.3 ("a head graph is a `GraphOverlay { base: Arc<Graph>, added, removed }`"), ADR-003; critical path (`INC-009 → DIFF-006`).
- **Scope:**
  - `materialize_head(store, cache, head_snapshot)` for persisted deltas, resolving the chain.
  - `HeadGraph::from_delta(base, delta)` for the in-process case right after INC-007.
  - `GraphPair` type shared by IMP/CTX/VER.
  - LRU integration, memory accounting, a consistency self-check hook.
- **Explicit non-scope:** Compaction (GS-007). Nested overlays (CG-010 forbids; chains are materialized by GS-005). Writing anything.
- **Files/modules expected to change:** `engine/crates/incremental/src/lib.rs`, `engine/crates/graph-storage/src/cache.rs` (pin API).
- **New files/modules expected:** `engine/crates/incremental/src/head.rs`, `engine/crates/incremental/tests/head.rs`.
- **Dependencies:** INC-007, GS-005 (`load_graph`, `load_delta`, `chain`), GS-008 (`GraphCache`), CG-010 (`GraphOverlay`), CG-007.
- **Implementation details:**
  ```rust
  pub struct HeadGraph { overlay: Arc<GraphOverlay>, pub snapshot: SnapshotId, pub base_snapshot: SnapshotId, pub commit: CommitSha }
  impl HeadGraph { pub fn query(&self) -> &dyn GraphQuery; pub fn delta(&self) -> &GraphDelta; pub fn from_delta(base: Arc<Graph>, delta: Arc<GraphDelta>, meta: HeadMeta) -> Result<Self, IncError>; }
  pub struct GraphPair { pub base: Arc<Graph>, pub head: HeadGraph }
  impl GraphPair { pub fn side(&self, s: GraphSide /*Base|Head*/) -> &dyn GraphQuery }
  pub async fn materialize_head(store: &dyn GraphStore, cache: &GraphCache, head: SnapshotId) -> Result<GraphPair, IncError>;
  ```
  1. `store.snapshot(head)` must be `Ready` and `kind == Delta`; a `Full` head returns `GraphPair` with an empty-delta overlay over itself (so callers have one shape).
  2. Parent snapshot `p = meta.base`: `cache.get_or_load(p, || store.load_graph(p))` — `load_graph` materializes the parent chain (GS-005); in the common case the parent is the default-branch full snapshot already in the LRU, so cost is one lookup.
  3. `store.load_delta(head)` returns only this snapshot's rows (O(|Δ|)); build `GraphOverlay::new(base, delta)` (O(|Δ| log |Δ|)).
  4. `GraphPair.base` is the *PR base* graph: when the head delta's parent equals the PR's base snapshot (the normal case) it is that parent; when the PR base is deeper in a chain it is the base snapshot's own materialization (also cached).
  5. The `Arc<Graph>` is pinned in the cache for the lifetime of the `GraphPair` (RAII guard) so eviction cannot drop it mid-review.
  - Optional `verify: bool` (config; on in tests and for 1% sampling in INC-013) runs `flatten()` and `validate` and compares overlay queries to the flattened graph on a node sample.
  - Memory: base shared; overlay O(|Δ|) plus overlay-local interner; reported via `heap_size_bytes()` to the cache gauge.
- **Data model changes:** None.
- **API/protocol changes:** `GraphPair` is the stable input for `impact`, `context-engine`, `verification` and the review-engine graph endpoints (API-013).
- **Concurrency semantics:** `GraphPair` is `Send + Sync`, cloneable via `Arc`; many reviewers read concurrently. Cache loads for the same snapshot are single-flighted (GS-008) so concurrent jobs do not load the base twice.
- **Failure behavior:** `IncError::{SnapshotNotReady, ChainBroken, SchemaMismatch}` → INC-013 maps `SchemaMismatch`/`ChainBroken` to rebuild (INC-011) and others to retryable errors. Never returns a partially materialized pair.
- **Idempotency considerations:** Materialization is a pure read; same snapshot → equivalent pair.
- **Security considerations:** `store` calls carry `RepoScope`; a snapshot of another repository returns `NotFound` (GS-001). Pairs are never cached across tenants (cache key includes snapshot id, which is tenant-unique).
- **Observability additions:** span `incremental.materialize_head` (attrs `snapshot`, `chain_depth`, `delta_rows`, `cache_hit`); histogram `head_materialize_duration_seconds`; gauge `graph_overlay_heap_bytes`; counter `head_materialize_total{cache=hit|miss}`.
- **Tests required:** `overlay_head_queries_equal_flattened_head`, `second_pair_for_same_base_reuses_cached_graph` (single `load_graph` call), `concurrent_materialize_single_flight`, `full_snapshot_head_has_empty_delta`, `non_ready_head_rejected`, `pinned_base_survives_eviction_pressure`, `golden_auth_bypass_head_pair` (head has `authorize` without the `PermissionService.check` call edge; base still has it), `chain_of_three_deltas_materializes`.
- **Benchmarks if applicable:** `head/materialize_1k_delta_on_cached_1m_base` < 20 ms; `head/neighbors_overlay_vs_base` overhead < 30% p95 (CG-010 target).
- **Acceptance criteria:** Pair for golden scenario answers `out_edges(authorize, Calls)` differently on base vs head; peak extra memory < 5% of base for a 1k-row delta.
- **Definition of done:** Global DoD; `GraphPair` documented in `docs/architecture/` data-flow note.

---

### INC-010 — Counters + tests proving unchanged files are not reparsed
Status: ☐

- **Task ID:** INC-010
- **Title:** `incremental::metrics` — first-class incremental counters (ADR-004), their metric export and persistence in snapshot stats, plus the instrumented tests that assert unchanged files are never reparsed.
- **Problem:** "Incremental" is a claim that must be measurable and regression-proof. Without counters asserted in tests, a refactor could silently reparse the world and nobody would notice until latency alarms.
- **Why it exists:** ADR-004 ("counters are first-class and asserted in tests"); target-architecture §3.5 step 7; milestone M3 exit; PRD §141 incremental acceptance.
- **Scope:**
  - `IncrementalCounters` (atomic, shared by INC-002..008) with the exact names: `files_reparsed_total`, `files_skipped_unchanged_total`, `symbols_{added,removed,modified,renamed}_total`, `edges_{added,removed}_total`, `graph_invalidations_total`.
  - `CountersSnapshot` (plain struct) stored into `snapshots.stats.incremental` and returned in the job result.
  - OTel instrument registration.
  - Test doubles (`CountingAnalyzer`, `CountingBlobSource`, `CountingIrCache`, `CountingGraph` wrapper) and the assertion suite.
- **Explicit non-scope:** Dashboards/alerts (OBS-007/008). Counters for storage or linker internals (their own tasks). Production sampling policy (INC-013).
- **Files/modules expected to change:** `engine/crates/incremental/src/{reparse,symbols,relink_out,relink_in,delta,invalidate}.rs` (replace local counters with this struct), `engine/crates/telemetry/src/metrics.rs` (register names).
- **New files/modules expected:** `engine/crates/incremental/src/metrics.rs`, `engine/crates/incremental/src/testkit.rs` (feature `testkit`), `engine/crates/incremental/tests/no_reparse.rs`.
- **Dependencies:** INC-002..INC-008, OBS-001 (instrument factory), GS-001.
- **Implementation details:**
  ```rust
  #[derive(Default)] pub struct IncrementalCounters { files_reparsed: AtomicU64, files_skipped_unchanged: AtomicU64, symbols_added: AtomicU64, symbols_removed: AtomicU64,
      symbols_modified: AtomicU64, symbols_renamed: AtomicU64, edges_added: AtomicU64, edges_removed: AtomicU64, graph_invalidations: AtomicU64,
      files_reparsed_for_relink: AtomicU64, inbound_files_relinked: AtomicU64, edges_relocated: AtomicU64 }
  #[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Clone)] pub struct CountersSnapshot { pub files_reparsed_total: u64, /* …one field per counter, same names… */ }
  impl IncrementalCounters { pub fn snapshot(&self) -> CountersSnapshot; pub fn publish(&self, m: &Meter, labels: &[KeyValue]); }
  ```
  - Semantics (documented once in `docs/graph-schema/incremental.md`): `files_reparsed_total` counts analyzer invocations or cache-served units for *changed-hash* paths only; `files_skipped_unchanged_total` counts changed-path entries whose hash equals base; `symbols_*` count identity-level transitions (renamed pairs are not double counted as added/removed; cosmetic range moves count nowhere); `edges_added/removed` count identity-level changes (location-only relocations are `edges_relocated`); `graph_invalidations_total` counts rows of INC-008's `InvalidationSet.dependents ∪ changed`.
  - Counters are accumulated in-process per update then published once (no per-item metric calls on hot paths); label set: `{repository_id, outcome}` only on spans, metrics use `{language}`/`{reason}` where listed in the originating task to keep cardinality bounded.
  - Persisted: `snapshots.stats = jsonb_set(stats, '{incremental}', $counters)` in INC-007 after `write_delta`.
  - **No-reparse test strategy:** build a fixture repo of ≥ 40 TS files with a scripted history; run a full index, then incremental updates for (a) one body edit, (b) one rename, (c) one file delete, (d) a revert. With `CountingAnalyzer` assert `analyzer.calls == files_reparsed_total == |changed paths with new hash|` and `CountingBlobSource.reads ⊆ changeset ∪ inbound-candidate fallback`; with `CountingIrCache` assert reads ⊆ candidates; assert zero `parse` calls for every unchanged path by name.
- **Data model changes:** `snapshots.stats.incremental` JSON object (no DDL).
- **API/protocol changes:** `CountersSnapshot` JSON Schema in `packages/contracts` (`IncrementalStats`), exposed by the index-status endpoint (IDX-002) for the latest delta.
- **Concurrency semantics:** Relaxed atomics; one `IncrementalCounters` per update (not global), merged after the update completes, so concurrent jobs cannot cross-contaminate assertions.
- **Failure behavior:** Counter publication failures are swallowed with a warn (metrics are advisory); persistence of stats failing fails nothing (stats are re-derivable) but is logged.
- **Idempotency considerations:** Counters describe one update attempt; retried attempts produce new snapshots/stats, never double-add to an existing snapshot.
- **Security considerations:** Numbers only; no repository content or paths in metric labels.
- **Observability additions:** metrics exactly as named above (monotonic counters, `unit="1"`); span events `incremental.counters` with the snapshot on span `incremental_graph_update`.
- **Tests required:**
  - `one_body_edit_reparses_exactly_one_file`
  - `revert_commit_skips_without_parse`
  - `rename_counts_renamed_not_add_remove`
  - `delete_file_counts_removals_only`
  - `unchanged_files_never_passed_to_analyzer`
  - `edge_counters_match_delta_row_counts`
  - `invalidations_counter_matches_table_rows`
  - `counters_snapshot_roundtrip_json`
  - `counters_not_shared_between_concurrent_updates`
- **Benchmarks if applicable:** `metrics/publish_overhead` < 1 µs per update path (sanity).
- **Acceptance criteria:** The suite fails if any code path calls the analyzer for an unchanged hash (mutation check: temporarily parse everything → tests red).
- **Definition of done:** Global DoD; counter semantics documented; metric names registered in the OBS catalog.

---

### INC-011 — Full-rebuild trigger evaluation (PRD §24)
Status: ☐

- **Task ID:** INC-011
- **Title:** `incremental::rebuild` — a pure decision function that returns `Incremental` or `Full{reasons, mode}` from versions, config components, snapshot health, the change set and operator flags.
- **Problem:** Incremental update is only valid when the base graph was produced by compatible code and configuration. Applying a delta onto an incompatible base silently corrupts the graph; rebuilding unconditionally wastes minutes per PR (PRD §21).
- **Why it exists:** PRD §24 (schema migration, parser incompatibility, config/source-root change, corruption, explicit request); target-architecture §3.5 last paragraph; ADR-015 table ("what each version bump invalidates"); ADR-004 (inconsistent snapshots force full rebuild).
- **Scope:**
  - `RebuildInputs`, `RebuildDecision`, `RebuildReason`, evaluation rules and ordering.
  - Recomputing head config components for changed config paths only (cheap), comparing with `snapshots.config_components`.
  - Mapping decisions to actions (`Reparse` vs `RelinkFromCache` per C11 vs compaction first).
  - `review graph rebuild` flag plumbing contract (CLI-003 sets it).
- **Explicit non-scope:** Performing the rebuild (IDX-001). Chain compaction (GS-007). The sampled consistency validator job (INC-013).
- **Files/modules expected to change:** `engine/crates/incremental/src/lib.rs`.
- **New files/modules expected:** `engine/crates/incremental/src/rebuild.rs`, `engine/crates/incremental/tests/rebuild.rs`.
- **Dependencies:** IDX-002 (`SnapshotVersions`, `ConfigComponents`, `compute_versions`), INC-001, GS-001 (`SnapshotMeta`), INIT-004/INIT-005 (config/manifest path detection).
- **Implementation details:**
  ```rust
  pub struct RebuildInputs<'a> { pub base: &'a SnapshotMeta, pub head_versions: &'a SnapshotVersions /* head commit, current binaries */,
      pub head_config: &'a ConfigComponents, pub changes: &'a ChangeSet, pub base_file_count: u64, pub flags: RebuildFlags }
  pub struct RebuildFlags { pub explicit: bool, pub consistency_failed: bool, pub inbound_escalation: bool }
  pub enum RebuildReason { Explicit, BaseNotReady, BaseInconsistent, ConsistencyCheckFailed, GraphSchemaChanged { from: u32, to: u32 },
      AnalyzerVersionChanged { language: Language, major: bool }, LinkerVersionChanged, ConfigChanged { component: ConfigComponent },
      ChangeTouchesSourceRoots, ChangeRatioTooHigh { ratio: f32 }, InboundEscalation }
  pub enum FullMode { Reparse, RelinkFromCache }
  pub enum RebuildDecision { Incremental, CompactThenIncremental, Full { reasons: Vec<RebuildReason>, mode: FullMode } }
  pub fn evaluate(inp: &RebuildInputs<'_>) -> RebuildDecision;
  ```
  - Rules (all collected, then sorted by severity; `Full` if any non-compaction reason fires):
    1. `flags.explicit` → `Explicit`; `flags.consistency_failed` / base status `Inconsistent` → full `Reparse`.
    2. `graph_schema_version` differs → full `Reparse` (decoding old rows is unsupported).
    3. Any analyzer version differs for a language present in the repo → full `Reparse` (parse cache keys include the analyzer version, so unaffected languages hit the cache); `major` is recorded for the metric only (conservative: no partial incremental across analyzer bumps, because unchanged files of that language would carry stale IR and break incremental == full).
    4. `linker` version differs → `RelinkFromCache` (C11: re-link from cached IR, no re-parse).
    5. Config: if `changes` touches `tsconfig*.json`, `package.json` workspaces, `pnpm-workspace.yaml`, `.review/config.yaml`, generated-glob inputs, recompute `ConfigComponents` at head (only these files are read) and compare per component; any difference → `ConfigChanged{component}` full `Reparse`. A tsconfig whitespace-only edit yields identical component hashes (IDX-002 normalization) → stays incremental.
    6. `changes.entries.len() / base_file_count > 0.5` → `ChangeRatioTooHigh` (rebuild is cheaper than overlay churn; threshold configurable).
    7. `inbound_escalation` (from INC-006) → `InboundEscalation`, mode `Reparse`.
    8. Chain depth ≥ 20 or delta edge churn ≥ 10% (ADR-003) → `CompactThenIncremental`; compaction is a separate job (INC-013 enqueues, never blocks the PR).
  - Pure and total: no I/O besides the cheap config-component recompute, which is injected as `&dyn ConfigReader` for tests.
- **Data model changes:** None.
- **API/protocol changes:** `RebuildDecision` serialized into the job result and `index_runs.stats`; `review graph rebuild` (CLI-003) and `POST /internal/repositories/:id/rebuild` (API-013) set `explicit`.
- **Concurrency semantics:** Pure, thread-safe.
- **Failure behavior:** Unreadable config at head → treated as `ConfigChanged` (deterministic error-marker hash from IDX-002), never a panic; unknown/older snapshot metadata fields → `Full`.
- **Idempotency considerations:** Same inputs → same decision and reason order.
- **Security considerations:** Config reads confined to repository paths from the change set.
- **Observability additions:** span `incremental.rebuild_decision` (attrs `decision`, `reasons`); counter `graph_rebuild_decisions_total{decision,reason}`; event with the human-readable reasons list.
- **Tests required:** `same_versions_and_small_change_is_incremental`, `schema_bump_forces_full_reparse`, `analyzer_minor_bump_forces_full_reparse`, `linker_bump_relinks_from_cache`, `tsconfig_paths_change_forces_full`, `tsconfig_whitespace_edit_stays_incremental`, `source_roots_change_forces_full`, `explicit_flag_wins`, `inconsistent_base_forces_full`, `half_the_repo_changed_forces_full`, `deep_chain_requests_compaction_not_rebuild`, `reasons_sorted_deterministically`.
- **Benchmarks if applicable:** None (microsecond-scale pure function).
- **Acceptance criteria:** Each PRD §24 example maps to a named reason with a passing test; INC-013 integration shows the correct action per decision.
- **Definition of done:** Global DoD; decision table in `docs/graph-schema/versioning.md`.

---

### INC-012 — Oracle property test: incremental == full rebuild under random edit sequences (proptest)
Status: ☐

- **Task ID:** INC-012
- **Title:** `incremental/tests/oracle.rs` — a proptest harness that generates valid TypeScript repositories and random multi-step edit sequences, runs the incremental pipeline and a full rebuild at every step, and requires `compare(a, b)` to be empty.
- **Problem:** Incremental resolution is subtle (inbound re-link, name ambiguity, renames, barrels). Hand-written examples cannot cover interactions; a divergence silently corrupts review context (risk R3, "Any oracle mismatch" is the early signal).
- **Why it exists:** ADR-004 (consistency validator is the oracle; mismatch fails tests); milestone M3 ("oracle property test passes 1,000 random edits"); master plan §11 property tests.
- **Scope:**
  - `RepoModel` generator (in-memory TS project) and `EditOp` strategies with shrinking.
  - `MemBlobSource` driving INC-002 and the full indexer over the same virtual files.
  - Oracle comparison with `codegraph::compare` (CG-012), including unresolved refs and locations.
  - Chain-of-deltas variant, deterministic regression corpus, CI wiring (smoke vs nightly case counts).
- **Explicit non-scope:** Real-git integration (covered in INC-013 tests). Performance measurement. Languages other than TypeScript.
- **Files/modules expected to change:** `engine/crates/incremental/Cargo.toml` (dev-deps `proptest`, `lang-typescript`, `graph-storage` with `testkit`), `engine/crates/incremental/src/testkit.rs`.
- **New files/modules expected:** `engine/crates/incremental/tests/oracle.rs`, `engine/crates/incremental/tests/oracle/{model.rs, ops.rs, driver.rs}`, `engine/crates/incremental/proptest-regressions/oracle.txt`.
- **Dependencies:** INC-009, CG-012 (`compare`, `validate`), IDX-001 (`FullIndexer` with `SourceSpec::Memory`), INC-001..008, TSA-* (analyzer), SID-006 (rename/move cases reused).
- **Implementation details:**
  ```rust
  pub struct RepoModel { files: BTreeMap<RepoPath, GenFile> }                // valid TS by construction (templates), 8–40 files, 3–6 dirs
  pub enum EditOp { AddFile, DeleteFile, ModifyBody{file,sym}, ChangeSignature{file,sym}, RenameSymbol{file,sym}, MoveSymbol{from,to,sym},
                    AddCall{from,to}, RemoveCall{from,to}, AddImport, RemoveImport, ToggleExport{file,sym}, AddDuplicateName{name}, RemoveDuplicateName,
                    EditBarrel{file}, AddInheritance{sub,sup}, TouchCommentOnly{file} }
  proptest! { #![proptest_config(ProptestConfig { cases: 256 /* smoke; 1000 nightly via PROPTEST_CASES */, max_shrink_iters: 4096, .. })]
      fn incremental_equals_full(repo in repo_model(), ops in vec(edit_op(), 1..8)) { … } }
  ```
  - Driver per case: index `repo` fully → `G0` (snapshot in `MemGraphStore`); for each step: apply op (some ops are multi-file), compute `ChangeSet` between virtual commits, run INC-011 (assert decision is `Incremental` unless the op class legitimately escalates), INC-002..008, `materialize_head`, `flatten()` → `Gi_inc`; full rebuild of the same virtual commit → `Gi_full`; assert `compare(Gi_inc, Gi_full, {ignore_locations:false, ignore_file_version_ids:true}).is_empty()`; also `validate(Gi_inc)` clean; the next step's base is the *incremental* result, so errors accumulate rather than hide.
  - Second property `delta_chain_equals_full`: apply steps as a chain of deltas without flattening between steps (GS-005 materialization) and compare at the end.
  - Generators guarantee: unique file paths, resolvable relative imports, class members with deterministic names, controlled name collisions (small identifier pool to force ambiguity), occasional parse errors (`Partial`) in leaf files.
  - On failure the harness prints a minimal repo + op list + `GraphDiffReport::render(50)` and writes the case to `proptest-regressions`; failing seeds are committed.
  - Counter checks inside the property: `files_reparsed_total ≤ |changed paths|` and zero analyzer calls for untouched paths (reuses INC-010 test doubles).
  - Complexity: each case ~ 10–50 ms; 1,000 cases ≤ ~60 s on the engine container.
- **Data model changes:** None.
- **API/protocol changes:** None.
- **Concurrency semantics:** Cases run sequentially; internal pipeline parallelism enabled (rayon) to also exercise determinism: a second run with 1 thread must equal the 8-thread run (asserted for 5% of cases).
- **Failure behavior:** Any mismatch fails the test with the rendered diff; flaky shrink is mitigated by a fixed `rng_seed` in CI and the regression file.
- **Idempotency considerations:** Fully deterministic given a seed; no wall-clock or env dependence.
- **Security considerations:** Test-only code; generated sources contain no secrets or real identifiers.
- **Observability additions:** Test output reports per-op-kind coverage (counts) so a generator that never produces `MoveSymbol` is caught by `generator_covers_all_ops` (asserts each `EditOp` variant appears in 1,000 samples).
- **Tests required:** `incremental_equals_full` (256 smoke / 1,000 nightly), `delta_chain_equals_full`, `thread_count_does_not_change_result`, `generator_produces_valid_typescript` (every generated repo parses `Ok`), `generator_covers_all_ops`, plus fixed regression tests `oracle_rename_with_unchanged_caller`, `oracle_move_symbol_between_files`, `oracle_unique_name_becomes_ambiguous`, `oracle_barrel_export_removed`.
- **Benchmarks if applicable:** None.
- **Acceptance criteria:** 1,000 cases pass in the nightly CI job; the four fixed regressions pass in smoke CI; mutation check: disabling INC-006 candidate rule (2) makes `oracle_barrel_export_removed` fail.
- **Definition of done:** Global DoD; CI job `oracle-nightly` added (CI-0xx); README in the test dir explains reproducing a failure.

---

### INC-013 — incremental-index job consumer
Status: ☐

- **Task ID:** INC-013
- **Title:** `pipeline::incremental::IncrementalIndexer` and the `incremental-index` queue consumer in `review-worker`: resolve base snapshot, decide incremental vs full, run INC-001..009, persist, enqueue follow-ups.
- **Problem:** The INC-* components are libraries. Something must claim a job for a PR head, ensure a base graph exists, choose the right path, persist a `Ready` delta snapshot, and report results idempotently so the review pipeline (PIPE) can continue.
- **Why it exists:** Master plan MVP exit steps 2–3; ADR-012 (queue `incremental-index`); target-architecture §8 span `incremental_graph_update`; PRD §141 incremental acceptance.
- **Scope:**
  - Job payload/result schemas, claim/heartbeat/cancel handling (PIPE-001 client).
  - Base resolution (default-branch full/delta snapshot for `base_sha`, else index it first).
  - Rebuild decision handling, compaction enqueue, invalidation fan-out enqueue.
  - Sampled consistency validation job.
  - `review-worker incremental-local` dev subcommand for tests/CLI parity.
- **Explicit non-scope:** Full index internals (IDX-001/004). Review stages after indexing (PIPE-003+). Embedding execution (SEM). Compaction logic (GS-007).
- **Files/modules expected to change:** `engine/crates/pipeline/src/lib.rs`, `engine/apps/review-worker/src/main.rs`.
- **New files/modules expected:** `engine/crates/pipeline/src/incremental/{mod.rs, job.rs, base.rs, validate_job.rs}`, `engine/apps/review-worker/src/cmd/incremental_local.rs`, `engine/crates/pipeline/tests/incremental_job.rs`.
- **Dependencies:** INC-008, INC-009, INC-011, GS-007, IDX-004, PIPE-001, DIFF-001.
- **Implementation details:**
  ```rust
  pub struct IncrementalIndexPayload { pub organization_id: Uuid, pub repository_id: Uuid, pub pull_request_id: Option<Uuid>, pub base_sha: CommitSha, pub head_sha: CommitSha, pub reason: IncReason /* PullRequest|DefaultBranchPush|Rebuild */ }
  pub struct IncrementalIndexResult { pub snapshot_id: Uuid, pub base_snapshot_id: Uuid, pub kind: SnapshotKind, pub decision: RebuildDecision, pub counters: CountersSnapshot }
  // idempotency_key = "incremental-index:{repository_id}:{base_sha}:{head_sha}:{fingerprint[..16]}"
  ```
  1. Claim (PIPE-001), restore `traceparent`, open span `incremental_graph_update`; verify the repository belongs to `organization_id`.
  2. Per-repository advisory lock shared with IDX-004 (`repo-index:{repository_id}`) taken `try`; busy → reschedule 30 s (attempts unchanged).
  3. Ensure objects: open the bare mirror (DIFF-001); if `head_sha` is absent, fetch via credential broker and retry once (token in memory only).
  4. Compute head versions (IDX-002); `find_ready(head fingerprint)` → complete immediately (idempotent).
  5. Resolve base: `find_ready(commit=base_sha, purpose=default_branch)`; none → enqueue `repository-index` for `base_sha` with idempotency key and reschedule this job after it (`run_after` + dependency note), never index inline.
  6. `GraphCache` load base + `BaseIndexes` (INC-004); evaluate INC-011. `Full{..}` → delegate to `FullIndexer::run` (IDX-001) with `purpose=pull_request`; `Incremental` → INC-001→002→003→004→005→006 (→ escalate to full on `InboundEscalation`)→007→008; persist invalidations; `CompactThenIncremental` additionally enqueues `repository-index{reason=compaction}`.
  7. Seed cache with the head overlay, write result, complete. Fan-out: enqueue `embedding-sync{snapshot_id}` when `embeddings` non-empty (SEM) — payload IDs only.
  8. Sampling: 1% of successful incremental updates (and 100% in non-prod) enqueue `graph-validate{snapshot_id}`; the validator full-rebuilds the head from source into a scratch snapshot, runs `compare`, and on mismatch transitions the delta snapshot to `Inconsistent`, increments `graph_consistency_failures_total`, and enqueues a forced rebuild (`explicit`-like flag `consistency_failed`).
  - Heartbeat every `lease/3`; cancellation token wired to job cancel (supersession SUP) and SIGTERM.
- **Data model changes:** None (`jobs`, `snapshots`, `index_runs{kind='incremental'}`, `graph_invalidations`).
- **API/protocol changes:** `incremental-index` payload/result JSON Schemas in `packages/contracts`; new job kinds `graph-validate`, `embedding-sync` payloads (IDs only).
- **Concurrency semantics:** One index/update per repository at a time (advisory lock); different repositories in parallel; duplicate deliveries collapse via idempotency key + fingerprint short-circuit. CPU phases in `spawn_blocking`/rayon.
- **Failure behavior:** Retryable (store backend, fetch network, lock) → job fails with backoff; non-retryable (invalid payload, base unrecoverable, `Inconsistent` after rebuild, tolerance exceeded) → `dead`. Snapshot always ends `Ready` or `Failed`; superseded/cancelled jobs mark the snapshot `Failed(cancelled)` and release the lease.
- **Idempotency considerations:** Idempotency key, fingerprint lookup, store CAS transitions; re-run after a crash creates a fresh snapshot id and never mutates a `Ready` one.
- **Security considerations:** Clone tokens in memory only, never logged; payload IDs only; tenant checks before any write; redacted `last_error`.
- **Observability additions:** root span `incremental_graph_update` (children `incremental.changeset|reparse|symbol_diff|name_delta|relink_out|relink_in|delta_emit|invalidate`); counters `incremental_jobs_total{decision,result}`, `graph_consistency_failures_total`; histogram `incremental_update_duration_seconds{decision}`; attrs `repository_id, organization_id, pull_request_id, commit_sha, job_id`.
- **Tests required:** `consumer_updates_fixture_pr_and_completes_job` (compose PG + bare repo from auth-bypass), `duplicate_delivery_returns_existing_snapshot`, `missing_base_snapshot_enqueues_repository_index`, `schema_mismatch_falls_back_to_full`, `inbound_escalation_falls_back_to_full`, `second_worker_reschedules_when_locked`, `sigterm_marks_snapshot_failed_and_releases_lease`, `sampled_validator_flags_corrupted_delta_inconsistent`, `embedding_sync_enqueued_only_with_invalidations`.
- **Benchmarks if applicable:** `incremental_job/pr_5_files_on_100k_repo` — PRD §119 "seconds" target (≤ 5 s warm base, recorded in PERF-005).
- **Acceptance criteria:** Golden PR processed end to end yields a `Ready` delta whose head overlay matches a full rebuild (`compare` empty) and counters `files_reparsed_total=2` at most.
- **Definition of done:** Global DoD; operations doc section for the queue and validator.
