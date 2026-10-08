//! GS-004/GS-005: the Postgres `GraphStore` against the GS-001 conformance suite and the
//! Postgres-specific cases. Run with:
//!
//!   TEST_DATABASE_URL=postgres://... cargo test -p graph-storage --features integration
//!
//! Each test creates a throwaway database; every conformance case gets fresh tenants.

#![cfg(feature = "integration")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use graph_storage::conformance::{
    edge, file, fixture_graph, key, run_all, sha, symbol, with_file_versions, Harness,
    HarnessFixture, FINGERPRINT, OTHER_FINGERPRINT,
};
use graph_storage::kinds::{Confidence, Direction, EdgeKind, EdgeKindSet};
use graph_storage::model::{flatten, EdgeCursor, FileChange, Graph, GraphDelta};
use graph_storage::pg::PgGraphStore;
use graph_storage::status::SnapshotStatus;
use graph_storage::types::{NewSnapshot, SnapshotPurpose};
use graph_storage::{GraphStore, StoreError};
use repository::store::RepoScope;
use review_core::ids::{OrganizationId, RepositoryId, SnapshotId};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::PgPool;
use uuid::Uuid;

static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

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
            .max_connections(24)
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

#[derive(Debug)]
struct PgHarness {
    pool: PgPool,
}

#[async_trait]
impl Harness for PgHarness {
    fn name(&self) -> &str {
        "pg"
    }

    async fn fresh(&self) -> Result<HarnessFixture, StoreError> {
        Ok(HarnessFixture {
            store: Arc::new(PgGraphStore::new(self.pool.clone())),
            scope: seed_scope(&self.pool).await,
            foreign: seed_scope(&self.pool).await,
        })
    }
}

async fn fixture(db: &TestDb) -> HarnessFixture {
    PgHarness {
        pool: db.pool.clone(),
    }
    .fresh()
    .await
    .unwrap()
}

async fn ready_full(f: &HarnessFixture, g: &Graph, fingerprint: [u8; 32]) -> SnapshotId {
    let meta = f
        .store
        .create_snapshot(NewSnapshot::full(
            f.scope,
            sha('a').unwrap(),
            SnapshotPurpose::DefaultBranch,
            fingerprint,
        ))
        .await
        .unwrap();
    to_persisting(f, meta.id).await;
    f.store.write_full(&f.scope, meta.id, g).await.unwrap();
    assert!(f
        .store
        .transition(
            &f.scope,
            meta.id,
            SnapshotStatus::Persisting,
            SnapshotStatus::Ready,
            None
        )
        .await
        .unwrap());
    meta.id
}

async fn to_persisting(f: &HarnessFixture, id: SnapshotId) {
    for (from, to) in [
        (SnapshotStatus::Pending, SnapshotStatus::Indexing),
        (SnapshotStatus::Indexing, SnapshotStatus::Persisting),
    ] {
        assert!(f
            .store
            .transition(&f.scope, id, from, to, None)
            .await
            .unwrap());
    }
}

async fn ready_delta(f: &HarnessFixture, base: SnapshotId, d: &GraphDelta, n: u8) -> SnapshotId {
    let mut fingerprint = [0u8; 32];
    fingerprint[0] = n;
    fingerprint[1] = 0xd;
    let meta = f
        .store
        .create_snapshot(NewSnapshot::delta(
            f.scope,
            sha('b').unwrap(),
            SnapshotPurpose::PullRequest,
            base,
            fingerprint,
        ))
        .await
        .unwrap();
    to_persisting(f, meta.id).await;
    f.store.write_delta(&f.scope, meta.id, d).await.unwrap();
    assert!(f
        .store
        .transition(
            &f.scope,
            meta.id,
            SnapshotStatus::Persisting,
            SnapshotStatus::Ready,
            None
        )
        .await
        .unwrap());
    meta.id
}

