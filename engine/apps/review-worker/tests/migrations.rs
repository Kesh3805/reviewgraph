//! Migration tests against a real Postgres (DOM-009). Run with:
//!
//!   TEST_DATABASE_URL=postgres://... cargo test -p review-worker --features integration
//!
//! Each test creates its own throwaway database, so tests are independent and parallel-safe. The
//! connecting role must be allowed to CREATE DATABASE.

#![cfg(feature = "integration")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::str::FromStr;

use review_core::finding::{FindingState, Severity};
use review_core::ids::ReviewRunId;
use review_core::review::ReviewState;
use review_core::ErrorClass;
use review_worker::migrate::{self, MIGRATOR};
use serde_json::json;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{PgPool, Row};
use uuid::Uuid;

const TABLES: [&str; 13] = [
    "organizations",
    "users",
    "memberships",
    "provider_installations",
    "repositories",
    "pull_requests",
    "review_runs",
    "reviewer_runs",
    "candidate_findings",
    "verified_findings",
    "published_findings",
    "finding_feedback",
    "webhook_deliveries",
];

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
            .max_connections(4)
            .connect_with(options)
            .await
            .unwrap();
        if apply_migrations {
            with_migration_lock(&admin, migrate::run(&pool))
                .await
                .unwrap();
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

/// SQLSTATE of a failed statement.
fn code<T: std::fmt::Debug>(result: Result<T, sqlx::Error>) -> String {
    match result {
        Ok(v) => panic!("expected a database error, got Ok({v:?})"),
        Err(e) => e
            .as_database_error()
            .and_then(|d| d.code().map(|c| c.to_string()))
            .unwrap_or_else(|| format!("non-database error: {e}")),
    }
}

const UNIQUE_VIOLATION: &str = "23505";
const FK_VIOLATION: &str = "23503";
const CHECK_VIOLATION: &str = "23514";

fn sha(c: char) -> String {
    c.to_string().repeat(40)
}

async fn insert_org(pool: &PgPool, slug: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO organizations (slug, display_name) VALUES ($1, $1) RETURNING id",
    )
    .bind(slug)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn insert_installation(pool: &PgPool, org: Uuid, number: i64) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO provider_installations
           (organization_id, provider, provider_installation_id, account_login, account_type)
         VALUES ($1, 'github', $2, 'acme', 'organization') RETURNING id",
    )
    .bind(org)
    .bind(number)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn insert_repo(pool: &PgPool, org: Uuid, installation: Uuid, name: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO repositories
           (organization_id, installation_id, provider, provider_repo_id, full_name,
            default_branch, visibility)
         VALUES ($1, $2, 'github', $3, $3, 'main', 'private') RETURNING id",
    )
    .bind(org)
    .bind(installation)
    .bind(name)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn try_insert_pr(
    pool: &PgPool,
    org: Uuid,
    repo: Uuid,
    number: i32,
) -> Result<Uuid, sqlx::Error> {
    sqlx::query_scalar(
        "INSERT INTO pull_requests
           (organization_id, repository_id, provider_number, title, author_login, base_ref,
            head_ref, base_sha, head_sha, state)
         VALUES ($1, $2, $3, 't', 'octocat', 'main', 'feature', $4, $5, 'open') RETURNING id",
    )
    .bind(org)
    .bind(repo)
    .bind(number)
    .bind(sha('a'))
    .bind(sha('b'))
    .fetch_one(pool)
    .await
}

async fn insert_pr(pool: &PgPool, org: Uuid, repo: Uuid, number: i32) -> Uuid {
    try_insert_pr(pool, org, repo, number).await.unwrap()
}

struct Seed {
    org: Uuid,
    repo: Uuid,
    pr: Uuid,
}

async fn seed(pool: &PgPool, tag: &str) -> Seed {
    let org = insert_org(pool, tag).await;
    let installation = insert_installation(pool, org, 1).await;
    let repo = insert_repo(pool, org, installation, &format!("{tag}/app")).await;
    let pr = insert_pr(pool, org, repo, 1).await;
    Seed { org, repo, pr }
}

