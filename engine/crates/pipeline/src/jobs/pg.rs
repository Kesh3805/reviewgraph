//! PostgreSQL implementation of [`JobQueue`] over the shared `jobs` table (PIPE-001).
//!
//! Every statement here is the Rust twin of the TypeScript adapter
//! (`apps/api/src/jobs/pg-job-queue.ts`) and of the claim statement in target-architecture
//! section 5: both languages claim with `FOR UPDATE SKIP LOCKED`, fence heartbeat, complete and
//! fail by `(locked_by, attempts)`, and refund the attempt for rate limits and shutdown releases.
//! Worker transactions opt in to cross-tenant access with `app.job_worker = 'on'` (SEC-001).

use std::time::Duration;

use sqlx::postgres::PgRow;
use sqlx::{PgConnection, PgPool, Row};
use uuid::Uuid;

use super::backoff::Backoff;
use super::model::{
    CancelPredicate, FailKind, FailOutcome, Job, JobError, JobQueue, NewJob, Queue,
    MAX_RATE_LIMIT_REQUEUES,
};

const MAX_PAYLOAD_BYTES: usize = 16 * 1024;
const MAX_LAST_ERROR_CHARS: usize = 2000;

const INSERT_SQL: &str = "INSERT INTO jobs
    (organization_id, queue, idempotency_key, payload, priority, max_attempts, trace_parent,
     run_after)
  VALUES ($1::uuid, $2, $3, $4::jsonb, $5, $6, $7, now() + $8::float8 * interval '1 millisecond')
  ON CONFLICT (idempotency_key) DO NOTHING
  RETURNING id::text";

/// The claim statement of target-architecture section 5 (shared with the TypeScript adapter).
const CLAIM_SQL: &str = "UPDATE jobs SET state = 'running', locked_by = $1,
         locked_until = now() + $2::float8 * interval '1 millisecond',
         attempts = attempts + 1
  WHERE id = (SELECT id FROM jobs
              WHERE queue = ANY($3::text[]) AND state = 'queued' AND run_after <= now()
              ORDER BY priority DESC, created_at
              FOR UPDATE SKIP LOCKED LIMIT 1)
  RETURNING id::text, queue, organization_id::text, payload::text, attempts, max_attempts,
            trace_parent";

const HEARTBEAT_SQL: &str = "UPDATE jobs
  SET locked_until = now() + $4::float8 * interval '1 millisecond'
  WHERE id = $1::uuid AND locked_by = $2 AND attempts = $3 AND state = 'running'";

const COMPLETE_SQL: &str = "UPDATE jobs
  SET state = 'succeeded', locked_by = NULL, locked_until = NULL, last_error = NULL
  WHERE id = $1::uuid AND locked_by = $2 AND attempts = $3 AND state = 'running'";

const FAIL_SQL: &str = "UPDATE jobs SET
    state = CASE WHEN $4 OR attempts >= max_attempts THEN 'dead' ELSE 'queued' END,
    run_after = CASE WHEN $4 OR attempts >= max_attempts THEN run_after
                     ELSE now() + $5::float8 * interval '1 millisecond' END,
    locked_by = NULL, locked_until = NULL, last_error = $6
  WHERE id = $1::uuid AND locked_by = $2 AND attempts = $3 AND state = 'running'
  RETURNING state";

/// A rate limit says nothing about the job itself: the attempt is refunded.
const FAIL_RATE_LIMITED_SQL: &str = "UPDATE jobs SET
    state = CASE WHEN rate_limit_requeues + 1 >= $4 THEN 'dead' ELSE 'queued' END,
    attempts = greatest(attempts - 1, 0),
    rate_limit_requeues = rate_limit_requeues + 1,
    run_after = now() + $5::float8 * interval '1 millisecond',
    locked_by = NULL, locked_until = NULL, last_error = 'rate_limited'
  WHERE id = $1::uuid AND locked_by = $2 AND attempts = $3 AND state = 'running'
  RETURNING state";

const RELEASE_SQL: &str = "UPDATE jobs
  SET state = 'queued', locked_by = NULL, locked_until = NULL,
      attempts = greatest(attempts - 1, 0), last_error = 'released_on_shutdown'
  WHERE locked_by = $1 AND state = 'running'
  RETURNING id::text, queue";

const CANCEL_SQL: &str = "UPDATE jobs SET state = 'cancelled'
  WHERE state = 'queued'
    AND ($1::text IS NULL OR queue = $1)
    AND ($2::text[] IS NULL OR payload ->> 'review_run_id' = ANY($2))
    AND ($3::text IS NULL OR starts_with(idempotency_key, $3))";

