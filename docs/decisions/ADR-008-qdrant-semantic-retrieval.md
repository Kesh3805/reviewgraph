# ADR-008 — Qdrant for semantic retrieval, subordinate to structure

**Status:** Accepted · 2026-10-02

## Context
Semantic similarity is useful for context retrieval, conventions, docs and finding history. It must not replace structural retrieval (Invariant 10).

## Decision
- **Role.** Qdrant is the vector store. It supplements graph and lexical retrieval and never replaces them. It is not a source of truth: everything in it can be rebuilt from PostgreSQL plus source.
- **Collections.** There is one collection per embedding space: `rg_{provider}_{model}_{dims}_v{n}`. The logical kind (`symbol_summary`, `code_chunk`, `doc`, `convention`, `finding_history`) is a payload field with a keyword index.
  - **Why one collection, not one per kind.** All kinds share one query pattern: tenant-filtered kNN in the same vector space. Context retrieval often wants mixed kinds in a single query, and separate collections multiply the HNSW overhead. Qdrant's payload index plus filtered HNSW handles selective filters.
  - Revisit this if the benchmark shows filtered recall degrading for one kind (task SEM-009).
- **Mandatory payload fields:**
  - `organization_id`, `repository_id`, `kind`, `language`, `module`
  - `symbol_key` or `chunk_key`, `file_path`, `start_line`, `end_line`
  - `content_hash`, `embedding_model`, `embedding_dims`, `embedding_version`
  - `snapshot_ids`
- **Point ID.** `uuid_v5(NAMESPACE, org|repo|kind|key|space)`.
- **Incremental writes.**
  - Compare the stored `content_hash` before embedding. Only changed or added items are embedded.
  - Delete points that are absent from the default-branch head and from every open-PR snapshot.
  - Use lineage (ADR-005) to re-key renamed symbols without re-embedding when `body_hash` is unchanged.
- **Tenant isolation.** The search API requires a `TenantScope { organization_id, repository_ids }` value, always rendered as a `must` filter. A test asserts that no query can be built without it.
- **Transport.** The REST API via `reqwest`, which keeps the dependency set small. Its latency is irrelevant next to model calls.

## Alternatives
| Option | Rejected because |
|---|---|
| pgvector | One fewer service, but weaker filtered ANN at about 1M vectors per tenant. The stack mandate is Qdrant. |
| Embed the whole repository and retrieve by similarity | Explicitly rejected. |

## Consequences
- An embedding model change means a new collection and a background re-embed, followed by a cut-over.
- Vectors from different spaces are never mixed.
