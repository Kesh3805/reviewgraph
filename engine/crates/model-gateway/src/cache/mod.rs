//! Tenant-scoped model response cache (GW-008).
//!
//! Only cache-allowed tasks (`TaskType::response_cacheable`) with `CachePolicy::PromptAndResponse`
//! use it, and only validated, complete outputs are stored. The key includes the organisation id
//! and every read repeats the organisation predicate, so entries can never cross tenants. A
//! different model id gives a different key, which is the "plus model version" invalidation.

#[cfg(feature = "pg")]
pub mod pg;

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use review_core::ids::OrganizationId;

use crate::types::{ModelOutput, Usage};

/// Default time to live (7 days).
pub const DEFAULT_TTL: Duration = Duration::from_secs(7 * 24 * 3600);

/// Cache read/write failure. Reads are treated as misses and writes are ignored by the gateway.
#[derive(Debug, thiserror::Error)]
#[error("model cache error: {0}")]
pub struct CacheError(pub String);

/// `blake3(organization_id || request_hash || provider || model || schema_hash)`.
pub fn cache_key(
    organization_id: OrganizationId,
    request_hash: &str,
    provider: &str,
    model: &str,
    schema_hash: Option<&str>,
) -> String {
    let mut h = blake3::Hasher::new();
    for part in [
        organization_id.to_string().as_str(),
        request_hash,
        provider,
        model,
        schema_hash.unwrap_or(""),
    ] {
        // Length-prefix every part so concatenation cannot be ambiguous.
        h.update(&(part.len() as u64).to_le_bytes());
        h.update(part.as_bytes());
    }
    h.finalize().to_hex().to_string()
}

/// What is stored per entry.
#[derive(Debug, Clone, PartialEq)]
pub struct CacheEntry {
    pub request_hash: String,
    pub provider: String,
    pub model: String,
    pub prompt_version: String,
    pub schema_hash: Option<String>,
    pub output: ModelOutput,
    /// Usage of the call that produced the entry (reported as `usage_original` on hits).
    pub usage: Usage,
}

#[async_trait]
pub trait ResponseCache: Send + Sync {
    /// An unexpired entry for exactly this organisation and key.
    async fn get(
        &self,
        organization_id: OrganizationId,
        key: &str,
    ) -> Result<Option<CacheEntry>, CacheError>;

    /// Stores an entry. A conflicting key is a no-op (first writer wins).
    async fn put(
        &self,
        organization_id: OrganizationId,
        key: &str,
        entry: CacheEntry,
        ttl: Duration,
    ) -> Result<(), CacheError>;

    /// Deletes expired entries; returns how many.
    async fn purge_expired(&self) -> Result<u64, CacheError>;
}

/// In-memory cache (tests and single-process runs).
#[derive(Debug, Default)]
pub struct MemoryCache {
    entries: Mutex<HashMap<(OrganizationId, String), (CacheEntry, SystemTime)>>,
}

impl MemoryCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.entries.lock().map(|e| e.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

fn poisoned<T>(_: T) -> CacheError {
    CacheError("poisoned lock".into())
}

#[async_trait]
impl ResponseCache for MemoryCache {
    async fn get(
        &self,
        organization_id: OrganizationId,
        key: &str,
    ) -> Result<Option<CacheEntry>, CacheError> {
        let map = self.entries.lock().map_err(poisoned)?;
        Ok(map
            .get(&(organization_id, key.to_owned()))
            .filter(|(_, expires)| *expires > SystemTime::now())
            .map(|(e, _)| e.clone()))
    }

    async fn put(
        &self,
        organization_id: OrganizationId,
        key: &str,
        entry: CacheEntry,
        ttl: Duration,
    ) -> Result<(), CacheError> {
        let mut map = self.entries.lock().map_err(poisoned)?;
        let k = (organization_id, key.to_owned());
        let live = map
            .get(&k)
            .is_some_and(|(_, expires)| *expires > SystemTime::now());
        if !live {
            map.insert(k, (entry, SystemTime::now() + ttl));
        }
        Ok(())
    }

    async fn purge_expired(&self) -> Result<u64, CacheError> {
        let mut map = self.entries.lock().map_err(poisoned)?;
        let before = map.len();
        map.retain(|_, (_, expires)| *expires > SystemTime::now());
        Ok((before - map.len()) as u64)
    }
}
