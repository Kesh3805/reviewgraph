//! GS-002/GS-003 migration tests. Run with:
//!
//!   TEST_DATABASE_URL=postgres://... cargo test -p graph-storage --features integration
//!
//! Each test creates a throwaway database so the suite is parallel-safe; the connecting role
//! must be allowed to CREATE DATABASE.

#![cfg(feature = "integration")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::str::FromStr;

use graph_storage::kinds::{EdgeKind, NodeKind, Provenance, ResolvedBy};
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
            .max_connections(8)
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

fn tag() -> String {
    format!("org-{}", &Uuid::new_v4().simple().to_string()[..12])
}

/// Lowercase hex, for bytea literals in EXPLAIN statements.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `(organization_id, repository_id)` of a fresh tenant.
async fn seed_tenants(pool: &PgPool, tag: &str) -> (Uuid, Uuid) {
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
    (org, repo)
}

async fn seed_file_version(pool: &PgPool, org: Uuid, repo: Uuid, path: &str) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO file_versions
           (organization_id, repository_id, path, content_hash, language,
            analyzer_version, parse_status, size_bytes)
         VALUES ($1, $2, $3, $4, 'typescript', '0.1.0', 'ok', 128)
         RETURNING id",
    )
    .bind(org)
    .bind(repo)
    .bind(path)
    .bind(&[9u8; 32][..])
    .fetch_one(pool)
    .await
    .unwrap()
}

#[derive(Clone, Copy)]
struct SnapshotSpec {
    kind: &'static str,
    base: Option<Uuid>,
    depth: i16,
    status: &'static str,
    fingerprint: &'static [u8],
}

async fn seed_snapshot(
    pool: &PgPool,
    org: Uuid,
    repo: Uuid,
    spec: SnapshotSpec,
) -> Result<Uuid, sqlx::Error> {
    sqlx::query_scalar(
        "INSERT INTO snapshots
           (id, organization_id, repository_id, commit_sha, kind, base_snapshot_id,
            chain_depth, purpose, status, graph_schema_version, analyzer_versions,
            config_hash, fingerprint)
         VALUES ($1, $2, $3, 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', $4, $5,
                 $6, 'default_branch', $7, 1, '{\"typescript\":\"0.1.0\"}'::jsonb,
                 $8, $8)
         RETURNING id",
    )
    .bind(Uuid::new_v4())
    .bind(org)
    .bind(repo)
    .bind(spec.kind)
    .bind(spec.base)
    .bind(spec.depth)
    .bind(spec.status)
    .bind(spec.fingerprint)
    .fetch_one(pool)
    .await
}

async fn ready_full(pool: &PgPool, org: Uuid, repo: Uuid, fingerprint: &'static [u8]) -> Uuid {
    seed_snapshot(
        pool,
        org,
        repo,
        SnapshotSpec {
            kind: "full",
            base: None,
            depth: 0,
            status: "ready",
            fingerprint,
        },
    )
    .await
    .unwrap()
}

fn db_code(error: &sqlx::Error) -> Option<String> {
    match error {
        sqlx::Error::Database(db) => db.code().map(|code| code.to_string()),
        _ => None,
    }
}

async fn assert_code<T: std::fmt::Debug>(result: Result<T, sqlx::Error>, expected: &str) {
    match result {
        Ok(value) => panic!("expected sqlstate {expected}, statement succeeded: {value:?}"),
        Err(error) => assert_eq!(db_code(&error).as_deref(), Some(expected), "error: {error}"),
    }
}