#[tokio::test]
async fn pg_conformance_suite() {
    let db = TestDb::new().await;
    let report = run_all(&PgHarness {
        pool: db.pool.clone(),
    })
    .await;
    assert_eq!(report.adapter, "pg");
    report.assert_ok();
    db.finish().await;
}

#[tokio::test]
async fn duplicate_ready_fingerprint_maps_to_conflict() {
    let db = TestDb::new().await;
    let f = fixture(&db).await;
    let g = with_file_versions(&f, &fixture_graph()).await.unwrap();
    ready_full(&f, &g, FINGERPRINT).await;

    let second = f
        .store
        .create_snapshot(NewSnapshot::full(
            f.scope,
            sha('c').unwrap(),
            SnapshotPurpose::DefaultBranch,
            FINGERPRINT,
        ))
        .await
        .unwrap();
    to_persisting(&f, second.id).await;
    f.store.write_full(&f.scope, second.id, &g).await.unwrap();
    match f
        .store
        .transition(
            &f.scope,
            second.id,
            SnapshotStatus::Persisting,
            SnapshotStatus::Ready,
            None,
        )
        .await
    {
        Err(StoreError::Conflict(message)) => assert!(message.contains("fingerprint")),
        other => panic!("expected a conflict, got {other:?}"),
    }
    db.finish().await;
}

#[tokio::test]
async fn write_full_rolls_back_on_error() {
    let db = TestDb::new().await;
    let f = fixture(&db).await;
    let meta = f
        .store
        .create_snapshot(NewSnapshot::full(
            f.scope,
            sha('a').unwrap(),
            SnapshotPurpose::DefaultBranch,
            FINGERPRINT,
        ))
        .await
        .unwrap();
    to_persisting(&f, meta.id).await;
    // The graph's unresolved row names a file with no file version: the write fails after the
    // file and edge rows were inserted, and none of them may survive.
    let mut g = fixture_graph();
    g.files.push(file("src/z.ts", FileChange::Present));
    g.unresolved
        .push(graph_storage::conformance::unresolved("src/z.ts", 0, "x"));
    let g = with_file_versions(&f, &g).await.unwrap();
    let mut broken = g.clone();
    for listed in &mut broken.files {
        if listed.path == "src/z.ts" {
            listed.file_version_id = None;
        }
    }
    assert!(f
        .store
        .write_full(&f.scope, meta.id, &broken)
        .await
        .is_err());

    let files: i64 =
        sqlx::query_scalar("SELECT count(*) FROM snapshot_files WHERE snapshot_id = $1")
            .bind(*meta.id.as_uuid())
            .fetch_one(&db.pool)
            .await
            .unwrap();
    let edges: i64 = sqlx::query_scalar("SELECT count(*) FROM graph_edges WHERE snapshot_id = $1")
        .bind(*meta.id.as_uuid())
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!((files, edges), (0, 0), "a failed write left rows behind");
    let still = f.store.snapshot(&f.scope, meta.id).await.unwrap().unwrap();
    assert_eq!(still.status, SnapshotStatus::Persisting);
    db.finish().await;
}

#[tokio::test]
async fn file_version_row_implies_complete_symbols() {
    let db = TestDb::new().await;
    let f = fixture(&db).await;
    let g = with_file_versions(&f, &fixture_graph()).await.unwrap();
    let mismatched: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT f.id, f.symbol_count::int8
           FROM file_versions f
          WHERE f.symbol_count <> (SELECT count(*) FROM symbols s WHERE s.file_version_id = f.id)",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert!(mismatched.is_empty(), "{mismatched:?}");
    assert!(g.files.iter().all(|file| file.file_version_id.is_some()));
    db.finish().await;
}

#[tokio::test]
async fn concurrent_upsert_same_file_version_returns_same_id() {
    let db = TestDb::new().await;
    let f = fixture(&db).await;
    let inputs = graph_storage::conformance::file_version_inputs(&fixture_graph());
    let calls = (0..8).map(|_| f.store.upsert_file_versions(&f.scope, &inputs));
    let results = futures::future::join_all(calls).await;
    let first: Vec<i64> = results[0].as_ref().unwrap().iter().map(|r| r.id).collect();
    for result in &results {
        let ids: Vec<i64> = result.as_ref().unwrap().iter().map(|r| r.id).collect();
        assert_eq!(ids, first);
    }
    db.finish().await;
}

