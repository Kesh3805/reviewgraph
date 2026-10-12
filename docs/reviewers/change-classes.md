# Change classes

Every non-cosmetic modified or renamed symbol of a pull request is classified by comparing its
syntax facts (TSA-006) on the base and head sides. Classification is deterministic: no source
text is read and no model is called. Added and removed symbols are not classified (an added
method is not "condition changed"); a symbol whose facts are unavailable (parse failure, an
analyzer run without `syntax_facts`) gets no classes and is listed in
`ChangedSymbolSet.facts_unavailable`.

Wire strings are the snake_case names of `review_core::change::ChangeClass`. Each classified
change carries `evidence` (removed facts on the base side, added facts on the head side),
bounded `detail` entries (counts and normalized fragments of at most 120 characters, never
source text) and a `certainty` of `Exact` or `Heuristic`.

## Fact diff

`diff_facts(base, head, kinds)` filters both fact lists to `kinds`, then computes a multiset
diff over `(kind, key)` and a `reordered` flag (the facts present on both sides appear in a
different order). Keys carry no positions, so moving or reformatting code changes nothing.

## Structural classes (CHG-002)

| Class | Fires when | Detail |
| --- | --- | --- |
| `control_flow_changed` | Branch points are inserted or removed (the ordered sequence of branch-point kinds differs), or the same branch points appear in a different order. Branch points are statement conditions (`if`, ternary, `switch`), loops, `throw`, `try` and `return`. A comparison used as a value (`return a === b`) is not a branch point. An edited condition is `condition_changed`, not this. | `added`, `removed`, `reordered` |
| `condition_changed` | A condition fact is added, removed or edited. Comparison and logical expressions in value position (`return user.role === 'admin'`, `const ok = a && b`) are condition facts too, so a changed predicate is visible wherever it sits. | `added`, `removed`, `old`, `new` (condition key hashes) |
| `loop_changed` | A loop is added, removed, changes kind (`for_of` to `while`) or gets a different header or condition. | `subtype` (`added`, `removed`, `kind_changed`, `header_changed`), counts, `old`, `new` |
| `return_changed` | The multiset of `return` expression shapes differs: a different expression, an added early return, a removed return. | `added`, `removed` |

### Syntax fact requirement

The TypeScript analyzer emits a `Condition` fact with key `expr:<hash>` and detail
`expression = true` for the outermost comparison (`===`, `!==`, `==`, `!=`, `<`, `>`, `<=`,
`>=`) or logical (`&&`, `||`) expression that is not an operand of another one and not the
condition of an `if`, loop or ternary (those already have their own facts).
