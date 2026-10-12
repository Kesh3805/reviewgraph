//! Conformance suite every `ProfileStore` adapter must pass.
//!
//! Test support compiled behind the `conformance` feature: failures panic by design.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use chrono::{TimeZone, Utc};
use review_core::ids::{RepositoryId, SnapshotId};
use review_core::version::ConfigHash;

use super::ProfileStore;
use crate::model::{LanguageShare, RepositoryProfile};

/// A fresh profile with a unique key.
pub fn sample_profile() -> RepositoryProfile {
    RepositoryProfile::empty(
        RepositoryId::new(),
        SnapshotId::new(),
        ConfigHash::of(b"cfg"),
        Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
    )
}

/// Runs the suite against `store`.
pub fn run_conformance(store: &dyn ProfileStore) {
    let p = sample_profile();
    assert!(store.get(&p.key()).unwrap().is_none());
    assert!(store.put(&p).unwrap(), "first put inserts");
    assert_eq!(store.get(&p.key()).unwrap().as_ref(), Some(&p));

    // Same key again is a no-op and keeps the first value.
    let mut again = p.clone();
    again.languages.push(LanguageShare {
        language: "ts".into(),
        files: 1,
        share: 1.0,
    });
    assert!(!store.put(&again).unwrap(), "conflict is a no-op");
    assert_eq!(store.get(&p.key()).unwrap().as_ref(), Some(&p));

    // A different config hash is a different profile.
    let mut other = p.clone();
    other.config_hash = ConfigHash::of(b"other");
    assert!(store.put(&other).unwrap());
    assert_eq!(store.get(&other.key()).unwrap().as_ref(), Some(&other));
}
