# Incremental indexing: symbol diff and lineage

This page records the rules of the two pure building blocks the incremental indexer sits on. Both
operate on analyzer output only (ids, names and hashes, never source text) and are deterministic.
Code: `engine/crates/incremental/src/symbol_diff.rs` and `src/matcher/`.

## Symbol diff

`incremental::symbol_diff::diff_units(base, head) -> FileSymbolDiff` compares the two analyzed
versions of **one file**. Symbols are keyed by their canonical `SymbolId`, which already contains
the module path, the qualified name, the kind and the ordinal, so a diff is meaningful for one path.
A created file diffs `None` against `Some`, a deleted one `Some` against `None`.

Classification of an id present on both sides:

| `signature_hash` | `body_hash` | `attr_hash` | Result |
|---|---|---|---|
| same | same | same | `Unchanged` (`moved_range` is set when the range moved) |
| differs | any | any | `Modified` with the `signature` flag |
| any | differs | any | `Modified` with the `body` flag |
| any | any | differs | `Modified` with the `attributes` flag |

The flags are independent and combine (`names()` returns them in `signature`, `body`,
`attributes` order). `attr_hash` covers decorators and export or visibility flags. An id present
only in the head is `Added`, only in the base `Removed`.

Properties that follow from the hashes (TSA-007):

- A reformat, a comment-only change or a reorder of whole declarations is `Unchanged`.
- A container's `body_hash` folds one placeholder per member, so adding a member is `Added` for
  the member plus `Modified{body}` on the container, while editing a member body does not touch the
  container.
- Same-name collisions are diffed independently because the ordinal is part of the id.
- A shifted ordinal is not observable here: inserting a duplicate before an existing symbol gives
  the new declaration the plain id, so the plain id is `Modified` and `~1` is `Added`. The rename
  matcher repairs this pairing (see below).

Degraded input is reported instead of guessed: when either unit failed to parse the diff is
`unknown` (the caller treats every base symbol of the file as affected); on a partial parse
a differing symbol with parse errors is `Modified` with `uncertain = true`.

Output is sorted by `(kind, id)` with the kinds ordered `Unchanged`, `Modified`, `Added`,
`Removed`. `diff(a, a)` is all `Unchanged` and `diff(a, b)` mirrors `diff(b, a)` (property tested).
The counters `symbols_{added,removed,modified}_total` are aggregated from `DiffCounts` by the
caller, and `symbol_diff_uncertain_total` from `has_uncertain()`.

`fixtures/repositories/diff-basic` holds one paired file per classification and an
`expected.json`; `tests/diff_basic.rs` checks every symbol against it.

## Rename and move matcher

`incremental::matcher::match_symbols(removed, added, cfg, hints) -> MatchResult` pairs symbols
that the per-file diffs reported as removed with symbols reported as added across all changed files
of a snapshot pair, and produces `LineageRecord`s (`symbol_lineage` rows). Identity includes the
module path and the qualified name, so without this a rename looks like a removal plus an addition
and history, embeddings and dedup are lost.

### Thresholds (`MatcherConfig`)

| Field | Default | Meaning |
|---|---|---|
| `min_tokens_exact` | 6 | minimum body tokens for an exact-body match |
| `min_tokens_fuzzy` | 12 | minimum body tokens for a fuzzy match |
| `jaccard_min` | 0.8 | minimum token-shingle Jaccard similarity for a fuzzy match |
| `max_candidates` | 64 | candidate pairs examined per symbol |
| `degrade_threshold` | 50 000 | above this many removed or added symbols only rule 1 runs |
| `ambiguity_margin` | 0.02 | a winner closer than this to the runner-up is flagged `ambiguous` |

### Rule ladder

Rules run in order, each over the symbols still unmatched. A pair is only a candidate for the same
`SymbolKind` (a method never matches a function).

1. `ExactBody`: identical `body_hash` and at least `min_tokens_exact` body tokens; similarity 1.0.
   A container is matched by this rule alone and is exempt from the token minimum, because its
   folded hash always has few tokens.
2. `SignatureAndName`: identical `signature_hash` and the same simple name with a different module
   path (a pure move); similarity is the body Jaccard, at least 0.8.
3. `TokenSimilarity`: body Jaccard of at least `jaccard_min` and at least `min_tokens_fuzzy`
   tokens; similarity is the Jaccard value.

Kinds are processed containers first (class, interface, enum, namespace), then functions,
constants, variables and type aliases, then members. Once a container pair is matched, member
candidates are restricted to the children of the matched removed and added parents. Members of an
unmatched container are matched globally, by rule 1 only and only above `min_tokens_exact`.

### Assignment and ambiguity

Candidate edges are sorted by `(similarity desc, same parent match desc, same name desc, directory
distance asc, from id asc, to id asc)` and taken greedily while both endpoints are free, which
gives a one-to-one assignment. The two id tie-breakers exist for determinism only and never decide
a pairing nothing else distinguishes: when several best candidates remain equally good after the
other keys, the symbol stays unmatched and is reported in `MatchResult.ambiguous`. A wrong pairing
is worse than none. The check is symmetric (an added symbol with several plausible predecessors and
a removed symbol with several plausible successors), and only the best candidates count, so a
unique best candidate wins however many weaker ones exist.

A file-rename hint from DIFF is recorded on the candidate but is not a sort key: it can never
create a match.

### Transitions and chains

`Renamed` (name or qualified name differs), `Moved` (only the module path differs) and
`RenamedAndMoved` are persisted as `renamed`, `moved`, `renamed_moved`. Splits and merges are
independent one-to-one matches; many-to-one merges are not matched. `follow` walks chained
records across snapshots with a 32-hop cycle guard.

### Failure and degraded mode

The matcher never errors. With more than `degrade_threshold` removed or added symbols it runs
rule 1 only and sets `MatchResult.degraded`, so lineage is conservative rather than slow or wrong.

### Tests

`tests/matcher.rs` covers the rules on hand-built symbols. `tests/rename_move.rs` runs the whole
chain (analyzer, ids, hashes, diff, matcher) over `fixtures/repositories/rename-move` and includes
a mutation check that a Jaccard threshold of 0.5 or 0.95 fails at least one scenario.