fn millis(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// Uniform `[0, 1]` jitter from a random UUID (no extra randomness dependency).
fn jitter() -> f64 {
    ((Uuid::new_v4().as_u128() >> 64) as u64) as f64 / u64::MAX as f64
}

fn truncate_error(error: &str) -> String {
    error.chars().take(MAX_LAST_ERROR_CHARS).collect()
}

fn job_from_row(row: &PgRow, worker_id: &str) -> Result<Job, JobError> {
    let queue_name: String = row.try_get("queue")?;
    let queue = Queue::parse(&queue_name)
        .ok_or_else(|| JobError::Invalid(format!("unknown queue {queue_name}")))?;
    Ok(Job {
        id: row.try_get("id")?,
        queue,
        organization_id: row.try_get("organization_id")?,
        payload: row.try_get("payload")?,
        trace_parent: row.try_get("trace_parent")?,
        attempts: row.try_get("attempts")?,
        max_attempts: row.try_get("max_attempts")?,
        locked_by: worker_id.to_owned(),
    })
}

fn validate(job: &NewJob) -> Result<(), JobError> {
    if job.payload.len() > MAX_PAYLOAD_BYTES {
        return Err(JobError::Invalid("payload exceeds 16 KiB".into()));
    }
    match serde_json::from_str::<serde_json::Value>(&job.payload) {
        Ok(serde_json::Value::Object(_)) => {}
        _ => return Err(JobError::Invalid("payload must be a JSON object".into())),
    }
    if !(1..=20).contains(&job.max_attempts) {
        return Err(JobError::Invalid("max_attempts must be 1..=20".into()));
    }
    if job.idempotency_key.is_empty() {
        return Err(JobError::Invalid("idempotency_key is empty".into()));
    }
    Ok(())
}

/// Enqueues inside the caller's transaction, so the job exists only if the business change
/// commits. Returns `(job id, created)`. A duplicate idempotency key returns the existing job
/// with `created = false` and sends no notification. The notification is delivered by Postgres
/// on commit. The caller's transaction must be allowed to see the row (tenant set, or
/// `app.job_worker`) for the duplicate lookup.
pub async fn enqueue_tx(conn: &mut PgConnection, job: &NewJob) -> Result<(String, bool), JobError> {
    validate(job)?;
    let inserted: Option<String> = sqlx::query_scalar(INSERT_SQL)
        .bind(&job.organization_id)
        .bind(job.queue.as_str())
        .bind(&job.idempotency_key)
        .bind(&job.payload)
        .bind(job.priority)
        .bind(job.max_attempts)
        .bind(&job.trace_parent)
        .bind(job.delay.map_or(0.0, millis))
        .fetch_optional(&mut *conn)
        .await?;
    match inserted {
        Some(id) => {
            sqlx::query("SELECT pg_notify($1, $2)")
                .bind(job.queue.channel())
                .bind(&id)
                .execute(&mut *conn)
                .await?;
            Ok((id, true))
        }
        None => {
            let existing: Option<String> =
                sqlx::query_scalar("SELECT id::text FROM jobs WHERE idempotency_key = $1")
                    .bind(&job.idempotency_key)
                    .fetch_optional(&mut *conn)
                    .await?;
            existing
                .map(|id| (id, false))
                .ok_or_else(|| JobError::Db("duplicate job is not visible to this tenant".into()))
        }
    }
}

/// Cancels matching `queued` jobs in the caller's transaction (SUP-001 uses this inside the
/// supersession transaction).
pub async fn cancel_where_tx(
    conn: &mut PgConnection,
    predicate: &CancelPredicate,
) -> Result<u64, JobError> {
    if predicate.is_empty() {
        return Err(JobError::Invalid(
            "cancel_where needs at least one selector".into(),
        ));
    }
    if predicate
        .review_run_ids
        .as_ref()
        .is_some_and(|ids| ids.is_empty())
    {
        return Ok(0);
    }
    let done = sqlx::query(CANCEL_SQL)
        .bind(predicate.queue.map(Queue::as_str))
        .bind(&predicate.review_run_ids)
        .bind(&predicate.key_prefix)
        .execute(conn)
        .await?;
    Ok(done.rows_affected())
}

/// The PostgreSQL job queue.
#[derive(Debug, Clone)]
pub struct PgJobQueue {
    pool: PgPool,
    backoff: Backoff,
}

impl PgJobQueue {
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            backoff: Backoff::default(),
        }
    }

    pub fn with_backoff(mut self, backoff: Backoff) -> Self {
        self.backoff = backoff;
        self
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// A transaction with the transaction-local worker opt-in the `jobs` RLS policy requires.
    pub(crate) async fn worker_tx(
        &self,
    ) -> Result<sqlx::Transaction<'static, sqlx::Postgres>, JobError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT set_config('app.job_worker', 'on', true)")
            .execute(&mut *tx)
            .await?;
        Ok(tx)
    }
}

impl JobQueue for PgJobQueue {
    async fn enqueue(&self, job: NewJob) -> Result<(String, bool), JobError> {
        let mut tx = self.worker_tx().await?;
        let out = enqueue_tx(&mut tx, &job).await?;
        tx.commit().await?;
        tracing::debug!(queue = job.queue.as_str(), created = out.1, "job_enqueue");
        Ok(out)
    }

