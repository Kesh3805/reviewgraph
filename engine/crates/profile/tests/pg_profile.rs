//! PROF-001 / PROF-006 against PostgreSQL: round trip, conformance, tenancy and the compute guard.

#![cfg(feature = "integration")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::{TimeZone, Utc};
use common::pg::{seed_scope, seed_snapshot, TestDb};
use profile::model::{Convention, ConventionSource, ProfileKey, RepositoryProfile};
use profile::pg::PgProfileStore;
use profile::store::conformance::run_conformance_with;
use profile::store::{ProfileStore, StoreError};
use repository::store::RepoScope;
use review_core::ids::SnapshotId;
use review_core::version::ConfigHash;

fn profile_for(scope: &RepoScope, snapshot: uuid::Uuid, cfg: &[u8]) -> RepositoryProfile {
    RepositoryProfile::empty(
        scope.repository_id,
        SnapshotId::from_uuid(snapshot),
        ConfigHash::of(cfg),
        Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
    )
}

fn convention(id: &str) -> Convention {
    Convention {
        id: id.to_owned(),
        rule: "a rule".to_owned(),
        scope: "src/**".to_owned(),
        samples: 12,
        violations: 1,
        consistency: 0.91,
        confidence: 0.93,
        exceptions: vec![],
        source: ConventionSource::Inferred,
        computed_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
        profile_version: 1,
    }
}

/// The synchronous port over the async adapter, so the shared conformance suite can drive it.
struct BlockingPg {
    store: PgProfileStore,
    scope: RepoScope,
}

impl ProfileStore for BlockingPg {
    fn put(&self, profile: &RepositoryProfile) -> Result<bool, StoreError> {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current()
                .block_on(self.store.put(&self.scope, profile))
                .map_err(|e| StoreError::Io(e.to_string()))
        })
    }

    fn get(&self, key: &ProfileKey) -> Result<Option<RepositoryProfile>, StoreError> {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current()
                .block_on(self.store.get(&self.scope, key))
                .map_err(|e| StoreError::Io(e.to_string()))
        })
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn profile_roundtrip_pg() {
    let db = TestDb::new().await;
    let scope = seed_scope(&db.pool).await;
    let snapshot = seed_snapshot(&db.pool, &scope).await;
    let store = PgProfileStore::new(db.pool.clone());

    let mut p = profile_for(&scope, snapshot, b"cfg");
    p.conventions
        .push(convention("controllers_do_not_access_repositories"));
    assert!(store.put(&scope, &p).await.unwrap());
    assert_eq!(
        store.get(&scope, &p.key()).await.unwrap().as_ref(),
        Some(&p)
    );

    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM profile_conventions")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(rows, 1);
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn profile_insert_conflict_is_noop_pg() {
    let db = TestDb::new().await;
    let scope = seed_scope(&db.pool).await;
    let snapshot = seed_snapshot(&db.pool, &scope).await;
    let store = PgProfileStore::new(db.pool.clone());

    let p = profile_for(&scope, snapshot, b"cfg");
    assert!(store.put(&scope, &p).await.unwrap());
    assert!(!store.put(&scope, &p).await.unwrap());
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM repository_profiles")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(n, 1);
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn profile_store_conformance_suite_pg() {
    let db = TestDb::new().await;
    let scope = seed_scope(&db.pool).await;
    let snapshot = seed_snapshot(&db.pool, &scope).await;
    let adapter = BlockingPg {
        store: PgProfileStore::new(db.pool.clone()),
        scope,
    };
    let fresh_scope = adapter.scope;
    run_conformance_with(&adapter, || profile_for(&fresh_scope, snapshot, b"cfg"));
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn profile_is_invisible_to_other_tenants() {
    let db = TestDb::new().await;
    let scope = seed_scope(&db.pool).await;
    let other = seed_scope(&db.pool).await;
    let snapshot = seed_snapshot(&db.pool, &scope).await;
    let store = PgProfileStore::new(db.pool.clone());

    let p = profile_for(&scope, snapshot, b"cfg");
    assert!(store.put(&scope, &p).await.unwrap());
    assert!(store.get(&other, &p.key()).await.unwrap().is_none());
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_compute_single_row() {
    let db = TestDb::new().await;
    let scope = seed_scope(&db.pool).await;
    let snapshot = seed_snapshot(&db.pool, &scope).await;
    let store = PgProfileStore::new(db.pool.clone());
    let template = profile_for(&scope, snapshot, b"cfg");
    let key = template.key();
    let computes = Arc::new(AtomicUsize::new(0));

    let mut tasks = Vec::new();
    for _ in 0..2 {
        let (store, computes, template) = (store.clone(), Arc::clone(&computes), template.clone());
        tasks.push(tokio::spawn(async move {
            store
                .get_or_compute(&scope, &key, move || async move {
                    computes.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    Ok(template)
                })
                .await
                .unwrap()
        }));
    }
    let mut computed_flags = Vec::new();
    for t in tasks {
        let (_, computed) = t.await.unwrap();
        computed_flags.push(computed);
    }

    assert_eq!(
        computes.load(Ordering::SeqCst),
        1,
        "the loser reads the winner's row"
    );
    assert_eq!(computed_flags.iter().filter(|c| **c).count(), 1);
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM repository_profiles")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(rows, 1);
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn failed_compute_leaves_no_row() {
    let db = TestDb::new().await;
    let scope = seed_scope(&db.pool).await;
    let snapshot = seed_snapshot(&db.pool, &scope).await;
    let store = PgProfileStore::new(db.pool.clone());
    let key = profile_for(&scope, snapshot, b"cfg").key();

    let err = store
        .get_or_compute(&scope, &key, || async { Err("boom".to_owned()) })
        .await;
    assert!(err.is_err());
    assert!(store.get(&scope, &key).await.unwrap().is_none());
    db.finish().await;
}
