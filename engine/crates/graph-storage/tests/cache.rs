//! GS-008: the in-process graph cache over the in-memory reference store.
//!
//! Runs with `cargo test -p graph-storage --features conformance` (the `integration` feature
//! implies it), because the fixtures come from the conformance suite.

#![cfg(feature = "conformance")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use graph_storage::cache::{GraphCache, GraphCacheConfig};
use graph_storage::conformance::{
    fixture_graph, sha, with_file_versions, HarnessFixture, FINGERPRINT,
};
use graph_storage::kinds::{Confidence, Direction, EdgeKindSet, NodeKey};
use graph_storage::mem::MemGraphStore;
use graph_storage::model::{EdgeCursor, Graph, GraphDelta};
use graph_storage::status::SnapshotStatus;
use graph_storage::types::{
    FileVersionInput, FileVersionKey, FileVersionRef, NeighborPage, NewSnapshot, SnapshotMeta,
    SnapshotPurpose, SnapshotQuery, SnapshotStats, StoredNode, WriteStats,
};
use graph_storage::{GraphStore, StoreError};
use repository::store::RepoScope;
use review_core::ids::{OrganizationId, RepositoryId, SnapshotId};

/// Delegates to `MemGraphStore` and counts `load_graph` calls.
#[derive(Debug, Default)]
struct CountingStore {
    inner: MemGraphStore,
    loads: AtomicUsize,
}

#[async_trait]
impl GraphStore for CountingStore {
    async fn create_snapshot(&self, req: NewSnapshot) -> Result<SnapshotMeta, StoreError> {
        self.inner.create_snapshot(req).await
    }
    async fn transition(
        &self,
        scope: &RepoScope,
        id: SnapshotId,
        from: SnapshotStatus,
        to: SnapshotStatus,
        error: Option<&str>,
    ) -> Result<bool, StoreError> {
        self.inner.transition(scope, id, from, to, error).await
    }
    async fn upsert_file_versions(
        &self,
        scope: &RepoScope,
        files: &[FileVersionInput],
    ) -> Result<Vec<FileVersionRef>, StoreError> {
        self.inner.upsert_file_versions(scope, files).await
    }
    async fn lookup_file_versions(
        &self,
        scope: &RepoScope,
        keys: &[FileVersionKey],
    ) -> Result<Vec<Option<FileVersionRef>>, StoreError> {
        self.inner.lookup_file_versions(scope, keys).await
    }
    async fn write_full(
        &self,
        scope: &RepoScope,
        id: SnapshotId,
        g: &Graph,
    ) -> Result<WriteStats, StoreError> {
        self.inner.write_full(scope, id, g).await
    }
    async fn write_delta(
        &self,
        scope: &RepoScope,
        id: SnapshotId,
        d: &GraphDelta,
    ) -> Result<WriteStats, StoreError> {
        self.inner.write_delta(scope, id, d).await
    }
    async fn load_graph(&self, scope: &RepoScope, id: SnapshotId) -> Result<Graph, StoreError> {
        self.loads.fetch_add(1, Ordering::SeqCst);
        // Give concurrent callers time to pile up on the same miss.
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        self.inner.load_graph(scope, id).await
    }
    async fn load_delta(
        &self,
        scope: &RepoScope,
        id: SnapshotId,
    ) -> Result<GraphDelta, StoreError> {
        self.inner.load_delta(scope, id).await
    }
    async fn snapshot(
        &self,
        scope: &RepoScope,
        id: SnapshotId,
    ) -> Result<Option<SnapshotMeta>, StoreError> {
        self.inner.snapshot(scope, id).await
    }
    async fn find_ready(
        &self,
        scope: &RepoScope,
        q: SnapshotQuery,
    ) -> Result<Option<SnapshotMeta>, StoreError> {
        self.inner.find_ready(scope, q).await
    }
    async fn chain(
        &self,
        scope: &RepoScope,
        id: SnapshotId,
    ) -> Result<Vec<SnapshotMeta>, StoreError> {
        self.inner.chain(scope, id).await
    }
    async fn neighbors(
        &self,
        scope: &RepoScope,
        id: SnapshotId,
        key: NodeKey,
        dir: Direction,
        kinds: EdgeKindSet,
        min_confidence: Confidence,
        limit: u32,
        cursor: Option<EdgeCursor>,
    ) -> Result<NeighborPage, StoreError> {
        self.inner
            .neighbors(scope, id, key, dir, kinds, min_confidence, limit, cursor)
            .await
    }
    async fn nodes(
        &self,
        scope: &RepoScope,
        id: SnapshotId,
        keys: &[NodeKey],
    ) -> Result<Vec<Option<StoredNode>>, StoreError> {
        self.inner.nodes(scope, id, keys).await
    }
    async fn set_stats(
        &self,
        scope: &RepoScope,
        id: SnapshotId,
        stats: SnapshotStats,
    ) -> Result<(), StoreError> {
        self.inner.set_stats(scope, id, stats).await
    }
}

fn scope() -> RepoScope {
    RepoScope {
        organization_id: OrganizationId::new(),
        repository_id: RepositoryId::new(),
    }
}

async fn advance(store: &dyn GraphStore, scope: &RepoScope, id: SnapshotId, to: SnapshotStatus) {
    let from = store.snapshot(scope, id).await.unwrap().unwrap().status;
    assert!(store.transition(scope, id, from, to, None).await.unwrap());
}

