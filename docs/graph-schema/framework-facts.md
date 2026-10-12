# Framework facts

Framework knowledge (NestJS decorators, TypeORM entities, BullMQ processors, Jest suites) stays in
the language adapter. What crosses into `codegraph` is an `IrFrameworkFact`: a fact kind plus a
`BTreeMap<String, AttrValue>`. `codegraph::framework` maps facts onto generic nodes and edges and
never contains a framework identifier (`tests/framework_mapping.rs` scans the crate sources for
them). This page is the contract an adapter has to satisfy; `FactCategory::REQUIRED` and the table
below are compared by `tests/schema_docs.rs`.

A missing or malformed attribute is never an error. The fact is skipped and a `FactIssue`
(`missing_attribute`, `invalid_attribute`, `unresolved_entity`, `unknown_symbol`) is returned so it
can be stored as a parse diagnostic.

## Fact kinds to categories

| IR fact kind | Category |
|---|---|
| `Controller` | `controller` |
| `HttpRoute`, `RouteMetadata` | `route` |
| `Middleware`, `MiddlewareBinding` | `guard` |
| `QueueProducer` | `queue_producer` |
| `QueueConsumer`, `QueueJobHandler` | `queue_consumer` |
| `OrmEntity` | `entity` |
| `OrmAccess` | `db_access` |
| `ConfigRead` | `env_read` |
| `TestSuite` | `test_suite` |
| `TestCase` | `test_case` |
| `HttpGlobalConfig` | `global_prefix` |

Every other kind (`ModuleDeclaration`, `DiInjection`, `GlobalProvider`, `OrmColumn`,
`OrmRelation`, `QueueRegistration`, `TestMock`, `Custom`, ...) carries no graph construct and is
ignored by the mapper.

## Required attributes

All values are strings unless noted.

| Category | Required attributes |
|---|---|
| `controller` | `symbol` |
| `route` | `symbol,method,path` |
| `guard` | `symbol` |
| `queue_producer` | `symbol,queue` |
| `queue_consumer` | `symbol,queue` |
| `entity` | `symbol,table` |
| `db_access` | `symbol,entity,op` |
| `env_read` | `symbol,name` |
| `test_suite` | `suite_path,name,range` |
| `test_case` | `suite_path,name,range` |
| `global_prefix` | `prefix` |

`range` is a list of four integers `[start_line, start_col, end_line, end_col]`.

## Graph output

| Category | Optional attributes | Output |
|---|---|---|
| `controller` | - | the class is refined to `Controller` |
| `route` | `controller`, `prefix`, `global` | node `http:{METHOD} {path}` (`ApiEndpoint`); `HANDLED_BY` endpoint to handler; `ROUTES_TO` endpoint to the controller class when `controller` is set; the handler is refined to `Handler` only when it is a function (methods stay `Method`); a `prefix` with `global=true` becomes a global prefix fact |
| `guard` | `targets` (string or list), `controller`, `global` | the guard is refined to `Middleware`; `AUTHORIZES` guard to every endpoint handled by a target or routed to a target controller; `global=true` also emits a global guard fact; a guard with neither targets, a controller nor `global=true` is an issue |
| `queue_producer` | `job` | node `queue:{name}`; `PRODUCES_JOB` symbol to queue; job names are collected, sorted, in the queue node attributes |
| `queue_consumer` | `job` | a class is refined to `QueueConsumer`, a method or function to `JobHandler`; `CONSUMES_JOB` handler to queue |
| `entity` | `schema` | the class is refined to `DatabaseEntity`; node `db:{schema}.{table}`; `REFERENCES` entity to table with the `MAPS_TABLE` flag |
| `db_access` | `schema` | the entity is resolved to its table through its `entity` fact; `READS_TABLE` or `WRITES_TABLE` caller to table depending on `op` (`read` or `write`); an unresolvable entity is an `unresolved_entity` issue and no edge |
| `env_read` | - | node `env:{NAME}`; `READS_CONFIG` symbol to variable. The name only is recorded, never a value, and the analyzer must not evaluate `.env` files |
| `test_suite` | - | node `test:{file}#{suite path}` (`TestSuite`); `CONTAINS` file to suite |
| `test_case` | - | node `test:{file}#{suite path} › {name}` (`TestCase`); `CONTAINS` suite to case; `TESTS` case to each non-test symbol called inside `range`, with confidence `derived(Framework, call edge confidence)` |
| `global_prefix` | - | a global prefix fact applied to every endpoint when the snapshot-wide globals are applied |

Edges produced here carry `Provenance::Framework` and `ResolvedBy::Framework`, except derived
edges (`TESTS`, table edges reached through a resolved call), which use `confidence::derived`.

## Refinement

Refinement changes the displayed kind of a node, never its key: the kind segment of a `SymbolId`
keeps the syntactic kind (clarification C3). The allowed source kinds per target are the
`Refined from` column of `node-kinds.md`.

## Global facts

Global guards and global route prefixes are returned separately from the per-file output and
applied by `FrameworkMapper::apply_globals` after every file is mapped, so an incremental update can
re-run them whenever the endpoint set changes. Endpoints are sorted by key before fan-out, which
keeps the result deterministic.

## Determinism and ownership

`map_file` is pure and processes facts in source order. Synthetic nodes are owned by no file; a node
with no incident edge is dropped at build time. Edge kinds, directions and endpoint rules are in
`edge-kinds.md`; node id schemes are in `node-kinds.md`.
