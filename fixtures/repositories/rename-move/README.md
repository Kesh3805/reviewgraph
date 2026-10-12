# rename-move

Two fixtures share this directory:

- `steps/` is a scripted git history built by `fixtures/build.sh` (pinned by `EXPECTED_SHAS`).
- The numbered scenario directories are the SID-006 rename/move acceptance suite, read directly by
  `engine/crates/incremental/tests/rename_move.rs`. `build.sh` only reads `steps/`, so the
  scenarios never change the history SHAs.

## Scenario layout

```
NN-<slug>/
  base/src/...     the tree before the change
  head/src/...     the tree after the change
  expected.json    what the analyzer -> diff -> matcher chain must report
```

`expected.json`:

```json
{
  "description": "one sentence",
  "matches": [
    {
      "from": "<SymbolId in base>",
      "to": "<SymbolId in head>",
      "transition": "renamed | moved | renamed_moved",
      "rule": "exact_body | signature_and_name | token_similarity",
      "min_similarity": 0.8,
      "max_similarity": 1.0
    }
  ],
  "unmatched_added": ["<SymbolId>"],
  "unmatched_removed": ["<SymbolId>"],
  "unchanged": ["<SymbolId that must be Unchanged>"],
  "modified": [{ "id": "<SymbolId>", "flags": ["body"] }],
  "ambiguous": 0
}
```

`matches`, `unmatched_added`, `unmatched_removed` and `ambiguous` must match exactly;
`unchanged` and `modified` list a subset of the per-file diff. `max_similarity` is exclusive and
optional. Ids are canonical `SymbolId`s, so the suite also pins naming (SID-002) end to end.

## Adding a scenario

1. Create `NN-<slug>/base` and `NN-<slug>/head` with realistic TypeScript (bodies above the token
   minimums of `MatcherConfig` unless the scenario is about tiny bodies). Synthetic code only.
2. Write `expected.json` from what the change *should* produce, not from what the code does.
3. Run `cargo test -p incremental --test rename_move`. On a mismatch the test prints the actual
   outcome in `expected.json` shape; review every difference before accepting it.
4. Add the directory name to `every_scenario_dir_has_expected_json` and a named test if the
   scenario covers a new rule.

## Notes

- A container's `body_hash` folds each member into a placeholder carrying the member's kind and
  name (TSA-007), so renaming a member also marks its class `Modified{body}`.
- `05-rename-plus-small-edit` puts the rename on the class rather than on the method: a member may
  only use the fuzzy rule inside a container that paired first (SID-005).
- The Jaccard mutation check (thresholds 0.5 and 0.95 each fail a scenario) is the test
  `jaccard_threshold_mutations_are_caught`.
