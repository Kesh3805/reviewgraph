#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
//! SEM-004: collection bootstrap, registry and cut-over.

mod common;

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use common::{convention_units, index_for, scope, snapshot, CountingProvider, FakeQdrant};
use semantic::collections::{activate, refresh_targets, retire, CollectionStatus};
use semantic::embedding::{standard, HashProvider};
use semantic::filter::fields;
use semantic::{
    bootstrap, bootstrap_until_ready, CollectionRegistry, Error, HnswCfg, MemoryRegistry,
    QdrantConfig, SemanticIndex,
};

const V1: &str = "rg_hash_fh256_256_v1";
const V2: &str = "rg_hash_fh256_256_v2";

fn index_v(server: &wiremock::MockServer, version: u16) -> SemanticIndex {
    SemanticIndex::new(
        QdrantConfig::new(server.uri()).without_backoff(),
        standard(HashProvider::new(256, version).unwrap()),
    )
    .unwrap()
}

#[tokio::test]
async fn bootstrap_creates_collection_and_indexes() {
    let (server, fake) = FakeQdrant::start().await;
    let index = index_v(&server, 1);
    let registry = MemoryRegistry::new();
    let report = bootstrap(&index, &registry, HnswCfg::default())
        .await
        .unwrap();
    assert!(report.created);
    assert_eq!(report.collection, V1);
    assert_eq!(report.indexes_created, fields::INDEXED.len());
    assert_eq!(report.state, CollectionStatus::Active);
    let c = fake.collection(V1).unwrap();
    assert_eq!(c.dims, 256);
    for (f, _) in fields::INDEXED {
        assert!(c.indexes.contains(f), "{f}");
    }
    let create = fake
        .requests("PUT", &format!("/collections/{V1}"))
        .pop()
        .unwrap();
    assert_eq!(create["vectors"]["distance"], "Cosine");
    assert_eq!(create["hnsw_config"]["m"], 16);
    assert_eq!(create["hnsw_config"]["ef_construct"], 128);
    assert_eq!(create["on_disk_payload"], true);
    let tenant_index = fake
        .requests("PUT", "/index")
        .into_iter()
        .find(|b| b["field_name"] == "organization_id")
        .unwrap();
    assert_eq!(tenant_index["field_schema"]["is_tenant"], true);
    assert_eq!(index.targets().unwrap().read, V1);
    assert_eq!(registry.list().await.unwrap().len(), 1);
}

#[tokio::test]
async fn bootstrap_idempotent() {
    let (server, fake) = FakeQdrant::start().await;
    let index = index_v(&server, 1);
    let registry = MemoryRegistry::new();
    bootstrap(&index, &registry, HnswCfg::default())
        .await
        .unwrap();
    let again = bootstrap(&index, &registry, HnswCfg::default())
        .await
        .unwrap();
    assert!(!again.created);
    assert_eq!(again.indexes_created, 0);
    assert_eq!(registry.list().await.unwrap().len(), 1);
    assert_eq!(fake.requests("PUT", &format!("/collections/{V1}")).len(), 1);
}

#[tokio::test]
async fn dims_mismatch_is_hard_error() {
    let (server, fake) = FakeQdrant::start().await;
    fake.create(V1, 384);
    let index = index_v(&server, 1);
    let err = bootstrap(&index, &MemoryRegistry::new(), HnswCfg::default())
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            Error::SpaceMismatch {
                expected: 256,
                actual: 384,
                ..
            }
        ),
        "{err:?}"
    );
    // A space mismatch is not retried by the start-up loop.
    let looped = bootstrap_until_ready(
        &index,
        &MemoryRegistry::new(),
        HnswCfg::default(),
        Duration::from_millis(1),
    )
    .await;
    assert!(looped.is_err());
}

#[tokio::test]
async fn activation_flips_previous_to_retiring() {
    let (server, fake) = FakeQdrant::start().await;
    let registry = MemoryRegistry::new();
    let v1 = index_v(&server, 1);
    let v2 = index_v(&server, 2);
    bootstrap(&v1, &registry, HnswCfg::default()).await.unwrap();
    let r2 = bootstrap(&v2, &registry, HnswCfg::default()).await.unwrap();
    assert_eq!(r2.state, CollectionStatus::Building);
    // Reads stay on the active collection until activation.
    assert_eq!(v2.targets().unwrap().read, V1);
    let previous = activate(&registry, V2).await.unwrap();
    assert_eq!(previous.as_deref(), Some(V1));
    let states: Vec<(String, CollectionStatus)> = registry
        .list()
        .await
        .unwrap()
        .into_iter()
        .map(|r| (r.name, r.state))
        .collect();
    assert_eq!(
        states,
        vec![
            (V1.to_owned(), CollectionStatus::Retiring),
            (V2.to_owned(), CollectionStatus::Active)
        ]
    );
    let t = refresh_targets(&v2, &registry).await.unwrap();
    assert_eq!(t.read, V2);
    assert_eq!(t.write, vec![V2.to_owned()]);
    // Retirement deletes the old collection.
    let retired = retire(&QdrantConfig::new(server.uri()), &registry, Duration::ZERO)
        .await
        .unwrap();
    assert_eq!(retired, vec![V1.to_owned()]);
    assert!(fake.collection(V1).is_none());
    assert!(fake.collection(V2).is_some());
}

