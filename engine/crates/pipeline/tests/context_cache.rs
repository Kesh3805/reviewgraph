#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use async_trait::async_trait;
use context_engine::cache::CtxKey;
use context_engine::hash::seal;
use context_engine::model::{
    ChangeSummary, ContextBudget, ContextItem, ContextPackage, ContextProvenance, ItemContent,
    ItemId, ItemKind, SignalVector,
};
use pipeline::cache::{get_or_compute, AsyncContextCache, CacheError, HitScope};
use review_core::location::DiffSide;
use review_core::reviewer_type::ReviewerType;

fn package(cluster: &str) -> ContextPackage {
    seal(ContextPackage::new(
        ReviewerType::Security,
        cluster,
        vec![],
        vec![],
        ContextBudget {
            max_tokens: 100,
            max_items: 10,
        },
        vec![],
        ChangeSummary::default(),
        ContextProvenance::default(),
    ))
}

fn big_package() -> ContextPackage {
    let item = ContextItem {
        id: ItemId::of(ItemKind::RelatedSymbol, None, DiffSide::Head, "big"),
        kind: ItemKind::RelatedSymbol,
        node: None,
        side: DiffSide::Head,
        title: "big".to_owned(),
        relation: None,
        distance: Some(1),
        path: vec![],
        sources: Default::default(),
        signals: SignalVector::default(),
        score: 0.5,
        content: ItemContent::Full {
            text: "x".repeat(1_300_000),
        },
        locations: vec![],
        tokens: 10,
        pinned: false,
    };
    seal(ContextPackage::new(
        ReviewerType::Security,
        "big",
        vec![item],
        vec![],
        ContextBudget {
            max_tokens: 100,
            max_items: 10,
        },
        vec![],
        ChangeSummary::default(),
        ContextProvenance::default(),
    ))
}

fn key(run: &str, input_hash: &str) -> CtxKey {
    CtxKey {
        review_run_id: run.to_owned(),
        cluster_id: "c1".to_owned(),
        reviewer: ReviewerType::Security,
        input_hash: input_hash.to_owned(),
    }
}

/// A cache whose backend is down.
struct Broken;

#[async_trait]
impl AsyncContextCache for Broken {
    async fn lookup(&self, _: &CtxKey) -> Result<Option<(ContextPackage, HitScope)>, CacheError> {
        Err(CacheError::Backend("connection refused".into()))
    }

    async fn put(&self, _: &CtxKey, _: &ContextPackage) -> Result<(), CacheError> {
        Err(CacheError::Backend("connection refused".into()))
    }
}

#[tokio::test]
async fn cache_failure_falls_back_to_compute() {
    let expected = package("c1");
    let (got, hit) = get_or_compute(&Broken, &key("r", &"a".repeat(64)), || async {
        package("c1")
    })
    .await;
    assert_eq!(got, expected);
    assert_eq!(hit, None);
}

#[cfg(feature = "integration")]
mod pg {
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::common::pg::{seed_review_run, seed_scope, TestDb};
    use pipeline::cache::context_pg::PgContextCache;
    use pipeline::cache::BlobStore;

    #[derive(Default)]
    struct MemBlobs(Mutex<BTreeMap<String, Vec<u8>>>);

    #[async_trait]
    impl BlobStore for MemBlobs {
        async fn put(&self, key: &str, bytes: Vec<u8>) -> Result<(), CacheError> {
            self.0.lock().unwrap().insert(key.to_owned(), bytes);
            Ok(())
        }

        async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, CacheError> {
            Ok(self.0.lock().unwrap().get(key).cloned())
        }
    }

    async fn rows(db: &TestDb, run: uuid::Uuid) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM stage_outputs WHERE review_run_id = $1")
            .bind(run)
            .fetch_one(&db.pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn pg_cache_roundtrip() {
        let db = TestDb::new().await;
        let scope = seed_scope(&db.pool).await;
        let run = seed_review_run(&db.pool, &scope).await;
        let cache = PgContextCache::new(db.pool.clone(), scope, None);
        let k = key(&run.to_string(), &"a".repeat(64));
        assert!(cache.lookup(&k).await.unwrap().is_none());
        let pkg = package("c1");
        cache.put(&k, &pkg).await.unwrap();
        let (got, hit) = cache.lookup(&k).await.unwrap().unwrap();
        assert_eq!(got, pkg);
        assert_eq!(hit, HitScope::Run);
        db.finish().await;
    }

    #[tokio::test]
    async fn concurrent_put_same_key_single_row() {
        let db = TestDb::new().await;
        let scope = seed_scope(&db.pool).await;
        let run = seed_review_run(&db.pool, &scope).await;
        let cache = Arc::new(PgContextCache::new(db.pool.clone(), scope, None));
        let k = key(&run.to_string(), &"b".repeat(64));
        let mut tasks = Vec::new();
        for _ in 0..8 {
            let (cache, k) = (Arc::clone(&cache), k.clone());
            tasks.push(tokio::spawn(async move {
                cache.put(&k, &package("c1")).await.unwrap();
            }));
        }
        for t in tasks {
            t.await.unwrap();
        }
        assert_eq!(rows(&db, run).await, 1);
        db.finish().await;
    }

    #[tokio::test]
    async fn cross_run_reuse_same_repo_only() {
        let db = TestDb::new().await;
        let scope = seed_scope(&db.pool).await;
        let run_a = seed_review_run(&db.pool, &scope).await;
        let run_b = seed_review_run(&db.pool, &scope).await;
        let cache = PgContextCache::new(db.pool.clone(), scope, None);
        let hash = "c".repeat(64);
        let pkg = package("c1");
        cache
            .put(&key(&run_a.to_string(), &hash), &pkg)
            .await
            .unwrap();

        // A superseding run of the same repository reuses it and gets its own row.
        let (got, hit) = cache
            .lookup(&key(&run_b.to_string(), &hash))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(got, pkg);
        assert_eq!(hit, HitScope::Repo);
        assert_eq!(rows(&db, run_b).await, 1);

        // A different repository (and tenant) never sees it.
        let other = seed_scope(&db.pool).await;
        let run_c = seed_review_run(&db.pool, &other).await;
        let other_cache = PgContextCache::new(db.pool.clone(), other, None);
        assert!(other_cache
            .lookup(&key(&run_c.to_string(), &hash))
            .await
            .unwrap()
            .is_none());
        db.finish().await;
    }

    #[tokio::test]
    async fn large_package_spills_to_blob_store() {
        let db = TestDb::new().await;
        let scope = seed_scope(&db.pool).await;
        let run = seed_review_run(&db.pool, &scope).await;
        let blobs = Arc::new(MemBlobs::default());
        let cache = PgContextCache::new(db.pool.clone(), scope, Some(blobs.clone()));
        let k = key(&run.to_string(), &"d".repeat(64));
        let pkg = big_package();
        cache.put(&k, &pkg).await.unwrap();
        let blob_rows: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM stage_outputs WHERE review_run_id = $1 AND blob_key IS NOT NULL AND output IS NULL",
        )
        .bind(run)
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert_eq!(blob_rows, 1);
        assert_eq!(blobs.0.lock().unwrap().len(), 1);
        let (got, _) = cache.lookup(&k).await.unwrap().unwrap();
        assert_eq!(got, pkg);
        db.finish().await;
    }
}
