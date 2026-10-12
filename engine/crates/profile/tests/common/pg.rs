//! Throwaway-database helpers for the PostgreSQL integration tests. Run with:
//!
//!   TEST_DATABASE_URL=postgres://... cargo test -p profile --features integration
//!
//! Each test creates its own database, so tests are independent and parallel-safe. The
//! connecting role must be allowed to CREATE DATABASE.

#![allow(dead_code)]

use std::str::FromStr;

use repository::store::RepoScope;
use review_core::ids::{OrganizationId, RepositoryId};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::PgPool;
use uuid::Uuid;

static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

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
        MIGRATOR.run(&pool).await.unwrap();
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

pub fn tag() -> String {
    format!("org-{}", &Uuid::new_v4().simple().to_string()[..12])
}

pub async fn seed_scope(pool: &PgPool) -> RepoScope {
    let tag = tag();
    let org: Uuid = sqlx::query_scalar(
        "INSERT INTO organizations (slug, display_name) VALUES ($1, $1) RETURNING id",
    )
    .bind(&tag)
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

/// A ready full snapshot of the scope's repository.
pub async fn seed_snapshot(pool: &PgPool, scope: &RepoScope) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO snapshots
           (id, organization_id, repository_id, commit_sha, kind, purpose, status,
            graph_schema_version, analyzer_versions, config_hash, fingerprint)
         VALUES ($1, $2, $3, $4, 'full', 'default_branch', 'ready', 1, '{}'::jsonb, $5, $6)",
    )
    .bind(id)
    .bind(scope.organization_id.as_uuid())
    .bind(scope.repository_id.as_uuid())
    .bind("a".repeat(40))
    .bind(vec![0u8; 32])
    .bind(id.as_bytes().repeat(2))
    .execute(pool)
    .await
    .unwrap();
    id
}

/// A review run (and its pull request) for the scope's repository.
pub async fn seed_review_run(pool: &PgPool, scope: &RepoScope) -> Uuid {
    let pr: Uuid = sqlx::query_scalar(
        "INSERT INTO pull_requests
           (organization_id, repository_id, provider_number, title, author_login, base_ref,
            head_ref, base_sha, head_sha, state)
         VALUES ($1, $2, $3, 't', 'dev', 'trunk', 'feature', $4, $5, 'open') RETURNING id",
    )
    .bind(scope.organization_id.as_uuid())
    .bind(scope.repository_id.as_uuid())
    .bind(i32::from(Uuid::new_v4().as_u128() as u16) + 1)
    .bind("a".repeat(40))
    .bind("b".repeat(40))
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query_scalar(
        "INSERT INTO review_runs
           (organization_id, repository_id, pull_request_id, base_sha, head_sha, state, trigger)
         VALUES ($1, $2, $3, $4, $5, 'ANALYZING', 'webhook') RETURNING id",
    )
    .bind(scope.organization_id.as_uuid())
    .bind(scope.repository_id.as_uuid())
    .bind(pr)
    .bind("a".repeat(40))
    .bind("b".repeat(40))
    .fetch_one(pool)
    .await
    .unwrap()
}