/// Count rows visible to `organization_id` under `app.organization_id` as `rg_engine` (RLS on).
async fn count_as(db: &TestDb, organization_id: Uuid, table: &str) -> i64 {
    let mut tx = db.pool.begin().await.unwrap();
    sqlx::query("SET LOCAL ROLE rg_engine")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("SELECT set_config('app.organization_id', $1, true)")
        .bind(organization_id.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    let sql = format!("SELECT count(*) FROM {table}");
    let count: i64 = sqlx::query_scalar(&sql).fetch_one(&mut *tx).await.unwrap();
    tx.rollback().await.unwrap();
    count
}

/// The plan of `sql` as literal text (EXPLAIN picks a specific plan for literals).
async fn explain(db: &TestDb, sql: &str) -> String {
    let rows: Vec<String> = sqlx::query_scalar(sql).fetch_all(&db.pool).await.unwrap();
    rows.join("\n")
}

// ---------------------------------------------------------------- GS-002

#[tokio::test]
async fn migrations_apply_on_empty_db() {
    let db = TestDb::new(false).await;
    migrate(&db.admin, &db.pool).await;
    migrate(&db.admin, &db.pool).await;

    let tenant_tables = [
        "file_versions",
        "symbols",
        "unresolved_refs",
        "snapshots",
        "snapshot_files",
        "graph_edges",
        "synthetic_nodes",
        "symbol_lineage",
    ];
    for table in tenant_tables {
        let (enabled, forced): (bool, bool) = sqlx::query_as(
            "SELECT relrowsecurity, relforcerowsecurity
               FROM pg_class WHERE relname = $1",
        )
        .bind(table)
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert!(enabled && forced, "{table} must enable and force RLS");
        let policy: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_policies
              WHERE schemaname = 'public' AND tablename = $1 AND policyname = 'tenant_isolation'",
        )
        .bind(table)
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert_eq!(policy, 1, "{table} must have the tenant_isolation policy");
    }

    let counts = [
        ("node_kinds", 44),
        ("edge_kinds", 33),
        ("resolved_by_kinds", 10),
        ("provenance_kinds", 6),
    ];
    for (table, expected) in counts {
        let n: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
            .fetch_one(&db.pool)
            .await
            .unwrap();
        assert_eq!(n, expected, "{table} seed size");
    }
    db.finish().await;
}

#[tokio::test]
async fn node_kinds_seed_matches_rust_enum() {
    let db = TestDb::new(true).await;
    let rows: Vec<(i16, String)> = sqlx::query_as("SELECT id, name FROM node_kinds ORDER BY id")
        .fetch_all(&db.pool)
        .await
        .unwrap();
    let expected: Vec<(i16, String)> = NodeKind::ALL
        .iter()
        .map(|kind| (kind.as_i16(), kind.as_str().to_string()))
        .collect();
    assert_eq!(
        rows, expected,
        "node_kinds must mirror codegraph::ALL_NODE_KINDS"
    );
    db.finish().await;
}

#[tokio::test]
async fn file_version_unique_key_enforced() {
    let db = TestDb::new(true).await;
    let (org, repo) = seed_tenants(&db.pool, &tag()).await;
    let first = seed_file_version(&db.pool, org, repo, "src/app.ts").await;
    let second = seed_file_version(&db.pool, org, repo, "src/other.ts").await;
    assert_ne!(first, second);

    let duplicate = sqlx::query(
        "INSERT INTO file_versions
           (organization_id, repository_id, path, content_hash, language,
            analyzer_version, parse_status, size_bytes)
         VALUES ($1, $2, 'src/app.ts', $3, 'typescript', '0.1.0', 'ok', 128)",
    )
    .bind(org)
    .bind(repo)
    .bind(&[9u8; 32][..])
    .execute(&db.pool)
    .await;
    assert_code(duplicate, "23505").await;

    // The same bytes under a different analyzer version are a different row.
    let other_analyzer = sqlx::query(
        "INSERT INTO file_versions
           (organization_id, repository_id, path, content_hash, language,
            analyzer_version, parse_status, size_bytes)
         VALUES ($1, $2, 'src/app.ts', $3, 'typescript', '0.2.0', 'ok', 128)",
    )
    .bind(org)
    .bind(repo)
    .bind(&[9u8; 32][..])
    .execute(&db.pool)
    .await;
    assert!(other_analyzer.is_ok());
    db.finish().await;
}