#[allow(clippy::too_many_arguments)]
async fn insert_run(
    pool: &PgPool,
    s: &Seed,
    pr: Uuid,
    head: &str,
    state: &str,
    failure_class: Option<&str>,
    superseded_by: Option<Uuid>,
) -> Result<Uuid, sqlx::Error> {
    sqlx::query_scalar(
        "INSERT INTO review_runs
           (organization_id, repository_id, pull_request_id, base_sha, head_sha, state, trigger,
            failure_class, superseded_by)
         VALUES ($1, $2, $3, $4, $5, $6, 'webhook', $7, $8) RETURNING id",
    )
    .bind(s.org)
    .bind(s.repo)
    .bind(pr)
    .bind(sha('a'))
    .bind(head)
    .bind(state)
    .bind(failure_class)
    .bind(superseded_by)
    .fetch_one(pool)
    .await
}

async fn insert_reviewer_run(
    pool: &PgPool,
    org: Uuid,
    run: Uuid,
    cluster_key: Option<&str>,
) -> Result<Uuid, sqlx::Error> {
    sqlx::query_scalar(
        "INSERT INTO reviewer_runs (organization_id, review_run_id, reviewer, cluster_key, state)
         VALUES ($1, $2, 'security', $3, 'pending') RETURNING id",
    )
    .bind(org)
    .bind(run)
    .bind(cluster_key)
    .fetch_one(pool)
    .await
}

#[allow(clippy::too_many_arguments)]
async fn insert_candidate(
    pool: &PgPool,
    org: Uuid,
    run: Uuid,
    reviewer_run: Uuid,
    fingerprint: &str,
    state: &str,
    severity: &str,
    suppression: Option<serde_json::Value>,
) -> Result<Uuid, sqlx::Error> {
    sqlx::query_scalar(
        "INSERT INTO candidate_findings
           (organization_id, review_run_id, reviewer_run_id, reviewer, category, title,
            description, changed_path, changed_side, changed_start_line, changed_end_line,
            severity_candidate, fingerprint, state, suppression)
         VALUES ($1, $2, $3, 'security', 'security', 'title', 'description', 'src/a.ts', 'head',
                 1, 2, $4, $5, $6, $7) RETURNING id",
    )
    .bind(org)
    .bind(run)
    .bind(reviewer_run)
    .bind(severity)
    .bind(fingerprint)
    .bind(state)
    .bind(suppression)
    .fetch_one(pool)
    .await
}

fn fingerprint(n: u32) -> String {
    format!("v1:{n:032x}")
}

/// org -> repo -> PR -> run -> reviewer run, ready for finding tests.
struct Chain {
    seed: Seed,
    run: Uuid,
    reviewer_run: Uuid,
}

async fn chain(pool: &PgPool, tag: &str) -> Chain {
    let seed = seed(pool, tag).await;
    let run = insert_run(pool, &seed, seed.pr, &sha('c'), "REVIEWING", None, None)
        .await
        .unwrap();
    let reviewer_run = insert_reviewer_run(pool, seed.org, run, None)
        .await
        .unwrap();
    Chain {
        seed,
        run,
        reviewer_run,
    }
}

async fn table_names(pool: &PgPool) -> BTreeSet<String> {
    sqlx::query_scalar(
        "SELECT table_name::text FROM information_schema.tables
         WHERE table_schema = 'public' AND table_type = 'BASE TABLE'",
    )
    .fetch_all(pool)
    .await
    .unwrap()
    .into_iter()
    .collect()
}

#[tokio::test]
async fn migrations_apply_on_empty_db() {
    let db = TestDb::new(false).await;
    let applied = with_migration_lock(&db.admin, migrate::run(&db.pool))
        .await
        .unwrap();
    // Every embedded migration applies; later phases add migrations, so the count tracks the
    // migrator rather than a literal.
    assert_eq!(applied.len(), MIGRATOR.migrations.len());
    assert!(
        applied.len() >= 5,
        "the DOM-009 baseline has five migrations"
    );
    // The DOM-009 tables are all present; later phases may add more tables alongside them.
    let mut expected: BTreeSet<String> = TABLES.iter().map(|t| (*t).to_owned()).collect();
    expected.insert("_sqlx_migrations".to_owned());
    let actual = table_names(&db.pool).await;
    let missing: Vec<&String> = expected.difference(&actual).collect();
    assert!(missing.is_empty(), "missing tables: {missing:?}");
    db.finish().await;
}

