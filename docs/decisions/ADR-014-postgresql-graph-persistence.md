# ADR-014 — PostgreSQL as the durable graph store, with no graph database by default

**Status:** Accepted · 2026-10-02

## Context
The PRD (§102–103) asks for graph operations that are not tied to any one graph database. We want the fewest stateful systems we can get away with.

## Decision
- **Storage:** PostgreSQL 16 holds the graph rows, using the ADR-003 layout and the PRD §103 indexes.
- **Traversals:** these run in the Rust in-memory graph. The graph is loaded once per snapshot and kept in an LRU cache. Traversals do not use recursive SQL.
- **SQL-side queries:** these cover single-hop, indexed lookups for the API.
- **Migrations:**
  - Files live at `engine/migrations/*.sql` and run through `sqlx::migrate!`.
  - Either `review-worker migrate` or `review migrate` applies them.
  - They are the only source of schema for both languages. Nothing runs DDL at runtime.
- **When a graph database comes back into consideration:** only if benchmarks on the 1M-symbol synthetic repository (PERF tasks) show at least one of the following:
  - direct-neighbour p95 latency above 20 ms from SQL
  - in-memory graph load time above 30 s
  - memory use above 50% of worker RAM

## Alternatives
| Option | Why it was rejected |
|---|---|
| Neo4j / Memgraph | Adds another stateful system with its own tenancy model. Our traversals are bounded (depth ≤ 3) and fit in memory. |
| SQLite per repository | Works well locally but poorly for multi-tenant SaaS. The local CLI uses a file adapter instead. |

## Consequences
- Graph load time and memory are tracked as first-class benchmarks.
