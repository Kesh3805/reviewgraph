# Context package

`ContextPackage` (engine/crates/context-engine/src/model) is the per-reviewer, per-cluster context handed to a model.

- `items`: sorted by pinned first, score descending, item id. Each item has kind, node, side, title, relation, distance, path, sources, signals, score, content, locations (`path:start-end@side`), tokens, pinned.
- `omitted`: items left out with an `OmitReason`, capped at 200 with `omitted_overflow` counting the rest.
- `budget`, `token_estimate`, `risk_signals`, `change_summary`, `provenance`, `package_hash`.
- `to_model_input()` maps items to the model input sections: changedSymbols, changeSummary, graphNeighborhood, tests, repositoryRules, riskSignals, deterministicFindings.
