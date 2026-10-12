//! Job queue port and pure queue policy (PIPE-001) and the PostgreSQL implementation
//! ([`PgJobQueue`]).
//!
//! The table is created by `migrations/20261008100000_jobs.sql` (API-007); the claim SQL is shared
//! with the TypeScript adapter (`apps/api/src/jobs/pg-job-queue.ts`), see
//! `docs/architecture/job-transport.md`.

pub mod backoff;
pub mod model;
pub mod pg;

pub use backoff::Backoff;
pub use model::{
    CancelPredicate, FailKind, FailOutcome, Job, JobError, JobQueue, NewJob, Queue,
    MAX_RATE_LIMIT_REQUEUES,
};
pub use pg::{cancel_where_tx, enqueue_tx, PgJobQueue};
