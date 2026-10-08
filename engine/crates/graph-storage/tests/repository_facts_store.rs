//! Tests of the PostgreSQL repository-facts adapter (INIT-013). Run with:
//!
//!   TEST_DATABASE_URL=postgres://... cargo test -p graph-storage --features integration
//!
//! Each test creates a throwaway database, so tests are independent and parallel-safe. The
//! connecting role must be allowed to CREATE DATABASE.

#![cfg(feature = "integration")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::str::FromStr;
use std::sync::Arc;

use graph_storage::pg::PgRepositoryFactsStore;
use repository::store::conformance::{run_conformance, sample_facts};
use repository::store::{RepoScope, RepositoryFactsStore};
use review_core::ids::{OrganizationId, RepositoryId};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::PgPool;
use uuid::Uuid;

static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

/// Key of the advisory lock that serializes migrations across the test databases.
const MIGRATION_LOCK_KEY: i64 = 0x5247_4d49_4752;

/// Applies the migrations while holding a cluster-wide advisory lock on the admin connection.
/// `20261002000006_db_roles` creates cluster-wide roles, so two throwaway databases migrating at
/// the same time race on `CREATE ROLE` (a duplicate key in `pg_authid`); every test of this
/// binary migrates through here, one database at a time.
async fn migrate(admin: &PgPool, pool: &PgPool) {
    let mut conn = admin.acquire().await.unwrap();
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(MIGRATION_LOCK_KEY)
        .execute(&mut *conn)
        .await
        .unwrap();
    let result = MIGRATOR.run(pool).await;
    sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(MIGRATION_LOCK_KEY)
        .execute(&mut *conn)
        .await
        .unwrap();
    result.unwrap();
}

struct TestDb {
    pool: PgPool,
    admin: PgPool,
    name: String,
}