#[tokio::test]
async fn symbol_key_length_check_enforced() {
    let db = TestDb::new(true).await;
    let (org, repo) = seed_tenants(&db.pool, &tag()).await;
    let file_version_id = seed_file_version(&db.pool, org, repo, "src/app.ts").await;

    let insert = |symbol_key: Vec<u8>, name: &'static str| {
        let pool = db.pool.clone();
        async move {
            sqlx::query(
                "INSERT INTO symbols
                   (file_version_id, organization_id, repository_id, symbol_key, symbol_id,
                    kind, name, qualified_name, start_line, start_col, end_line, end_col)
                 VALUES ($1, $2, $3, $4, $5, 20, $6, $6, 1, 1, 2, 5)",
            )
            .bind(file_version_id)
            .bind(org)
            .bind(repo)
            .bind(symbol_key.as_slice())
            .bind(format!("s-{name}"))
            .bind(name)
            .execute(&pool)
            .await
        }
    };

    assert_code(insert(vec![7u8; 15], "short").await, "23514").await;
    assert_code(insert(vec![7u8; 17], "long").await, "23514").await;
    assert!(insert(vec![7u8; 16], "right").await.is_ok());
    assert_code(insert(vec![7u8; 16], "right").await, "23505").await;
    db.finish().await;
}

#[tokio::test]
async fn path_check_rejects_traversal() {
    let db = TestDb::new(true).await;
    let (org, repo) = seed_tenants(&db.pool, &tag()).await;
    for bad in [
        "/etc/passwd",
        "../secrets.env",
        "..",
        "src/../../etc/passwd",
    ] {
        let result = sqlx::query(
            "INSERT INTO file_versions
               (organization_id, repository_id, path, content_hash, language,
                analyzer_version, parse_status, size_bytes)
             VALUES ($1, $2, $3, $4, 'typescript', '0.1.0', 'ok', 1)",
        )
        .bind(org)
        .bind(repo)
        .bind(bad)
        .bind(&[1u8; 32][..])
        .execute(&db.pool)
        .await;
        assert_code(result, "23514").await;
    }
    let good = sqlx::query(
        "INSERT INTO file_versions
           (organization_id, repository_id, path, content_hash, language,
            analyzer_version, parse_status, size_bytes)
         VALUES ($1, $2, 'src/app.ts', $3, 'typescript', '0.1.0', 'ok', 1)",
    )
    .bind(org)
    .bind(repo)
    .bind(&[1u8; 32][..])
    .execute(&db.pool)
    .await;
    assert!(good.is_ok());
    db.finish().await;
}

#[tokio::test]
async fn rls_hides_other_org_rows() {
    let db = TestDb::new(true).await;
    let (org_a, repo_a) = seed_tenants(&db.pool, &tag()).await;
    let (org_b, _repo_b) = seed_tenants(&db.pool, &tag()).await;
    let file_version_id = seed_file_version(&db.pool, org_a, repo_a, "src/app.ts").await;
    sqlx::query(
        "INSERT INTO symbols
           (file_version_id, organization_id, repository_id, symbol_key, symbol_id,
            kind, name, qualified_name, start_line, start_col, end_line, end_col)
         VALUES ($1, $2, $3, $4, 's-a', 20, 'a', 'a', 1, 1, 2, 5)",
    )
    .bind(file_version_id)
    .bind(org_a)
    .bind(repo_a)
    .bind(&[4u8; 16][..])
    .execute(&db.pool)
    .await
    .unwrap();

    assert_eq!(count_as(&db, org_a, "file_versions").await, 1);
    assert_eq!(count_as(&db, org_a, "symbols").await, 1);
    assert_eq!(count_as(&db, org_b, "file_versions").await, 0);
    assert_eq!(count_as(&db, org_b, "symbols").await, 0);
    assert_eq!(count_as(&db, Uuid::new_v4(), "file_versions").await, 0);
    db.finish().await;
}

