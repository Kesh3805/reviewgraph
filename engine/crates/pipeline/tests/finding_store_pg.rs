//! VER-001 against PostgreSQL. Run with:
//!
//!   TEST_DATABASE_URL=postgres://... cargo test -p pipeline --features integration
//!
//! Each test creates a throwaway database, so tests are independent and parallel-safe. The
//! connecting role must be allowed to CREATE DATABASE.

#![cfg(feature = "integration")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::str::FromStr;

use pipeline::store::PgFindingStore;
use review_core::finding::{FindingState, Suppression, SuppressionReason};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::PgPool;
use uuid::Uuid;
use verification::store::{
    FindingStore, NewCandidate, NewEvidence, NewVerification, Transition, TransitionError,
};

static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

/// Key of the advisory lock that serializes migrations across the test databases
/// (`20261002000006_db_roles` creates cluster-wide roles).
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
            .max_connections(12)
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
        drop(conn);
        result.unwrap();
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

struct Tenant {
    org: Uuid,
    run: Uuid,
    reviewer_run: Uuid,
}

/// org -> installation -> repository -> pull request -> review run -> reviewer run.
async fn seed_tenant(pool: &PgPool) -> Tenant {
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
    let pr: Uuid = sqlx::query_scalar(
        "INSERT INTO pull_requests
           (organization_id, repository_id, provider_number, title, author_login, base_ref,
            head_ref, base_sha, head_sha, state)
         VALUES ($1, $2, 1, 't', 'dev', 'trunk', 'feature', $3, $4, 'open') RETURNING id",
    )
    .bind(org)
    .bind(repo)
    .bind("a".repeat(40))
    .bind("b".repeat(40))
    .fetch_one(pool)
    .await
    .unwrap();
    let run: Uuid = sqlx::query_scalar(
        "INSERT INTO review_runs
           (organization_id, repository_id, pull_request_id, base_sha, head_sha, state, trigger)
         VALUES ($1, $2, $3, $4, $5, 'ANALYZING', 'webhook') RETURNING id",
    )
    .bind(org)
    .bind(repo)
    .bind(pr)
    .bind("a".repeat(40))
    .bind("b".repeat(40))
    .fetch_one(pool)
    .await
    .unwrap();
    let reviewer_run: Uuid = sqlx::query_scalar(
        "INSERT INTO reviewer_runs (organization_id, review_run_id, reviewer, state)
         VALUES ($1, $2, 'security', 'pending') RETURNING id",
    )
    .bind(org)
    .bind(run)
    .fetch_one(pool)
    .await
    .unwrap();
    Tenant {
        org,
        run,
        reviewer_run,
    }
}

fn candidate(t: &Tenant, n: u32) -> NewCandidate {
    NewCandidate {
        id: Uuid::new_v4(),
        review_run_id: t.run,
        reviewer_run_id: t.reviewer_run,
        reviewer: "security".into(),
        category: "security".into(),
        title: format!("title {n}"),
        description: "description".into(),
        changed_path: "src/a.ts".into(),
        changed_side: "head".into(),
        changed_start_line: 1,
        changed_end_line: 2,
        severity_candidate: "high".into(),
        confidence_candidate: Some(0.9),
        affected_symbols: vec!["sym".into()],
        evidence: serde_json::json!([]),
        fingerprint: format!("v1:{n:032x}"),
        idempotency_key: format!("key-{}-{n}", t.reviewer_run),
        raw_output_hash: format!("hash-{n}"),
    }
}

fn step(
    id: Uuid,
    from: FindingState,
    version: i32,
    to: FindingState,
    reason: Option<&str>,
    suppression: Option<Suppression>,
) -> Transition {
    Transition {
        candidate_id: id,
        from,
        expected_version: version,
        to,
        reason_code: reason.map(str::to_owned),
        stage: "test".into(),
        detail: None,
        suppression,
    }
}