    async fn claim(
        &self,
        worker_id: &str,
        queues: &[Queue],
        lease: Duration,
    ) -> Result<Option<Job>, JobError> {
        let names: Vec<&str> = queues.iter().map(|q| q.as_str()).collect();
        let mut tx = self.worker_tx().await?;
        let row = sqlx::query(CLAIM_SQL)
            .bind(worker_id)
            .bind(millis(lease))
            .bind(&names)
            .fetch_optional(&mut *tx)
            .await?;
        tx.commit().await?;
        row.map(|r| job_from_row(&r, worker_id)).transpose()
    }

    async fn heartbeat(&self, job: &Job, lease: Duration) -> Result<bool, JobError> {
        let mut tx = self.worker_tx().await?;
        let done = sqlx::query(HEARTBEAT_SQL)
            .bind(&job.id)
            .bind(&job.locked_by)
            .bind(job.attempts)
            .bind(millis(lease))
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(done.rows_affected() > 0)
    }

    async fn complete(&self, job: &Job) -> Result<(), JobError> {
        let mut tx = self.worker_tx().await?;
        let done = sqlx::query(COMPLETE_SQL)
            .bind(&job.id)
            .bind(&job.locked_by)
            .bind(job.attempts)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        if done.rows_affected() == 0 {
            return Err(JobError::LeaseLost);
        }
        Ok(())
    }

    async fn fail(&self, job: &Job, kind: FailKind, error: &str) -> Result<FailOutcome, JobError> {
        let mut tx = self.worker_tx().await?;
        let state: Option<String> = match &kind {
            FailKind::RateLimited { retry_after } => {
                sqlx::query_scalar(FAIL_RATE_LIMITED_SQL)
                    .bind(&job.id)
                    .bind(&job.locked_by)
                    .bind(job.attempts)
                    .bind(MAX_RATE_LIMIT_REQUEUES)
                    .bind(millis(*retry_after))
                    .fetch_optional(&mut *tx)
                    .await?
            }
            FailKind::Transient | FailKind::Permanent => {
                let attempts = u32::try_from(job.attempts).unwrap_or(1);
                let delay = self.backoff.delay(attempts, jitter());
                sqlx::query_scalar(FAIL_SQL)
                    .bind(&job.id)
                    .bind(&job.locked_by)
                    .bind(job.attempts)
                    .bind(kind == FailKind::Permanent)
                    .bind(millis(delay))
                    .bind(truncate_error(error))
                    .fetch_optional(&mut *tx)
                    .await?
            }
        };
        tx.commit().await?;
        match state.as_deref() {
            Some("dead") => {
                tracing::warn!(job_id = %job.id, queue = job.queue.as_str(), "job_dead");
                Ok(FailOutcome::Dead)
            }
            Some(_) => Ok(FailOutcome::Requeued),
            None => Err(JobError::LeaseLost),
        }
    }

    async fn release_worker(&self, worker_id: &str) -> Result<u64, JobError> {
        let mut tx = self.worker_tx().await?;
        let rows = sqlx::query(RELEASE_SQL)
            .bind(worker_id)
            .fetch_all(&mut *tx)
            .await?;
        for row in &rows {
            let id: String = row.try_get("id")?;
            let queue: String = row.try_get("queue")?;
            sqlx::query("SELECT pg_notify($1, $2)")
                .bind(format!("jobs_{queue}"))
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(rows.len() as u64)
    }

    async fn cancel_where(&self, predicate: CancelPredicate) -> Result<u64, JobError> {
        let mut tx = self.worker_tx().await?;
        let n = cancel_where_tx(&mut tx, &predicate).await?;
        tx.commit().await?;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_job(payload: &str) -> NewJob {
        NewJob {
            organization_id: Uuid::nil().to_string(),
            queue: Queue::PrReview,
            idempotency_key: "k".into(),
            payload: payload.into(),
            priority: 0,
            max_attempts: 5,
            trace_parent: None,
            delay: None,
        }
    }

    #[test]
    fn payload_must_be_a_small_json_object() {
        assert!(validate(&new_job(r#"{"review_run_id":"x"}"#)).is_ok());
        assert!(validate(&new_job("[]")).is_err());
        assert!(validate(&new_job("not json")).is_err());
        let big = format!(r#"{{"a":"{}"}}"#, "x".repeat(MAX_PAYLOAD_BYTES));
        assert!(validate(&new_job(&big)).is_err());
    }

    #[test]
    fn max_attempts_bounds() {
        let mut j = new_job("{}");
        j.max_attempts = 0;
        assert!(validate(&j).is_err());
        j.max_attempts = 21;
        assert!(validate(&j).is_err());
    }

    #[test]
    fn jitter_is_unit_interval() {
        for _ in 0..1000 {
            let j = jitter();
            assert!((0.0..=1.0).contains(&j));
        }
    }

    #[test]
    fn error_text_is_truncated_on_char_boundaries() {
        let long = "é".repeat(5000);
        assert_eq!(truncate_error(&long).chars().count(), MAX_LAST_ERROR_CHARS);
    }
}