#[tokio::test]
async fn explain_symbols_repo_key_uses_index() {
    let db = TestDb::new(true).await;
    let (org, repo) = seed_tenants(&db.pool, &tag()).await;
    let file_version_id = seed_file_version(&db.pool, org, repo, "src/app.ts").await;
    for ordinal in 0..500i64 {
        let mut symbol_key = [0u8; 16];
        symbol_key[..8].copy_from_slice(&ordinal.to_le_bytes());
        sqlx::query(
            "INSERT INTO symbols
               (file_version_id, organization_id, repository_id, symbol_key, symbol_id,
                kind, name, qualified_name, start_line, start_col, end_line, end_col)
             VALUES ($1, $2, $3, $4, $5, 20, $6, $6, 1, 1, 2, 5)",
        )
        .bind(file_version_id)
        .bind(org)
        .bind(repo)
        .bind(&symbol_key[..])
        .bind(format!("s-{ordinal}"))
        .bind(format!("fn_{ordinal}"))
        .execute(&db.pool)
        .await
        .unwrap();
    }
    sqlx::query("ANALYZE symbols")
        .execute(&db.pool)
        .await
        .unwrap();

    // The plain lookup must be an index scan.
    let lookup = format!(
        "EXPLAIN SELECT symbol_id FROM symbols
           WHERE repository_id = '{org}' AND symbol_key = '\\x{}'::bytea",
        hex(&[5u8; 16])
    );
    let plan = explain(&db, &lookup).await;
    assert!(
        plan.contains("Index Scan"),
        "expected an index scan, got:\n{plan}"
    );

    // Isolate the index under test: `symbols_repo_name` has the same leading column and the
    // planner treats both as equal-cost paths (tie broken arbitrarily), so drop it inside a
    // transaction that is rolled back and assert that `symbols_repo_key` serves the lookup.
    let mut tx = db.pool.begin().await.unwrap();
    sqlx::query("DROP INDEX symbols_repo_name")
        .execute(&mut *tx)
        .await
        .unwrap();
    let rows: Vec<String> = sqlx::query_scalar(&lookup)
        .fetch_all(&mut *tx)
        .await
        .unwrap();
    tx.rollback().await.unwrap();
    let isolated = rows.join("\n");
    assert!(
        isolated.contains("symbols_repo_key"),
        "expected an index scan on symbols_repo_key, got:\n{isolated}"
    );
    db.finish().await;
}

// ---------------------------------------------------------------- GS-003

#[tokio::test]
async fn edge_kinds_seed_matches_rust_enum() {
    let db = TestDb::new(true).await;
    let rows: Vec<(i16, String)> = sqlx::query_as("SELECT id, name FROM edge_kinds ORDER BY id")
        .fetch_all(&db.pool)
        .await
        .unwrap();
    let expected: Vec<(i16, String)> = EdgeKind::ALL
        .iter()
        .map(|kind| (kind.as_i16(), kind.as_str().to_string()))
        .collect();
    assert_eq!(rows, expected);
    db.finish().await;
}

#[tokio::test]
async fn resolved_by_seed_matches_rust_enum() {
    let db = TestDb::new(true).await;
    let rows: Vec<(i16, String)> =
        sqlx::query_as("SELECT id, name FROM resolved_by_kinds ORDER BY id")
            .fetch_all(&db.pool)
            .await
            .unwrap();
    let expected: Vec<(i16, String)> = ResolvedBy::ALL
        .iter()
        .map(|kind| (kind.as_i16(), kind.as_str().to_string()))
        .collect();
    assert_eq!(rows, expected);
    db.finish().await;
}

#[tokio::test]
async fn provenance_seed_matches_rust_enum() {
    let db = TestDb::new(true).await;
    let rows: Vec<(i16, String)> =
        sqlx::query_as("SELECT id, name FROM provenance_kinds ORDER BY id")
            .fetch_all(&db.pool)
            .await
            .unwrap();
    let expected: Vec<(i16, String)> = Provenance::ALL
        .iter()
        .map(|kind| (kind.as_i16(), kind.as_str().to_string()))
        .collect();
    assert_eq!(rows, expected);
    db.finish().await;
}

#[tokio::test]
async fn full_snapshot_must_not_have_base() {
    let db = TestDb::new(true).await;
    let (org, repo) = seed_tenants(&db.pool, &tag()).await;
    let base = ready_full(&db.pool, org, repo, &[1u8; 32]).await;

    let with_base = seed_snapshot(
        &db.pool,
        org,
        repo,
        SnapshotSpec {
            kind: "full",
            base: Some(base),
            depth: 0,
            status: "pending",
            fingerprint: &[2u8; 32],
        },
    )
    .await;
    assert_code(with_base.map(|_| 0u8), "23514").await;

    let full_with_depth = seed_snapshot(
        &db.pool,
        org,
        repo,
        SnapshotSpec {
            kind: "full",
            base: None,
            depth: 1,
            status: "pending",
            fingerprint: &[3u8; 32],
        },
    )
    .await;
    assert_code(full_with_depth.map(|_| 0u8), "23514").await;

    let ok = seed_snapshot(
        &db.pool,
        org,
        repo,
        SnapshotSpec {
            kind: "full",
            base: None,
            depth: 0,
            status: "pending",
            fingerprint: &[4u8; 32],
        },
    )
    .await;
    assert!(ok.is_ok());
    db.finish().await;
}