#[tokio::test]
async fn migrate_twice_is_noop() {
    let db = TestDb::new(true).await;
    let again = with_migration_lock(&db.admin, migrate::run(&db.pool))
        .await
        .unwrap();
    assert!(again.is_empty());
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(count, MIGRATOR.migrations.len() as i64);
    assert!(count >= 5, "the DOM-009 baseline has five migrations");
    db.finish().await;
}

#[tokio::test]
async fn every_tenant_table_has_organization_id() {
    let db = TestDb::new(true).await;
    for table in TABLES
        .iter()
        .filter(|t| !["users", "organizations"].contains(t))
    {
        let present: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM information_schema.columns
               WHERE table_schema = 'public' AND table_name = $1 AND column_name = 'organization_id')",
        )
        .bind(table)
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert!(present, "{table} lacks organization_id");
    }
    db.finish().await;
}

#[tokio::test]
async fn webhook_deliveries_org_nullable_by_design() {
    let db = TestDb::new(true).await;
    let rows = sqlx::query(
        "SELECT table_name::text AS t, is_nullable::text AS n FROM information_schema.columns
         WHERE table_schema = 'public' AND column_name = 'organization_id'",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert!(!rows.is_empty());
    for row in rows {
        let table: String = row.get("t");
        let nullable: String = row.get("n");
        assert_eq!(nullable == "YES", table == "webhook_deliveries", "{table}");
    }
    db.finish().await;
}

#[tokio::test]
async fn every_table_has_updated_at_trigger() {
    let db = TestDb::new(true).await;
    for table in TABLES {
        let triggers: Vec<String> = sqlx::query_scalar(
            "SELECT t.tgname::text FROM pg_trigger t JOIN pg_class c ON c.oid = t.tgrelid
             WHERE c.relname = $1 AND NOT t.tgisinternal",
        )
        .bind(table)
        .fetch_all(&db.pool)
        .await
        .unwrap();
        assert!(
            triggers.contains(&format!("{table}_set_updated_at")),
            "{table}: {triggers:?}"
        );
    }
    db.finish().await;
}

#[tokio::test]
async fn updated_at_trigger_bumps_timestamp() {
    let db = TestDb::new(true).await;
    let org = insert_org(&db.pool, "acme").await;
    let before: chrono_free::Ts =
        sqlx::query_scalar("SELECT updated_at FROM organizations WHERE id = $1")
            .bind(org)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    sqlx::query("SELECT pg_sleep(0.05)")
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE organizations SET display_name = 'Acme Inc' WHERE id = $1")
        .bind(org)
        .execute(&db.pool)
        .await
        .unwrap();
    let after: chrono_free::Ts =
        sqlx::query_scalar("SELECT updated_at FROM organizations WHERE id = $1")
            .bind(org)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert!(after > before, "{after:?} should be later than {before:?}");
    db.finish().await;
}

/// `timestamptz` decoded without importing chrono into the test crate.
mod chrono_free {
    pub type Ts = sqlx::types::chrono::DateTime<sqlx::types::chrono::Utc>;
}

#[tokio::test]
async fn cross_tenant_fk_rejected() {
    let db = TestDb::new(true).await;
    let a = seed(&db.pool, "tenant-a").await;
    let org_b = insert_org(&db.pool, "tenant-b").await;
    // A pull request owned by B that points at A's repository.
    assert_eq!(
        code(try_insert_pr(&db.pool, org_b, a.repo, 99).await),
        FK_VIOLATION
    );
    // A run owned by B that points at A's pull request.
    let b_seed = Seed {
        org: org_b,
        repo: a.repo,
        pr: a.pr,
    };
    assert_eq!(
        code(insert_run(&db.pool, &b_seed, a.pr, &sha('d'), "RECEIVED", None, None).await),
        FK_VIOLATION
    );
    db.finish().await;
}

#[tokio::test]
async fn review_state_check_matches_rust_enum() {
    let db = TestDb::new(true).await;
    let s = seed(&db.pool, "acme").await;
    // A run to point SUPERSEDED rows at.
    let other = insert_run(&db.pool, &s, s.pr, &sha('0'), "COMPLETED", None, None)
        .await
        .unwrap();
    for (i, state) in ReviewState::ALL.iter().enumerate() {
        let pr = insert_pr(&db.pool, s.org, s.repo, 100 + i as i32).await;
        let failure = state.is_failed().then_some("internal");
        let by = (*state == ReviewState::Superseded).then_some(other);
        insert_run(&db.pool, &s, pr, &sha('e'), state.as_str(), failure, by)
            .await
            .unwrap_or_else(|e| panic!("{state} rejected: {e}"));
    }
    let pr = insert_pr(&db.pool, s.org, s.repo, 500).await;
    for bad in ["APPROVED", "completed", "FAILED", ""] {
        assert_eq!(
            code(insert_run(&db.pool, &s, pr, &sha('f'), bad, None, None).await),
            CHECK_VIOLATION,
            "{bad:?}"
        );
    }
    db.finish().await;
}

#[tokio::test]
async fn finding_state_check_matches_rust_enum() {
    let db = TestDb::new(true).await;
    let c = chain(&db.pool, "acme").await;
    for (i, state) in FindingState::ALL.iter().enumerate() {
        let suppression = (state.is_suppressed() || *state == FindingState::Invalidated)
            .then(|| json!({"reason": {"type": "preexisting"}, "detail": "", "stage": 5}));
        insert_candidate(
            &db.pool,
            c.seed.org,
            c.run,
            c.reviewer_run,
            &fingerprint(i as u32),
            state.as_str(),
            "high",
            suppression,
        )
        .await
        .unwrap_or_else(|e| panic!("{state} rejected: {e}"));
    }
    for bad in ["BOGUS", "published", "SUPPRESSED"] {
        assert_eq!(
            code(
                insert_candidate(
                    &db.pool,
                    c.seed.org,
                    c.run,
                    c.reviewer_run,
                    &fingerprint(99),
                    bad,
                    "high",
                    None
                )
                .await
            ),
            CHECK_VIOLATION,
            "{bad}"
        );
    }
    db.finish().await;
}

#[tokio::test]
async fn severity_check_matches_rust_enum() {
    let db = TestDb::new(true).await;
    let c = chain(&db.pool, "acme").await;
    for (i, sev) in Severity::ALL.iter().enumerate() {
        insert_candidate(
            &db.pool,
            c.seed.org,
            c.run,
            c.reviewer_run,
            &fingerprint(i as u32),
            "GENERATED",
            sev.as_str(),
            None,
        )
        .await
        .unwrap_or_else(|e| panic!("{sev} rejected: {e}"));
    }
    // Uppercase and legacy labels are rejected, as in the Rust parser.
    for bad in ["HIGH", "P1", "warning"] {
        assert_eq!(
            code(
                insert_candidate(
                    &db.pool,
                    c.seed.org,
                    c.run,
                    c.reviewer_run,
                    &fingerprint(99),
                    "GENERATED",
                    bad,
                    None
                )
                .await
            ),
            CHECK_VIOLATION,
            "{bad}"
        );
    }
    db.finish().await;
}

#[tokio::test]
async fn error_class_check_matches_rust_enum() {
    let db = TestDb::new(true).await;
    let s = seed(&db.pool, "acme").await;
    for (i, class) in ErrorClass::ALL.iter().enumerate() {
        let pr = insert_pr(&db.pool, s.org, s.repo, 100 + i as i32).await;
        insert_run(
            &db.pool,
            &s,
            pr,
            &sha('e'),
            "FAILED_REVIEW",
            Some(class.as_str()),
            None,
        )
        .await
        .unwrap_or_else(|e| panic!("{class:?} rejected: {e}"));
    }
    let pr = insert_pr(&db.pool, s.org, s.repo, 500).await;
    assert_eq!(
        code(
            insert_run(
                &db.pool,
                &s,
                pr,
                &sha('f'),
                "FAILED_REVIEW",
                Some("Transient"),
                None
            )
            .await
        ),
        CHECK_VIOLATION
    );
    db.finish().await;
}

#[tokio::test]
async fn one_active_run_per_pr_enforced() {
    let db = TestDb::new(true).await;
    let s = seed(&db.pool, "acme").await;
    let first = insert_run(&db.pool, &s, s.pr, &sha('1'), "INDEXING", None, None)
        .await
        .unwrap();
    assert_eq!(
        code(insert_run(&db.pool, &s, s.pr, &sha('2'), "RECEIVED", None, None).await),
        UNIQUE_VIOLATION
    );
    // Another PR is unaffected.
    let other_pr = insert_pr(&db.pool, s.org, s.repo, 2).await;
    insert_run(&db.pool, &s, other_pr, &sha('2'), "RECEIVED", None, None)
        .await
        .unwrap();
    // Once the first run is no longer active, a new head can start.
    sqlx::query("UPDATE review_runs SET state = 'COMPLETED', completed_at = now() WHERE id = $1")
        .bind(first)
        .execute(&db.pool)
        .await
        .unwrap();
    insert_run(&db.pool, &s, s.pr, &sha('2'), "RECEIVED", None, None)
        .await
        .unwrap();
    // A replayed first run for an already-seen head is rejected, a retry is allowed.
    sqlx::query("UPDATE review_runs SET state = 'CANCELLED' WHERE pull_request_id = $1 AND state = 'RECEIVED'")
        .bind(s.pr)
        .execute(&db.pool)
        .await
        .unwrap();
    assert_eq!(
        code(insert_run(&db.pool, &s, s.pr, &sha('1'), "RECEIVED", None, None).await),
        UNIQUE_VIOLATION
    );
    db.finish().await;
}

#[tokio::test]
async fn superseded_requires_superseded_by() {
    let db = TestDb::new(true).await;
    let s = seed(&db.pool, "acme").await;
    let other = insert_run(&db.pool, &s, s.pr, &sha('0'), "COMPLETED", None, None)
        .await
        .unwrap();
    let pr = insert_pr(&db.pool, s.org, s.repo, 2).await;
    assert_eq!(
        code(insert_run(&db.pool, &s, pr, &sha('1'), "SUPERSEDED", None, None).await),
        CHECK_VIOLATION
    );
    assert_eq!(
        code(insert_run(&db.pool, &s, pr, &sha('1'), "CANCELLED", None, Some(other)).await),
        CHECK_VIOLATION
    );
    insert_run(&db.pool, &s, pr, &sha('1'), "SUPERSEDED", None, Some(other))
        .await
        .unwrap();
    db.finish().await;
}

#[tokio::test]
async fn failed_requires_failure_class() {
    let db = TestDb::new(true).await;
    let s = seed(&db.pool, "acme").await;
    assert_eq!(
        code(insert_run(&db.pool, &s, s.pr, &sha('1'), "FAILED_INDEXING", None, None).await),
        CHECK_VIOLATION
    );
    assert_eq!(
        code(
            insert_run(
                &db.pool,
                &s,
                s.pr,
                &sha('1'),
                "INDEXING",
                Some("internal"),
                None
            )
            .await
        ),
        CHECK_VIOLATION
    );
    insert_run(
        &db.pool,
        &s,
        s.pr,
        &sha('1'),
        "FAILED_INDEXING",
        Some("transient"),
        None,
    )
    .await
    .unwrap();
    db.finish().await;
}

#[tokio::test]
async fn suppressed_requires_suppression() {
    let db = TestDb::new(true).await;
    let c = chain(&db.pool, "acme").await;
    let suppression =
        json!({"reason": {"type": "policy", "rule": "cap"}, "detail": "", "stage": null});
    let insert = |n: u32, state: &'static str, sup: Option<serde_json::Value>| {
        let pool = db.pool.clone();
        let (org, run, rr) = (c.seed.org, c.run, c.reviewer_run);
        async move { insert_candidate(&pool, org, run, rr, &fingerprint(n), state, "low", sup).await }
    };
    assert_eq!(
        code(insert(1, "SUPPRESSED_POLICY", None).await),
        CHECK_VIOLATION
    );
    assert_eq!(code(insert(2, "INVALIDATED", None).await), CHECK_VIOLATION);
    assert_eq!(
        code(insert(3, "GENERATED", Some(suppression.clone())).await),
        CHECK_VIOLATION
    );
    insert(4, "SUPPRESSED_POLICY", Some(suppression.clone()))
        .await
        .unwrap();
    insert(5, "INVALIDATED", Some(suppression)).await.unwrap();
    insert(6, "GENERATED", None).await.unwrap();
    db.finish().await;
}

