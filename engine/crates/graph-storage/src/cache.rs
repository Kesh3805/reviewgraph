//! In-process graph cache (GS-008): a byte-bounded, single-flight LRU of materialized graphs.
//!
//! Snapshots are immutable, so entries never expire; they leave when the byte budget needs the
//! room or when [`GraphCache::evict`] is called for a snapshot marked `inconsistent`. Concurrent
//! misses for one snapshot share a single `load_graph` call, errors are never cached, and a cached
//! graph is only ever returned to the tenant whose scope loaded it.
//!
//! The effective peak can exceed `max_bytes` by the graphs callers still hold: an evicted
//! `Arc<Graph>` stays alive until its last user drops it.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use moka::future::Cache;
use moka::notification::RemovalCause;
use repository::store::RepoScope;
use review_core::ids::SnapshotId;
use tracing::Instrument;

use crate::model::Graph;
use crate::port::GraphStore;
use crate::StoreError;

/// Lower bound of the default budget.
pub const MIN_CACHE_BYTES: u64 = 512 * 1024 * 1024;

/// Share of the container memory limit the default budget takes.
pub const MEMORY_SHARE: f64 = 0.40;

/// Where the cgroup v2 memory limit is published.
const CGROUP_MEMORY_MAX: &str = "/sys/fs/cgroup/memory.max";

/// A cached graph and the scope that was authorized to load it.
#[derive(Debug, Clone)]
pub struct CachedGraph {
    pub scope: RepoScope,
    pub graph: Arc<Graph>,
}

/// Cache sizing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GraphCacheConfig {
    pub max_bytes: u64,
}

impl GraphCacheConfig {
    /// 40% of the cgroup memory limit when one is set, otherwise `fallback_bytes`; never below
    /// [`MIN_CACHE_BYTES`].
    pub fn from_environment(fallback_bytes: u64) -> Self {
        let limit = std::fs::read_to_string(CGROUP_MEMORY_MAX)
            .ok()
            .and_then(|text| text.trim().parse::<u64>().ok());
        Self::from_limit(limit, fallback_bytes)
    }

    /// The budget for a known memory limit (`None` when unlimited or unknown).
    pub fn from_limit(limit: Option<u64>, fallback_bytes: u64) -> Self {
        let budget = match limit {
            Some(bytes) => (bytes as f64 * MEMORY_SHARE) as u64,
            None => fallback_bytes,
        };
        Self {
            max_bytes: budget.max(MIN_CACHE_BYTES),
        }
    }
}

/// Counters of one cache, also the source of the `graph_cache_*` metrics.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub oversize: u64,
    /// Entries currently retained (approximate until pending maintenance runs).
    pub entries: u64,
    /// Retained weight in bytes (KiB granularity).
    pub bytes: u64,
}

#[derive(Debug, Default)]
struct Counters {
    hits: AtomicU64,
    misses: AtomicU64,
    evictions: AtomicU64,
    oversize: AtomicU64,
}

/// Weight of a cached graph in KiB, at least 1 so tiny graphs still count.
fn weight_kib(graph: &Graph) -> u32 {
    let kib = graph.heap_size_bytes() / 1024;
    u32::try_from(kib).unwrap_or(u32::MAX).max(1)
}

/// The byte-bounded graph cache.
pub struct GraphCache {
    inner: Cache<SnapshotId, CachedGraph>,
    store: Arc<dyn GraphStore>,
    counters: Arc<Counters>,
    max_kib: u64,
}

impl std::fmt::Debug for GraphCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GraphCache")
            .field("max_kib", &self.max_kib)
            .field("stats", &self.stats())
            .finish_non_exhaustive()
    }
}

