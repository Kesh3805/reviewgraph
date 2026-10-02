# ADR-004 — Incremental graph strategy

**Status:** Accepted · 2026-10-02

## Context
PRD §21–24 forbid rebuilding the graph for a PR review. The work done must scale with the size of the change and the code that depends on it, not with the size of the repository.

## Decision
The algorithm is specified in target-architecture §3.5:
1. Skip unchanged files by comparing content hashes.
2. Parse only the changed files, reusing the parse cache.
3. Diff the symbols of each changed file, including rename and move matching.
4. Re-link the outgoing references of every changed file. Re-link inbound references only from unchanged files whose targets were removed or renamed, or whose names became ambiguous. Find those files through the reverse index and a name-index delta, never by scanning.
5. Emit a delta snapshot.
6. Compute the invalidation set: the changed symbols plus their 1-hop dependents, as defined by policy.

Counters are first-class and asserted in tests:
- `files_reparsed_total`
- `files_skipped_unchanged_total`
- `symbols_{added,removed,modified,renamed}_total`
- `edges_{added,removed}_total`
- `graph_invalidations_total`

A **consistency validator** is the oracle for incremental updates. It rebuilds the graph from scratch and diffs it against the incremental result. It runs in tests and as a sampled background job.
- In tests, a mismatch fails the test.
- In production, a mismatch marks the snapshot `inconsistent` and forces a full rebuild.

## Alternatives
| Option | Rejected because |
|---|---|
| Re-link every reference on each PR | Costs O(references). Kept as the oracle, and acceptable on small repositories, but not at 1M symbols. |
| Invalidate at file level only | Misses name-resolution changes in files that did not change. |

## Consequences
- The linker must expose a name index that supports deltas.
- Resolution must be deterministic: candidates are sorted and ties are broken stably.
