//! The claim loop (PIPE-002): wake-up, claim, and idle polling.
//!
//! The consumer first acquires a concurrency permit, then claims, so a worker never holds a lease
//! it has no capacity to work on. After a job it claims again straight away; when the queue is
//! empty it waits for a notification or the poll interval (default 5 s plus up to 1 s of jitter),
//! which also picks up delayed jobs and notifications that were lost.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::model::{Job, JobQueue, Queue};
use super::pg::PgJobQueue;

/// Settings of one consumer.
#[derive(Debug, Clone)]
pub struct ClaimLoopConfig {
    pub worker_id: String,
    pub queues: Vec<Queue>,
    pub lease: Duration,
    pub poll_interval: Duration,
}

impl ClaimLoopConfig {
    pub fn new(worker_id: impl Into<String>, queues: Vec<Queue>) -> Self {
        Self {
            worker_id: worker_id.into(),
            queues,
            lease: Duration::from_secs(60),
            poll_interval: Duration::from_secs(5),
        }
    }
}

fn poll_delay(base: Duration) -> Duration {
    let jitter_ms = (Uuid::new_v4().as_u128() % 1000) as u64;
    base + Duration::from_millis(jitter_ms)
}

/// Runs until `cancel` fires. `on_job` receives each claimed job with its permit and must spawn
/// the work, holding the permit until the job is finished. Cancelling stops claiming only; it
/// never interrupts a handler.
pub async fn claim_loop<F, Fut>(
    queue: &PgJobQueue,
    config: &ClaimLoopConfig,
    notify: Arc<Notify>,
    permits: Arc<Semaphore>,
    cancel: CancellationToken,
    mut on_job: F,
) where
    F: FnMut(Job, OwnedSemaphorePermit) -> Fut,
    Fut: Future<Output = ()>,
{
    loop {
        let permit = tokio::select! {
            _ = cancel.cancelled() => return,
            p = permits.clone().acquire_owned() => match p {
                Ok(p) => p,
                Err(_) => return,
            },
        };
        if cancel.is_cancelled() {
            return;
        }
        match queue
            .claim(&config.worker_id, &config.queues, config.lease)
            .await
        {
            Ok(Some(job)) => {
                on_job(job, permit).await;
                continue;
            }
            Ok(None) => {}
            Err(e) => tracing::warn!(error = %e, "job_claim_failed"),
        }
        drop(permit);
        tokio::select! {
            _ = cancel.cancelled() => return,
            _ = notify.notified() => {}
            _ = tokio::time::sleep(poll_delay(config.poll_interval)) => {}
        }
    }
}
