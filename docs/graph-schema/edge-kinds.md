# Edge kinds

`codegraph::EdgeKind` holds the **33 stored edge kinds** of PRD section 18. Three further
relations, `CALLED_BY`, `DEPENDED_ON_BY` and `TESTED_BY`, are **views** over the reverse index and
are never stored (clarification C2): `ReverseView` maps each to a stored kind read in the `In`
direction, and `EdgeSelector::parse` accepts either spelling in API kind filters. No `CalledBy`,
`DependedOnBy` or `TestedBy` variant exists in `EdgeKind` (a test asserts it).

The Rust code is the source of truth; `engine/crates/codegraph/tests/schema_docs.rs` parses every
table on this page and fails when a row disagrees with the code.

## Stored kinds

`Source` and `Target` are the allowed endpoint categories from `schema_rules::allowed`
(see `node-kinds.md` for the category of each node kind). A violation is a validator **warning**
(CG-012), never a build error, so language extensions stay possible. The `Direction` column is
`schema_rules::convention`.

| Disc | Kind | Direction | Source | Target |
|---|---|---|---|---|
| 0 | CONTAINS | parent -> child (Directory -> File/Directory, File -> top-level symbol, Class -> member) | structural, type, framework, database, messaging, test, build | structural, type, callable, data, framework, database, messaging, config, test, external, build, governance |
| 1 | DECLARES | File -> symbol declared at top level | structural, type, framework, database, messaging, test, build | structural, type, callable, data, framework, database, messaging, config, test, external, build, governance |
| 2 | IMPORTS | File -> File, or File -> ExternalDependency | structural, build | structural, external |
| 3 | EXPORTS | File -> exported symbol | structural, type, framework, database, messaging, test, build | structural, type, callable, data, framework, database, messaging, config, test, external, build, governance |
| 4 | CALLS | caller -> callee | structural, callable, framework, messaging, test, build, governance | callable |
| 5 | READS | reader -> value read | structural, callable, framework, messaging, test, build | data |
| 6 | WRITES | writer -> value written | structural, callable, framework, messaging, test, build | data |
| 7 | IMPLEMENTS | sub -> super | type, framework, database, messaging, test, build, governance | type |
| 8 | EXTENDS | sub -> super | type, framework, database, messaging, test, build, governance | type |
| 9 | OVERRIDES | sub method -> super method | callable | callable |
| 10 | REFERENCES | referencing symbol -> referenced symbol | structural, type, callable, data, framework, database, config | type, data, database, external |
| 11 | USES_TYPE | user of a type -> type | structural, type, callable, data, framework, database, messaging, config, test, external, build, governance | type |
| 12 | RETURNS_TYPE | callable -> returned type | structural, type, callable, data, framework, database, messaging, config, test, external, build, governance | type |
| 13 | ACCEPTS_TYPE | callable -> accepted type | structural, type, callable, data, framework, database, messaging, config, test, external, build, governance | type |
| 14 | ROUTES_TO | ApiEndpoint -> Controller | framework | framework |
| 15 | HANDLED_BY | ApiEndpoint -> handler (Method/Function) | structural, type, callable, framework, messaging, test, build, governance | callable |
| 16 | TESTS | TestCase -> symbol under test | structural, type, callable, framework, messaging, test, build, governance | callable |
| 17 | COVERS | TestCase -> covered symbol | structural, type, callable, framework, messaging, test, build, governance | callable |
| 18 | PRODUCES_JOB | producer -> Queue | structural, type, callable, data, framework, database, messaging, config, test, external, build, governance | messaging |
| 19 | CONSUMES_JOB | JobHandler -> Queue | callable, framework, messaging | messaging |
| 20 | READS_CONFIG | callable -> EnvironmentVariable\|Configuration | structural, type, callable, data, framework, database, messaging, config, test, external, build, governance | config |
| 21 | WRITES_CONFIG | callable -> EnvironmentVariable\|Configuration | structural, type, callable, data, framework, database, messaging, config, test, external, build, governance | config |
| 22 | READS_TABLE | callable -> DatabaseTable | structural, type, callable, data, framework, database, messaging, config, test, external, build, governance | database |
| 23 | WRITES_TABLE | callable -> DatabaseTable | structural, type, callable, data, framework, database, messaging, config, test, external, build, governance | database |
| 24 | DEPENDS_ON | File\|Package -> Package\|ExternalDependency | structural, build | structural, external, build |
| 25 | THROWS | throwing callable -> thrown type | structural, type, callable, data, framework, database, messaging, config, test, external, build, governance | type |
| 26 | CATCHES | catching callable -> caught type | structural, type, callable, data, framework, database, messaging, config, test, external, build, governance | type |
| 27 | SERIALIZES | serializing callable -> type serialized | structural, type, callable, data, framework, database, messaging, config, test, external, build, governance | type |
| 28 | DESERIALIZES | deserializing callable -> type deserialized | structural, type, callable, data, framework, database, messaging, config, test, external, build, governance | type |
| 29 | VALIDATES | validating callable -> validated value/type | callable, framework, test | type, data |
| 30 | AUTHORIZES | Middleware -> ApiEndpoint | framework | framework |
| 31 | PUBLISHES | publisher -> Queue | structural, type, callable, data, framework, database, messaging, config, test, external, build, governance | messaging |
| 32 | SUBSCRIBES | subscriber -> Queue | structural, type, callable, data, framework, database, messaging, config, test, external, build, governance | messaging |

## Reverse views

| View | Reads | Direction |
|---|---|---|
| CALLED_BY | CALLS | in |
| DEPENDED_ON_BY | DEPENDS_ON | in |
| TESTED_BY | TESTS | in |

## Edge payload

An `Edge` carries `kind`, `source`, `target` (node keys), `confidence` (permille, 0..=1000),
`resolved_by`, `provenance`, `flags`, an optional `location`, `occurrences` and `origin_file`
(the file whose content produced the edge, which owns it for re-link). Identity is
`(source, kind, target)` (clarification C5); `Ord for Edge` follows it. `merge_occurrence` keeps
the minimum location, the maximum confidence together with its `resolved_by`, and sums
occurrences, and is commutative and associative.

`EdgeFlags` bits (persisted in `graph_edges.flags`): `INSTANTIATES` 1, `DECORATOR` 2,
`TYPE_ONLY` 4, `DYNAMIC` 8, `MAPS_TABLE` 16, `GLOBAL_SCOPE` 32.

## Resolved by

| Disc | Resolved by | Confidence (permille) |
|---|---|---|
| 0 | STRUCTURAL | 1000 |
| 1 | IMPORT | 950 |
| 2 | THIS_MEMBER | 950 |
| 3 | DI_CONSTRUCTOR | 850 |
| 4 | TYPE_ANNOTATION | 800 |
| 5 | NAME_UNIQUE | 600 |
| 6 | NAME_AMBIGUOUS | 300 |
| 7 | FRAMEWORK | 900 |
| 8 | TYPE_CHECKER | 1000 |
| 9 | HEURISTIC | 500 |

The confidence column is `codegraph::confidence::TABLE`, the single source of truth for
`resolved_by -> confidence` (ADR-007). A derived edge takes the weaker of its rule and its input
(`derived`). Values change only together with a `LINKER_VERSION` bump.

## Provenance

| Disc | Provenance |
|---|---|
| 0 | ANALYZER |
| 1 | FRAMEWORK |
| 2 | LINKER |
| 3 | TYPE_CHECKER |
| 4 | HEURISTIC |
| 5 | POLICY |
