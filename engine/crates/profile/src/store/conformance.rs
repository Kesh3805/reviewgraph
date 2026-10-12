//! The shared `ProfileStore` conformance suite (PROF-001). Every adapter runs it.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::future::Future;

use chrono::{TimeZone, Utc};
use repository::store::RepoScope;
use review_core::ids::{OrganizationId, RepositoryId, SnapshotId};
use review_core::version::{ConfigHash, ProfileVersion};

use super::ProfileStore;
use crate::model::{
    Convention, ConventionSource, Layer, ProfileKey, ProfileNote, RepositoryProfile,
};

/// A profile with enough content to catch lossy round trips.
pub fn sample_profile(scope: &RepoScope, snapshot_id: SnapshotId, version: u32) -> RepositoryProfile {
    let key = ProfileKey {
        repository_id: scope.repository_id,
        snapshot_id,
        profile_version: ProfileVersion(version),
        config_hash: ConfigHash::of(b"config"),
    };
    let computed_at = Utc.with_ymd_and_hms(2026, 10, 1, 12, 0, 0).unwrap();
    let mut profile = RepositoryProfile::empty(key, computed_at);
    profile.architecture.layers.push(Layer {
        name: "controller".to_owned(),
        globs: vec!["**/*.controller.ts".to_owned()],
        member_count: 4,
        role_confidence: 0.85,
    });
    profile.conventions.push(Convention {
        id: "controllers_do_not_access_repositories".to_owned(),
        rule: "Controllers do not depend on repositories".to_owned(),
        scope: "src/**".to_owned(),
        samples: 12,
        violations: 1,
        consistency: 0.916_666_7,
        confidence: 0.75,
        enforceable: false,
        exceptions: Vec::new(),
        violation_examples: Vec::new(),
        source: ConventionSource::Inferred,
        miner_version: 1,
        computed_at,
        profile_version: ProfileVersion(version),
    });
    profile.notes.push(ProfileNote {
        code: "sample".to_owned(),
        message: "conformance".to_owned(),
    });
    profile
}

/// Runs every conformance case. `fresh` returns a store and a scope that no other case uses.
pub async fn run_conformance<S, F, Fut>(fresh: F)
where
    S: ProfileStore,
    F: Fn() -> Fut,
    Fut: Future<Output = (S, RepoScope)>,
{
    roundtrip(&fresh().await).await;
    conflict_is_noop(&fresh().await).await;
    scope_isolation(&fresh().await).await;
    latest_for_snapshot(&fresh().await).await;
    rejects_foreign_repository(&fresh().await).await;
}

async fn roundtrip<S: ProfileStore>((store, scope): &(S, RepoScope)) {
    let profile = sample_profile(scope, SnapshotId::new(), 1);
    assert!(store.save(scope, &profile).await.unwrap().created);
    let back = store.get(scope, &profile.key()).await.unwrap().unwrap();
    assert_eq!(back, profile);
}

async fn conflict_is_noop<S: ProfileStore>((store, scope): &(S, RepoScope)) {
    let first = sample_profile(scope, SnapshotId::new(), 1);
    store.save(scope, &first).await.unwrap();
    let mut second = first.clone();
    second.notes.clear();
    second.computed_at = Utc.with_ymd_and_hms(2026, 10, 2, 0, 0, 0).unwrap();
    assert!(!store.save(scope, &second).await.unwrap().created);
    let kept = store.get(scope, &first.key()).await.unwrap().unwrap();
    assert_eq!(kept, first, "the first write wins");
}

async fn scope_isolation<S: ProfileStore>((store, scope): &(S, RepoScope)) {
    let profile = sample_profile(scope, SnapshotId::new(), 1);
    store.save(scope, &profile).await.unwrap();
    let other = RepoScope {
        organization_id: OrganizationId::new(),
        repository_id: RepositoryId::new(),
    };
    let mut foreign_key = profile.key();
    foreign_key.repository_id = other.repository_id;
    assert!(store.get(&other, &foreign_key).await.unwrap().is_none());
    assert!(store
        .latest_for_snapshot(&other, profile.snapshot_id)
        .await
        .unwrap()
        .is_none());
}

async fn latest_for_snapshot<S: ProfileStore>((store, scope): &(S, RepoScope)) {
    let snapshot = SnapshotId::new();
    let old = sample_profile(scope, snapshot, 1);
    let mut new = sample_profile(scope, snapshot, 2);
    new.computed_at = Utc.with_ymd_and_hms(2026, 10, 3, 0, 0, 0).unwrap();
    store.save(scope, &old).await.unwrap();
    store.save(scope, &new).await.unwrap();
    let latest = store
        .latest_for_snapshot(scope, snapshot)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(latest.profile_version, ProfileVersion(2));
    assert!(store
        .latest_for_snapshot(scope, SnapshotId::new())
        .await
        .unwrap()
        .is_none());
}

async fn rejects_foreign_repository<S: ProfileStore>((store, scope): &(S, RepoScope)) {
    let mut profile = sample_profile(scope, SnapshotId::new(), 1);
    profile.repository_id = RepositoryId::new();
    assert!(store.save(scope, &profile).await.is_err());
}