/// A ready full snapshot of the fixture graph in `scope`.
async fn ready(store: &Arc<CountingStore>, scope: &RepoScope, n: u8) -> SnapshotId {
    let fixture = HarnessFixture {
        store: store.clone(),
        scope: *scope,
        foreign: *scope,
    };
    let g = with_file_versions(&fixture, &fixture_graph())
        .await
        .unwrap();
    let mut fingerprint = FINGERPRINT;
    fingerprint[0] = n;
    let meta = store
        .create_snapshot(NewSnapshot::full(
            *scope,
            sha('a').unwrap(),
            SnapshotPurpose::DefaultBranch,
            fingerprint,
        ))
        .await
        .unwrap();
    advance(store.as_ref(), scope, meta.id, SnapshotStatus::Indexing).await;
    advance(store.as_ref(), scope, meta.id, SnapshotStatus::Persisting).await;
    store.write_full(scope, meta.id, &g).await.unwrap();
    advance(store.as_ref(), scope, meta.id, SnapshotStatus::Ready).await;
    meta.id
}

fn cache(store: &Arc<CountingStore>, max_bytes: u64) -> GraphCache {
    GraphCache::new(store.clone(), GraphCacheConfig { max_bytes })
}

#[tokio::test]
async fn concurrent_gets_load_once() {
    let store = Arc::new(CountingStore::default());
    let scope = scope();
    let id = ready(&store, &scope, 1).await;
    let cache = cache(&store, 64 * 1024 * 1024);
    let gets = (0..50).map(|_| cache.get(&scope, id));
    let results = futures::future::join_all(gets).await;
    assert!(results.iter().all(Result::is_ok));
    assert_eq!(store.loads.load(Ordering::SeqCst), 1);
    let again = cache.get(&scope, id).await.unwrap();
    assert_eq!(*again, store.inner.load_graph(&scope, id).await.unwrap());
    assert_eq!(store.loads.load(Ordering::SeqCst), 1);
    assert!(cache.stats().hits >= 1);
}

#[tokio::test]
async fn eviction_respects_byte_budget() {
    let store = Arc::new(CountingStore::default());
    let scope = scope();
    let one = graph_storage::model::flatten(&fixture_graph(), &[]).heap_size_bytes() as u64;
    let budget = one.max(1024) * 3;
    let cache = cache(&store, budget);
    for n in 1..=8u8 {
        let id = ready(&store, &scope, n).await;
        cache.get(&scope, id).await.unwrap();
    }
    cache.sync().await;
    let stats = cache.stats();
    assert!(stats.bytes <= budget + 1024, "{stats:?} over {budget}");
    assert!(stats.entries < 8, "{stats:?}");
    assert!(stats.evictions > 0, "{stats:?}");
}

#[tokio::test]
async fn oversize_graph_not_retained() {
    let store = Arc::new(CountingStore::default());
    let scope = scope();
    let id = ready(&store, &scope, 1).await;
    let mut big = Graph::new(1);
    big.files = (0..2_000)
        .map(|n| {
            graph_storage::conformance::file(
                &format!("src/f{n}.ts"),
                graph_storage::model::FileChange::Present,
            )
        })
        .collect();
    let cache = cache(&store, 1024);
    cache.insert(&scope, id, Arc::new(big)).await;
    cache.sync().await;
    assert_eq!(cache.stats().entries, 0);
    assert_eq!(cache.stats().oversize, 1);
}

#[tokio::test]
async fn errors_are_not_cached() {
    let store = Arc::new(CountingStore::default());
    let scope = scope();
    let meta = store
        .create_snapshot(NewSnapshot::full(
            scope,
            sha('a').unwrap(),
            SnapshotPurpose::DefaultBranch,
            FINGERPRINT,
        ))
        .await
        .unwrap();
    let cache = cache(&store, 64 * 1024 * 1024);
    assert!(cache.get(&scope, meta.id).await.is_err());
    advance(store.as_ref(), &scope, meta.id, SnapshotStatus::Indexing).await;
    advance(store.as_ref(), &scope, meta.id, SnapshotStatus::Persisting).await;
    store
        .write_full(&scope, meta.id, &Graph::new(1))
        .await
        .unwrap();
    advance(store.as_ref(), &scope, meta.id, SnapshotStatus::Ready).await;
    assert!(cache.get(&scope, meta.id).await.is_ok());
    assert_eq!(store.loads.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn evict_removes_entry() {
    let store = Arc::new(CountingStore::default());
    let scope = scope();
    let id = ready(&store, &scope, 1).await;
    let cache = cache(&store, 64 * 1024 * 1024);
    cache.get(&scope, id).await.unwrap();
    cache.evict(id).await;
    cache.get(&scope, id).await.unwrap();
    assert_eq!(store.loads.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn scope_mismatch_is_rejected() {
    let store = Arc::new(CountingStore::default());
    let owner = scope();
    let stranger = scope();
    let id = ready(&store, &owner, 1).await;
    let cache = cache(&store, 64 * 1024 * 1024);
    cache.get(&owner, id).await.unwrap();
    match cache.get(&stranger, id).await {
        Err(error) => assert!(matches!(*error, StoreError::NotFound(_))),
        Ok(_) => panic!("a cached graph leaked to another tenant"),
    }
}
