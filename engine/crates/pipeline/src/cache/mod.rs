//! Stage caches (CTX-009): the async cache port for context packages and its helpers.
//!
//! A cache failure never fails a stage: lookups that error count as misses and writes are best
//! effort, so the stage computes its output and carries on.

#[cfg(feature = "pg")]
pub mod context_pg;

use std::future::Future;

use async_trait::async_trait;
use context_engine::cache::CtxKey;
use context_engine::model::ContextPackage;

/// Why a cache operation failed. Callers treat every variant as "no cache".
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CacheError {
    #[error("cache backend: {0}")]
    Backend(String),
    #[error("stored package is invalid: {0}")]
    Corrupt(String),
}

/// Where a cache hit came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitScope {
    /// The same review run.
    Run,
    /// A previous run of the same repository with identical inputs.
    Repo,
}

/// Cache of context packages keyed by `(review_run, cluster, reviewer, input_hash)`.
#[async_trait]
pub trait AsyncContextCache: Send + Sync {
    async fn lookup(&self, key: &CtxKey) -> Result<Option<(ContextPackage, HitScope)>, CacheError>;

    /// First write wins: storing a key that already exists is a no-op.
    async fn put(&self, key: &CtxKey, package: &ContextPackage) -> Result<(), CacheError>;
}

/// Object storage for packages too large to keep in the row (`> 1 MiB`).
#[async_trait]
pub trait BlobStore: Send + Sync {
    async fn put(&self, key: &str, bytes: Vec<u8>) -> Result<(), CacheError>;
    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, CacheError>;
}

/// Returns the cached package for `key`, or computes it and stores it. Cache errors are
/// swallowed: the result is then simply computed.
pub async fn get_or_compute<C, F, Fut>(
    cache: &C,
    key: &CtxKey,
    compute: F,
) -> (ContextPackage, Option<HitScope>)
where
    C: AsyncContextCache + ?Sized,
    F: FnOnce() -> Fut,
    Fut: Future<Output = ContextPackage>,
{
    if let Ok(Some((package, scope))) = cache.lookup(key).await {
        return (package, Some(scope));
    }
    let package = compute().await;
    // Best effort: a failed write only means the next attempt computes again.
    let _ = cache.put(key, &package).await;
    (package, None)
}
