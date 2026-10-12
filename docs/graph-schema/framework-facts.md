# Framework facts

Framework adapters in `lang-typescript` (ADR-006) turn decorators and calls into neutral
`IrFrameworkFact`s: `{ adapter, kind, symbol, attrs, range, confidence }`. Attribute maps are
sorted (`BTreeMap`), lists keep source order, and no value of the analyzed program is stored: only
names, kinds and flags. `symbol` attributes hold the qualified name (`Class.method`), the string
the graph mapper (`codegraph::framework`) resolves declarations by.

Adapters (selected with `AnalyzerConfig.enabled_adapters`): `nestjs`, `typeorm`, `bullmq`, `jest`,
`config`. An adapter runs on a file when its package is in the repository signals or the file
imports it (`jest` and `config` also run when the file mentions their globals).

Origin rule shared by decorator-driven facts: a decorator bound to the framework package (also
aliased, or through a namespace import) is 0.95; an unbound identifier of the same name is 0.7;
anything else is ignored.

## module (`ModuleDeclaration`, adapter `nestjs`)

| attr | meaning |
|---|---|
| `module_name`, `global`, `dynamic` | class name, `@Global()` present, a static method returns `{ module: X, ... }` |
| `imports`, `providers`, `controllers`, `exports` | lists of `{ name, form, import_specifier?, imported?, args? }`; `form` is `class`, `for_root`, `for_root_async`, `for_feature`, `register`, `register_queue`, `provider_object`, `forward_ref`, `spread`, `dynamic_expr` |
| provider objects | `provide` (Expr), `use_class`, `use_existing`, `use_factory` (marker only), `use_value` (kind only: `string`, `number`, `object`, `ident`, `other`), `inject` (names), `scope` |
| `metadata_ref` | set when the `@Module` argument is not an object literal (plus an `UnsupportedConstruct` diagnostic) |
| `dynamic_method` | on the extra fact emitted for each `static` method that returns a dynamic module object |

Confidence: 0.95 for literal metadata, 0.7 when any `spread` or `dynamic_expr` entry exists.

## controller, route, global config (`Controller`, `HttpRoute`, `HttpGlobalConfig`)

- `controller`: `symbol`, `prefixes` (list), `version?` (string, list or `NEUTRAL`), `host?`.
- `route`: `symbol` (handler), `controller`, `method`, `path` (controller prefix + handler path,
  normalized: leading `/`, no repeated or trailing `/`, parameter regexes removed, case kept),
  `raw_path`, `controller_prefix`, `version?` (handler `@Version` overrides the controller),
  `status_code?`, `redirect?` and `redirect_status?` (the URL is never stored), `header_names`
  (names only), `sse`, `dynamic`. A path that is not a literal or same-file constant becomes
  `{<expr>}` with `dynamic = true` and confidence 0.5. One fact per (controller prefix, path).
- `global config`: `kind = global_prefix` with `prefix`, `exclude`; or `kind = versioning` with
  `type`, `default_version?`, `version_prefix?`, `header?`, `key?`. Detected on receivers typed
  `INestApplication` / `NestExpressApplication` or assigned from `NestFactory.create(...)`, in any
  file. The mapper ignores `versioning` facts for the path prefix.

## di (`DiInjection`)

One fact per constructor parameter (and per `@Inject()` property, `param_index = -1`) of a class
with a Nest class decorator or a `*Guard|*Interceptor|*Pipe|*Filter` heritage, or with an `@Inject*`
decorator. Attributes: `symbol` (class), `param_index`, `param_name`, `type`, `type_args`, `token?`
(Expr, secret-looking text replaced by `<redacted>`), `token_kind` (`class`, `string`, `symbol`,
`forward_ref`, `typeorm_repository`, `typeorm_entity_manager`, `typeorm_data_source`,
`bullmq_queue`, `model`, `unknown`), `optional`, `scope_flags`, `property?`, `entity?`, `queue?`.
Confidence: class token 0.9, string or symbol token 0.7, interface or primitive without a token
0.3 (`unknown`).

## guard, pipe, global provider, route metadata (`Middleware`, `MiddlewareBinding`, `GlobalProvider`, `MetadataDecoratorDefinition`, `RouteMetadata`)

- `Middleware`: `symbol`, `role` (`guard`, `interceptor`, `pipe`, `filter`, `middleware`), `name`,
  `entry_symbol?` (local id of `canActivate` etc.), `uses_reflector`, `reads_metadata_keys`,
  `implements`, `global?`. Confidence 0.95 by `implements`/`@Catch`, 0.9 by `extends AuthGuard`,
  0.8 by naming plus entry method, 0.6 by name only.
