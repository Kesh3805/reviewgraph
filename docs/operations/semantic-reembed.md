# Semantic collections: bootstrap, re-embed and cut-over

Runbook for SEM-004 (ADR-008). One Qdrant collection exists per embedding space and epoch, named
`rg_{provider}_{model}_{dims}_v{version}`. The PostgreSQL table `semantic_collections` tracks
each collection through `building -> active -> retiring -> retired`. At most one collection is
active (partial unique index `one_active_collection`).

## Bootstrap

A worker calls `semantic::bootstrap` (or `bootstrap_until_ready`) for its configured space under
the advisory lock `semantic_bootstrap`. It:

- creates the collection when missing (cosine distance, HNSW `m=16`, `ef_construct=128`, on-disk
  payload);
- refuses an existing collection with different dimensions (`SpaceMismatch`, a hard error);
- creates the payload indexes: `organization_id` (keyword, tenant index), `repository_id`, `kind`,
  `language`, `module`, `symbol_key`, `chunk_key`, `file_path`, `content_hash`, `snapshot_ids`
  (keyword) and `embedding_version` (integer);
- registers the collection. The first collection ever registered becomes active at once (there is
  nothing to migrate from); later ones start as `building`.

If Qdrant is unavailable the gauge `semantic_available` is 0, reviews continue with structural
context only, and `bootstrap_until_ready` retries every 60 s.

## Changing the model or forcing a re-embed

1. Configure the new space (`SEMANTIC_EMBEDDING_PROVIDER`, `_MODEL`, `_DIMS`), or bump
   `SEMANTIC_EMBEDDING_VERSION` to re-embed with the same model. Restart the workers: the new
   collection is created as `building`.
2. Writes go to every active or building collection of the writer's space (dual-write; vectors are
   embedded once). For a different model, run an index instance per space; the old one keeps
   serving reads until the cut-over.
3. Run a full sync for each repository. A space or `UNIT_TEMPLATE_VERSION` change makes the
   invalidation planner return a full plan; content hashes keep re-runs cheap.
4. Activate with `review semantic activate <name>`, or automatically once sync coverage reaches 99%
   of active units (`activate_if_covered`). The previous active collection becomes `retiring` in
   the same transaction.
5. After 7 days, `review semantic retire --older-than 7d` deletes retiring collections from Qdrant
   and marks them `retired`.

`review semantic collections` lists the registry. All commands read `DATABASE_URL`; `retire` also
needs `QDRANT_URL`.

## Tenant isolation

Collections are shared by all tenants. Every request carries the `organization_id` and
`repository_id` conditions rendered from a `TenantScope`; see
[../security/tenant-filter-audit.md](../security/tenant-filter-audit.md). Alert on
`qdrant_scope_violation_total > 0`.