#[tokio::test]
async fn load_chain_of_20_deltas_equals_flattened_overlays() {
    let db = TestDb::new().await;
    let f = fixture(&db).await;
    let base_g = with_file_versions(&f, &fixture_graph()).await.unwrap();
    let mut head = ready_full(&f, &base_g, FINGERPRINT).await;
    let mut deltas = Vec::new();
    for n in 0..20u8 {
        let mut d = GraphDelta::default();
        if n % 3 == 0 {
            d.edges_removed.push(edge(1, EdgeKind::Calls, 3, 900));
        } else {
            d.edges_added
                .push(edge(1, EdgeKind::Calls, 3, 500 + u16::from(n) * 10));
        }
        d.edges_added
            .push(edge(2, EdgeKind::Calls, 4, 300 + u16::from(n)));
        head = ready_delta(&f, head, &d, n).await;
        deltas.push(d);
    }
    let loaded = f.store.load_graph(&f.scope, head).await.unwrap();
    assert_eq!(loaded, flatten(&base_g, &deltas));
    let chain = f.store.chain(&f.scope, head).await.unwrap();
    assert_eq!(chain.len(), 21);
    db.finish().await;
}

#[tokio::test]
async fn neighbors_returns_latest_version_in_chain() {
    let db = TestDb::new().await;
    let f = fixture(&db).await;
    let base_g = with_file_versions(&f, &fixture_graph()).await.unwrap();
    let base = ready_full(&f, &base_g, FINGERPRINT).await;
    let d = GraphDelta {
        edges_added: vec![edge(1, EdgeKind::Calls, 3, 1000)],
        ..GraphDelta::default()
    };
    let head = ready_delta(&f, base, &d, 1).await;
    let page = f
        .store
        .neighbors(
            &f.scope,
            head,
            key(1),
            Direction::Out,
            EdgeKindSet::of(EdgeKind::Calls),
            Confidence::MIN,
            100,
            None,
        )
        .await
        .unwrap();
    let to_three = page
        .edges
        .iter()
        .find(|e| e.target == key(3))
        .expect("the edge is still there");
    assert_eq!(to_three.confidence, Confidence::from_permille(1000));
    db.finish().await;
}

#[tokio::test]
async fn neighbors_hides_tombstoned_edges() {
    let db = TestDb::new().await;
    let f = fixture(&db).await;
    let base_g = with_file_versions(&f, &fixture_graph()).await.unwrap();
    let base = ready_full(&f, &base_g, FINGERPRINT).await;
    let d = GraphDelta {
        edges_removed: vec![edge(1, EdgeKind::Calls, 3, 900)],
        ..GraphDelta::default()
    };
    let head = ready_delta(&f, base, &d, 1).await;
    let page = f
        .store
        .neighbors(
            &f.scope,
            head,
            key(1),
            Direction::Out,
            EdgeKindSet::ALL,
            Confidence::MIN,
            100,
            None,
        )
        .await
        .unwrap();
    assert!(!page
        .edges
        .iter()
        .any(|e| e.kind == EdgeKind::Calls && e.target == key(3)));
    let incoming = f
        .store
        .neighbors(
            &f.scope,
            head,
            key(3),
            Direction::In,
            EdgeKindSet::of(EdgeKind::Calls),
            Confidence::MIN,
            100,
            None,
        )
        .await
        .unwrap();
    assert!(!incoming.edges.iter().any(|e| e.source == key(1)));
    db.finish().await;
}