- `MiddlewareBinding`: `role`, `scope` (`class`, `method`, `param`), `target`, `middleware` (list of
  `{ name, form: class|instance|call|forward_ref|dynamic_expr, args?, order, import_specifier?, imported? }`),
  plus mapper attributes `symbol`, `targets` / `controller`. An expression that cannot be parsed is
  kept as `dynamic_expr` at confidence 0.4 rather than dropped.
- `GlobalProvider`: `role`, `middleware`, `via` (`module_provider` with `module`, or `main`).
- `MetadataDecoratorDefinition`: `name`, `key`, `kind` (`set_metadata`, `create_decorator`).
- `RouteMetadata`: `symbol`, `scope`, `key`, `value` (Expr), `via_decorator`, `convention_based`
  (0.6 when the decorator is defined in another file and matched by the conventional name).

## entity, column, relation, db_access (adapter `typeorm`)

- `OrmEntity`: `symbol`, `entity`, `table`, `schema?`, `table_inferred`, `kind` (`entity`, `view`,
  `child`), `extends?`. Default table name is TypeORM's snake case of the class name.
- `OrmColumn`: `symbol`, `entity`, `property`, `column`, `column_type?`, `primary`, `generated`,
  `nullable`, `unique`, `length?`, `kind`. Defaults are not stored.
- `OrmRelation`: `symbol`, `entity`, `property`, `relation`, `target`, `inverse_property?`,
  `cascade?`, `eager`, `nullable`, `join_column?`, `join_column_name?`, `join_table?`,
  `join_table_name?`.
- `OrmAccess`: `symbol` (enclosing symbol), `entity?`, `op` (`read` or `write`), `method`, `via`
  (`repository`, `manager`, `query_builder`, `raw_sql`), `receiver_declared_type?`, `table_hint?`,
  `in_transaction`. Writes: `save insert update upsert delete remove softDelete softRemove restore
  increment decrement clear recover` and QueryBuilder chains ending in `execute()` that contain
  `insert/update/delete/softDelete/restore`. Reads: `find*`, `count*`, `exist*`, `sum`, `average`,
  `minimum`, `maximum` and builder terminals (`getOne`, `getMany`, `getRawOne`, `getRawMany`,
  `getCount`, `getManyAndCount`, `stream`). `create`, `merge`, `preload` are ignored. Raw `query()`
  is classified by its first keyword; only table names are kept, never the SQL text. Entity
  resolution: generic argument of the receiver type (0.9), `@InjectRepository` entity (0.9), first
  argument of a manager call (0.85), `getRepository(X)` / builder `from/into/update(X)` (0.8);
  unresolved entities are kept without `entity` at confidence 0.4.

## queue (adapter `bullmq`)

- `QueueRegistration`: `queue`, `queue_dynamic`, `symbol`, `via` (`register_queue`, `new_queue`,
  `new_worker`, `flow`), `options_keys` (names only), `default_job_id_set`.
- `QueueConsumer`: `symbol`, `queue`, `queue_dynamic`, `concurrency?`, `style` (`worker_host`,
  `legacy_bull`, `worker_instance`), `process_symbol?`.
- `QueueJobHandler`: `symbol`, `queue`, `job` (name or `*`), `via` (`process_decorator`,
  `switch_case`, `worker_host_process`, `worker_event`), `event?` (events carry no `job`).
- `QueueProducer`: `symbol`, `queue`, `queue_dynamic?`, `job`, `method` (`add`, `add_bulk`,
  `upsert_scheduler`, `flow_add`), `job_id_set`, `job_id_expr_kind?`, `delay_set`, `repeat_set`,
  `receiver_declared_type?`. Queue names resolve from literals and same-file constants or enum
  members; otherwise `{<expr>}` at confidence 0.5. Job payloads and connection options are never
  stored.

## test and config (adapters `jest`, `config`)

- `TestSuite` / `TestCase`: `symbol`, `suite_path` (enclosing suites joined by ` › `, empty at the
  top), `suite_path_parts`, `name`, `name_dynamic`, `modifier` (`none`, `skip`, `only`, `todo`,
  `each`), `each`, `range` (`[line, col, line, col]`), `is_test_file`; cases also `index`,
  `concurrent`. Duplicate sibling names get ` #2`, ` #3`. Facts in non-test files are 0.7.
- `TestMock`: `symbol`, `kind` (`module_mock`, `spy`, `fn`, `testing_module`, `override_provider`,
  `provide_value`), `target?`, `provider?`, `value_kind?` (never the value).
- `ConfigRead`: `symbol`, `source` (`process_env`, `config_service`, `config_service_namespaced`),
  `name`, `is_env_name`, `has_default`, `or_throw`, `declared_type_arg?`. Names must match
  `^[A-Za-z0-9_.:-]{1,128}$`; defaults and values are never stored.
