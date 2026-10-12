# diff-basic

Paired `base/` and `head/` trees for the SID-004 per-file symbol diff, read directly by
`engine/crates/incremental/tests/diff_basic.rs`. `steps/` only exists so `fixtures/build.sh --all`
treats this directory like every other fixture; the trees are never part of that history.

One file per classification (`src/<name>.ts`):

| File | Base to head | Expected |
|---|---|---|
| `unchanged.ts` | identical | `Unchanged` |
| `signature.ts` | parameter added | `Modified{signature}` |
| `body.ts` | statement edited | `Modified{body}` |
| `attribute.ts` | class decorator arguments changed | `Modified{attributes}` on the class, method `Unchanged` |
| `combined.ts` | parameter added and statement edited | `Modified{signature, body}` |
| `added.ts` | function appended | `Added` |
| `removed.ts` | function deleted | `Removed` |
| `reordered.ts` | two functions swapped | both `Unchanged`, `moved_range` |
| `reformatted.ts` | comment and blank lines added | `Unchanged`, `moved_range` |
| `collision.ts` | first of two same-name functions edited | `Modified{body}` on the plain id, `~1` `Unchanged` |
| `newfile.ts` | only in `head/` | every symbol `Added` |
| `oldfile.ts` | only in `base/` | every symbol `Removed` |

`expected.json` lists symbol ids in canonical `SymbolId` form, so it also pins naming and
ordinals end to end. `unchanged`, `added`, `removed` and `moved_range` are subsets (module symbols
are not listed); `modified` is the complete set of `Modified` symbols in the whole tree, each with
exactly the listed flags.

To add a case: add the file to `base/src` and/or `head/src` and list the expected ids in
`expected.json`.
