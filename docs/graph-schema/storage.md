# Graph storage adapters

`graph-storage` persists snapshots behind one port, `GraphStore`. Three adapters implement it and
all three run the same conformance suite (`graph_storage::conformance::run_all`): `MemGraphStore`
(the reference), the Postgres adapter (SaaS) and the file adapter (the local CLI, no database).

The payload types are `graph_storage::model::{Graph, GraphDelta}`, the storage-side wire shape. The
graph lane's `codegraph::Graph` and `GraphDelta` are converted to it at the adapter boundary, and
`GRAPH_SCHEMA_VERSION` must stay equal to `codegraph::SCHEMA_VERSION` (`versioning.md`).
`model::flatten` is the single definition of `base + delta chain` (ADR-003), so adapters cannot
drift. The Postgres tables are created by `engine/migrations`; this page documents the file
adapter's format.

## File adapter (GS-006)

```text
.review/graph/
  manifest.json              source of truth
  snapshots/{uuid}.bin.zst   one container per snapshot
  .lock                      advisory lock file
```

### `manifest.json`

```json
{
  "format": 1,
  "repository_key": "<hex>",
  "seq": 12,
  "next_file_version": 40,
  "snapshots": [ { "id": "...", "status": "ready", "file": "<id>.bin.zst", "bytes": 1234, "blake3": "<hex>" } ],
  "heads": { "<org>/<repo>:<purpose>": "<snapshot id>" },
  "file_versions": { "<org>/<repo>": { "<path>\u0000<hash hex>\u0000<analyzer>": 17 } }
}
```

- A snapshot entry carries everything `SnapshotMeta` does (scope, commit, kind, base, chain depth,
  purpose, status, versions, stats, error, timestamps) plus `seq`, the file name, its size and its
  blake3. The file name is generated from the snapshot id, never from caller input.
- `heads` records the newest `Ready` snapshot per scope and purpose; retention never deletes one.
- `file_versions` maps `FileVersionKey::manifest_key` to the local id, per scope.
- Every row is scoped: a snapshot id from another tenant is `NotFound`, exactly as in the other
  adapters.
- A corrupt manifest (invalid JSON, unknown `format`) is `StoreError::Integrity` with a hint to run
  `review graph rebuild`.

### Snapshot container

`header || zstd(JSON payload)`, with a 52-byte header written field by field:

| Offset | Size | Field |
|---|---|---|
| 0 | 4 | magic `RGFS` |
| 4 | 1 | format = 1 |
| 5 | 1 | payload: 1 = `Graph`, 2 = `GraphDelta` |
| 6 | 2 | reserved, zero |
| 8 | 4 | `schema_version`, little-endian |
| 12 | 8 | uncompressed length, little-endian |
| 20 | 32 | blake3 of the uncompressed payload |

The payload is JSON because node attributes are free-form `serde_json::Value`s, which `bincode`
cannot read back. Files are untrusted input: the manifest's `bytes` and `blake3` are checked before
anything is decoded, the declared length is bounded (8 GiB) before inflating, the inflated stream is
capped at the declared length and the payload hash is verified before parsing. A mismatch is
`Integrity` and moves the snapshot to `Inconsistent` (counter
`graph_store_file_integrity_errors_total`). This container is separate from the `codegraph` codec
of `versioning.md`, which encodes `codegraph::Graph`.

### Writes and crash safety

A file is written as temp (`{name}.tmp-{pid}-{random}`), `sync_all`, rename over the final name,
then the directory is synced on Unix. `std::fs::rename` replaces an existing file on every platform
(`MoveFileExW` with `MOVEFILE_REPLACE_EXISTING` on Windows). The snapshot file is written before the
manifest, and the manifest is replaced the same way, so a crash leaves the old manifest or the new
one. A file written before a crash but never committed is overwritten by the retry. Files are created
with mode 0600 on Unix. Leftover `*.tmp-*` files are removed when a store is opened.

### Locking

Mutations hold an exclusive lock on `.lock`, reads a shared one, with a 30 second timeout that
fails as `StoreError::Conflict("graph store locked")`. Every operation re-reads the manifest under
the lock, so two processes on one repository serialize their mutations. Inside one process a mutex
orders operations first. Blocking I/O runs in `spawn_blocking`.

### Retention

When a snapshot becomes `Ready`, the newest 10 `Ready` snapshots are kept together with every head,
every snapshot that is not `Ready` yet, and the base chain of all of those. Other `Ready` snapshots
are removed from the manifest and their files deleted after the manifest commit (best effort).

### Loading

`load_graph`, `neighbors` and `nodes` materialize the chain (full snapshot plus deltas, through
`flatten`) and keep the last result in a one-entry cache keyed by the `(id, blake3)` of every
snapshot in the chain.
