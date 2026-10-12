//! LISTEN/NOTIFY wake-up and the claim loop (PIPE-002). Run with `--features integration`.

#![cfg(feature = "integration")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use pipeline::jobs::{
    claim_loop, ClaimLoopConfig, JobQueue, NewJob, PgJobQueue, Queue, QueueWaker,
};
use support::{seed_org, TestDb};
use tokio::sync::{mpsc, Semaphore};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

fn job(org: Uuid, key: &str, delay: Option<Duration>) -> NewJob {
    NewJob {
        organization_id: org.to_string(),
        queue: Queue::PrReview,
        idempotency_key: key.to_owned(),
        payload: format!(r#"{{"review_run_id":"{key}"}}"#),
        priority: 0,
        max_attempts: 5,
        trace_parent: None,
        delay,
    }
}

#[tokio::test]
async fn notify_wakes_idle_consumer_within_a_second() {
    let db = TestDb::new().await;
    let org = seed_org(&db.pool).await;
    let q = PgJobQueue::new(db.pool.clone());
    let waker = QueueWaker::spawn(db.pool.clone(), &[Queue::PrReview]);
    let notify = waker.notify_for(Queue::PrReview);
    // The initial wake-up after subscribing is consumed first.
    tokio::time::timeout(Duration::from_secs(5), notify.notified())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let started = Instant::now();
    q.enqueue(job(org, "n1", None)).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), notify.notified())
        .await
        .unwrap();
    assert!(started.elapsed() < Duration::from_secs(1));
    waker.shutdown().await;
    db.finish().await;
}

#[tokio::test]
async fn claim_loop_recovers_lost_notification_by_polling() {
    let db = TestDb::new().await;
    let org = seed_org(&db.pool).await;
    let q = PgJobQueue::new(db.pool.clone());
    // No waker at all: only the poll interval can find the job.
    q.enqueue(job(org, "p1", None)).await.unwrap();
    let mut cfg = ClaimLoopConfig::new("w", vec![Queue::PrReview]);
    cfg.poll_interval = Duration::from_millis(50);
    let cancel = CancellationToken::new();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let loop_q = q.clone();
    let loop_cancel = cancel.clone();
    let task = tokio::spawn(async move {
        claim_loop(
            &loop_q,
            &cfg,
            Arc::new(tokio::sync::Notify::new()),
            Arc::new(Semaphore::new(2)),
            loop_cancel,
            |job, permit| {
                let tx = tx.clone();
                async move {
                    tx.send(job.id).unwrap();
                    drop(permit);
                }
            },
        )
        .await;
    });
    let id = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(!id.is_empty());
    cancel.cancel();
    task.await.unwrap();
    db.finish().await;
}

#[tokio::test]
async fn delayed_job_picked_up_after_run_after() {
    let db = TestDb::new().await;
    let org = seed_org(&db.pool).await;
    let q = PgJobQueue::new(db.pool.clone());
    q.enqueue(job(org, "dl", Some(Duration::from_millis(600))))
        .await
        .unwrap();
    assert!(q
        .claim("w", &[Queue::PrReview], Duration::from_secs(60))
        .await
        .unwrap()
        .is_none());
    let mut cfg = ClaimLoopConfig::new("w", vec![Queue::PrReview]);
    cfg.poll_interval = Duration::from_millis(100);
    let cancel = CancellationToken::new();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let loop_q = q.clone();
    let loop_cancel = cancel.clone();
    let task = tokio::spawn(async move {
        claim_loop(
            &loop_q,
            &cfg,
            Arc::new(tokio::sync::Notify::new()),
            Arc::new(Semaphore::new(1)),
            loop_cancel,
            |job, permit| {
                let tx = tx.clone();
                async move {
                    tx.send(job.id).unwrap();
                    drop(permit);
                }
            },
        )
        .await;
    });
    tokio::time::timeout(Duration::from_secs(10), rx.recv())
        .await
        .unwrap()
        .unwrap();
    cancel.cancel();
    task.await.unwrap();
    db.finish().await;
}

#[tokio::test]
async fn listener_reconnect_triggers_wakeup() {
    let db = TestDb::new().await;
    let waker = QueueWaker::spawn(db.pool.clone(), &[Queue::PrReview]);
    let notify = waker.notify_for(Queue::PrReview);
    tokio::time::timeout(Duration::from_secs(5), notify.notified())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    // Kill the listener's backend: the waker must reconnect and wake the consumer.
    sqlx::query(
        "SELECT pg_terminate_backend(pid) FROM pg_stat_activity
         WHERE datname = current_database() AND pid <> pg_backend_pid()
           AND query LIKE 'LISTEN%'",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(15), notify.notified())
        .await
        .unwrap();
    waker.shutdown().await;
    db.finish().await;
}
