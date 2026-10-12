//! Lease reaper and queue depth sampling (PIPE-002).
//!
//! A crashed worker leaves jobs in `running`. Every worker runs the reaper (safe, because the
//! batch is selected with `SKIP LOCKED`): expired leases go back to `queued`, or to `dead` once
//! `max_attempts` is reached. Re-queueing refunds nothing, so a poison-pill job that always
//! crashes its worker ends dead after `max_attempts`.

use std::time::Duration;

use sqlx::Row;
use tokio_util::sync::CancellationToken;

use super::model::{JobError, Queue};
use super::pg::PgJobQueue;

const REAP_SQL: &str = "UPDATE jobs
  SET state = CASE WHEN attempts >= max_attempts THEN 'dead' ELSE 'queued' END,
      locked_by = NULL, locked_until = NULL, last_error = 'lease_expired',
      run_after = CASE WHEN attempts >= max_attempts THEN run_after
                       ELSE now() + interval '5 seconds' END
  WHERE id IN (SELECT id FROM jobs
               WHERE state = 'running' AND locked_until < now()
               ORDER BY locked_until
               FOR UPDATE SKIP LOCKED LIMIT 100)
  RETURNING id::text, queue, state";

const DEPTH_SQL: &str = "SELECT queue, state, count(*)::bigint AS n FROM jobs GROUP BY 1, 2";

/// One job returned to the queue (or dead-lettered) by the reaper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reaped {
    pub id: String,
    pub queue: Queue,
    pub dead: bool,
}

/// Re-queues expired leases and notifies the queues that got work back. Returns what was reaped.
pub async fn reap_expired(queue: &PgJobQueue) -> Result<Vec<Reaped>, JobError> {
    let mut tx = queue.worker_tx().await?;
    let rows = sqlx::query(REAP_SQL).fetch_all(&mut *tx).await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        let id: String = row.try_get("id")?;
        let queue_name: String = row.try_get("queue")?;
        let state: String = row.try_get("state")?;
        let Some(q) = Queue::parse(&queue_name) else {
            continue;
        };
        let dead = state == "dead";
        if !dead {
            sqlx::query("SELECT pg_notify($1, $2)")
                .bind(q.channel())
                .bind(&id)
                .execute(&mut *tx)
                .await?;
        }
        out.push(Reaped { id, queue: q, dead });
    }
    tx.commit().await?;
    for r in &out {
        if r.dead {
            // Id and queue only, never the payload.
            tracing::warn!(job_id = %r.id, queue = r.queue.as_str(), "job_dead_after_lease_expiry");
        }
    }
    Ok(out)
}

/// `(queue, state, count)` for every non-empty group; sampled every 30 s for the depth gauge.
pub async fn sample_queue_depth(
    queue: &PgJobQueue,
) -> Result<Vec<(String, String, i64)>, JobError> {
    let mut tx = queue.worker_tx().await?;
    let rows = sqlx::query(DEPTH_SQL).fetch_all(&mut *tx).await?;
    tx.commit().await?;
    rows.iter()
        .map(|r| {
            Ok((
                r.try_get::<String, _>("queue")?,
                r.try_get::<String, _>("state")?,
                r.try_get::<i64, _>("n")?,
            ))
        })
        .collect()
}

/// Periodic reaper. A reaper error is logged and retried on the next tick.
#[derive(Debug, Clone)]
pub struct Reaper {
    queue: PgJobQueue,
    interval: Duration,
    depth_interval: Duration,
}

impl Reaper {
    pub fn new(queue: PgJobQueue) -> Self {
        Self {
            queue,
            interval: Duration::from_secs(15),
            depth_interval: Duration::from_secs(30),
        }
    }

    pub fn with_interval(mut self, interval: Duration) -> Self {
        self.interval = interval;
        self
    }

    /// Runs until `cancel` fires.
    pub async fn run(self, cancel: CancellationToken) {
        let mut since_depth = Duration::ZERO;
        loop {
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = tokio::time::sleep(self.interval) => {}
            }
            match reap_expired(&self.queue).await {
                Ok(reaped) if !reaped.is_empty() => {
                    tracing::info!(count = reaped.len(), "jobs_reaped");
                }
                Ok(_) => {}
                Err(e) => tracing::warn!(error = %e, "reaper_failed"),
            }
            since_depth += self.interval;
            if since_depth >= self.depth_interval {
                since_depth = Duration::ZERO;
                match sample_queue_depth(&self.queue).await {
                    Ok(depth) => {
                        for (q, state, n) in depth {
                            tracing::debug!(queue = %q, state = %state, depth = n, "queue_depth");
                        }
                    }
                    Err(e) => tracing::warn!(error = %e, "queue_depth_sample_failed"),
                }
            }
        }
    }
}
