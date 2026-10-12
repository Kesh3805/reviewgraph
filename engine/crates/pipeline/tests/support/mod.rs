//! Shared helpers for the database-backed tests of this crate (feature `integration`).
//!
//! Each test creates a throwaway database from `TEST_DATABASE_URL` (or `DATABASE_URL`), so tests
//! are independent and parallel-safe. The connecting role must be allowed to CREATE DATABASE.

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::str::FromStr;

use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::PgPool;
use uuid::Uuid;

static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

/// Key of the advisory lock that serializes migrations across the test databases
/// (`20261002000006_db_roles` creates cluster-wide roles).
const MIGRATION_LOCK_KEY: i64 = 0x5247_4d49_4752;

pub struct TestDb {
    pub pool: PgPool,
    admin: PgPool,
    name: String,
}

impl TestDb {
    pub async fn new() -> Self {
        let url = std::env::var("TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .expect("set TEST_DATABASE_URL or DATABASE_URL to run the integration tests");
        let admin = PgPoolOptions::new()
            .max_connections(2)
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
            .max_connections(40)
            .connect_with(options)
            .await
            .unwrap();
        let mut conn = admin.acquire().await.unwrap();
        sqlx::query("SELECT pg_advisory_lock($1)")
            .bind(MIGRATION_LOCK_KEY)
            .execute(&mut *conn)
            .await
            .unwrap();
        let result = MIGRATOR.run(&pool).await;
        sqlx::query("SELECT pg_advisory_unlock($1)")
            .bind(MIGRATION_LOCK_KEY)
            .execute(&mut *conn)
            .await
            .unwrap();
        result.unwrap();
        drop(conn);
        Self { pool, admin, name }
    }

    pub async fn finish(self) {
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

/// Inserts an organization and returns its id.
pub async fn seed_org(pool: &PgPool) -> Uuid {
    let tag = format!("org-{}", &Uuid::new_v4().simple().to_string()[..12]);
    sqlx::query_scalar(
        "INSERT INTO organizations (slug, display_name) VALUES ($1, $1) RETURNING id",
    )
    .bind(tag)
    .fetch_one(pool)
    .await
    .unwrap()
}

pub struct SeededRun {
    pub org: Uuid,
    pub repository: Uuid,
    pub pull_request: Uuid,
    pub run: Uuid,
}

/// Inserts organization, installation, repository, pull request and one `RECEIVED` review run.
pub async fn seed_run(pool: &PgPool, head_sha: &str) -> SeededRun {
    let org = seed_org(pool).await;
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
    let repository: Uuid = sqlx::query_scalar(
        "INSERT INTO repositories
           (organization_id, installation_id, provider, provider_repo_id, full_name,
            default_branch, visibility)
         VALUES ($1, $2, 'github', $3, $3, 'main', 'private') RETURNING id",
    )
    .bind(org)
    .bind(installation)
    .bind(Uuid::new_v4().to_string())
    .fetch_one(pool)
    .await
    .unwrap();
    let pull_request: Uuid = sqlx::query_scalar(
        "INSERT INTO pull_requests
           (organization_id, repository_id, provider_number, title, author_login, state,
            base_ref, head_ref, base_sha, head_sha)
         VALUES ($1, $2, 1, 't', 'a', 'open', 'main', 'feat', $3, $4) RETURNING id",
    )
    .bind(org)
    .bind(repository)
    .bind("a".repeat(40))
    .bind(head_sha)
    .fetch_one(pool)
    .await
    .unwrap();
    let run: Uuid = sqlx::query_scalar(
        "INSERT INTO review_runs
           (organization_id, repository_id, pull_request_id, base_sha, head_sha, state, trigger)
         VALUES ($1, $2, $3, $4, $5, 'RECEIVED', 'webhook') RETURNING id",
    )
    .bind(org)
    .bind(repository)
    .bind(pull_request)
    .bind("a".repeat(40))
    .bind(head_sha)
    .fetch_one(pool)
    .await
    .unwrap();
    SeededRun {
        org,
        repository,
        pull_request,
        run,
    }
}
