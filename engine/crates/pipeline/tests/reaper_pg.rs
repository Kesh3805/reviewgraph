//! Lease reaper (PIPE-002). Run with `--features integration`.

#![cfg(feature = "integration")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::time::Duration;

use pipeline::jobs::{reap_expired, sample_queue_depth, JobQueue, NewJob, PgJobQueue, Queue};
use support::{seed_org, TestDb};
use uuid::Uuid;

fn job(org: Uuid, key: &str, max_attempts: i32) -> NewJob {
    NewJob {
        organization_id: org.to_string(),
        queue: Queue::PrReview,
        idempotency_key: key.to_owned(),
        payload: format!(r#"{{"review_run_id":"{key}"}}"#),
        priority: 0,
        max_attempts,
        trace_parent: None,
        delay: None,
    }
}

async fn expire(db: &TestDb, id: &str) {
    sqlx::query("UPDATE jobs SET locked_until = now() - interval '1 second' WHERE id = $1::uuid")
        .bind(id)
        .execute(&db.pool)
        .await
        .unwrap();
}

async fn state_of(db: &TestDb, id: &str) -> String {
    sqlx::query_scalar("SELECT state FROM jobs WHERE id = $1::uuid")
        .bind(id)
        .fetch_one(&db.pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn expired_lease_requeues_job() {
    let db = TestDb::new().await;
    let org = seed_org(&db.pool).await;
    let q = PgJobQueue::new(db.pool.clone());
    q.enqueue(job(org, "e1", 5)).await.unwrap();
    let j = q
        .claim("w", &[Queue::PrReview], Duration::from_secs(60))
        .await
        .unwrap()
        .unwrap();
    expire(&db, &j.id).await;
    let reaped = reap_expired(&q).await.unwrap();
    assert_eq!(reaped.len(), 1);
    assert!(!reaped[0].dead);
    assert_eq!(state_of(&db, &j.id).await, "queued");
    db.finish().await;
}

#[tokio::test]
async fn expired_lease_at_max_attempts_goes_dead() {
    let db = TestDb::new().await;
    let org = seed_org(&db.pool).await;
    let q = PgJobQueue::new(db.pool.clone());
    q.enqueue(job(org, "e2", 1)).await.unwrap();
    let j = q
        .claim("w", &[Queue::PrReview], Duration::from_secs(60))
        .await
        .unwrap()
        .unwrap();
    expire(&db, &j.id).await;
    let reaped = reap_expired(&q).await.unwrap();
    assert!(reaped[0].dead);
    assert_eq!(state_of(&db, &j.id).await, "dead");
    db.finish().await;
}

#[tokio::test]
async fn live_lease_not_reaped() {
    let db = TestDb::new().await;
    let org = seed_org(&db.pool).await;
    let q = PgJobQueue::new(db.pool.clone());
    q.enqueue(job(org, "e3", 5)).await.unwrap();
    let j = q
        .claim("w", &[Queue::PrReview], Duration::from_secs(60))
        .await
        .unwrap()
        .unwrap();
    assert!(reap_expired(&q).await.unwrap().is_empty());
    assert_eq!(state_of(&db, &j.id).await, "running");
    db.finish().await;
}

#[tokio::test]
async fn two_reapers_reap_disjoint_batches() {
    let db = TestDb::new().await;
    let org = seed_org(&db.pool).await;
    let q = PgJobQueue::new(db.pool.clone());
    for i in 0..40 {
        q.enqueue(job(org, &format!("d{i}"), 5)).await.unwrap();
    }
    while let Some(j) = q
        .claim("w", &[Queue::PrReview], Duration::from_secs(60))
        .await
        .unwrap()
    {
        expire(&db, &j.id).await;
    }
    let (a, b) = tokio::join!(reap_expired(&q), reap_expired(&q));
    let (a, b) = (a.unwrap(), b.unwrap());
    assert_eq!(a.len() + b.len(), 40);
    let ids: std::collections::HashSet<_> =
        a.iter().chain(b.iter()).map(|r| r.id.clone()).collect();
    assert_eq!(ids.len(), 40);
    db.finish().await;
}

#[tokio::test]
async fn reaped_job_is_claimable_again() {
    let db = TestDb::new().await;
    let org = seed_org(&db.pool).await;
    let q = PgJobQueue::new(db.pool.clone());
    q.enqueue(job(org, "e4", 5)).await.unwrap();
    let j = q
        .claim("w1", &[Queue::PrReview], Duration::from_secs(60))
        .await
        .unwrap()
        .unwrap();
    expire(&db, &j.id).await;
    reap_expired(&q).await.unwrap();
    // The reaper delays the retry by 5 s; make it claimable now.
    sqlx::query("UPDATE jobs SET run_after = now() WHERE id = $1::uuid")
        .bind(&j.id)
        .execute(&db.pool)
        .await
        .unwrap();
    let again = q
        .claim("w2", &[Queue::PrReview], Duration::from_secs(60))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(again.id, j.id);
    assert_eq!(again.attempts, 2);
    db.finish().await;
}

#[tokio::test]
async fn queue_depth_gauge_reflects_states() {
    let db = TestDb::new().await;
    let org = seed_org(&db.pool).await;
    let q = PgJobQueue::new(db.pool.clone());
    q.enqueue(job(org, "g1", 5)).await.unwrap();
    q.enqueue(job(org, "g2", 5)).await.unwrap();
    q.claim("w", &[Queue::PrReview], Duration::from_secs(60))
        .await
        .unwrap();
    let depth = sample_queue_depth(&q).await.unwrap();
    let get = |state: &str| {
        depth
            .iter()
            .find(|(qn, s, _)| qn == "pr-review" && s == state)
            .map(|(_, _, n)| *n)
    };
    assert_eq!(get("queued"), Some(1));
    assert_eq!(get("running"), Some(1));
    db.finish().await;
}
