//! The `GraphStore` port (GS-001): what every graph adapter must do.
//!
//! Two adapters implement it — Postgres (GS-004/005) for SaaS and the file adapter (GS-006) for
//! the CLI — and both must pass the same [`conformance`](crate::conformance) suite unchanged
//! (target-architecture §3.4). `MemGraphStore` is the reference implementation the suite is
//! validated against.
//!
//! ## Status machine
//!
//! ```text
//! Pending ─▶ Indexing ─▶ Persisting ─▶ Ready ─▶ Inconsistent
//!    │           │            │
//!    └───────────┴────────────┴──────▶ Failed
//! ```
//!
//! [`transition`] is a compare-and-swap: exactly one of N concurrent callers with the same
//! `from` receives `true`, every other one receives `false` — never an error, so retries are
//! safe. Everything not in [`ALLOWED_TRANSITIONS`](crate::status::ALLOWED_TRANSITIONS) returns
//! `false`.
//!
//! ## Contracts every adapter upholds
//!
//! * Every method takes or resolves a [`RepoScope`] and filters on `organization_id` and
//!   `repository_id`; an id from another tenant is [`StoreError::NotFound`] — never an
//!   existence oracle.
//! * Readers (`load_graph`, `load_delta`, `neighbors`, `nodes`, `chain`) only see `Ready`
//!   snapshots; anything else is [`StoreError::InvalidStatus`].
//! * `write_full`/`write_delta` require `Persisting` and refuse an already `Ready` snapshot
//!   with [`StoreError::InvalidStatus`]; a failed write leaves the row in `Persisting` for the
//!   caller to fail.
//! * `write_delta` additionally requires the delta's base to be `Ready` in the same scope.
//! * A loaded graph never mixes tenants or repositories, and `load_graph` refuses a snapshot
//!   written by a different [`GRAPH_SCHEMA_VERSION`](crate::types::GRAPH_SCHEMA_VERSION).
//!
//! ## Spans
//!
//! Adapters instrument these span names with the attribute `adapter = pg | file | mem`:
//! [`spans::WRITE_FULL`], [`spans::WRITE_DELTA`], [`spans::LOAD_GRAPH`], [`spans::NEIGHBORS`].

use async_trait::async_trait;
use repository::store::RepoScope;
use review_core::ids::SnapshotId;

use crate::kinds::{Confidence, Direction, EdgeKindSet, NodeKey};
use crate::model::{Graph, GraphDelta};
use crate::status::SnapshotStatus;
use crate::types::{
    FileVersionInput, FileVersionKey, FileVersionRef, NeighborPage, NewSnapshot, SnapshotMeta,
    SnapshotQuery, SnapshotStats, StoredNode, WriteStats,
};
use crate::StoreError;

/// Span names adapters must use (GS-001 observability).
pub mod spans {
    pub const WRITE_FULL: &str = "graph_store.write_full";
    pub const WRITE_DELTA: &str = "graph_store.write_delta";
    pub const LOAD_GRAPH: &str = "graph_store.load_graph";
    pub const NEIGHBORS: &str = "graph_store.neighbors";
}

/// Persistence of graph snapshots. All methods take `&self` and may run concurrently.
#[async_trait]
pub trait GraphStore: Send + Sync {
    /// Creates the snapshot row in `Pending`. Status stays `Pending` until the caller
    /// transitions it, so a crash before indexing leaves nothing readable.
    async fn create_snapshot(&self, req: NewSnapshot) -> Result<SnapshotMeta, StoreError>;

    /// Compare-and-swap on the status column. Returns `false` when `id`'s status is not `from`
    /// or when `from -> to` does not exist (so retries are safe and never error), and
    /// [`StoreError::NotFound`] when the snapshot is not visible to the caller's scope.
    /// `error` is recorded on the row when moving to `Failed`.
    async fn transition(
        &self,
        scope: &RepoScope,
        id: SnapshotId,
        from: SnapshotStatus,
        to: SnapshotStatus,
        error: Option<&str>,
    ) -> Result<bool, StoreError>;

