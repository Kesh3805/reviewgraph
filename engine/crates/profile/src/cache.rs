//! Profile reuse and the in-process cache (PROF-006, target-architecture §7).
//!
//! The cache key is `(snapshot_id, profile_version, config_hash)` (plus the repository). A pull
//! request head (a delta snapshot) never recomputes conventions: it resolves to the nearest full
//! ancestor on the default branch and reuses that profile. A profile is recomputed only for a
//! full snapshot whose key is not stored yet — a new default-branch snapshot, a new
//! `config_hash` or a new `profile_version`. Profiles never go to Redis; the cache is a small
//! in-process LRU of `Arc<RepositoryProfile>`.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use repository::store::RepoScope;
use review_core::ids::SnapshotId;
use review_core::version::{ConfigHash, ProfileVersion};

use crate::model::{ProfileKey, RepositoryProfile};
use crate::store::{ProfileStore, ProfileStoreError};

/// Entries the LRU holds.
pub const CACHE_CAPACITY: usize = 32;
/// Longest delta chain followed to a full snapshot.
const MAX_CHAIN: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotKind {
    Full,
    Delta,
}

/// What profile resolution needs to know about a snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotRef {
    pub id: SnapshotId,
    pub kind: SnapshotKind,
    /// The snapshot a delta is based on (`snapshots.base_snapshot_id`).
    pub base_snapshot_id: Option<SnapshotId>,
}

/// The full snapshot whose profile serves `snapshot`: itself when full, else the nearest full
/// ancestor through the `base_snapshot_id` chain. `None` when the chain is broken.
pub fn resolve_profile_snapshot(
    snapshot: &SnapshotRef,
    lookup: &dyn Fn(SnapshotId) -> Option<SnapshotRef>,
) -> Option<SnapshotId> {
    let mut current = *snapshot;
    for _ in 0..MAX_CHAIN {
        match current.kind {
            SnapshotKind::Full => return Some(current.id),
            SnapshotKind::Delta => current = lookup(current.base_snapshot_id?)?,
        }
    }
    None
}

/// A small LRU of profiles keyed by [`ProfileKey`].
#[derive(Debug)]
pub struct ProfileCache {
    capacity: usize,
    entries: Mutex<VecDeque<(ProfileKey, Arc<RepositoryProfile>)>>,
}

impl Default for ProfileCache {
    fn default() -> Self {
        Self::new(CACHE_CAPACITY)
    }
}

impl ProfileCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            entries: Mutex::new(VecDeque::new()),
        }
    }

    pub fn get(&self, key: &ProfileKey) -> Option<Arc<RepositoryProfile>> {
        let mut entries = self.entries.lock().ok()?;
        let at = entries.iter().position(|(k, _)| k == key)?;
        let entry = entries.remove(at)?;
        let profile = entry.1.clone();
        entries.push_front(entry);
        Some(profile)
    }

    pub fn insert(&self, profile: Arc<RepositoryProfile>) {
        let Ok(mut entries) = self.entries.lock() else {
            return;
        };
        let key = profile.key();
        entries.retain(|(k, _)| *k != key);
        entries.push_front((key, profile));
        entries.truncate(self.capacity);
    }

    pub fn len(&self) -> usize {
        self.entries.lock().map(|e| e.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The profile a review uses, with how it was obtained.
#[derive(Debug, Clone)]
pub struct ProfileRef {
    pub profile: Arc<RepositoryProfile>,
    /// Served from the cache or the store, without computing.
    pub reused: bool,
    /// The requested key failed to compute; this is the previous profile of the snapshot.
    pub stale: bool,
}

/// Resolves, caches and (only when needed) computes profiles.
#[derive(Debug)]
pub struct ProfileService<S> {
    store: S,
    cache: ProfileCache,
    profile_version: ProfileVersion,
}

impl<S: ProfileStore> ProfileService<S> {
    pub fn new(store: S, profile_version: ProfileVersion) -> Self {
        Self {
            store,
            cache: ProfileCache::default(),
            profile_version,
        }
    }

    pub fn store(&self) -> &S {
        &self.store
    }

    pub fn cache(&self) -> &ProfileCache {
        &self.cache
    }

    /// The profile for `snapshot` under `config_hash`.
    ///
    /// `compute` is called with the full snapshot to profile only when no stored profile matches
    /// the key; a delta snapshot always resolves to its full ancestor first, so a pull request
    /// head never computes for itself. A compute failure falls back to the snapshot's previous
    /// profile, marked `stale`.
    pub async fn profile_for<F>(
        &self,
        scope: &RepoScope,
        snapshot: &SnapshotRef,
        lookup: &(dyn Fn(SnapshotId) -> Option<SnapshotRef> + Sync),
        config_hash: ConfigHash,
        compute: F,
    ) -> Result<ProfileRef, ProfileStoreError>
    where
        F: FnOnce(SnapshotId) -> Result<RepositoryProfile, String>,
    {
        let span = tracing::info_span!(
            "profile_resolve",
            snapshot_id = %snapshot.id,
            profile_reused = tracing::field::Empty,
        );
        let target = resolve_profile_snapshot(snapshot, lookup).ok_or_else(|| {
            ProfileStoreError::Backend(format!(
                "snapshot {} has no full ancestor to take a profile from",
                snapshot.id
            ))
        })?;
        let key = ProfileKey {
            repository_id: scope.repository_id,
            snapshot_id: target,
            profile_version: self.profile_version,
            config_hash,
        };
        if let Some(profile) = self.cache.get(&key) {
            crate::metrics::profile_cache(true);
            span.record("profile_reused", true);
            return Ok(ProfileRef {
                profile,
                reused: true,
                stale: false,
            });
        }
        crate::metrics::profile_cache(false);
        if let Some(stored) = self.store.get(scope, &key).await? {
            let profile = Arc::new(stored);
            self.cache.insert(profile.clone());
            span.record("profile_reused", true);
            return Ok(ProfileRef {
                profile,
                reused: true,
                stale: false,
            });
        }
        span.record("profile_reused", false);
        match compute(target) {
            Ok(profile) => {
                self.store.save(scope, &profile).await?;
                let profile = Arc::new(profile);
                self.cache.insert(profile.clone());
                Ok(ProfileRef {
                    profile,
                    reused: false,
                    stale: false,
                })
            }
            Err(reason) => {
                tracing::warn!(snapshot_id = %target, %reason, "profile computation failed");
                match self.store.latest_for_snapshot(scope, target).await? {
                    Some(previous) => Ok(ProfileRef {
                        profile: Arc::new(previous),
                        reused: true,
                        stale: true,
                    }),
                    None => Err(ProfileStoreError::Backend(reason)),
                }
            }
        }
    }
}