#[tokio::test]
async fn inline_published_requires_location() {
    let db = TestDb::new(true).await;
    let c = chain(&db.pool, "acme").await;
    let candidate = insert_candidate(
        &db.pool,
        c.seed.org,
        c.run,
        c.reviewer_run,
        &fingerprint(1),
        "PRIORITIZED",
        "high",
        None,
    )
    .await
    .unwrap();
    let verified: Uuid = sqlx::query_scalar(
        "INSERT INTO verified_findings
           (organization_id, candidate_finding_id, review_run_id, computed_confidence, severity,
            band, verification_version, stage_outcomes, evidence)
         VALUES ($1, $2, $3, 0.9, 'high', 'publish', 1, '[]', '[]') RETURNING id",
    )
    .bind(c.seed.org)
    .bind(candidate)
    .bind(c.run)
    .fetch_one(&db.pool)
    .await
    .unwrap();

    let publish = |placement: &'static str,
                   path: Option<&'static str>,
                   start: Option<i32>,
                   comment: &'static str| {
        let pool = db.pool.clone();
        let (org, run, pr) = (c.seed.org, c.run, c.seed.pr);
        async move {
            sqlx::query_scalar::<_, Uuid>(
                "INSERT INTO published_findings
                   (organization_id, verified_finding_id, review_run_id, pull_request_id, provider,
                    placement, path, start_line, end_line, head_sha, provider_comment_id, published_at)
                 VALUES ($1, $2, $3, $4, 'github', $5, $6, $7, $7, $8, $9, now()) RETURNING id",
            )
            .bind(org)
            .bind(verified)
            .bind(run)
            .bind(pr)
            .bind(placement)
            .bind(path)
            .bind(start)
            .bind(sha('b'))
            .bind(comment)
            .fetch_one(&pool)
            .await
        }
    };
    assert_eq!(
        code(publish("inline", None, None, "c1").await),
        CHECK_VIOLATION
    );
    assert_eq!(
        code(publish("inline", Some("src/a.ts"), None, "c1").await),
        CHECK_VIOLATION
    );
    assert_eq!(
        code(publish("summary", Some("src/a.ts"), Some(3), "c1").await),
        CHECK_VIOLATION
    );
    assert_eq!(
        code(publish("pinned", None, None, "c1").await),
        CHECK_VIOLATION
    );
    publish("inline", Some("src/a.ts"), Some(3), "c1")
        .await
        .unwrap();
    // One published finding per verified finding.
    assert_eq!(
        code(publish("summary", None, None, "c2").await),
        UNIQUE_VIOLATION
    );
    db.finish().await;
}

