//! PROF-001 and PROF-006 against PostgreSQL.

#![cfg(feature = "integration")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use common::pg::{seed_scope, seed_snapshot, TestDb};
use profile::pg::PgProfileStore;
use profile::store::conformance::sample_profile;
use profile::store::ProfileStore;
use review_core::ids::SnapshotId;

#[tokio::test]
async fn profile_roundtrip_pg() {
    let db = TestDb::new().await;
    let scope = seed_scope(&db.pool).await;
    let snapshot = SnapshotId::from_uuid(seed_snapshot(&db.pool, &scope).await);
    let store = PgProfileStore::new(db.pool.clone());
    let profile = sample_profile(&scope, snapshot, 1);
    assert!(store.save(&scope, &profile).await.unwrap().created);
    assert!(!store.save(&scope, &profile).await.unwrap().created);
    assert_eq!(
        store.get(&scope, &profile.key()).await.unwrap(),
        Some(profile.clone())
    );
    assert_eq!(
        store
            .latest_for_snapshot(&scope, snapshot)
            .await
            .unwrap()
            .map(|p| p.key()),
        Some(profile.key())
    );
    let conventions: i64 = sqlx::query_scalar("SELECT count(*) FROM profile_conventions")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(conventions, 1);
    db.finish().await;
}

#[tokio::test]
async fn profile_store_conformance_pg() {
    let db = TestDb::new().await;
    // The conformance suite invents snapshot ids; the foreign key needs real rows, so the PG run
    // covers the same cases with seeded snapshots.
    let scope = seed_scope(&db.pool).await;
    let store = PgProfileStore::new(db.pool.clone());
    let s1 = SnapshotId::from_uuid(seed_snapshot(&db.pool, &scope).await);
    let old = sample_profile(&scope, s1, 1);
    let mut new = sample_profile(&scope, s1, 2);
    new.computed_at = old.computed_at + chrono::Duration::days(1);
    store.save(&scope, &old).await.unwrap();
    store.save(&scope, &new).await.unwrap();
    let latest = store.latest_for_snapshot(&scope, s1).await.unwrap().unwrap();
    assert_eq!(latest.profile_version.0, 2);

    let other = seed_scope(&db.pool).await;
    let mut foreign = old.key();
    foreign.repository_id = other.repository_id;
    assert!(store.get(&other, &foreign).await.unwrap().is_none());

    let mut wrong = old.clone();
    wrong.repository_id = other.repository_id;
    assert!(store.save(&scope, &wrong).await.is_err());
    db.finish().await;
}

#[tokio::test]
async fn concurrent_compute_single_row() {
    let db = TestDb::new().await;
    let scope = seed_scope(&db.pool).await;
    let snapshot = SnapshotId::from_uuid(seed_snapshot(&db.pool, &scope).await);
    let store = PgProfileStore::new(db.pool.clone());
    let profile = sample_profile(&scope, snapshot, 1);
    let key = profile.key();
    let computed = Arc::new(AtomicUsize::new(0));

    let tasks: Vec<_> = (0..2)
        .map(|_| {
            let store = store.clone();
            let profile = profile.clone();
            let computed = computed.clone();
            tokio::spawn(async move {
                store
                    .get_or_compute(&scope, &key, move || {
                        computed.fetch_add(1, Ordering::SeqCst);
                        std::thread::sleep(std::time::Duration::from_millis(100));
                        profile
                    })
                    .await
                    .unwrap()
            })
        })
        .collect();
    let mut fresh = 0;
    for task in tasks {
        let (got, was_computed) = task.await.unwrap();
        assert_eq!(got.key(), key);
        fresh += usize::from(was_computed);
    }
    assert_eq!(fresh, 1);
    assert_eq!(computed.load(Ordering::SeqCst), 1);
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM repository_profiles")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(rows, 1);
    db.finish().await;
}
