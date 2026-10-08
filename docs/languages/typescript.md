# TypeScript / JavaScript analyzer reference

The `lang-typescript` crate turns one TS/TSX/JS file into a `ParsedUnit` (see
[`analysis-ir`](../../engine/crates/analysis-ir/src/unit.rs)). This page is the language reference:
what becomes a symbol, how it is named, and how identity stays stable. The rules below are a
frozen contract. Changing one needs an `ANALYZER_VERSION` major bump, which forces a re-parse of
TypeScript/JavaScript files only (ADR-015).

## Grammar selection

| Dialect | Grammar |
|---|---|
| `.ts`, `.mts`, `.cts`, `.d.ts` | tree-sitter-typescript `typescript` |
| `.tsx` | `tsx` |
| `.js`, `.jsx`, `.mjs`, `.cjs` | `tsx` (JS is a syntactic subset and `.js` files often contain JSX); language tag stays `javascript`, ids use `js:` |

Parsing is bounded: a size cap (default 1 MiB), a NUL sniff, a deadline (default 2 s, enforced by
the parser progress callback) and a 50-entry diagnostic cap. Bad source never fails a unit: ERROR
and MISSING nodes become diagnostics, declarations inside error regions are still extracted and
symbols that overlap an error are flagged `has_errors`.

## Declarations (TSA-003)

| Construct | Kind | Name | Notes |
|---|---|---|---|
| class, abstract class | `class` | declared name | heritage, decorators, type parameters kept as attributes |
| class expression bound to a variable | `class` | declarator name | `attrs.binding = const_class` |
| interface | `interface` | declared name | members: `property` / `method`; method overloads fold into one symbol |
| type alias | `type_alias` | declared name | `body_range` is the aliased type |
| enum, const enum | `enum` | declared name | `CONST_ENUM` modifier; members are `enum_member` with literal `const_value` |
| function | `function` | declared name | overload signatures fold into the implementation; with no implementation (ambient) the first signature is the symbol |
| method, getter, setter, constructor | `method`, `get`, `set`, `constructor` | member name | `static`, `async`, accessibility, `override` are modifiers |
| class property | `property` | member name | a property whose value is an arrow/function expression is a `method` (`attrs.binding = arrow_property`) |
| constructor parameter property | `property` | parameter name | synthetic, parent is the class, `attrs.from_constructor_param` |
| module `const` / `let` / `var` | `constant` / `variable` | declarator name | destructuring: one symbol per bound name, `attrs.destructured` |
| `const f = () => {}` / `function () {}` | `function` | declarator name | `attrs.binding = const_arrow` or `const_function_expr` |
| object-literal `const o = { a() {}, b: () => {} }` | `constant` plus `method` / `function` | `o`, `o.a`, `o.b`, `o.c.d` | members up to two levels below `o`; computed keys are skipped with a diagnostic |
| `namespace A.B.C {}` | `namespace` x3 | `A`, `A.B`, `A.B.C` | `declare module "pkg"` is one namespace named `pkg`; `declare global` is `global` |
| `export default class/function {}` (anonymous) | `class` / `function` | `default` | members are `default.m` |
| `export default { ... }` | `constant` | `default` | members `default.k` |
| `export default identifier` | none | | an export of an existing binding |

Not symbols: local variables inside function bodies, parameters, members of type literals,
anonymous functions (unless `AnonymousFnPolicy::Emit`, an experiment: they become
`<anonymous>` functions with ordinals starting at 1).

Values of constants whose name looks like a credential (`secret`, `token`, `password`, `api key`,
`private key`) are never stored: `const_value` is empty and `attrs.redacted = true`.

## Naming (SID-002)

`naming::qualify(parent, construct)` is pure. A symbol's qualified name is the parent's name
plus one segment; the module symbol is `["__module__"]` and top-level symbols do not repeat it.
Member name forms: identifier, string-literal key (`'a-b'` becomes `a-b`), numeric key, private
`#x` (kept, so it cannot collide with a public `x`), well-known symbols `[Symbol.iterator]`
(`@@iterator`); other computed names are skipped with `ComputedMemberName`. Segments are
NFC-normalized and capped at 256 bytes.

## Ordinals (SID-003)

`(qualified name, kind)` collisions in one file (merged interfaces, duplicate functions, a static
and an instance member with the same name) get an ordinal: members are ordered by
`(is_static, source start)`, the first keeps ordinal 0 (no suffix) and the rest get 1, 2, and so
on. Adding a duplicate after an existing symbol therefore never changes the existing symbol's
identity. Limitation: inserting an earlier duplicate shifts later ordinals; the rename matcher
(SID-005) pairs them through `body_hash`. The pass is idempotent and independent of input order.