#[tokio::test]
async fn duplicate_webhook_delivery_rejected() {
    let db = TestDb::new(true).await;
    let insert = |delivery: &'static str| {
        let pool = db.pool.clone();
        async move {
            sqlx::query_scalar::<_, Uuid>(
                "INSERT INTO webhook_deliveries
                   (provider, delivery_id, event, payload_sha256, signature_valid, status)
                 VALUES ('github', $1, 'pull_request', $2, true, 'received') RETURNING id",
            )
            .bind(delivery)
            .bind("a".repeat(64))
            .fetch_one(&pool)
            .await
        }
    };
    insert("d-1").await.unwrap();
    assert_eq!(code(insert("d-1").await), UNIQUE_VIOLATION);
    insert("d-2").await.unwrap();
    db.finish().await;
}

#[tokio::test]
async fn candidate_reinsert_same_fingerprint_conflicts() {
    let db = TestDb::new(true).await;
    let c = chain(&db.pool, "acme").await;
    let fp = fingerprint(7);
    insert_candidate(
        &db.pool,
        c.seed.org,
        c.run,
        c.reviewer_run,
        &fp,
        "GENERATED",
        "low",
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        code(
            insert_candidate(
                &db.pool,
                c.seed.org,
                c.run,
                c.reviewer_run,
                &fp,
                "GENERATED",
                "low",
                None
            )
            .await
        ),
        UNIQUE_VIOLATION
    );
    // The retry path: ON CONFLICT DO NOTHING inserts no row.
    let inserted = sqlx::query(
        "INSERT INTO candidate_findings
           (organization_id, review_run_id, reviewer_run_id, reviewer, category, title, description,
            changed_path, changed_side, changed_start_line, changed_end_line, severity_candidate,
            fingerprint, state)
         VALUES ($1, $2, $3, 'security', 'security', 't', 'd', 'a.ts', 'head', 1, 1, 'low', $4, 'GENERATED')
         ON CONFLICT (reviewer_run_id, fingerprint) DO NOTHING",
    )
    .bind(c.seed.org)
    .bind(c.run)
    .bind(c.reviewer_run)
    .bind(&fp)
    .execute(&db.pool)
    .await
    .unwrap()
    .rows_affected();
    assert_eq!(inserted, 0);
    db.finish().await;
}

