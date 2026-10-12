# Performance benchmarks

Scale targets come from PRD section 120 (100,000 source files, 1,000,000 symbols, multi-million
edge graph) and ADR-014 (graph database trigger thresholds). No real repository is that large, so
the benchmarks run on a generated one.

## Synthetic repository generator (PERF-001)

`benchmarks/perf/synth-repo` is a deterministic NestJS-shaped TypeScript generator.

```
cd engine
cargo run --release -p synth-repo -- --profile s --seed 42 --out /tmp/synth-s
cargo run --release -p synth-repo -- --profile l --seed 42 --dry-run     # manifest only
cargo run --release -p synth-repo -- --profile s --out /tmp/s --git --churn 9
```

| Profile | Modules x features | Files    | Symbols (target) | Use                           |
| ------- | ------------------ | -------- | ---------------- | ----------------------------- |
| `s`     | 4 x 30             | ~1,000   | ~15,000          | mirrors the reference repo    |
| `m`     | 40 x 30            | ~10,000  | ~100,000         | PERF-005 scaling comparison   |
| `l`     | 400 x 30           | ~100,000 | ~1,000,000       | PRD section 120 target        |

Profile definitions live in `profiles/*.yaml`; a test keeps them in sync with the built-in values.

Properties:

- `(seed, profile, generator_version)` fully determines every byte. Each module is generated from
  its own RNG stream (SplitMix64 seeded by `blake3(seed || "module" || index)`), merged in module
  order, so output does not depend on the thread count.
- `manifest.json` records exact counts (files, symbols by IR kind, routes, entities, processors,
  planted call edges by class, hub fan-in) and `tree_hash`, the `blake3` of the sorted
  `(path, blake3(bytes))` list.
- Cross-file calls: 80% constructor-injected services of the same module, 15% shared util
  functions, 5% hub services (20 hubs) of other modules chosen by a Zipf(1) distribution; 2% of method
  statements call a name on an untyped receiver (deliberately ambiguous). About 3% of features have
  a `generated/` file carrying a `@generated-by synth-repo` header.
- `--git` commits `src/` with a fixed author and date (stable SHAs). `--churn N` adds a head commit
  that modifies exactly N service files, cycling through three edit classes: body-only edit, hub
  signature change, file rename.
- Refuses to write into a non-empty directory unless it holds a matching, intact manifest (then it
  verifies and exits 0). A free-disk precheck (estimated bytes x 1.2) fails fast.

Generated trees are never committed (cache them under a path ignored by git, for example
`/tmp/synth-*` or `benchmarks/perf/.cache/`).

Reference VM for reported numbers: GitHub-hosted `ubuntu-24.04` runner (4 vCPU, 16 GB RAM); the
bench workflow records the CPU model in its summary.
