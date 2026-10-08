# Tenant filter audit

SEC-002 proves that the data plane never returns another tenant's data, even when two tenants hold
identical code. Symbol keys and content hashes are content-derived, so they collide across tenants
by design; isolation must come from tenant predicates, never from key uniqueness.

## Qdrant

- **Construction.** The raw Qdrant client is private to the `semantic` crate. `SemanticIndex` is
  the only public way to search, count, scroll, update or delete points, and every method takes a
  `TenantScope` (one organization and a non-empty set of its repositories). Filters come from one
  function that always starts `must` with `organization_id == org` and `repository_id in repos`.
  Callers can only add conditions through `ExtraFilter`, which has no `should` clause and rejects
  tenant keys. Point payload tenant keys are written from the scope; a unit whose repository is
  outside the scope is rejected before anything is embedded or written.
- **Defence in depth.** Every returned point is re-checked against the scope. A mismatch is
  dropped, logged at error level and counted in `qdrant_scope_violation_total` (alert on > 0).
- **Point ids** are `uuid_v5(org | repo | kind | key | space)`: identical code in two tenants never
  shares a point.
- **Audit layer.** With the `audit` feature (enabled only by dev-dependencies),
  `SemanticIndex::with_audit` records every request body and checks that each `search`, `scroll`,
  `count`, `delete` and `set_payload` request carries both tenant conditions in `filter.must` (and
  none in `should` or `must_not`), and that every upserted point carries both tenant keys. The
  `semantic` suites run with it and assert that it stays clean.
- **Tests.** `engine/crates/semantic/tests/tenant_isolation.rs` runs against an in-process Qdrant
  emulator. `tests/cross_tenant_qdrant.rs` runs against the live Qdrant of the CI integration job:
  two organizations index the same fixture concurrently into one collection; 25 queries (fixture
  symbol names and random vectors) return only the caller's points and the post-check never has to
  drop anything, while an unscoped raw HTTP probe made with plain `reqwest` inside the test (not
  part of the crate) sees both organizations, proving the test is sensitive. Compile-fail doctests
  on the crate root show that the raw client is unreachable and that a search without a scope does
  not compile.

## PostgreSQL (not implemented yet)

The statement recorder in `graph-storage`, the `xtask audit-sql` static scan with its allow-list
(`engine/security/sql-audit-allow.toml`), and the graph-read and context-package cross-tenant
tests are still open; they belong with the graph-storage and context-engine work.

## Allow-list policy

An exemption is allowed only for a statement that is cross-tenant by nature, for example a queue
job claim or the global `semantic_collections` registry (which holds no tenant data). Each entry
names the statement and gives a justification; an entry without one fails the audit.
