//! Shared state: the key ring, the replay cache, the backend, the per-snapshot graph cache and
//! the CPU budget for traversals.
//!
//! The graph cache is an LRU of `ENGINE_GRAPH_CACHE` snapshots. A miss starts one load on its own
//! task and every concurrent request for the same snapshot awaits that same shared future
//! (single flight). A request waits at most [`EngineSettings::load_wait`] (2 s); past it the
//! caller gets 503 with `Retry-After: 2` while the load carries on in the background. Failed loads
//! are never cached.

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use codegraph::{Graph, GraphQuery};
use futures::future::{BoxFuture, Shared};
use futures::FutureExt;
use opentelemetry::metrics::{Gauge, Histogram};
use opentelemetry::KeyValue;
use repository::store::RepoScope;
use review_core::ids::SnapshotId;
use tokio::sync::Semaphore;
use tracing::Instrument;
use uuid::Uuid;

use crate::auth::{JtiCache, ServiceKeyRing};
use crate::backend::{BackendError, EngineBackend};
use crate::error::ApiError;

pub const DEFAULT_GRAPH_CACHE: usize = 8;
pub const DEFAULT_LOAD_WAIT: Duration = Duration::from_secs(2);
const JTI_CAPACITY: usize = 100_000;

/// Tunables; `Default` is the production setting.
#[derive(Debug, Clone, Copy)]
pub struct EngineSettings {
    /// Snapshots kept in memory (`ENGINE_GRAPH_CACHE`).
    pub graph_cache: usize,
    /// How long a request waits for a loading snapshot before answering 503.
    pub load_wait: Duration,
    /// Concurrent traversals; the CPU count by default.
    pub query_parallelism: usize,
}

impl Default for EngineSettings {
    fn default() -> Self {
        Self {
            graph_cache: DEFAULT_GRAPH_CACHE,
            load_wait: DEFAULT_LOAD_WAIT,
            query_parallelism: std::thread::available_parallelism().map_or(4, usize::from),
        }
    }
}

type CacheKey = (Uuid, Uuid, Uuid);
type LoadResult = Result<Arc<Graph>, Arc<BackendError>>;
type SharedLoad = Shared<BoxFuture<'static, LoadResult>>;

struct CacheEntry {
    id: u64,
    load: SharedLoad,
}

#[derive(Default)]
struct GraphCache {
    next_id: u64,
    entries: HashMap<CacheKey, CacheEntry>,
    /// Least recently used first.
    order: VecDeque<CacheKey>,
}

impl GraphCache {
    fn touch(&mut self, key: &CacheKey) {
        self.order.retain(|k| k != key);
        self.order.push_back(*key);
    }

    fn remove_if(&mut self, key: &CacheKey, id: u64) {
        if self.entries.get(key).is_some_and(|entry| entry.id == id) {
            self.entries.remove(key);
            self.order.retain(|k| k != key);
        }
    }
}

struct Metrics {
    duration: Histogram<f64>,
    cache_entries: Gauge<u64>,
    nodes: Gauge<u64>,
    edges: Gauge<u64>,
}

impl Metrics {
    fn new() -> Self {
        let meter = opentelemetry::global::meter("review-engine");
        Self {
            duration: meter
                .f64_histogram("graph_query_duration_seconds")
                .with_description("Wall time of an engine graph query")
                .with_unit("s")
                .build(),
            cache_entries: meter
                .u64_gauge("graph_cache_entries")
                .with_description("Snapshots held by the graph cache")
                .build(),
            nodes: meter
                .u64_gauge("graph_nodes_total")
                .with_description("Nodes of a loaded snapshot")
                .build(),
            edges: meter
                .u64_gauge("graph_edges_total")
                .with_description("Edges of a loaded snapshot")
                .build(),
        }
    }
}

struct Inner {
    keys: ServiceKeyRing,
    jti: JtiCache,
    backend: Arc<dyn EngineBackend>,
    settings: EngineSettings,
    cache: Mutex<GraphCache>,
    permits: Arc<Semaphore>,
    metrics: Metrics,
}

#[derive(Clone)]
pub struct AppState {
    inner: Arc<Inner>,
}

impl fmt::Debug for AppState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AppState")
            .field("settings", &self.inner.settings)
            .finish_non_exhaustive()
    }
}

impl AppState {
    pub fn new(
        keys: ServiceKeyRing,
        backend: Arc<dyn EngineBackend>,
        settings: EngineSettings,
    ) -> Self {
        let parallelism = settings.query_parallelism.max(1);
        Self {
            inner: Arc::new(Inner {
                keys,
                jti: JtiCache::new(JTI_CAPACITY),
                backend,
                settings,
                cache: Mutex::new(GraphCache::default()),
                permits: Arc::new(Semaphore::new(parallelism)),
                metrics: Metrics::new(),
            }),
        }
    }