#[tokio::test]
async fn dual_write_during_building() {
    let (server, fake) = FakeQdrant::start().await;
    let registry = MemoryRegistry::new();
    let v1 = index_v(&server, 1);
    bootstrap(&v1, &registry, HnswCfg::default()).await.unwrap();
    let t = index_for(&server.uri(), 256, None);
    let v2 = SemanticIndex::new(
        QdrantConfig::new(server.uri()).without_backoff(),
        standard(Arc::clone(&t.provider)),
    )
    .unwrap();
    // The counting provider is version 1; bootstrap a v2 space explicitly.
    let v2_space = index_v(&server, 2);
    bootstrap(&v2_space, &registry, HnswCfg::default())
        .await
        .unwrap();
    let targets = refresh_targets(&v2, &registry).await.unwrap();
    assert_eq!(targets.write, vec![V1.to_owned(), V2.to_owned()]);
    let (scope, repo) = scope();
    v2.upsert_units(&scope, &convention_units(repo, snapshot(), 3))
        .await
        .unwrap();
    assert_eq!(fake.points(V1).len(), 3);
    assert_eq!(fake.points(V2).len(), 3);
    assert_eq!(
        t.provider.calls(),
        1,
        "vectors are embedded once and written twice"
    );
}

#[tokio::test]
async fn qdrant_down_starts_disabled_and_recovers() {
    let (server, fake) = FakeQdrant::start().await;
    let index = index_v(&server, 1);
    let registry = MemoryRegistry::new();
    // Down for longer than one attempt with its retries.
    fake.fail_next.store(10, Ordering::SeqCst);
    let first = bootstrap(&index, &registry, HnswCfg::default()).await;
    assert!(first.is_err());
    assert!(
        index.targets().is_err(),
        "no targets while semantic is disabled"
    );
    let (report, attempts) = bootstrap_until_ready(
        &index,
        &registry,
        HnswCfg::default(),
        Duration::from_millis(1),
    )
    .await
    .unwrap();
    assert!(attempts >= 2, "{attempts}");
    assert_eq!(report.collection, "rg_hash_fh256_256_v1");
    assert!(index.targets().is_ok());
}

#[tokio::test]
async fn activate_requires_building() {
    let registry = MemoryRegistry::new();
    assert!(matches!(
        activate(&registry, "rg_missing_v1").await,
        Err(Error::Registry(_))
    ));
    let provider = CountingProvider::new(256);
    let rec = semantic::CollectionRecord::for_space(
        semantic::EmbeddingProvider::space(&*provider),
        CollectionStatus::Active,
    );
    registry.insert_if_absent(&rec).await.unwrap();
    assert!(activate(&registry, &rec.name).await.is_err());
}

#[cfg(feature = "integration")]
mod pg {
    use super::*;
    use semantic::collections::PgRegistry;
    use semantic::CollectionRecord;
    use semantic::EmbeddingSpace;
    use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
    use sqlx::PgPool;
    use std::str::FromStr;
    use uuid::Uuid;

    static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

    struct TestDb {
        pool: PgPool,
        admin: PgPool,
        name: String,
    }

    impl TestDb {
        async fn new() -> Self {
            let url = std::env::var("TEST_DATABASE_URL")
                .or_else(|_| std::env::var("DATABASE_URL"))
                .expect("set TEST_DATABASE_URL or DATABASE_URL");
            let admin = PgPoolOptions::new()
                .max_connections(1)
                .connect(&url)
                .await
                .unwrap();
            let name = format!("rg_test_{}", Uuid::new_v4().simple());
            sqlx::query(&format!("CREATE DATABASE \"{name}\""))
                .execute(&admin)
                .await
                .unwrap();
            let options = PgConnectOptions::from_str(&url).unwrap().database(&name);
            let pool = PgPoolOptions::new()
                .max_connections(4)
                .connect_with(options)
                .await
                .unwrap();
            MIGRATOR.run(&pool).await.unwrap();
            Self { pool, admin, name }
        }

