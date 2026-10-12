# Node kinds

`codegraph::NodeKind` is the node taxonomy of PRD section 17: **44 kinds, including `Repository`**
(clarification C1: the "46 + Repository" figure in the target architecture is a miscount).
The Rust enum is the source of truth; `engine/crates/codegraph/tests/schema_docs.rs` parses the
table below and fails when a row disagrees with the code, so this page cannot drift silently.

Rules that hold for every kind:

- The `repr(u8)` discriminant and the spelling in the `Kind` column are persisted
  (`node_kinds` lookup table in `graph-storage`). A discriminant is never reused or renumbered;
  removing a kind requires a `SCHEMA_VERSION` bump (see `versioning.md`). Gaps between blocks of
  ten leave room for language-specific kinds.
- `Category` is `NodeKind::category()` (`structural`, `type`, `callable`, `data`, `framework`,
  `database`, `messaging`, `config`, `test`, `external`, `build`, `governance`).
- `Refined from` is `NodeKind::refinable_from()`: the syntactic kinds a framework fact may
  refine a node into this kind from (clarification C3). Refinement changes the displayed kind only;
  the kind segment of a `SymbolId` keeps the syntactic kind, so the node key never changes.
- `Synthetic` marks kinds that only ever exist as synthetic nodes (`NodeKind::is_synthetic_only()`),
  which the CG-012 orphan warning relies on.
- `ID scheme` is the deterministic node id for kinds that are not source symbols. Every other kind
  is identified by its `SymbolId` (`symbol-id.md`). The node key is always the 128-bit blake3
  prefix of the id string (`NodeId::key`).

## Kinds

| Disc | Kind | Category | Refined from | Synthetic | ID scheme |
|---|---|---|---|---|---|
| 0 | Repository | structural | - | yes | `repo:/` |
| 1 | Package | structural | - | no | `package:{dir}` |
| 2 | Module | structural | - | no | `SymbolId` |
| 3 | Directory | structural | - | yes | `dir:{path}` (root `dir:.`) |
| 4 | File | structural | - | no | `file:{path}` |
| 10 | Namespace | structural | - | no | `SymbolId` |
| 11 | Class | type | - | no | `SymbolId` |
| 12 | Interface | type | - | no | `SymbolId` |
| 13 | Struct | type | - | no | `SymbolId` |
| 14 | Trait | type | - | no | `SymbolId` |
| 15 | Enum | type | - | no | `SymbolId` |
| 16 | TypeAlias | type | - | no | `SymbolId` |
| 20 | Function | callable | - | no | `SymbolId` |
| 21 | Method | callable | - | no | `SymbolId` |
| 22 | Constructor | callable | - | no | `SymbolId` |
| 23 | Property | data | - | no | `SymbolId` |
| 24 | Field | data | - | no | `SymbolId` |
| 25 | Parameter | data | - | no | `SymbolId` |
| 26 | Variable | data | - | no | `SymbolId` |
| 27 | Constant | data | - | no | `SymbolId` |
| 30 | ApiEndpoint | framework | - | yes | `http:{METHOD} {path}` |
| 31 | Controller | framework | Class | no | `SymbolId` |
| 32 | Handler | framework | Function | no | `SymbolId` |
| 33 | Middleware | framework | Class, Function | no | `SymbolId` |
| 40 | DatabaseEntity | database | Class | no | `SymbolId` |
| 41 | DatabaseTable | database | - | yes | `db:{schema}.{table}` |
| 42 | DatabaseColumn | database | - | yes | `SymbolId` |
| 43 | Migration | database | - | no | `SymbolId` |
| 50 | Queue | messaging | - | yes | `queue:{name}` |
| 51 | QueueProducer | messaging | - | no | `SymbolId` |
| 52 | QueueConsumer | messaging | Class | no | `SymbolId` |
| 53 | JobHandler | messaging | Method, Function | no | `SymbolId` |
| 60 | Configuration | config | - | no | `SymbolId` |
| 61 | EnvironmentVariable | config | - | yes | `env:{NAME}` |
| 70 | TestSuite | test | - | yes | `test:{file}#{suite path}` |
| 71 | TestCase | test | - | yes | `test:{file}#{suite path} › {name}` |
| 72 | Fixture | test | - | yes | `SymbolId` |
| 80 | ExternalDependency | external | - | yes | `pkg:{ecosystem}/{name}` |
| 81 | ExternalApi | external | - | no | `SymbolId` |
| 90 | BuildTarget | build | - | no | `SymbolId` |
| 91 | CliCommand | build | Class, Function | no | `SymbolId` |
| 92 | Worker | build | Class | no | `SymbolId` |
| 100 | DocumentationRule | governance | - | no | `SymbolId` |
| 101 | ArchitecturalBoundary | governance | - | no | `SymbolId` |

## Synthetic id normalization

All constructors live on `NodeId` (`node_id.rs`), are pure functions of their inputs and are
idempotent (`normalize(normalize(x)) == normalize(x)`, property-tested).

| Kind | Constructor | Normalization |
|---|---|---|
| Repository | `NodeId::repository()` | singleton; the repository itself is a namespace column |
| Directory | `NodeId::directory(&RepoPath)` | repo-relative, `/` separators, no trailing `/`, root is `dir:.` |
| File | `NodeId::file(&RepoPath)` | repo-relative, extension kept |
| Package | `NodeId::workspace_package(&RepoPath)` | workspace package directory |
| ApiEndpoint | `NodeId::http(method, path)` | method upper-cased (`ALL` allowed); path gets a leading `/`, `//` collapsed, trailing `/` stripped except for the root, path parameters (`:id`, `{id}`, `[id]`, `*`) become `{}`, query string removed, case preserved |
| Queue | `NodeId::queue(name)` | trimmed, case preserved |
| DatabaseTable | `NodeId::table(schema, table)` | lower-cased unless quoted, default schema `public` |
| EnvironmentVariable | `NodeId::env(name)` | exact case, must match `^[A-Za-z_][A-Za-z0-9_]*$`; the name only, never a value |
| ExternalDependency | `NodeId::package(ecosystem, spec)` | npm `@scope/name` or `name`; deep imports (`lodash/fp`) reduced to the package; `node:` builtins become `pkg:node/{module}`; no version |
| TestSuite / TestCase | `NodeId::test(file, suite_path, name)` | suites joined by U+203A, whitespace collapsed, duplicates in a file get a `~{n}` ordinal in source order |

Failures are values, never panics: `NodeIdError::{EmptyName, InvalidEnvName, InvalidHttpMethod,
PathOutsideRepository}`.

## Reserved prefixes

`RESERVED_PREFIXES` (`repo`, `dir`, `file`, `package`, `http`, `queue`, `db`, `env`, `pkg`, `test`)
may not collide with a language prefix of `SymbolId` (`ts`, `js`, ...); a unit test asserts it.

## IR symbol kinds

`NodeKind::from_ir_symbol_kind` maps the syntactic kinds of `analysis-ir`: enum members become
`Constant` (the analyzer records `enum_member` as an attribute), getters and setters become
`Property`; every other kind maps to the node kind of the same name.