#[tokio::test]
async fn delta_must_have_base_and_depth() {
    let db = TestDb::new(true).await;
    let (org, repo) = seed_tenants(&db.pool, &tag()).await;
    let base = ready_full(&db.pool, org, repo, &[10u8; 32]).await;

    let without_base = seed_snapshot(
        &db.pool,
        org,
        repo,
        SnapshotSpec {
            kind: "delta",
            base: None,
            depth: 1,
            status: "pending",
            fingerprint: &[11u8; 32],
        },
    )
    .await;
    assert_code(without_base.map(|_| 0u8), "23514").await;

    let zero_depth = seed_snapshot(
        &db.pool,
        org,
        repo,
        SnapshotSpec {
            kind: "delta",
            base: Some(base),
            depth: 0,
            status: "pending",
            fingerprint: &[12u8; 32],
        },
    )
    .await;
    assert_code(zero_depth.map(|_| 0u8), "23514").await;

    let ok = seed_snapshot(
        &db.pool,
        org,
        repo,
        SnapshotSpec {
            kind: "delta",
            base: Some(base),
            depth: 1,
            status: "pending",
            fingerprint: &[13u8; 32],
        },
    )
    .await;
    assert!(ok.is_ok());
    db.finish().await;
}

#[tokio::test]
async fn ready_fingerprint_unique_for_full() {
    let db = TestDb::new(true).await;
    let (org, repo) = seed_tenants(&db.pool, &tag()).await;
    let _first = ready_full(&db.pool, org, repo, &[20u8; 32]).await;

    let duplicate = seed_snapshot(
        &db.pool,
        org,
        repo,
        SnapshotSpec {
            kind: "full",
            base: None,
            depth: 0,
            status: "ready",
            fingerprint: &[20u8; 32],
        },
    )
    .await;
    assert_code(duplicate.map(|_| 0u8), "23505").await;

    // Before the snapshot is ready the fingerprint may repeat: two workers can still race.
    let pending = seed_snapshot(
        &db.pool,
        org,
        repo,
        SnapshotSpec {
            kind: "full",
            base: None,
            depth: 0,
            status: "pending",
            fingerprint: &[20u8; 32],
        },
    )
    .await;
    assert!(pending.is_ok());

    let _other = ready_full(&db.pool, org, repo, &[21u8; 32]).await;
    db.finish().await;
}

#[tokio::test]
async fn ready_delta_unique_per_base() {
    let db = TestDb::new(true).await;
    let (org, repo) = seed_tenants(&db.pool, &tag()).await;
    let base_a = ready_full(&db.pool, org, repo, &[30u8; 32]).await;
    let base_b = ready_full(&db.pool, org, repo, &[31u8; 32]).await;

    let delta_on = |fingerprint: &'static [u8], base: Uuid| {
        let pool = db.pool.clone();
        async move {
            seed_snapshot(
                &pool,
                org,
                repo,
                SnapshotSpec {
                    kind: "delta",
                    base: Some(base),
                    depth: 1,
                    status: "ready",
                    fingerprint,
                },
            )
            .await
        }
    };
    assert!(delta_on(&[32u8; 32], base_a).await.is_ok());
    assert_code(delta_on(&[32u8; 32], base_a).await.map(|_| 0u8), "23505").await;
    // The same fingerprint against another base is a different snapshot.
    assert!(delta_on(&[32u8; 32], base_b).await.is_ok());
    db.finish().await;
}