    pub fn keys(&self) -> &ServiceKeyRing {
        &self.inner.keys
    }

    pub fn jti(&self) -> &JtiCache {
        &self.inner.jti
    }

    pub fn backend(&self) -> &Arc<dyn EngineBackend> {
        &self.inner.backend
    }

    /// The graph of a snapshot, loading it at most once however many requests miss at the same
    /// time. A snapshot outside `scope` is `NotFound` (the backend enforces the scope).
    ///
    /// # Errors
    ///
    /// [`ApiError::Loading`] when the load is still running after the wait budget,
    /// [`ApiError::NotFound`] for an unknown snapshot, [`ApiError::Unavailable`] otherwise.
    pub async fn graph(
        &self,
        scope: RepoScope,
        snapshot: SnapshotId,
    ) -> Result<Arc<Graph>, ApiError> {
        let key: CacheKey = (
            *scope.organization_id.as_uuid(),
            *scope.repository_id.as_uuid(),
            *snapshot.as_uuid(),
        );
        let (id, load) = {
            let mut cache = self
                .inner
                .cache
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if let Some(entry) = cache.entries.get(&key) {
                let found = (entry.id, entry.load.clone());
                cache.touch(&key);
                found
            } else {
                let id = cache.next_id;
                cache.next_id += 1;
                let load = self.start_load(scope, snapshot);
                cache.entries.insert(
                    key,
                    CacheEntry {
                        id,
                        load: load.clone(),
                    },
                );
                cache.touch(&key);
                while cache.order.len() > self.inner.settings.graph_cache.max(1) {
                    if let Some(oldest) = cache.order.pop_front() {
                        cache.entries.remove(&oldest);
                    }
                }
                self.inner
                    .metrics
                    .cache_entries
                    .record(cache.entries.len() as u64, &[]);
                (id, load)
            }
        };
        match tokio::time::timeout(self.inner.settings.load_wait, load).await {
            Err(_) => Err(ApiError::Loading),
            Ok(Ok(graph)) => Ok(graph),
            Ok(Err(error)) => {
                self.inner
                    .cache
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .remove_if(&key, id);
                Err(match error.as_ref() {
                    BackendError::NotFound => ApiError::NotFound,
                    BackendError::Unavailable(detail) => ApiError::Unavailable(detail.clone()),
                })
            }
        }
    }

    fn start_load(&self, scope: RepoScope, snapshot: SnapshotId) -> SharedLoad {
        let backend = Arc::clone(&self.inner.backend);
        let state = self.clone();
        let span = tracing::info_span!("graph_load", snapshot_id = %snapshot);
        // The load runs on its own task, so a request that gives up waiting does not cancel it.
        let handle = tokio::spawn(
            async move {
                let graph = backend
                    .load_graph(scope, snapshot)
                    .await
                    .map_err(Arc::new)?;
                let label = [KeyValue::new("snapshot_id", snapshot.to_string())];
                state
                    .inner
                    .metrics
                    .nodes
                    .record(graph.node_count() as u64, &label);
                state
                    .inner
                    .metrics
                    .edges
                    .record(graph.edge_count() as u64, &label);
                Ok::<_, Arc<BackendError>>(graph)
            }
            .instrument(span),
        );
        async move {
            match handle.await {
                Ok(result) => result,
                Err(join) => Err(Arc::new(BackendError::Unavailable(join.to_string()))),
            }
        }
        .boxed()
        .shared()
    }

    /// Runs a CPU-bound traversal on the blocking pool, at most `query_parallelism` at a time.
    ///
    /// # Errors
    ///
    /// Whatever `work` returns, or [`ApiError::Unavailable`] when the worker task fails.
    pub async fn blocking<T, F>(&self, route: &'static str, work: F) -> Result<T, ApiError>
    where
        T: Send + 'static,
        F: FnOnce() -> Result<T, ApiError> + Send + 'static,
    {
        let permit = Arc::clone(&self.inner.permits)
            .acquire_owned()
            .await
            .map_err(|_| ApiError::Unavailable("engine is shutting down".to_owned()))?;
        let span = tracing::info_span!("engine_query", route);
        let started = Instant::now();
        let outcome = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let _entered = span.enter();
            work()
        })
        .await
        .map_err(|e| ApiError::Unavailable(format!("query task failed: {e}")))?;
        self.inner.metrics.duration.record(
            started.elapsed().as_secs_f64(),
            &[KeyValue::new("route", route)],
        );
        outcome
    }
}
