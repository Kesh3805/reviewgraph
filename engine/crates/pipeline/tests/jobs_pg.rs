//! PgJobQueue against a real Postgres (PIPE-001). Run with:
//!
//!   TEST_DATABASE_URL=postgres://... cargo test -p pipeline --features integration

#![cfg(feature = "integration")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use pipeline::jobs::{
    enqueue_tx, Backoff, CancelPredicate, FailKind, FailOutcome, JobError, JobQueue, NewJob,
    PgJobQueue, Queue,
};
use support::{seed_org, TestDb};
use uuid::Uuid;

const LEASE: Duration = Duration::from_secs(60);

fn job(org: Uuid, key: &str, queue: Queue) -> NewJob {
    NewJob {
        organization_id: org.to_string(),
        queue,
        idempotency_key: key.to_owned(),
        payload: format!(r#"{{"review_run_id":"{key}"}}"#),
        priority: 0,
        max_attempts: 5,
        trace_parent: None,
        delay: None,
    }
}

async fn state_of(db: &TestDb, id: &str) -> (String, i32, Option<String>) {
    sqlx::query_as::<_, (String, i32, Option<String>)>(
        "SELECT state, attempts, locked_by FROM jobs WHERE id = $1::uuid",
    )
    .bind(id)
    .fetch_one(&db.pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn enqueue_is_idempotent_by_key() {
    let db = TestDb::new().await;
    let org = seed_org(&db.pool).await;
    let q = PgJobQueue::new(db.pool.clone());
    let (id1, created1) = q.enqueue(job(org, "k1", Queue::PrReview)).await.unwrap();
    let (id2, created2) = q.enqueue(job(org, "k1", Queue::PrReview)).await.unwrap();
    assert!(created1);
    assert!(!created2);
    assert_eq!(id1, id2);
    db.finish().await;
}

#[tokio::test]
async fn enqueue_rolled_back_leaves_no_job() {
    let db = TestDb::new().await;
    let org = seed_org(&db.pool).await;
    let mut tx = db.pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.job_worker', 'on', true)")
        .execute(&mut *tx)
        .await
        .unwrap();
    enqueue_tx(&mut tx, &job(org, "rb", Queue::PrReview))
        .await
        .unwrap();
    tx.rollback().await.unwrap();
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM jobs")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
    db.finish().await;
}

#[tokio::test]
async fn claim_is_disjoint_under_32_concurrent_workers() {
    let db = TestDb::new().await;
    let org = seed_org(&db.pool).await;
    let q = Arc::new(PgJobQueue::new(db.pool.clone()));
    const JOBS: usize = 400;
    for i in 0..JOBS {
        q.enqueue(job(org, &format!("c{i}"), Queue::PrReview))
            .await
            .unwrap();
    }
    let mut handles = Vec::new();
    for w in 0..32 {
        let q = q.clone();
        handles.push(tokio::spawn(async move {
            let mut got = Vec::new();
            while let Some(j) = q
                .claim(&format!("w{w}"), &[Queue::PrReview], LEASE)
                .await
                .unwrap()
            {
                got.push(j.id);
            }
            got
        }));
    }
    let mut seen = HashSet::new();
    let mut total = 0;
    for h in handles {
        for id in h.await.unwrap() {
            total += 1;
            assert!(seen.insert(id), "a job was claimed twice");
        }
    }
    assert_eq!(total, JOBS);
    db.finish().await;
}

#[tokio::test]
async fn claim_respects_priority_then_age() {
    let db = TestDb::new().await;
    let org = seed_org(&db.pool).await;
    let q = PgJobQueue::new(db.pool.clone());
    let (old, _) = q.enqueue(job(org, "old", Queue::PrReview)).await.unwrap();
    let (newer, _) = q.enqueue(job(org, "newer", Queue::PrReview)).await.unwrap();
    let mut urgent = job(org, "urgent", Queue::PrReview);
    urgent.priority = 10;
    let (urgent_id, _) = q.enqueue(urgent).await.unwrap();
    let order: Vec<String> = {
        let mut v = Vec::new();
        while let Some(j) = q.claim("w", &[Queue::PrReview], LEASE).await.unwrap() {
            v.push(j.id);
        }
        v
    };
    assert_eq!(order, vec![urgent_id, old, newer]);
    db.finish().await;
}

#[tokio::test]
async fn claim_skips_future_run_after_and_other_queues() {
    let db = TestDb::new().await;
    let org = seed_org(&db.pool).await;
    let q = PgJobQueue::new(db.pool.clone());
    let mut later = job(org, "later", Queue::PrReview);
    later.delay = Some(Duration::from_secs(3600));
    q.enqueue(later).await.unwrap();
    q.enqueue(job(org, "other", Queue::ReviewPublish))
        .await
        .unwrap();
    assert!(q
        .claim("w", &[Queue::PrReview], LEASE)
        .await
        .unwrap()
        .is_none());
    db.finish().await;
}

#[tokio::test]
async fn heartbeat_extends_lease_only_for_owner() {
    let db = TestDb::new().await;
    let org = seed_org(&db.pool).await;
    let q = PgJobQueue::new(db.pool.clone());
    q.enqueue(job(org, "hb", Queue::PrReview)).await.unwrap();
    let claimed = q
        .claim("owner", &[Queue::PrReview], Duration::from_secs(5))
        .await
        .unwrap()
        .unwrap();
    assert!(q
        .heartbeat(&claimed, Duration::from_secs(600))
        .await
        .unwrap());
    let mut thief = claimed.clone();
    thief.locked_by = "thief".into();
    assert!(!q.heartbeat(&thief, Duration::from_secs(600)).await.unwrap());
    let extended: bool = sqlx::query_scalar(
        "SELECT locked_until > now() + interval '300 seconds' FROM jobs WHERE id = $1::uuid",
    )
    .bind(&claimed.id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert!(extended);
    db.finish().await;
}

#[tokio::test]
async fn stale_worker_complete_rejected() {
    let db = TestDb::new().await;
    let org = seed_org(&db.pool).await;
    let q = PgJobQueue::new(db.pool.clone());
    q.enqueue(job(org, "stale", Queue::PrReview)).await.unwrap();
    let first = q
        .claim("w1", &[Queue::PrReview], LEASE)
        .await
        .unwrap()
        .unwrap();
    // The first worker loses its lease; a second worker re-claims the job.
    q.release_worker("w1").await.unwrap();
    let second = q
        .claim("w2", &[Queue::PrReview], LEASE)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(q.complete(&first).await, Err(JobError::LeaseLost)));
    assert!(matches!(
        q.fail(&first, FailKind::Transient, "x").await,
        Err(JobError::LeaseLost)
    ));
    q.complete(&second).await.unwrap();
    assert_eq!(state_of(&db, &second.id).await.0, "succeeded");
    db.finish().await;
}

#[tokio::test]
async fn fail_transient_backs_off_with_jitter_bounds() {
    let db = TestDb::new().await;
    let org = seed_org(&db.pool).await;
    let q = PgJobQueue::new(db.pool.clone()).with_backoff(Backoff::default());
    q.enqueue(job(org, "tr", Queue::PrReview)).await.unwrap();
    let j = q
        .claim("w", &[Queue::PrReview], LEASE)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        q.fail(&j, FailKind::Transient, "boom").await.unwrap(),
        FailOutcome::Requeued
    );
    let secs: f64 = sqlx::query_scalar(
        "SELECT extract(epoch from run_after - now())::float8 FROM jobs WHERE id = $1::uuid",
    )
    .bind(&j.id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    // First attempt: ceiling 5 s, full jitter.
    assert!(secs <= 5.1, "delay {secs}");
    let (state, _, locked_by) = state_of(&db, &j.id).await;
    assert_eq!(state, "queued");
    assert!(locked_by.is_none());
    db.finish().await;
}

#[tokio::test]
async fn fail_goes_dead_at_max_attempts() {
    let db = TestDb::new().await;
    let org = seed_org(&db.pool).await;
    let q = PgJobQueue::new(db.pool.clone()).with_backoff(Backoff {
        base: Duration::ZERO,
        cap: Duration::ZERO,
    });
    let mut nj = job(org, "max", Queue::PrReview);
    nj.max_attempts = 2;
    q.enqueue(nj).await.unwrap();
    let j1 = q
        .claim("w", &[Queue::PrReview], LEASE)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        q.fail(&j1, FailKind::Transient, "e").await.unwrap(),
        FailOutcome::Requeued
    );
    let j2 = q
        .claim("w", &[Queue::PrReview], LEASE)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(j2.attempts, 2);
    assert_eq!(
        q.fail(&j2, FailKind::Transient, "e").await.unwrap(),
        FailOutcome::Dead
    );
    assert_eq!(state_of(&db, &j2.id).await.0, "dead");
    db.finish().await;
}

#[tokio::test]
async fn fail_permanent_goes_dead_immediately() {
    let db = TestDb::new().await;
    let org = seed_org(&db.pool).await;
    let q = PgJobQueue::new(db.pool.clone());
    q.enqueue(job(org, "perm", Queue::PrReview)).await.unwrap();
    let j = q
        .claim("w", &[Queue::PrReview], LEASE)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        q.fail(&j, FailKind::Permanent, "bad").await.unwrap(),
        FailOutcome::Dead
    );
    db.finish().await;
}

#[tokio::test]
async fn rate_limited_requeues_without_consuming_attempt() {
    let db = TestDb::new().await;
    let org = seed_org(&db.pool).await;
    let q = PgJobQueue::new(db.pool.clone());
    q.enqueue(job(org, "rl", Queue::PrReview)).await.unwrap();
    let j = q
        .claim("w", &[Queue::PrReview], LEASE)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(j.attempts, 1);
    let out = q
        .fail(
            &j,
            FailKind::RateLimited {
                retry_after: Duration::from_secs(30),
            },
            "limited",
        )
        .await
        .unwrap();
    assert_eq!(out, FailOutcome::Requeued);
    let (attempts, requeues): (i32, i32) =
        sqlx::query_as("SELECT attempts, rate_limit_requeues FROM jobs WHERE id = $1::uuid")
            .bind(&j.id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!((attempts, requeues), (0, 1));
    db.finish().await;
}

#[tokio::test]
async fn cancel_where_only_cancels_queued() {
    let db = TestDb::new().await;
    let org = seed_org(&db.pool).await;
    let q = PgJobQueue::new(db.pool.clone());
    q.enqueue(job(org, "run-a", Queue::PrReview)).await.unwrap();
    q.enqueue(job(org, "run-b", Queue::PrReview)).await.unwrap();
    // run-a is running: it must not be cancelled.
    let running = q
        .claim("w", &[Queue::PrReview], LEASE)
        .await
        .unwrap()
        .unwrap();
    let n = q
        .cancel_where(CancelPredicate {
            queue: Some(Queue::PrReview),
            review_run_ids: Some(vec!["run-a".into(), "run-b".into()]),
            key_prefix: None,
        })
        .await
        .unwrap();
    assert_eq!(n, 1);
    assert_eq!(state_of(&db, &running.id).await.0, "running");
    assert!(q.cancel_where(CancelPredicate::default()).await.is_err());
    db.finish().await;
}

#[tokio::test]
async fn release_worker_returns_running_jobs_and_refunds_attempt() {
    let db = TestDb::new().await;
    let org = seed_org(&db.pool).await;
    let q = PgJobQueue::new(db.pool.clone());
    q.enqueue(job(org, "rel", Queue::PrReview)).await.unwrap();
    let j = q
        .claim("w", &[Queue::PrReview], LEASE)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(q.release_worker("w").await.unwrap(), 1);
    let (state, attempts, locked_by) = state_of(&db, &j.id).await;
    assert_eq!((state.as_str(), attempts), ("queued", 0));
    assert!(locked_by.is_none());
    db.finish().await;
}
