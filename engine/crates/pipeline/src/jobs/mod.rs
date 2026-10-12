//! Job queue port and pure queue policy (PIPE-001), the PostgreSQL implementation
//! ([`PgJobQueue`]) and the lease reaper, wake-up and claim loop (PIPE-002).
//!
//! The table is created by `migrations/20261008100000_jobs.sql` (API-007); the claim SQL is shared
//! with the TypeScript adapter (`apps/api/src/jobs/pg-job-queue.ts`), see
//! `docs/architecture/job-transport.md`.

pub mod backoff;
pub mod claim_loop;
pub mod model;
pub mod pg;
pub mod reaper;
pub mod waker;

pub use backoff::Backoff;
pub use claim_loop::{claim_loop, ClaimLoopConfig};
pub use model::{
    CancelPredicate, FailKind, FailOutcome, Job, JobError, JobQueue, NewJob, Queue,
    MAX_RATE_LIMIT_REQUEUES,
};
pub use pg::{cancel_where_tx, enqueue_tx, PgJobQueue};
pub use reaper::{reap_expired, sample_queue_depth, Reaped, Reaper};
pub use waker::QueueWaker;