#[tokio::test]
async fn edge_pk_rejects_duplicate_identity() {
    let db = TestDb::new(true).await;
    let (org, repo) = seed_tenants(&db.pool, &tag()).await;
    let snapshot_id = ready_full(&db.pool, org, repo, &[40u8; 32]).await;

    let insert = |source: &[u8; 16], removed: bool| {
        let pool = db.pool.clone();
        let source = *source;
        async move {
            sqlx::query(
                "INSERT INTO graph_edges
                   (snapshot_id, organization_id, source_key, kind, target_key, confidence,
                    resolved_by, provenance, removed)
                 VALUES ($1, $2, $3, 4, $4, 0.9, 0, 0, $5)",
            )
            .bind(snapshot_id)
            .bind(org)
            .bind(&source[..])
            .bind(&[6u8; 16][..])
            .bind(removed)
            .execute(&pool)
            .await
        }
    };

    assert!(insert(&[8u8; 16], false).await.is_ok());
    // The same (source, kind, target) is the same identity, even as a tombstone.
    assert_code(insert(&[8u8; 16], false).await, "23505").await;
    assert_code(insert(&[8u8; 16], true).await, "23505").await;
    assert!(insert(&[9u8; 16], false).await.is_ok());
    db.finish().await;
}

#[tokio::test]
async fn explain_neighbors_uses_pk_and_target_index() {
    let db = TestDb::new(true).await;
    let (org, repo) = seed_tenants(&db.pool, &tag()).await;
    let snapshot_id = ready_full(&db.pool, org, repo, &[50u8; 32]).await;
    let mut target_of_last = [0u8; 16];
    for ordinal in 0..600i64 {
        let mut source = [0u8; 16];
        source[..8].copy_from_slice(&ordinal.to_le_bytes());
        let mut target = [1u8; 16];
        target[8..].copy_from_slice(&ordinal.to_le_bytes());
        target_of_last = target;
        sqlx::query(
            "INSERT INTO graph_edges
               (snapshot_id, organization_id, source_key, kind, target_key, confidence,
                resolved_by, provenance)
             VALUES ($1, $2, $3, 4, $4, 0.5, 0, 0)",
        )
        .bind(snapshot_id)
        .bind(org)
        .bind(&source[..])
        .bind(&target[..])
        .execute(&db.pool)
        .await
        .unwrap();
    }
    sqlx::query("ANALYZE graph_edges")
        .execute(&db.pool)
        .await
        .unwrap();

    let mut source_key = [0u8; 16];
    source_key[..8].copy_from_slice(&599i64.to_le_bytes());
    let out_sql = format!(
        "EXPLAIN SELECT target_key FROM graph_edges
           WHERE snapshot_id = '{snapshot_id}' AND source_key = '\\x{}'::bytea
             AND kind = 4 AND target_key = '\\x{}'::bytea",
        hex(&source_key),
        hex(&target_of_last)
    );
    let in_sql = format!(
        "EXPLAIN SELECT source_key FROM graph_edges
           WHERE snapshot_id = '{snapshot_id}' AND target_key = '\\x{}'::bytea AND kind = 4",
        hex(&target_of_last)
    );

    // Both directions must be index scans.
    for sql in [&out_sql, &in_sql] {
        let plan = explain(&db, sql).await;
        assert!(
            plan.contains("Index Scan") || plan.contains("Index Only Scan"),
            "expected an index scan, got:\n{plan}"
        );
    }

    // `graph_edges_pkey` and `graph_edges_target` are equal-cost paths for this data and the
    // planner breaks the tie arbitrarily, so each index is isolated in its own transaction
    // (rolled back) to prove it serves its own direction.
    let mut tx = db.pool.begin().await.unwrap();
    sqlx::query("DROP INDEX graph_edges_target")
        .execute(&mut *tx)
        .await
        .unwrap();
    let out_rows: Vec<String> = sqlx::query_scalar(&out_sql)
        .fetch_all(&mut *tx)
        .await
        .unwrap();
    tx.rollback().await.unwrap();
    let out_plan = out_rows.join("\n");
    assert!(
        out_plan.contains("graph_edges_pkey"),
        "expected the primary key index, got:\n{out_plan}"
    );

    let mut tx = db.pool.begin().await.unwrap();
    sqlx::query("ALTER TABLE graph_edges DROP CONSTRAINT graph_edges_pkey")
        .execute(&mut *tx)
        .await
        .unwrap();
    let in_rows: Vec<String> = sqlx::query_scalar(&in_sql)
        .fetch_all(&mut *tx)
        .await
        .unwrap();
    tx.rollback().await.unwrap();
    let in_plan = in_rows.join("\n");
    assert!(
        in_plan.contains("graph_edges_target"),
        "expected graph_edges_target, got:\n{in_plan}"
    );
    db.finish().await;
}

