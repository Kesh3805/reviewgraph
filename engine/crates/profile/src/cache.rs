//! Profile reuse and the in-process cache (PROF-006).
//!
//! A pull-request head (delta snapshot) reuses the profile of its nearest full ancestor; a PR
//! never recomputes conventions. A profile is recomputed only for a new full snapshot, or when
//! the `config_hash` or `profile_version` changes (both are part of [`ProfileKey`]).

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use review_core::ids::SnapshotId;

use crate::metrics;
use crate::model::{ProfileKey, RepositoryProfile};

/// Capacity of the in-process LRU.
pub const CACHE_CAPACITY: usize = 32;

/// The parts of a snapshot the resolver needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotLink {
    pub id: SnapshotId,
    /// `None` for a full snapshot, the base snapshot for a delta.
    pub base: Option<SnapshotId>,
}

/// Resolves a snapshot to the snapshot whose profile applies: itself when full, otherwise the
/// nearest full ancestor along the `base_snapshot_id` chain. `None` when the chain is broken or
/// cyclic.
pub fn profile_snapshot(
    start: SnapshotId,
    lookup: &dyn Fn(SnapshotId) -> Option<SnapshotLink>,
) -> Option<SnapshotId> {
    let mut current = start;
    // A chain is bounded by compaction; the cap also turns a cycle into `None`.
    for _ in 0..1024 {
        let link = lookup(current)?;
        match link.base {
            None => return Some(link.id),
            Some(base) => current = base,
        }
    }
    None
}

/// Whether the profile for `snapshot` may be reused instead of recomputed.
pub fn is_reused(snapshot: &SnapshotLink) -> bool {
    snapshot.base.is_some()
}

/// A small LRU of shared profiles keyed by [`ProfileKey`].
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
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let found = match entries.iter().position(|(k, _)| k == key) {
            Some(i) => entries.remove(i).map(|entry| {
                let value = Arc::clone(&entry.1);
                entries.push_front(entry);
                value
            }),
            None => None,
        };
        metrics::profile_cache(found.is_some());
        found
    }

    pub fn insert(&self, profile: Arc<RepositoryProfile>) {
        let key = profile.key();
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.retain(|(k, _)| *k != key);
        entries.push_front((key, profile));
        entries.truncate(self.capacity);
    }

    pub fn len(&self) -> usize {
        self.entries.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use chrono::{TimeZone, Utc};
    use review_core::ids::RepositoryId;
    use review_core::version::ConfigHash;

    use super::*;

    fn profile(snapshot: SnapshotId, cfg: &[u8]) -> Arc<RepositoryProfile> {
        Arc::new(RepositoryProfile::empty(
            RepositoryId::new(),
            snapshot,
            ConfigHash::of(cfg),
            Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
        ))
    }

    #[test]
    fn pr_head_reuses_base_profile() {
        let (full, d1, d2) = (SnapshotId::new(), SnapshotId::new(), SnapshotId::new());
        let mut m = HashMap::new();
        m.insert(
            full,
            SnapshotLink {
                id: full,
                base: None,
            },
        );
        m.insert(
            d1,
            SnapshotLink {
                id: d1,
                base: Some(full),
            },
        );
        m.insert(
            d2,
            SnapshotLink {
                id: d2,
                base: Some(d1),
            },
        );
        let lookup = |id: SnapshotId| m.get(&id).copied();
        assert_eq!(profile_snapshot(d2, &lookup), Some(full));
        assert_eq!(profile_snapshot(full, &lookup), Some(full));
        assert!(is_reused(&m[&d2]));
        assert!(!is_reused(&m[&full]));
    }

    #[test]
    fn broken_or_cyclic_chain_is_none() {
        let (a, b) = (SnapshotId::new(), SnapshotId::new());
        let mut m = HashMap::new();
        m.insert(
            a,
            SnapshotLink {
                id: a,
                base: Some(b),
            },
        );
        m.insert(
            b,
            SnapshotLink {
                id: b,
                base: Some(a),
            },
        );
        assert_eq!(profile_snapshot(a, &|id| m.get(&id).copied()), None);
        assert_eq!(profile_snapshot(SnapshotId::new(), &|_| None), None);
    }

    #[test]
    fn config_hash_change_recomputes() {
        let snap = SnapshotId::new();
        let cache = ProfileCache::default();
        let p = profile(snap, b"one");
        cache.insert(Arc::clone(&p));
        assert!(cache.get(&p.key()).is_some());
        let mut changed_key = p.key();
        changed_key.config_hash = ConfigHash::of(b"two");
        assert!(cache.get(&changed_key).is_none());
    }

    #[test]
    fn lru_evicts_least_recently_used() {
        let cache = ProfileCache::new(2);
        let (a, b, c) = (
            profile(SnapshotId::new(), b"x"),
            profile(SnapshotId::new(), b"x"),
            profile(SnapshotId::new(), b"x"),
        );
        cache.insert(a.clone());
        cache.insert(b.clone());
        assert!(cache.get(&a.key()).is_some()); // a is now most recent
        cache.insert(c.clone());
        assert!(cache.get(&b.key()).is_none());
        assert!(cache.get(&a.key()).is_some());
        assert_eq!(cache.len(), 2);
    }
}