#[tokio::test]
async fn neighbors_pagination_is_stable() {
    let db = TestDb::new().await;
    let f = fixture(&db).await;
    let base_g = with_file_versions(&f, &fixture_graph()).await.unwrap();
    let base = ready_full(&f, &base_g, FINGERPRINT).await;
    let mut seen = Vec::new();
    let mut cursor: Option<EdgeCursor> = None;
    for _ in 0..10 {
        let page = f
            .store
            .neighbors(
                &f.scope,
                base,
                key(1),
                Direction::Out,
                EdgeKindSet::ALL,
                Confidence::MIN,
                1,
                cursor,
            )
            .await
            .unwrap();
        seen.extend(page.edges.iter().map(|e| (e.kind, e.target)));
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    let all = base_g.neighbors(
        key(1),
        Direction::Out,
        EdgeKindSet::ALL,
        Confidence::MIN,
        1000,
        None,
    );
    let expected: Vec<_> = all.iter().map(|e| (e.kind, e.target)).collect();
    assert_eq!(seen, expected);
    db.finish().await;
}

#[tokio::test]
async fn load_rejects_schema_mismatch() {
    let db = TestDb::new().await;
    let f = fixture(&db).await;
    let base_g = with_file_versions(&f, &fixture_graph()).await.unwrap();
    let id = ready_full(&f, &base_g, FINGERPRINT).await;
    sqlx::query("UPDATE snapshots SET graph_schema_version = 999 WHERE id = $1")
        .bind(*id.as_uuid())
        .execute(&db.pool)
        .await
        .unwrap();
    match f.store.load_graph(&f.scope, id).await {
        Err(StoreError::SchemaMismatch { found: 999, .. }) => {}
        other => panic!("expected SchemaMismatch, got {other:?}"),
    }
    db.finish().await;
}

#[tokio::test]
async fn load_with_broken_chain_errors() {
    let db = TestDb::new().await;
    let f = fixture(&db).await;
    let base_g = with_file_versions(&f, &fixture_graph()).await.unwrap();
    let base = ready_full(&f, &base_g, FINGERPRINT).await;
    let head = ready_delta(&f, base, &GraphDelta::default(), 1).await;
    sqlx::query("UPDATE snapshots SET status = 'inconsistent' WHERE id = $1")
        .bind(*base.as_uuid())
        .execute(&db.pool)
        .await
        .unwrap();
    match f.store.load_graph(&f.scope, head).await {
        Err(StoreError::InvalidStatus { id, .. }) => assert_eq!(id, base),
        Err(StoreError::ChainBroken { .. }) => {}
        other => panic!("expected a broken chain, got {other:?}"),
    }
    db.finish().await;
}

#[tokio::test]
async fn delta_symbols_follow_their_file_versions() {
    let db = TestDb::new().await;
    let f = fixture(&db).await;
    let base_g = with_file_versions(&f, &fixture_graph()).await.unwrap();
    let base = ready_full(&f, &base_g, OTHER_FINGERPRINT).await;
    let listing = Graph {
        files: vec![file("src/n.ts", FileChange::Added)],
        nodes: vec![symbol(9, "src/n.ts", "fresh")],
        ..Graph::new(base_g.schema_version)
    };
    let listing = with_file_versions(&f, &listing).await.unwrap();
    let d = GraphDelta {
        files: listing.files.clone(),
        nodes_added: listing.nodes.clone(),
        ..GraphDelta::default()
    };
    let head = ready_delta(&f, base, &d, 2).await;
    let nodes = f
        .store
        .nodes(&f.scope, head, &[key(9), key(42)])
        .await
        .unwrap();
    assert!(nodes[0].is_some());
    assert!(nodes[1].is_none());
    db.finish().await;
}

#[test]
fn no_format_built_sql_in_pg() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/pg");
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let text = std::fs::read_to_string(&path).unwrap();
        for needle in [
            "query(&format!",
            "query(format!",
            "query_as(&format!",
            "query_scalar(&format!",
        ] {
            assert!(
                !text.contains(needle),
                "{} builds SQL with format!",
                path.display()
            );
        }
    }
}
