#![cfg(feature = "integration")]
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
//! PostgreSQL cache and ledger tests. Need `TEST_DATABASE_URL` (or `DATABASE_URL`) for a role
//! that may CREATE DATABASE; each test uses a throwaway database with every migration applied.

use std::str::FromStr;
use std::time::Duration;

use model_gateway::accounting::ledger::{pg::spawn_writer, ChannelLedger};
use model_gateway::cache::pg::PgCache;
use model_gateway::cache::{cache_key, CacheEntry};
use model_gateway::{LedgerRecord, LedgerSink, ModelOutput, ResponseCache, ServedFrom, Usage};
use review_core::ids::{OrganizationId, RepositoryId};
use serde_json::json;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{PgPool, Row};
use uuid::Uuid;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

/// Key of the advisory lock that serializes migrations across the test databases.
const MIGRATION_LOCK_KEY: i64 = 0x5247_4d49_4752;

/// Runs `work` (a migration) while holding a cluster-wide advisory lock on the admin connection.
/// `20261002000006_db_roles` creates cluster-wide roles, so two throwaway databases migrating at
/// the same time race on `CREATE ROLE` (a duplicate key in `pg_authid`).
async fn with_migration_lock<T>(admin: &PgPool, work: impl std::future::Future<Output = T>) -> T {
    let mut conn = admin.acquire().await.unwrap();
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(MIGRATION_LOCK_KEY)
        .execute(&mut *conn)
        .await
        .unwrap();
    let out = work.await;
    sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(MIGRATION_LOCK_KEY)
        .execute(&mut *conn)
        .await
        .unwrap();
    out
}

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
        with_migration_lock(&admin, MIGRATOR.run(&pool))
            .await
            .unwrap();
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

fn entry(n: i64) -> CacheEntry {
    CacheEntry {
        request_hash: "h".into(),
        provider: "anthropic".into(),
        model: "m".into(),
        prompt_version: "v1".into(),
        schema_hash: Some("s".into()),
        output: ModelOutput::Json(json!({"n": n})),
        usage: Usage {
            input_uncached: 1,
            cache_write: 2,
            cache_read: 3,
            output: 4,
            reasoning: 0,
        },
    }
}

#[tokio::test]
async fn cache_roundtrip_and_cross_tenant_miss() {
    let db = TestDb::new().await;
    let cache = PgCache::new(db.pool.clone());
    let (a, b) = (OrganizationId::new(), OrganizationId::new());
    let key = cache_key(a, "h", "anthropic", "m", Some("s"));
    cache
        .put(a, &key, entry(1), Duration::from_secs(60))
        .await
        .unwrap();
    let hit = cache.get(a, &key).await.unwrap().expect("hit");
    assert_eq!(hit.output, ModelOutput::Json(json!({"n": 1})));
    assert_eq!(hit.usage.cache_read, 3);
    // Same key text, another organisation: miss (predicate), and a key built for B never equals A's.
    assert!(cache.get(b, &key).await.unwrap().is_none());
    assert_ne!(cache_key(b, "h", "anthropic", "m", Some("s")), key);
    db.finish().await;
}

#[tokio::test]
async fn cache_expired_is_miss() {
    let db = TestDb::new().await;
    let cache = PgCache::new(db.pool.clone());
    let org = OrganizationId::new();
    cache.put(org, "k", entry(1), Duration::ZERO).await.unwrap();
    assert!(cache.get(org, "k").await.unwrap().is_none());
    db.finish().await;
}

#[tokio::test]
async fn cache_put_conflict_is_noop() {
    let db = TestDb::new().await;
    let cache = PgCache::new(db.pool.clone());
    let org = OrganizationId::new();
    cache
        .put(org, "k", entry(1), Duration::from_secs(60))
        .await
        .unwrap();
    cache
        .put(org, "k", entry(2), Duration::from_secs(60))
        .await
        .unwrap();
    assert_eq!(
        cache.get(org, "k").await.unwrap().unwrap().output,
        ModelOutput::Json(json!({"n": 1}))
    );
    db.finish().await;
}

#[tokio::test]
async fn purge_deletes_only_expired() {
    let db = TestDb::new().await;
    let cache = PgCache::new(db.pool.clone());
    let org = OrganizationId::new();
    cache
        .put(org, "old", entry(1), Duration::ZERO)
        .await
        .unwrap();
    cache
        .put(org, "new", entry(2), Duration::from_secs(600))
        .await
        .unwrap();
    assert_eq!(cache.purge_expired().await.unwrap(), 1);
    assert!(cache.get(org, "new").await.unwrap().is_some());
    let left: i64 = sqlx::query("SELECT count(*) FROM model_cache")
        .fetch_one(&db.pool)
        .await
        .unwrap()
        .get(0);
    assert_eq!(left, 1);
    db.finish().await;
}

fn record(org: OrganizationId, n: u32) -> LedgerRecord {
    LedgerRecord {
        id: Uuid::now_v7(),
        organization_id: org,
        repository_id: RepositoryId::new(),
        review_run_id: None,
        reviewer_run_id: None,
        task: "correctness_review".into(),
        tier: "review_reasoner".into(),
        provider: "anthropic".into(),
        model: "claude-sonnet-5-5".into(),
        attempt: 1,
        request_hash: format!("hash-{n}"),
        served_from: ServedFrom::Live,
        outcome: "ok".into(),
        usage: Usage {
            input_uncached: 10,
            cache_write: 0,
            cache_read: 0,
            output: 5,
            reasoning: 0,
        },
        cost_usd_micros: Some(70),
        latency_ms: 12,
        prices_as_of: chrono::NaiveDate::from_ymd_opt(2026, 10, 3),
    }
}

#[tokio::test]
async fn ledger_batches_and_flushes() {
    let db = TestDb::new().await;
    let (ledger, rx) = ChannelLedger::channel(1000);
    let writer = spawn_writer(db.pool.clone(), rx);
    let (a, b) = (OrganizationId::new(), OrganizationId::new());
    for n in 0..250 {
        ledger.record(record(if n % 2 == 0 { a } else { b }, n));
    }
    // Closing the channel flushes whatever is left.
    drop(ledger);
    writer.await.unwrap();
    let rows: i64 = sqlx::query("SELECT count(*) FROM model_calls")
        .fetch_one(&db.pool)
        .await
        .unwrap()
        .get(0);
    assert_eq!(rows, 250);
    let cost: i64 = sqlx::query(
        "SELECT sum(cost_usd_micros)::bigint FROM model_calls WHERE organization_id = $1",
    )
    .bind(a.into_uuid())
    .fetch_one(&db.pool)
    .await
    .unwrap()
    .get(0);
    assert_eq!(cost, 125 * 70);
    db.finish().await;
}

#[tokio::test]
async fn ledger_flushes_on_timer() {
    let db = TestDb::new().await;
    let (ledger, rx) = ChannelLedger::channel(10);
    let _writer = spawn_writer(db.pool.clone(), rx);
    ledger.record(record(OrganizationId::new(), 1));
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let rows: i64 = sqlx::query("SELECT count(*) FROM model_calls")
        .fetch_one(&db.pool)
        .await
        .unwrap()
        .get(0);
    assert_eq!(rows, 1);
    db.finish().await;
}
