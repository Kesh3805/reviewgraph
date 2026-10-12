//! Job queue port and pure queue policy (PIPE-001).
//!
//! `PgJobQueue` (sqlx implementation of [`JobQueue`]) is pending; the table is in
//! `migrations/*_jobs.sql`.

pub mod backoff;
pub mod model;

pub use backoff::Backoff;
pub use model::{FailKind, FailOutcome, Job, JobError, JobQueue, NewJob, Queue};