## Syntax facts (TSA-006)

`ParsedUnit.facts` lists, per symbol, a position-free summary of what the symbol does. Facts are
attributed to the innermost enclosing symbol by range; callbacks and anonymous functions are not
symbols under the default `AnonymousFnPolicy::Attribute`, so their facts fold into the enclosing
symbol, and module-level statements land on the module symbol (`LocalId(0)`). Facts are emitted in
source order; groups are sorted by `LocalId`; at most 2,000 facts are kept per symbol (an
`UnsupportedConstruct` info diagnostic marks the cap). `AnalyzerConfig.syntax_facts = false` turns
the pass off.

`h8` is the first 8 hex characters of a BLAKE3 hash (domain `rg.fact.v1`) over the normalized token
stream of a node, so whitespace, comments, quote style and trailing commas never change a key.

| Kind | Key | Detail |
| --- | --- | --- |
| `Call` | `call:{receiver}.{name}/{argc}` (receiver chain with `this` kept, literals dropped, inner calls written `name()`) | `validation` for `validate*`, `assert*`, `plainToInstance`, `.parse`/`.safeParse` on `z.*` or `*Schema` receivers |
| `New` | `new:{Name}/{argc}` | `validation` for `new ValidationPipe` |
| `Condition` | `if:{h8}`, `ternary:{h8}`, `switch:{h8}` over the condition or switch value | `has_else`, `early_exit` (if only: the consequence is a lone `return`/`throw`), `compares_null`, `negated`, `idents` (at most 8, sorted); `cases` for `switch` |
| `Loop` | `loop:{for,for_of,for_in,while,do,foreach}:{h8 of the header}` | `awaits_inside` |
| `Throw` | `throw:{ClassName}` for `throw new X(...)`, else `throw:expr` | |
| `TryCatch` | `try:{catch,nocatch}:{finally,nofinally}` | `empty_catch`, `rethrows`, `catch_param` |
| `Await` | `await:{callee}`, `await:expr`, `await:for_of` for `for await` | |
| `Return` | `return:{void,null,undefined,true,false,lit,ident,obj}`, `return:call:{callee}`, `return:expr:{h8}` | |
| `Assignment` | `assign:{member chain}` for member targets (`assign:this.total`) | `augmented` |
| `DbWriteLike` / `DbReadLike` | `dbw:{method}:{entity or ?}` / `dbr:{method}:{entity or ?}` | `method`, `entity`, `receiver_declared_type`, `confidence` (0.9 typed, 0.75 name only, 0.6 raw SQL) |
| `TransactionWrapper` | `tx:{callee}` for `.transaction(...)`, `.runInTransaction(...)`, `start/commit/rollbackTransaction`; `tx:@Transactional` | facts inside the callback carry `in_transaction = true` |
| `GuardDecorator` | `guard:{Name}:{h8 of the arguments}` | `name` |
| `ConfigRead` | `env:NAME` for `process.env.NAME`, `process.env['NAME']`, `const { NAME } = process.env` | `name` (never a value) |

Database heuristics (`src/visit/db_heuristics.rs`): a call qualifies only when its method is in the
write table (`save, insert, update, upsert, delete, remove, softDelete, softRemove, restore,
increment, decrement, destroy, bulkCreate`, `create` on a Model-like receiver, `execute` after an
`insert()/update()/delete()` builder step, `query` whose SQL starts with `INSERT, UPDATE, DELETE,
ALTER, DROP, TRUNCATE`) or the read table (`find, findOne, findOneBy, findBy, findAndCount, count,
exists, getMany, getOne, getRawMany`, `query` with `SELECT`), *and* the receiver's declared type
ends in `Repository, EntityManager, DataSource, QueryRunner, Model, Prisma, Knex`, or its name
contains `repo, repository, manager, db, dataSource, queryRunner, prisma, knex` (or is `em`), or the
chain goes through `createQueryBuilder` (then only the terminal call counts). SQL text is classified
by its first keyword and never stored.

Guard decorators default to `^(UseGuards|Roles?|Permissions?|Public|Auth\w*|Authorize\w*|Skip\w*Auth\w*)$`
on the last name segment; `AnalyzerConfig.guard_decorator_names` replaces the pattern with an exact
list. `analysis_ir::facts::compare_keys(base, head)` returns the multiset difference by
`(kind, key)` that the change classifier consumes.
