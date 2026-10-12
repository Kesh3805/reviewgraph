//! Parse cache keyed by `(path, content_hash, analyzer_version)` (IDX-005).
//!
//! The port is deliberately small: the parse step looks keys up in batches, parses the misses
//! and stores the results. Durable adapters (object store, `.review/cache/ir`) implement
//! [`ParseCache`]; [`InMemoryParseCache`] is the reference used by tests and `index-local`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use analysis_ir::{ParsedUnit, IR_SCHEMA_VERSION};
use async_trait::async_trait;

/// Cache identity of one parsed file.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ParseCacheKey {
    pub path: String,
    pub content_hash: [u8; 32],
    pub analyzer_version: String,
}

/// Why a lookup produced no unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissReason {
    NoRow,
    BlobMissing,
    DecodeError,
    IrFormatChanged,
}

/// Result of one key in a batch lookup.
#[derive(Debug, Clone)]
pub enum CacheLookup {
    Hit(Arc<ParsedUnit>),
    Miss(MissReason),
}

impl CacheLookup {
    pub fn is_hit(&self) -> bool {
        matches!(self, Self::Hit(_))
    }
}

/// Failure of the cache backend. Callers degrade to "parse everything".
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("parse cache unavailable: {0}")]
pub struct ParseCacheError(pub String);

#[async_trait]
pub trait ParseCache: Send + Sync {
    /// One result per key, in key order.
    async fn lookup_many(
        &self,
        keys: &[ParseCacheKey],
    ) -> Result<Vec<CacheLookup>, ParseCacheError>;

    /// Best-effort store; a failure never fails the index.
    async fn store(&self, key: ParseCacheKey, unit: Arc<ParsedUnit>)
        -> Result<(), ParseCacheError>;
}

/// Process-local cache with hit/miss counters.
#[derive(Debug, Default)]
pub struct InMemoryParseCache {
    entries: Mutex<HashMap<ParseCacheKey, Arc<ParsedUnit>>>,
    hits: AtomicU64,
    misses: AtomicU64,
}

impl InMemoryParseCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn hits(&self) -> u64 {
        self.hits.load(Ordering::Relaxed)
    }

    pub fn misses(&self) -> u64 {
        self.misses.load(Ordering::Relaxed)
    }

    pub fn len(&self) -> usize {
        self.entries.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[async_trait]
impl ParseCache for InMemoryParseCache {
    async fn lookup_many(
        &self,
        keys: &[ParseCacheKey],
    ) -> Result<Vec<CacheLookup>, ParseCacheError> {
        let map = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let out = keys
            .iter()
            .map(|k| match map.get(k) {
                Some(unit) if unit.ir_schema != IR_SCHEMA_VERSION => {
                    self.misses.fetch_add(1, Ordering::Relaxed);
                    CacheLookup::Miss(MissReason::IrFormatChanged)
                }
                Some(unit) => {
                    self.hits.fetch_add(1, Ordering::Relaxed);
                    CacheLookup::Hit(Arc::clone(unit))
                }
                None => {
                    self.misses.fetch_add(1, Ordering::Relaxed);
                    CacheLookup::Miss(MissReason::NoRow)
                }
            })
            .collect();
        Ok(out)
    }

    async fn store(
        &self,
        key: ParseCacheKey,
        unit: Arc<ParsedUnit>,
    ) -> Result<(), ParseCacheError> {
        self.entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key, unit);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(path: &str, hash: u8, version: &str) -> ParseCacheKey {
        ParseCacheKey {
            path: path.to_owned(),
            content_hash: [hash; 32],
            analyzer_version: version.to_owned(),
        }
    }

    #[tokio::test]
    async fn empty_cache_misses_every_key() {
        let cache = InMemoryParseCache::new();
        let out = cache
            .lookup_many(&[key("a.ts", 1, "1.0.0"), key("b.ts", 2, "1.0.0")])
            .await;
        let out = out.unwrap_or_default();
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|l| !l.is_hit()));
        assert_eq!(cache.misses(), 2);
        assert_eq!(cache.hits(), 0);
        assert!(cache.is_empty());
    }

    #[test]
    fn key_differs_by_each_component() {
        let base = key("a.ts", 1, "1.0.0");
        assert_ne!(base, key("b.ts", 1, "1.0.0"));
        assert_ne!(base, key("a.ts", 2, "1.0.0"));
        assert_ne!(base, key("a.ts", 1, "1.0.1"));
        assert_eq!(base, key("a.ts", 1, "1.0.0"));
    }
}