#[tokio::test]
async fn rls_hides_other_org_snapshots() {
    let db = TestDb::new(true).await;
    let (org_a, repo_a) = seed_tenants(&db.pool, &tag()).await;
    let (org_b, _repo_b) = seed_tenants(&db.pool, &tag()).await;
    let snapshot_id = ready_full(&db.pool, org_a, repo_a, &[60u8; 32]).await;
    sqlx::query(
        "INSERT INTO graph_edges
           (snapshot_id, organization_id, source_key, kind, target_key, confidence,
            resolved_by, provenance)
         VALUES ($1, $2, $3, 4, $4, 0.5, 0, 0)",
    )
    .bind(snapshot_id)
    .bind(org_a)
    .bind(&[7u8; 16][..])
    .bind(&[8u8; 16][..])
    .execute(&db.pool)
    .await
    .unwrap();

    assert_eq!(count_as(&db, org_a, "snapshots").await, 1);
    assert_eq!(count_as(&db, org_a, "graph_edges").await, 1);
    assert_eq!(count_as(&db, org_b, "snapshots").await, 0);
    assert_eq!(count_as(&db, org_b, "graph_edges").await, 0);
    db.finish().await;
}

#[tokio::test]
async fn cascade_delete_snapshot_removes_children() {
    let db = TestDb::new(true).await;
    let (org, repo) = seed_tenants(&db.pool, &tag()).await;
    let doomed = ready_full(&db.pool, org, repo, &[70u8; 32]).await;
    let survivor = ready_full(&db.pool, org, repo, &[71u8; 32]).await;
    let file_version_id = seed_file_version(&db.pool, org, repo, "src/app.ts").await;

    for snapshot_id in [doomed, survivor] {
        sqlx::query(
            "INSERT INTO snapshot_files
               (snapshot_id, organization_id, path, file_version_id, change)
             VALUES ($1, $2, 'src/app.ts', $3, 'present')",
        )
        .bind(snapshot_id)
        .bind(org)
        .bind(file_version_id)
        .execute(&db.pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO graph_edges
               (snapshot_id, organization_id, source_key, kind, target_key, confidence,
                resolved_by, provenance, file_version_id, origin_path)
             VALUES ($1, $2, $3, 4, $4, 0.5, 0, 0, $5, 'src/app.ts')",
        )
        .bind(snapshot_id)
        .bind(org)
        .bind(&[10u8; 16][..])
        .bind(&[11u8; 16][..])
        .bind(file_version_id)
        .execute(&db.pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO synthetic_nodes
               (snapshot_id, organization_id, node_key, node_id, kind)
             VALUES ($1, $2, $3, 'react:root', 1)",
        )
        .bind(snapshot_id)
        .bind(org)
        .bind(&[12u8; 16][..])
        .execute(&db.pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO unresolved_refs
               (snapshot_id, organization_id, repository_id, file_version_id, ordinal,
                name, ref_kind, reason, line, col)
             VALUES ($1, $2, $3, $4, 0, 'missing', 1, 0, 3, 8)",
        )
        .bind(snapshot_id)
        .bind(org)
        .bind(repo)
        .bind(file_version_id)
        .execute(&db.pool)
        .await
        .unwrap();
    }

    sqlx::query("DELETE FROM snapshots WHERE id = $1")
        .bind(doomed)
        .execute(&db.pool)
        .await
        .unwrap();

    for table in [
        "snapshot_files",
        "graph_edges",
        "synthetic_nodes",
        "unresolved_refs",
    ] {
        let count = |snapshot_id: Uuid| {
            let pool = db.pool.clone();
            let sql = format!("SELECT count(*) FROM {table} WHERE snapshot_id = $1");
            async move {
                let n: i64 = sqlx::query_scalar(&sql)
                    .bind(snapshot_id)
                    .fetch_one(&pool)
                    .await
                    .unwrap();
                n
            }
        };
        assert_eq!(count(doomed).await, 0, "{table} must cascade");
        assert_eq!(count(survivor).await, 1, "{table} must be kept");
    }
    db.finish().await;
}
