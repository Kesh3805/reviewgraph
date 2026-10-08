# Symbol identity (ADR-005)

Every symbol in a repository has one canonical identifier, and it never changes because code was
moved, reformatted or edited inside its own body. Line numbers are never part of it.

Implementation: [`engine/crates/review-core/src/symbol_id.rs`](../../engine/crates/review-core/src/symbol_id.rs).
Storage key: [`engine/crates/review-core/src/ids.rs`](../../engine/crates/review-core/src/ids.rs).
The examples below are executed as doctests (`cargo test -p review-core --doc`).

## Grammar

```text
id       = lang ":" module_path "#" qn "/" kind [ "~" ordinal ]
qn       = seg *( "." seg )
seg      = 1*byte                       ; percent-escaped, see below
ordinal  = 1*DIGIT                      ; decimal, >= 1, no leading zeros
```

| Part | Rule |
|---|---|
| `lang` | `[a-z][a-z0-9_]{0,15}`. TypeScript files use `ts`, JavaScript files (including `.jsx`, `.mjs`, `.cjs`) use `js` (`Language::id_prefix`). The graph-node prefixes reserved by CG-001 (`repo`, `dir`, `file`, `package`, `http`, `queue`, `db`, `env`, `pkg`, `test`) are rejected. |
| `module_path` | Repository-relative, forward slashes, no leading `./` or `/`, no `..`, NFC-normalized, case preserved. The final extension is stripped for `.ts .tsx .mts .cts .js .jsx .mjs .cjs`, so `foo.d.ts` becomes `foo.d` and never collides with `foo.ts`. |
| `qn` | Raw qualified-name segments (SID-002). The module symbol is `__module__`; every other symbol's name is its parent's name plus one segment. |
| `kind` | Exactly a `SymbolKind::as_id_str()` value (`method`, `get`, `set`, `enum_member`, ...). Unknown kinds fail to parse. |
| `ordinal` | Present only when symbols share `(qualified name, kind)` in one file (SID-003). Absence means "no ordinal". |

Limits: module path 1024 bytes, segment 256 bytes, whole id 2048 bytes. Over-long parts fail with
`SymbolIdError::TooLong`; they are never truncated, because truncation would silently create
collisions. `SymbolIdParts::format_lossy` is the explicit, documented degradation (an over-long
segment becomes `text…~deadbeef`, the hash being `blake3` over the full segment under the
`rg.lossy.v1` domain) and is counted as `symbol_id_lossy_total`.

The repository is a **namespace column**, not part of the string: the same id means the same symbol
in every fork and mirror of the repository.

## Escaping

Inside the module path and inside each name segment, the bytes `%`, `#`, `/`, `~`, ASCII control
characters and the space are percent-encoded as `%XX` with **uppercase** hex. `.` is additionally
escaped inside name segments (it is the segment separator) but stays literal in a module path,
where it is part of a file name such as `a.service`. Everything else, including non-ASCII UTF-8,
is kept literally.

Parsing splits on the **first** `#` and on the **last** `/` before an optional `~`, then unescapes
each part, so an escaped `%2F` or `%23` can never change the structure of an id. `SymbolId::parse`
is strict: it re-formats the parts it parsed and requires the result to equal the input, so
non-canonical spellings (lowercase escapes, escapes of bytes that need none) are rejected.

```rust
use review_core::ids::SymbolId;
use review_core::symbol::{ModulePath, SymbolKind};

// A method of a class: `AuthService.authorize`.
let method = SymbolId::parse("ts:src/auth/auth.service#AuthService.authorize/method").unwrap();
assert_eq!(method.as_str(), "ts:src/auth/auth.service#AuthService.authorize/method");

// The module symbol of the same file.
let module = SymbolId::parse("ts:src/auth/auth.service#__module__/module").unwrap();
assert_eq!(module.as_str(), "ts:src/auth/auth.service#__module__/module");

// Names are untrusted text: structure characters are escaped, then round-trip.
let parts = review_core::symbol_id::SymbolIdParts::new(
    "ts",
    ModulePath::of(&review_core::location::RepoPath::new("src/we#ird/a b.ts").unwrap()),
    vec!["a.b".to_owned(), "x#y".to_owned(), "100%".to_owned()],
    SymbolKind::Function,
);
let id = parts.format().unwrap();
assert_eq!(id.as_str(), "ts:src/we%23ird/a%20b#a%2Eb.x%23y.100%25/function");
let parsed = SymbolId::parse(id.as_str()).unwrap().parts().unwrap();
assert_eq!(parsed.qualified_name, vec!["a.b", "x#y", "100%"]);
```

## Module-path derivation and collisions

`module_path_for(path, language)` strips the final extension and normalizes the path. When several
files of one language would map to the same module path (`a.ts` and `a.tsx`), the file that comes
first in byte-lexicographic order keeps the stripped path and every later file keeps its extension
(`src/a.tsx`), so ids stay unique. `module_path_collisions` reports those groups so IDX can tell an
operator which files were disambiguated.

```rust
use review_core::language::Language;
use review_core::location::RepoPath;
use review_core::symbol_id::{module_path_collisions, module_paths};

let ts = RepoPath::new("src/a.ts").unwrap();
let tsx = RepoPath::new("src/a.tsx").unwrap();
let dts = RepoPath::new("src/a.d.ts").unwrap();
let entries = [
    (Language::Typescript, ts.clone()),
    (Language::Typescript, tsx.clone()),
    (Language::Typescript, dts.clone()),
];
let paths = module_paths(&entries);
assert_eq!(paths[&ts].as_str(), "src/a");
assert_eq!(paths[&tsx].as_str(), "src/a.tsx");
assert_eq!(paths[&dts].as_str(), "src/a.d");

let collisions = module_path_collisions(&entries);
assert_eq!(collisions.len(), 1);
assert_eq!(collisions[0].paths, vec![ts, tsx]);
```

## Storage key

`SymbolKey = hex(blake3(SymbolId)[..16])`: 32 lowercase hex characters, the first 128 bits of the
BLAKE3 digest of the canonical id string. Golden vectors are committed in
`engine/crates/review-core/tests/data/symbol_key_vectors.json` and asserted by
`cargo test -p review-core --test symbol_id`, so a future refactor cannot silently change stored
keys.

```rust
use review_core::ids::{SymbolId, SymbolKey};

let id = SymbolId::parse("ts:src/auth/auth.service#AuthService.authorize/method").unwrap();
assert_eq!(SymbolKey::of(&id).to_string(), "97ac70ec2191e38555c6678614fc4699");
```

## What is deliberately not part of identity

* **Line numbers and file offsets.** An edit that only moves code keeps the id.
* **The signature.** It is an attribute, classified with `signature_hash` (ADR-005): changing a
  parameter is `modified`, not delete plus add.
* **Renames and moves.** Those change the id, so `incremental::matcher` pairs the removed and added
  symbols and records `symbol_lineage` records; see
  [`docs/architecture/incremental.md`](../architecture/incremental.md).