//! Job model and the `JobQueue` port.

use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Queue {
    RepositoryIndex,
    IncrementalIndex,
    PrReview,
    ReviewPublish,
    HistoryIngest,
}

impl Queue {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RepositoryIndex => "repository-index",
            Self::IncrementalIndex => "incremental-index",
            Self::PrReview => "pr-review",
            Self::ReviewPublish => "review-publish",
            Self::HistoryIngest => "history-ingest",
        }
    }

    /// NOTIFY channel: `jobs_<queue>`.
    pub fn channel(self) -> String {
        format!("jobs_{}", self.as_str())
    }
}

/// Enqueue request. Payloads are IDs only (JSON object, at most 16 KiB).
#[derive(Debug, Clone)]
pub struct NewJob {
    pub organization_id: String,
    pub queue: Queue,
    pub idempotency_key: String,
    pub payload: String,
    pub priority: i32,
    pub max_attempts: i32,
    pub trace_parent: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Job {
    pub id: String,
    pub queue: Queue,
    pub payload: String,
    pub attempts: i32,
    pub max_attempts: i32,
    pub locked_by: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailKind {
    Transient,
    Permanent,
    RateLimited { retry_after: Duration },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailOutcome {
    Requeued,
    Dead,
}

/// Maximum rate-limit requeues before a job goes dead.
pub const MAX_RATE_LIMIT_REQUEUES: i32 = 50;

impl FailKind {
    /// Pure decision used by the SQL layer: what a failure does to a job.
    pub fn outcome(
        &self,
        attempts: i32,
        max_attempts: i32,
        rate_limit_requeues: i32,
    ) -> FailOutcome {
        match self {
            Self::Permanent => FailOutcome::Dead,
            Self::Transient if attempts >= max_attempts => FailOutcome::Dead,
            Self::Transient => FailOutcome::Requeued,
            Self::RateLimited { .. } if rate_limit_requeues + 1 > MAX_RATE_LIMIT_REQUEUES => {
                FailOutcome::Dead
            }
            Self::RateLimited { .. } => FailOutcome::Requeued,
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum JobError {
    #[error("lease lost")]
    LeaseLost,
    #[error("database error: {0}")]
    Db(String),
}

/// Durable at-least-once queue (ADR-012). `PgJobQueue` implements this over PostgreSQL with
/// `FOR UPDATE SKIP LOCKED`; complete/fail/heartbeat are fenced by `(locked_by, attempts)`.
#[allow(async_fn_in_trait)]
pub trait JobQueue {
    /// Returns `(job id, created)`; a duplicate idempotency key returns `created = false`.
    async fn enqueue(&self, job: NewJob) -> Result<(String, bool), JobError>;
    async fn claim(
        &self,
        worker_id: &str,
        queues: &[Queue],
        lease: Duration,
    ) -> Result<Option<Job>, JobError>;
    /// `false` means the lease was lost and the handler must stop.
    async fn heartbeat(&self, job: &Job, lease: Duration) -> Result<bool, JobError>;
    async fn complete(&self, job: &Job) -> Result<(), JobError>;
    async fn fail(&self, job: &Job, kind: FailKind, error: &str) -> Result<FailOutcome, JobError>;
    async fn release_worker(&self, worker_id: &str) -> Result<u64, JobError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fail_outcomes() {
        assert_eq!(FailKind::Permanent.outcome(1, 5, 0), FailOutcome::Dead);
        assert_eq!(FailKind::Transient.outcome(5, 5, 0), FailOutcome::Dead);
        assert_eq!(FailKind::Transient.outcome(2, 5, 0), FailOutcome::Requeued);
        let rl = FailKind::RateLimited {
            retry_after: Duration::from_secs(1),
        };
        assert_eq!(rl.outcome(5, 5, 0), FailOutcome::Requeued);
        assert_eq!(rl.outcome(1, 5, 50), FailOutcome::Dead);
    }

    #[test]
    fn channel_name() {
        assert_eq!(Queue::PrReview.channel(), "jobs_pr-review");
    }
}
