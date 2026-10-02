# ADR-007 — tree-sitter syntax plus optional semantic enrichment

**Status:** Accepted · 2026-10-02

## Context
tree-sitter is fast, incremental, error-tolerant and polyglot, but it is not a type checker. The reference CodeGraph shows that heuristic resolution reaches useful precision on NestJS when its confidence is recorded:
- import-bound edges: 0.9
- constructor-typed dependency injection: 0.7–0.9

## Decision
- **Primary layer.** tree-sitter 0.25 with tree-sitter-typescript 0.23. Both are verified to build in the engine container.
- **Heuristic resolution.** Resolution heuristics carry a calibrated confidence. All values live in one table, `codegraph::confidence`.
- **TypeScript semantic enrichment** is an optional Node helper, `engine/tools/ts-semantic`, built on the `typescript` program and type checker.
  - It is invoked only for ambiguous references in changed or impacted regions. It never runs over the whole repository for a PR.
  - Results are cached by the hash of the file-version set.
  - It is **disabled by default in the MVP**. It is enabled per repository when the benchmark shows a precision gain that justifies its latency.

## Alternatives
| Option | Rejected because |
|---|---|
| Run the TypeScript compiler for everything | Slow: a full program build per PR violates the latency targets. It also requires an `npm install` of the target repository. |
| tree-sitter only, permanently | Caps precision on dynamic dispatch. Keeping the provider as an option preserves the upgrade path. |

## Consequences
- With the helper enabled, edges carry `resolved_by=type_checker` and confidence 1.0.
- The benchmark (BEN tasks) measures edge precision against hand-labelled fixtures, with and without the helper.