impl GraphCache {
    pub fn new(store: Arc<dyn GraphStore>, cfg: GraphCacheConfig) -> Self {
        let max_kib = (cfg.max_bytes / 1024).max(1);
        let counters = Arc::new(Counters::default());
        let listener = Arc::clone(&counters);
        let inner = Cache::builder()
            .max_capacity(max_kib)
            .weigher(|_id: &SnapshotId, cached: &CachedGraph| weight_kib(&cached.graph))
            .eviction_listener(move |_id, _cached, cause| {
                if matches!(cause, RemovalCause::Size) {
                    listener.evictions.fetch_add(1, Ordering::Relaxed);
                }
            })
            .build();
        Self {
            inner,
            store,
            counters,
            max_kib,
        }
    }

    /// The materialized graph of `id`, loading it at most once however many callers miss at the
    /// same time. A graph cached for another scope is `NotFound`, never returned.
    pub async fn get(
        &self,
        scope: &RepoScope,
        id: SnapshotId,
    ) -> Result<Arc<Graph>, Arc<StoreError>> {
        if let Some(cached) = self.inner.get(&id).await {
            if cached.scope != *scope {
                return Err(Arc::new(StoreError::NotFound(id)));
            }
            self.counters.hits.fetch_add(1, Ordering::Relaxed);
            return Ok(cached.graph);
        }
        self.counters.misses.fetch_add(1, Ordering::Relaxed);
        let span = tracing::info_span!("graph_cache.load", snapshot_id = %id);
        let store = Arc::clone(&self.store);
        let owner = *scope;
        let load = async move {
            let graph = store.load_graph(&owner, id).await?;
            Ok::<CachedGraph, StoreError>(CachedGraph {
                scope: owner,
                graph: Arc::new(graph),
            })
        };
        let loaded = self.inner.try_get_with(id, load.instrument(span)).await?;
        if loaded.scope != *scope {
            return Err(Arc::new(StoreError::NotFound(id)));
        }
        self.note_oversize(id, &loaded).await;
        Ok(loaded.graph)
    }

    /// Caches a graph that was just written (a fresh full index), replacing any entry.
    pub async fn insert(&self, scope: &RepoScope, id: SnapshotId, graph: Arc<Graph>) {
        let cached = CachedGraph {
            scope: *scope,
            graph,
        };
        self.inner.insert(id, cached.clone()).await;
        self.note_oversize(id, &cached).await;
    }

    /// Drops the entry of `id` (a snapshot marked `inconsistent`).
    pub async fn evict(&self, id: SnapshotId) {
        self.inner.invalidate(&id).await;
    }

    /// Runs pending maintenance (evictions) now; the cache otherwise does it lazily.
    pub async fn sync(&self) {
        self.inner.run_pending_tasks().await;
    }

    pub fn stats(&self) -> CacheStats {
        CacheStats {
            hits: self.counters.hits.load(Ordering::Relaxed),
            misses: self.counters.misses.load(Ordering::Relaxed),
            evictions: self.counters.evictions.load(Ordering::Relaxed),
            oversize: self.counters.oversize.load(Ordering::Relaxed),
            entries: self.inner.entry_count(),
            bytes: self.inner.weighted_size().saturating_mul(1024),
        }
    }

    /// A graph larger than the whole budget is handed to the caller but not retained.
    async fn note_oversize(&self, id: SnapshotId, cached: &CachedGraph) {
        if u64::from(weight_kib(&cached.graph)) > self.max_kib {
            self.counters.oversize.fetch_add(1, Ordering::Relaxed);
            self.inner.invalidate(&id).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_is_a_share_of_the_limit_with_a_floor() {
        let gib = 1024 * 1024 * 1024;
        assert_eq!(
            GraphCacheConfig::from_limit(Some(10 * gib), 0).max_bytes,
            (10.0 * gib as f64 * MEMORY_SHARE) as u64
        );
        assert_eq!(
            GraphCacheConfig::from_limit(Some(gib), 0).max_bytes,
            MIN_CACHE_BYTES
        );
        assert_eq!(
            GraphCacheConfig::from_limit(None, 2 * gib).max_bytes,
            2 * gib
        );
    }

    #[test]
    fn tiny_graphs_weigh_at_least_one_kib() {
        assert_eq!(weight_kib(&Graph::new(1)), 1);
    }
}