        async fn finish(self) {
            self.pool.close().await;
            sqlx::query(&format!(
                "DROP DATABASE IF EXISTS \"{}\" WITH (FORCE)",
                self.name
            ))
            .execute(&self.admin)
            .await
            .unwrap();
        }
    }

    fn rec(version: u16, state: CollectionStatus) -> CollectionRecord {
        let space =
            EmbeddingSpace::new(semantic::ProviderName::Hash, "fh256", 256, version).unwrap();
        CollectionRecord::for_space(&space, state)
    }

    #[tokio::test]
    async fn activation_flips_previous_to_retiring_pg() {
        let db = TestDb::new().await;
        let registry = PgRegistry::new(db.pool.clone());
        registry
            .insert_if_absent(&rec(1, CollectionStatus::Active))
            .await
            .unwrap();
        let (row, inserted) = registry
            .insert_if_absent(&rec(2, CollectionStatus::Building))
            .await
            .unwrap();
        assert!(inserted);
        assert_eq!(row.state, CollectionStatus::Building);
        let (_, again) = registry
            .insert_if_absent(&rec(2, CollectionStatus::Building))
            .await
            .unwrap();
        assert!(!again);
        let previous = activate(&registry, V2).await.unwrap();
        assert_eq!(previous.as_deref(), Some(V1));
        let states: Vec<_> = registry
            .list()
            .await
            .unwrap()
            .into_iter()
            .map(|r| (r.name, r.state))
            .collect();
        assert_eq!(
            states,
            vec![
                (V1.to_owned(), CollectionStatus::Retiring),
                (V2.to_owned(), CollectionStatus::Active)
            ]
        );
        let old = registry.retiring_older_than(Duration::ZERO).await.unwrap();
        assert_eq!(old.len(), 1);
        assert!(registry
            .retiring_older_than(Duration::from_secs(3600))
            .await
            .unwrap()
            .is_empty());
        registry.mark_retired(V1).await.unwrap();
        db.finish().await;
    }

    #[tokio::test]
    async fn only_one_active_enforced_by_index() {
        let db = TestDb::new().await;
        let registry = PgRegistry::new(db.pool.clone());
        registry
            .insert_if_absent(&rec(1, CollectionStatus::Active))
            .await
            .unwrap();
        registry
            .insert_if_absent(&rec(2, CollectionStatus::Building))
            .await
            .unwrap();
        let err = sqlx::query("UPDATE semantic_collections SET state = 'active' WHERE name = $1")
            .bind(V2)
            .execute(&db.pool)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("one_active_collection"), "{err}");
        db.finish().await;
    }

    #[tokio::test]
    async fn bootstrap_lock_is_exclusive() {
        let db = TestDb::new().await;
        let registry = PgRegistry::new(db.pool.clone());
        let held = registry.lock("semantic_bootstrap").await.unwrap();
        let second = tokio::time::timeout(
            Duration::from_millis(300),
            registry.lock("semantic_bootstrap"),
        )
        .await;
        assert!(second.is_err(), "second lock must wait");
        drop(held);
        let third =
            tokio::time::timeout(Duration::from_secs(5), registry.lock("semantic_bootstrap")).await;
        assert!(third.is_ok());
        db.finish().await;
    }

    #[tokio::test]
    async fn bootstrap_against_live_qdrant_and_pg() {
        let Some(cfg) = QdrantConfig::from_lookup(|k| std::env::var(k).ok()) else {
            return;
        };
        let db = TestDb::new().await;
        let registry = PgRegistry::new(db.pool.clone());
        // A unique epoch so parallel runs never share a collection.
        let version = u16::try_from(Uuid::new_v4().as_u128() % 30_000).unwrap() + 1_000;
        let index = SemanticIndex::new(
            cfg.clone(),
            standard(HashProvider::new(256, version).unwrap()),
        )
        .unwrap();
        let first = bootstrap(&index, &registry, HnswCfg::default())
            .await
            .unwrap();
        assert!(first.created);
        let second = bootstrap(&index, &registry, HnswCfg::default())
            .await
            .unwrap();
        assert!(!second.created);
        assert_eq!(second.indexes_created, 0);
        // Clean up the live collection directly.
        reqwest::Client::new()
            .delete(format!("{}/collections/{}", cfg.url, first.collection))
            .send()
            .await
            .unwrap();
        db.finish().await;
    }
}
