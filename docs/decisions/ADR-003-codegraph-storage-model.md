# ADR-003 — CodeGraph storage model: content-addressed file versions plus snapshot overlays

**Status:** Accepted · 2026-10-02

## Context
We need a durable graph for every commit we review. The constraints:
- Every PR compares a base graph with a head graph.
- Many PRs share the same base.
- Repositories can reach 1M symbols and several million edges.

Copying the whole graph for each commit costs O(repository size) per PR, which violates Invariant 1.

## Decision
- **Per-file data is content-addressed.** The `file_versions` table holds each file's symbols, unresolved references and parse result, keyed by `(repository, path, content_hash, analyzer_version)`. A file that has not changed is stored once and shared by every snapshot that contains it.
- **Resolved cross-file edges belong to a snapshot.**
  - A *full* snapshot stores every edge.
  - A *delta* snapshot stores only the edges it adds and tombstones for the edges it removes, relative to `base_snapshot_id`.
  - Readers materialize `base ⊕ deltas`.
- **Delta chains are compacted.** A new full snapshot is written once a chain passes 20 deltas or 10% edge churn.
- **In memory:** an immutable `Graph` (CSR-style adjacency, forward and reverse, partitioned by edge kind), plus a `GraphOverlay` for PR heads.

## Alternatives
| Option | Rejected because |
|---|---|
| Full copy per snapshot | O(repository size) writes for every PR. |
| Temporal `valid_from`/`valid_to` commit ranges on rows | History branches: PR heads start from arbitrary bases, so the ranges are not linear. |
| Graph database | Rejected by default (ADR-014). |

## Tradeoffs
- Reads have to resolve the overlay.
- Compaction runs as a background job.

## Consequences
The `GraphStore` trait exposes:
- `load_graph(snapshot)`
- `write_full`
- `write_delta`
- `neighbors`, for single-hop SQL lookups from the API

## Migration implications
None. This is new.
