//! Review state CAS transitions against Postgres (PIPE-007). Run with `--features integration`.

#![cfg(feature = "integration")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use pipeline::store::{PgReviewStateStore, ReviewStateStore, TransitionResult};
use review_core::ids::{OrganizationId, ReviewRunId};
use review_core::review::{ReviewState, RunFailure, TransitionInput};
use review_core::ErrorClass;
use support::{seed_run, TestDb};
use ReviewState as S;

struct Fixture {
    db: TestDb,
    store: PgReviewStateStore,
    org: OrganizationId,
    run: ReviewRunId,
}

async fn fixture() -> Fixture {
    let db = TestDb::new().await;
    let s = seed_run(&db.pool, &"b".repeat(40)).await;
    Fixture {
        store: PgReviewStateStore::new(db.pool.clone()),
        org: OrganizationId::from_uuid(s.org),
        run: ReviewRunId::from_uuid(s.run),
        db,
    }
}

impl Fixture {
    async fn advance(&self, from: ReviewState, to: ReviewState) -> TransitionResult {
        self.store
            .transition(self.org, self.run, from, to, TransitionInput::Advance, None)
            .await
            .unwrap()
    }

    async fn audit_rows(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM review_run_transitions WHERE review_run_id = $1")
            .bind(self.run.as_uuid())
            .fetch_one(&self.db.pool)
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn cas_applies_and_audits_in_one_tx() {
    let f = fixture().await;
    assert_eq!(
        f.advance(S::Received, S::Indexing).await,
        TransitionResult::Applied
    );
    assert_eq!(
        f.store.current_state(f.org, f.run).await.unwrap(),
        S::Indexing
    );
    assert_eq!(f.audit_rows().await, 1);
    f.db.finish().await;
}

#[tokio::test]
async fn illegal_edge_rejected_before_sql() {
    let f = fixture().await;
    let r = f
        .store
        .transition(
            f.org,
            f.run,
            S::Received,
            S::Reviewing,
            TransitionInput::Advance,
            None,
        )
        .await;
    assert!(r.is_err());
    assert_eq!(f.audit_rows().await, 0);
    f.db.finish().await;
}

#[tokio::test]
async fn concurrent_advance_exactly_one_wins() {
    let f = fixture().await;
    let mut tasks = Vec::new();
    for _ in 0..16 {
        let store = f.store.clone();
        let (org, run) = (f.org, f.run);
        tasks.push(tokio::spawn(async move {
            store
                .transition(
                    org,
                    run,
                    S::Received,
                    S::Indexing,
                    TransitionInput::Advance,
                    None,
                )
                .await
                .unwrap()
        }));
    }
    let mut applied = 0;
    for t in tasks {
        match t.await.unwrap() {
            TransitionResult::Applied => applied += 1,
            TransitionResult::AlreadyAtOrAfter(S::Indexing) => {}
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!(applied, 1);
    assert_eq!(f.audit_rows().await, 1);
    f.db.finish().await;
}

#[tokio::test]
async fn supersede_beats_in_flight_advance_and_late_advance_is_lost() {
    let f = fixture().await;
    f.advance(S::Received, S::Indexing).await;
    // The superseding run must exist for the deferred foreign key; use the same org's run id of
    // a freshly inserted second run on another head.
    let by = sqlx::query_scalar::<_, uuid::Uuid>(
        "INSERT INTO review_runs
           (organization_id, repository_id, pull_request_id, base_sha, head_sha, state, trigger)
         SELECT organization_id, repository_id, pull_request_id, base_sha, $2, 'RECEIVED', 'webhook'
         FROM review_runs WHERE id = $1 RETURNING id",
    )
    .bind(f.run.as_uuid())
    .bind("c".repeat(40))
    .fetch_one(&f.db.pool)
    .await;
    // The one-active-run index rejects a second active run; supersede points at a run id that is
    // inserted afterwards, as the control plane does (the foreign key is deferred).
    assert!(by.is_err());
    let by = ReviewRunId::new();
    let mut tx = f.db.pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.organization_id', $1, true)")
        .bind(f.org.as_uuid().to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    let applied = PgReviewStateStore::transition_in(
        &mut tx,
        f.org,
        f.run,
        S::Indexing,
        S::Superseded,
        TransitionInput::Supersede { by },
        None,
    )
    .await
    .unwrap();
    assert_eq!(applied, TransitionResult::Applied);
    sqlx::query(
        "INSERT INTO review_runs
           (id, organization_id, repository_id, pull_request_id, base_sha, head_sha, state, trigger)
         SELECT $2, organization_id, repository_id, pull_request_id, base_sha, $3, 'RECEIVED',
                'webhook'
         FROM review_runs WHERE id = $1",
    )
    .bind(f.run.as_uuid())
    .bind(by.as_uuid())
    .bind("c".repeat(40))
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    // The in-flight stage now tries to advance: it lost.
    assert_eq!(
        f.advance(S::Indexing, S::Analyzing).await,
        TransitionResult::Lost {
            observed: S::Superseded
        }
    );
    f.db.finish().await;
}

#[tokio::test]
async fn duplicate_advance_is_already_at_or_after() {
    let f = fixture().await;
    f.advance(S::Received, S::Indexing).await;
    f.advance(S::Indexing, S::Analyzing).await;
    f.advance(S::Analyzing, S::Reviewing).await;
    // A retried job re-applies INDEXING -> ANALYZING to a run that is already REVIEWING.
    assert_eq!(
        f.advance(S::Indexing, S::Analyzing).await,
        TransitionResult::AlreadyAtOrAfter(S::Reviewing)
    );
    assert_eq!(f.audit_rows().await, 3);
    f.db.finish().await;
}

#[tokio::test]
async fn terminal_state_never_left() {
    let f = fixture().await;
    f.store
        .cancel(f.org, f.run, S::Received, None)
        .await
        .unwrap();
    // Every later edge from the cancelled run is rejected in memory...
    assert!(f
        .store
        .transition(
            f.org,
            f.run,
            S::Cancelled,
            S::Indexing,
            TransitionInput::Advance,
            None
        )
        .await
        .is_err());
    // ...and a stage that still believes the run is RECEIVED loses the CAS.
    assert_eq!(
        f.advance(S::Received, S::Indexing).await,
        TransitionResult::Lost {
            observed: S::Cancelled
        }
    );
    f.db.finish().await;
}

#[tokio::test]
async fn fail_run_maps_stage_to_failed_state() {
    let f = fixture().await;
    f.advance(S::Received, S::Indexing).await;
    let failure = RunFailure::new(ErrorClass::Permanent, "index exploded").unwrap();
    assert_eq!(
        f.store
            .fail_run(f.org, f.run, S::Indexing, failure, None)
            .await
            .unwrap(),
        TransitionResult::Applied
    );
    assert_eq!(
        f.store.current_state(f.org, f.run).await.unwrap(),
        S::FailedIndexing
    );
    let class: Option<String> =
        sqlx::query_scalar("SELECT failure_class FROM review_runs WHERE id = $1")
            .bind(f.run.as_uuid())
            .fetch_one(&f.db.pool)
            .await
            .unwrap();
    assert_eq!(class.as_deref(), Some("permanent"));
    // RECEIVED has no failure state.
    let again = RunFailure::new(ErrorClass::Permanent, "x").unwrap();
    assert!(f
        .store
        .fail_run(f.org, f.run, S::Received, again, None)
        .await
        .is_err());
    f.db.finish().await;
}

#[tokio::test]
async fn failure_detail_truncated_and_redacted() {
    let f = fixture().await;
    f.advance(S::Received, S::Indexing).await;
    let detail = format!("token ghp_{} {}", "a".repeat(36), "x".repeat(1500));
    let failure = RunFailure::new(ErrorClass::Permanent, detail).unwrap();
    f.store
        .fail_run(f.org, f.run, S::Indexing, failure, None)
        .await
        .unwrap();
    let stored: String = sqlx::query_scalar("SELECT failure_detail FROM review_runs WHERE id = $1")
        .bind(f.run.as_uuid())
        .fetch_one(&f.db.pool)
        .await
        .unwrap();
    assert!(!stored.contains("ghp_aaaa"));
    assert!(stored.chars().count() <= 2000);
    f.db.finish().await;
}

#[tokio::test]
async fn cross_tenant_transition_updates_nothing() {
    let f = fixture().await;
    let other = seed_run(&f.db.pool, &"c".repeat(40)).await;
    let r = f
        .store
        .transition(
            OrganizationId::from_uuid(other.org),
            f.run,
            S::Received,
            S::Indexing,
            TransitionInput::Advance,
            None,
        )
        .await;
    assert!(r.is_err(), "a foreign run is not visible: {r:?}");
    assert_eq!(
        f.store.current_state(f.org, f.run).await.unwrap(),
        S::Received
    );
    f.db.finish().await;
}
