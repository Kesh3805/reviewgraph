//! Full index against Postgres: progress rows (IDX-002), store parity (IDX-001) and parse
//! diagnostics (IDX-003). Run with:
//!
//!   TEST_DATABASE_URL=postgres://... cargo test -p pipeline --features integration
//!
//! Each test creates a throwaway database.

#![cfg(feature = "integration")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use graph_storage::mem::MemGraphStore;
use graph_storage::pg::{PgGraphStore, PgIndexRuns};
use graph_storage::status::SnapshotStatus;
use graph_storage::types::SnapshotPurpose;
use graph_storage::GraphStore;
use pipeline::index::{
    typescript_registry, FullIndexRequest, FullIndexer, NoopProgress, PgProgressSink, SourceSpec,
};
use repository::store::RepoScope;
use review_core::ids::{CommitSha, OrganizationId, RepositoryId};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::PgPool;
use uuid::Uuid;

static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

/// Key of the advisory lock that serializes migrations across the test databases.
const MIGRATION_LOCK_KEY: i64 = 0x5247_4d49_4752;

struct TestDb {
    pool: PgPool,
    admin: PgPool,
    name: String,
}

impl TestDb {
    async fn new() -> Self {
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
            .max_connections(8)
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

/// A fresh organization with one repository.
async fn seed_scope(pool: &PgPool) -> RepoScope {
    let tag = format!("org-{}", &Uuid::new_v4().simple().to_string()[..12]);
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

fn write(root: &Path, rel: &str, body: &str) {
    let full = root.join(rel);
    std::fs::create_dir_all(full.parent().unwrap()).unwrap();
    std::fs::write(full, body).unwrap();
}

fn small_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "src/a.ts",
        "export function helper(): number { return 1; }\n",
    );
    write(
        dir.path(),
        "src/b.ts",
        "import { helper } from './a';\nexport function main() { return helper(); }\n",
    );
    write(dir.path(), "src/broken.ts", "export function ( {{{ \n");
    dir
}

fn request(scope: RepoScope, root: &Path) -> FullIndexRequest {
    FullIndexRequest::new(
        scope,
        "b".repeat(40).parse::<CommitSha>().unwrap(),
        SourceSpec::WorkingTree {
            root: root.to_path_buf(),
        },
        SnapshotPurpose::Local,
    )
}

fn indexer(store: Arc<dyn GraphStore>) -> FullIndexer {
    let (registry, parsers) = typescript_registry();
    FullIndexer::new(store, registry, 2)
        .unwrap()
        .with_parser_versions(parsers)
}

#[tokio::test]
async fn index_runs_reflects_the_finished_run() {
    let db = TestDb::new().await;
    let scope = seed_scope(&db.pool).await;
    let repo = small_repo();
    let runs = PgIndexRuns::new(db.pool.clone());
    let sink = PgProgressSink::new(runs.clone(), scope, None);
    let store: Arc<dyn GraphStore> = Arc::new(PgGraphStore::new(db.pool.clone()));
    let out = indexer(Arc::clone(&store))
        .run(request(scope, repo.path()), &sink)
        .await
        .unwrap();
    assert_eq!(out.snapshot.status, SnapshotStatus::Ready);

    let run = runs.latest(&scope).await.unwrap().expect("a run row");
    assert_eq!(run.snapshot_id, *out.snapshot.id.as_uuid());
    assert_eq!(run.kind, "full");
    assert_eq!(run.phase, "done");
    assert!(run.finished_at.is_some());
    assert_eq!(run.files_done, 3);
    assert_eq!(run.files_total, Some(3));
    assert_eq!(run.stats["files_ok"], 2);
    assert_eq!(run.stats["files_partial"], 1);
    assert!(run.error.is_none());
    db.finish().await;
}

#[tokio::test]
async fn pre_cancelled_run_leaves_no_run_row() {
    let db = TestDb::new().await;
    let scope = seed_scope(&db.pool).await;
    let repo = small_repo();
    let runs = PgIndexRuns::new(db.pool.clone());
    let sink = PgProgressSink::new(runs.clone(), scope, None);
    let store: Arc<dyn GraphStore> = Arc::new(PgGraphStore::new(db.pool.clone()));
    let mut req = request(scope, repo.path());
    req.cancel.cancel();
    // A pre-cancelled run stops before any snapshot exists, so the run row is absent.
    assert!(indexer(Arc::clone(&store)).run(req, &sink).await.is_err());
    assert!(runs.latest(&scope).await.unwrap().is_none());
    db.finish().await;
}

#[tokio::test]
async fn file_and_pg_stores_produce_identical_graphs() {
    let db = TestDb::new().await;
    let scope = seed_scope(&db.pool).await;
    let repo = small_repo();

    let pg: Arc<dyn GraphStore> = Arc::new(PgGraphStore::new(db.pool.clone()));
    let pg_out = indexer(Arc::clone(&pg))
        .run(request(scope, repo.path()), &NoopProgress)
        .await
        .unwrap();
    let mut pg_graph = pg.load_graph(&scope, pg_out.snapshot.id).await.unwrap();

    let mem: Arc<dyn GraphStore> = Arc::new(MemGraphStore::new());
    let mem_scope = RepoScope {
        organization_id: OrganizationId::new(),
        repository_id: RepositoryId::new(),
    };
    let mem_out = indexer(Arc::clone(&mem))
        .run(request(mem_scope, repo.path()), &NoopProgress)
        .await
        .unwrap();
    let mut mem_graph = mem
        .load_graph(&mem_scope, mem_out.snapshot.id)
        .await
        .unwrap();

    // File version ids are adapter-allocated; everything else must match.
    for graph in [&mut pg_graph, &mut mem_graph] {
        for file in &mut graph.files {
            file.file_version_id = None;
        }
    }
    assert_eq!(pg_graph, mem_graph);
    db.finish().await;
}