impl TestDb {
    async fn new(apply_migrations: bool) -> Self {
        let url = std::env::var("TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .expect("set TEST_DATABASE_URL or DATABASE_URL to run the integration tests");
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
            .max_connections(12)
            .connect_with(options)
            .await
            .unwrap();
        if apply_migrations {
            migrate(&admin, &pool).await;
        }
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

async fn seed_scope(pool: &PgPool, tag: &str) -> RepoScope {
    let org: Uuid = sqlx::query_scalar(
        "INSERT INTO organizations (slug, display_name) VALUES ($1, $1) RETURNING id",
    )
    .bind(tag)
    .fetch_one(pool)
    .await
    .unwrap();
    let installation: Uuid = sqlx::query_scalar(
        "INSERT INTO provider_installations
           (organization_id, provider, provider_installation_id, account_login, account_type)
         VALUES ($1, 'github', $2, 'acme', 'organization') RETURNING id",
    )
    .bind(org)
    .bind(i64::from(Uuid::new_v4().as_u128() as u32))
    .fetch_one(pool)
    .await
    .unwrap();
    let repo: Uuid = sqlx::query_scalar(
        "INSERT INTO repositories
           (organization_id, installation_id, provider, provider_repo_id, full_name,
            default_branch, visibility)
         VALUES ($1, $2, 'github', $3, $3, 'trunk', 'private') RETURNING id",
    )
    .bind(org)
    .bind(installation)
    .bind(format!("{tag}/app"))
    .fetch_one(pool)
    .await
    .unwrap();
    RepoScope {
        organization_id: OrganizationId::from_uuid(org),
        repository_id: RepositoryId::from_uuid(repo),
    }
}

fn tag() -> String {
    format!("org-{}", &Uuid::new_v4().simple().to_string()[..12])
}

#[tokio::test]
async fn pg_store_passes_conformance() {
    let db = TestDb::new(true).await;
    let pool = db.pool.clone();
    run_conformance(|| {
        let pool = pool.clone();
        async move {
            let scope = seed_scope(&pool, &tag()).await;
            (PgRepositoryFactsStore::new(pool), scope)
        }
    })
    .await;
    db.finish().await;
}

#[tokio::test]
async fn pointer_advances_forward_only_and_summary_columns_are_set() {
    let db = TestDb::new(true).await;
    let scope = seed_scope(&db.pool, &tag()).await;
    let store = PgRepositoryFactsStore::new(db.pool.clone());
    let newer = sample_facts('b', "2026-01-02T00:00:00Z");
    let older = sample_facts('a', "2026-01-01T00:00:00Z");
    let saved_newer = store.save(&scope, &newer).await.unwrap();
    store.save(&scope, &older).await.unwrap();
    let (pointer, branch, initialized): (Option<Uuid>, String, Option<chrono::DateTime<chrono::Utc>>) =
        sqlx::query_as(
            "SELECT latest_init_facts_id, default_branch, initialized_at FROM repositories WHERE id = $1",
        )
        .bind(scope.repository_id.as_uuid())
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(
        pointer,
        Some(saved_newer.id.0),
        "an older late save must not regress the pointer"
    );
    assert_eq!(branch, "main");
    assert!(initialized.is_some());
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM repository_init_facts")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(rows, 2);
    db.finish().await;
}

#[tokio::test]
async fn rls_hides_other_org_rows() {
    let db = TestDb::new(true).await;
    let a = seed_scope(&db.pool, &tag()).await;
    let b = seed_scope(&db.pool, &tag()).await;
    let store = PgRepositoryFactsStore::new(db.pool.clone());
    store
        .save(&a, &sample_facts('a', "2026-01-01T00:00:00Z"))
        .await
        .unwrap();

    // Through the adapter, org B cannot see org A's repository facts.
    let cross = RepoScope {
        organization_id: b.organization_id,
        repository_id: a.repository_id,
    };
    assert!(store.latest(&cross).await.unwrap().is_none());

    // Directly, as a non-bypass role, the policy filters rows by app.organization_id.
    let count_as = |org: Uuid| {
        let pool = db.pool.clone();
        async move {
            let mut tx = pool.begin().await.unwrap();
            sqlx::query("SET LOCAL ROLE rg_engine")
                .execute(&mut *tx)
                .await
                .unwrap();
            sqlx::query("SELECT set_config('app.organization_id', $1, true)")
                .bind(org.to_string())
                .execute(&mut *tx)
                .await
                .unwrap();
            let n: i64 = sqlx::query_scalar("SELECT count(*) FROM repository_init_facts")
                .fetch_one(&mut *tx)
                .await
                .unwrap();
            tx.rollback().await.unwrap();
            n
        }
    };
    assert_eq!(count_as(*a.organization_id.as_uuid()).await, 1);
    assert_eq!(count_as(*b.organization_id.as_uuid()).await, 0);
    assert_eq!(count_as(Uuid::new_v4()).await, 0);
    db.finish().await;
}

#[tokio::test]
async fn concurrent_identical_saves_single_row() {
    let db = TestDb::new(true).await;
    let scope = seed_scope(&db.pool, &tag()).await;
    let store = Arc::new(PgRepositoryFactsStore::new(db.pool.clone()));
    let facts = Arc::new(sample_facts('a', "2026-01-01T00:00:00Z"));
    let mut tasks = Vec::new();
    for _ in 0..10 {
        let (store, facts) = (store.clone(), facts.clone());
        tasks.push(tokio::spawn(async move {
            store.save(&scope, &facts).await.unwrap()
        }));
    }
    let mut ids = Vec::new();
    let mut created = 0;
    for task in tasks {
        let saved = task.await.unwrap();
        created += usize::from(saved.created);
        ids.push(saved.id);
    }
    ids.dedup();
    assert_eq!(ids.len(), 1, "every save converges on one row");
    assert_eq!(created, 1);
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM repository_init_facts")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(rows, 1);
    db.finish().await;
}

#[tokio::test]
async fn migration_applies_on_empty_db_and_is_idempotent() {
    let db = TestDb::new(false).await;
    migrate(&db.admin, &db.pool).await;
    migrate(&db.admin, &db.pool).await;
    let rls: bool = sqlx::query_scalar(
        "SELECT relrowsecurity AND relforcerowsecurity FROM pg_class WHERE relname = 'repository_init_facts'",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert!(rls, "row level security must be enabled and forced");
    let columns: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM information_schema.columns
          WHERE table_name = 'repositories'
            AND column_name IN ('latest_init_facts_id', 'primary_language', 'initialized_at')",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(columns, 3);
    db.finish().await;
}