    /// Idempotent upsert of content-addressed file versions. Returns one ref per input, in
    /// input order; concurrent calls for the same key converge on one id.
    async fn upsert_file_versions(
        &self,
        scope: &RepoScope,
        files: &[FileVersionInput],
    ) -> Result<Vec<FileVersionRef>, StoreError>;

    /// Looks up file versions by key, returning `None` per missing key (same length as
    /// `keys`).
    async fn lookup_file_versions(
        &self,
        scope: &RepoScope,
        keys: &[FileVersionKey],
    ) -> Result<Vec<Option<FileVersionRef>>, StoreError>;

    /// Writes a full snapshot payload. Requires `Persisting`; writes nothing on failure.
    async fn write_full(
        &self,
        scope: &RepoScope,
        id: SnapshotId,
        g: &Graph,
    ) -> Result<WriteStats, StoreError>;

    /// Writes one delta of a chain. Requires `Persisting`, and the delta's base must be
    /// `Ready` in the same scope.
    async fn write_delta(
        &self,
        scope: &RepoScope,
        id: SnapshotId,
        d: &GraphDelta,
    ) -> Result<WriteStats, StoreError>;

    /// Materializes `base ⊕ … ⊕ id` (ADR-003). Requires `Ready`.
    async fn load_graph(&self, scope: &RepoScope, id: SnapshotId) -> Result<Graph, StoreError>;

    /// The delta rows of one snapshot only, without its ancestors. Requires `Ready`.
    async fn load_delta(&self, scope: &RepoScope, id: SnapshotId)
        -> Result<GraphDelta, StoreError>;

    /// Metadata of one snapshot, `None` when it does not exist in this scope.
    async fn snapshot(
        &self,
        scope: &RepoScope,
        id: SnapshotId,
    ) -> Result<Option<SnapshotMeta>, StoreError>;

    /// Newest `Ready` snapshot matching the query (commit, fingerprint, kind, purpose).
    async fn find_ready(
        &self,
        scope: &RepoScope,
        q: SnapshotQuery,
    ) -> Result<Option<SnapshotMeta>, StoreError>;

    /// `[full, delta1, …, id]`, oldest first. Errors with
    /// [`StoreError::ChainBroken`] when the chain does not reach a full snapshot.
    async fn chain(
        &self,
        scope: &RepoScope,
        id: SnapshotId,
    ) -> Result<Vec<SnapshotMeta>, StoreError>;

    /// Single-hop neighbours of `key` in `dir`, filtered by kind and confidence, ordered by
    /// `(kind, other_key)` and paginated with `cursor`. Requires `Ready`.
    ///
    /// The parameter list mirrors the specification verbatim, hence the allow.
    #[allow(clippy::too_many_arguments)]
    async fn neighbors(
        &self,
        scope: &RepoScope,
        id: SnapshotId,
        key: NodeKey,
        dir: Direction,
        kinds: EdgeKindSet,
        min_confidence: Confidence,
        limit: u32,
        cursor: Option<crate::model::EdgeCursor>,
    ) -> Result<NeighborPage, StoreError>;

    /// Looks up nodes by key, `None` per missing key (same length as `keys`). Requires
    /// `Ready`.
    async fn nodes(
        &self,
        scope: &RepoScope,
        id: SnapshotId,
        keys: &[NodeKey],
    ) -> Result<Vec<Option<StoredNode>>, StoreError>;

    /// Records the counters provenance of a snapshot (GS-007 sets `compacted_from` and
    /// `replaced_chain`). Same CAS rules as [`GraphStore::transition`]: the snapshot must be
    /// writable.
    async fn set_stats(
        &self,
        scope: &RepoScope,
        id: SnapshotId,
        stats: SnapshotStats,
    ) -> Result<(), StoreError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_are_the_documented_names() {
        assert_eq!(spans::WRITE_FULL, "graph_store.write_full");
        assert_eq!(spans::WRITE_DELTA, "graph_store.write_delta");
        assert_eq!(spans::LOAD_GRAPH, "graph_store.load_graph");
        assert_eq!(spans::NEIGHBORS, "graph_store.neighbors");
    }

    #[test]
    fn store_is_object_safe() {
        fn assert_object_safe(_: Option<&dyn GraphStore>) {}
        assert_object_safe(None);
    }
}