#[tokio::test]
async fn reviewer_run_cluster_key_nulls_not_distinct() {
    let db = TestDb::new(true).await;
    let c = chain(&db.pool, "acme").await;
    // `chain` already created a (run, security, NULL) row.
    assert_eq!(
        code(insert_reviewer_run(&db.pool, c.seed.org, c.run, None).await),
        UNIQUE_VIOLATION
    );
    let key = "0123456789abcdef0123456789abcdef";
    insert_reviewer_run(&db.pool, c.seed.org, c.run, Some(key))
        .await
        .unwrap();
    assert_eq!(
        code(insert_reviewer_run(&db.pool, c.seed.org, c.run, Some(key)).await),
        UNIQUE_VIOLATION
    );
    assert_eq!(
        code(insert_reviewer_run(&db.pool, c.seed.org, c.run, Some("NOT-HEX")).await),
        CHECK_VIOLATION
    );
    db.finish().await;
}

#[tokio::test]
async fn no_credential_columns() {
    let db = TestDb::new(true).await;
    // Token *usage counters* (input_tokens, ...) are accounting, not credentials.
    let offenders: Vec<String> = sqlx::query_scalar(
        "SELECT table_name::text || '.' || column_name::text FROM information_schema.columns
         WHERE table_schema = 'public'
           AND column_name ~* 'token|secret|password|private_key'
           AND column_name !~ '^(input|output|cached_read|cached_write)_tokens$'",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert!(
        offenders.is_empty(),
        "credential-like columns: {offenders:?}"
    );
    db.finish().await;
}

#[derive(sqlx::FromRow)]
struct RunIdRow {
    #[sqlx(try_from = "Uuid")]
    id: ReviewRunId,
}

#[tokio::test]
async fn uuid_ids_decode_via_try_from() {
    let db = TestDb::new(true).await;
    let s = seed(&db.pool, "acme").await;
    let generated = ReviewRunId::new();
    // Bind the typed ID through its UUID; decode it back with `#[sqlx(try_from = "Uuid")]`.
    sqlx::query(
        "INSERT INTO review_runs
           (id, organization_id, repository_id, pull_request_id, base_sha, head_sha, state, trigger)
         VALUES ($1, $2, $3, $4, $5, $6, 'RECEIVED', 'cli')",
    )
    .bind(generated.as_uuid())
    .bind(s.org)
    .bind(s.repo)
    .bind(s.pr)
    .bind(sha('a'))
    .bind(sha('b'))
    .execute(&db.pool)
    .await
    .unwrap();
    let row: RunIdRow = sqlx::query_as("SELECT id FROM review_runs WHERE id = $1")
        .bind(generated.as_uuid())
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(row.id, generated);
    db.finish().await;
}