async fn row(pool: &PgPool, id: Uuid) -> (String, i32, Option<String>) {
    sqlx::query_as("SELECT state, state_version, reason_code FROM candidate_findings WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn audit_count(pool: &PgPool, id: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM finding_state_transitions WHERE candidate_finding_id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn candidate_insert_idempotent() {
    let db = TestDb::new().await;
    let t = seed_tenant(&db.pool).await;
    let store = PgFindingStore::new(db.pool.clone(), t.org);
    let c = candidate(&t, 1);
    assert!(store.insert_candidate(c.clone()).await.unwrap());
    // A retried job re-inserts the same candidate (same idempotency key, fresh id).
    let retry = NewCandidate {
        id: Uuid::new_v4(),
        ..c
    };
    assert!(!store.insert_candidate(retry).await.unwrap());
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM candidate_findings")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(n, 1);
    db.finish().await;
}

#[tokio::test]
async fn transitions_audited_in_same_tx() {
    let db = TestDb::new().await;
    let t = seed_tenant(&db.pool).await;
    let store = PgFindingStore::new(db.pool.clone(), t.org);
    let c = candidate(&t, 1);
    store.insert_candidate(c.clone()).await.unwrap();

    let v = store
        .transition(step(
            c.id,
            FindingState::Generated,
            0,
            FindingState::EvidenceCollected,
            None,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(v, 1);
    assert_eq!(audit_count(&db.pool, c.id).await, 1);

    // A lost CAS writes neither the state nor an audit row.
    let lost = store
        .transition(step(
            c.id,
            FindingState::Generated,
            0,
            FindingState::EvidenceCollected,
            None,
            None,
        ))
        .await;
    assert_eq!(lost.unwrap_err(), TransitionError::Conflict);
    assert_eq!(audit_count(&db.pool, c.id).await, 1);
    assert_eq!(row(&db.pool, c.id).await.0, "EVIDENCE_COLLECTED");

    let (from, to): (String, String) = sqlx::query_as(
        "SELECT from_state, to_state FROM finding_state_transitions WHERE candidate_finding_id = $1",
    )
    .bind(c.id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        (from.as_str(), to.as_str()),
        ("GENERATED", "EVIDENCE_COLLECTED")
    );
    db.finish().await;
}

#[tokio::test]
async fn cas_conflict_detected() {
    let db = TestDb::new().await;
    let t = seed_tenant(&db.pool).await;
    let store = PgFindingStore::new(db.pool.clone(), t.org);
    let c = candidate(&t, 1);
    store.insert_candidate(c.clone()).await.unwrap();

    let race = |store: PgFindingStore| {
        let id = c.id;
        async move {
            store
                .transition(step(
                    id,
                    FindingState::Generated,
                    0,
                    FindingState::EvidenceCollected,
                    None,
                    None,
                ))
                .await
        }
    };
    let (a, b) = tokio::join!(race(store.clone()), race(store.clone()));
    let outcomes = [a, b];
    assert_eq!(outcomes.iter().filter(|r| r.is_ok()).count(), 1);
    assert!(outcomes
        .iter()
        .any(|r| matches!(r, Err(TransitionError::Conflict))));

    let missing = store
        .transition(step(
            Uuid::new_v4(),
            FindingState::Generated,
            0,
            FindingState::EvidenceCollected,
            None,
            None,
        ))
        .await;
    assert_eq!(missing.unwrap_err(), TransitionError::NotFound);
    db.finish().await;
}

#[tokio::test]
async fn illegal_transition_never_reaches_sql() {
    let db = TestDb::new().await;
    let t = seed_tenant(&db.pool).await;
    let store = PgFindingStore::new(db.pool.clone(), t.org);
    let c = candidate(&t, 1);
    store.insert_candidate(c.clone()).await.unwrap();
    let err = store
        .transition(step(
            c.id,
            FindingState::Generated,
            0,
            FindingState::Published,
            None,
            None,
        ))
        .await
        .unwrap_err();
    assert!(matches!(err, TransitionError::IllegalTransition { .. }));
    assert_eq!(row(&db.pool, c.id).await, ("GENERATED".into(), 0, None));
    assert_eq!(audit_count(&db.pool, c.id).await, 0);
    db.finish().await;
}

#[tokio::test]
async fn rejected_invalid_carries_reason_code_only() {
    let db = TestDb::new().await;
    let t = seed_tenant(&db.pool).await;
    let store = PgFindingStore::new(db.pool.clone(), t.org);
    let c = candidate(&t, 1);
    store.insert_candidate(c.clone()).await.unwrap();
    store
        .transition(step(
            c.id,
            FindingState::Generated,
            0,
            FindingState::RejectedInvalid,
            Some("anchor_unresolvable"),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(
        row(&db.pool, c.id).await,
        (
            "REJECTED_INVALID".into(),
            1,
            Some("anchor_unresolvable".into())
        )
    );
    db.finish().await;
}

#[tokio::test]
async fn suppressed_transition_persists_suppression_object() {
    let db = TestDb::new().await;
    let t = seed_tenant(&db.pool).await;
    let store = PgFindingStore::new(db.pool.clone(), t.org);
    let c = candidate(&t, 1);
    store.insert_candidate(c.clone()).await.unwrap();
    let s = Suppression::new(SuppressionReason::Preexisting, "on base", Some(5)).unwrap();
    store
        .transition(step(
            c.id,
            FindingState::Generated,
            0,
            FindingState::SuppressedPreexisting,
            Some("PREEXISTING"),
            Some(s),
        ))
        .await
        .unwrap();
    let stage: Option<i16> =
        sqlx::query_scalar("SELECT suppressed_at_stage FROM candidate_findings WHERE id = $1")
            .bind(c.id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(stage, Some(5));
    db.finish().await;
}

#[tokio::test]
async fn bulk_invalidate_skips_terminal() {
    let db = TestDb::new().await;
    let t = seed_tenant(&db.pool).await;
    let store = PgFindingStore::new(db.pool.clone(), t.org);
    let (live, rejected, evidence) = (candidate(&t, 1), candidate(&t, 2), candidate(&t, 3));
    for c in [&live, &rejected, &evidence] {
        store.insert_candidate(c.clone()).await.unwrap();
    }
    store
        .transition(step(
            rejected.id,
            FindingState::Generated,
            0,
            FindingState::RejectedInvalid,
            Some("bad"),
            None,
        ))
        .await
        .unwrap();
    store
        .transition(step(
            evidence.id,
            FindingState::Generated,
            0,
            FindingState::EvidenceCollected,
            None,
            None,
        ))
        .await
        .unwrap();

    assert_eq!(store.invalidate_run(t.run).await.unwrap(), 2);
    assert_eq!(
        row(&db.pool, live.id).await,
        ("INVALIDATED".into(), 1, Some("HEAD_SUPERSEDED".into()))
    );
    assert_eq!(row(&db.pool, evidence.id).await.0, "INVALIDATED");
    assert_eq!(
        row(&db.pool, rejected.id).await.0,
        "REJECTED_INVALID",
        "terminal rows are untouched"
    );
    // Audited with the state each candidate actually left.
    let from: String = sqlx::query_scalar(
        "SELECT from_state FROM finding_state_transitions
          WHERE candidate_finding_id = $1 AND to_state = 'INVALIDATED'",
    )
    .bind(evidence.id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(from, "EVIDENCE_COLLECTED");
    assert_eq!(audit_count(&db.pool, rejected.id).await, 1);
    // Idempotent: nothing left to invalidate.
    assert_eq!(store.invalidate_run(t.run).await.unwrap(), 0);
    db.finish().await;
}

#[tokio::test]
async fn invalidated_candidate_cannot_be_verified() {
    let db = TestDb::new().await;
    let t = seed_tenant(&db.pool).await;
    let store = PgFindingStore::new(db.pool.clone(), t.org);
    let c = candidate(&t, 1);
    store.insert_candidate(c.clone()).await.unwrap();
    store
        .transition(step(
            c.id,
            FindingState::Generated,
            0,
            FindingState::EvidenceCollected,
            None,
            None,
        ))
        .await
        .unwrap();
    // Supersession lands while verification is in flight.
    store.invalidate_run(t.run).await.unwrap();
    let late = store
        .transition(step(
            c.id,
            FindingState::EvidenceCollected,
            1,
            FindingState::Verified,
            None,
            None,
        ))
        .await;
    assert_eq!(late.unwrap_err(), TransitionError::Conflict);
    assert_eq!(row(&db.pool, c.id).await.0, "INVALIDATED");
    db.finish().await;
}

#[tokio::test]
async fn verification_unique_per_version() {
    let db = TestDb::new().await;
    let t = seed_tenant(&db.pool).await;
    let store = PgFindingStore::new(db.pool.clone(), t.org);
    let c = candidate(&t, 1);
    store.insert_candidate(c.clone()).await.unwrap();
    let v = |version: &str| NewVerification {
        id: Uuid::new_v4(),
        candidate_id: c.id,
        verification_version: version.into(),
        base_snapshot_id: Uuid::new_v4(),
        head_snapshot_id: Uuid::new_v4(),
        depth: "standard".into(),
        stage_outcomes: serde_json::json!([]),
        signals: serde_json::json!({}),
        confidence: Some(0.873),
        eligibility: Some("publish".into()),
        placement: Some("inline".into()),
        cache_key: "k".into(),
        from_cache: false,
        duration_ms: 12,
    };
    assert!(store.record_verification(v("v1")).await.unwrap());
    assert!(!store.record_verification(v("v1")).await.unwrap());
    assert!(store.record_verification(v("v2")).await.unwrap());

    let verification: Uuid = sqlx::query_scalar(
        "SELECT id FROM finding_verifications WHERE verification_version = 'v1'",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    store
        .add_evidence(vec![NewEvidence {
            candidate_id: c.id,
            verification_id: Some(verification),
            kind: "graph_path".into(),
            source: "graph".into(),
            strength: "supporting".into(),
            polarity: "supports".into(),
            path: Some("src/a.ts".into()),
            side: Some("head".into()),
            start_line: Some(1),
            end_line: Some(2),
            symbol_keys: vec!["sym".into()],
            payload: serde_json::json!({}),
        }])
        .await
        .unwrap();
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM finding_evidence")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(n, 1);
    db.finish().await;
}

#[tokio::test]
async fn rls_blocks_cross_org_select() {
    let db = TestDb::new().await;
    let a = seed_tenant(&db.pool).await;
    let b = seed_tenant(&db.pool).await;
    let store_a = PgFindingStore::new(db.pool.clone(), a.org);
    let c = candidate(&a, 1);
    store_a.insert_candidate(c.clone()).await.unwrap();
    store_a
        .transition(step(
            c.id,
            FindingState::Generated,
            0,
            FindingState::EvidenceCollected,
            None,
            None,
        ))
        .await
        .unwrap();

    // The store of another tenant cannot move the row.
    let store_b = PgFindingStore::new(db.pool.clone(), b.org);
    let cross = store_b
        .transition(step(
            c.id,
            FindingState::EvidenceCollected,
            1,
            FindingState::Verified,
            None,
            None,
        ))
        .await;
    assert_eq!(cross.unwrap_err(), TransitionError::NotFound);

    // Directly, as the non-bypass engine role, the policy filters by app.organization_id.
    let count_as = |org: Uuid, table: &'static str| {
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
            let n: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
                .fetch_one(&mut *tx)
                .await
                .unwrap();
            tx.rollback().await.unwrap();
            n
        }
    };
    for table in ["candidate_findings", "finding_state_transitions"] {
        assert_eq!(count_as(a.org, table).await, 1, "{table}");
        assert_eq!(count_as(b.org, table).await, 0, "{table}");
    }
    db.finish().await;
}
