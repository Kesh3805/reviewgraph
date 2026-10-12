//! Persistence of the review pipeline: stage outputs for idempotent resume (PIPE-005), run
//! claims and retries (PIPE-005), compare-and-set run state transitions (PIPE-007), and the
//! finding lifecycle store (VER-001, feature `pg`).
//!
//! Every query carries `organization_id` and runs in a transaction that sets
//! `app.organization_id`, so row-level security applies even when the connecting role does not
//! bypass it.

pub mod blob;
#[cfg(feature = "pg")]
mod finding_metrics;
#[cfg(feature = "pg")]
pub mod findings;
pub mod input_hash;
pub mod metrics;
pub mod review_state;
pub mod run_claims;
pub mod stage_outputs;

use review_core::review::ReviewState;
use review_core::{Classify, CoreError, ErrorClass};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

pub use blob::{BlobStore, MemoryBlobStore};
#[cfg(feature = "pg")]
pub use findings::PgFindingStore;
pub use input_hash::{canonical_json, hash_hex, input_hash, InputHashParts};
pub use review_state::{PgReviewStateStore, ReviewStateStore, TransitionResult};
pub use run_claims::{NewRun, RunClaims, RunRow, REUSABLE_ACROSS_RETRY};
pub use stage_outputs::{
    MemoryStageStore, NewStageOutput, PgStageStore, PutOutcome, StageStatus, StageStore,
    StoredStageOutput, SPILL_THRESHOLD_BYTES,
};

/// Errors of the persistence layer. Database and blob failures are transient (the job retries).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StoreError {
    #[error(transparent)]
    Core(#[from] CoreError),
    #[error("database error: {0}")]
    Db(String),
    #[error("blob store error: {0}")]
    Blob(String),
    #[error("not found")]
    NotFound,
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("serialization error: {0}")]
    Serde(String),
}

impl From<sqlx::Error> for StoreError {
    fn from(e: sqlx::Error) -> Self {
        Self::Db(e.to_string())
    }
}

impl From<serde_json::Error> for StoreError {
    fn from(e: serde_json::Error) -> Self {
        Self::Serde(e.to_string())
    }
}

impl Classify for StoreError {
    fn class(&self) -> ErrorClass {
        match self {
            Self::Core(e) => e.class(),
            Self::Db(_) | Self::Blob(_) => ErrorClass::Transient,
            Self::NotFound => ErrorClass::NotFound,
            Self::Conflict(_) => ErrorClass::Conflict,
            Self::Serde(_) => ErrorClass::Internal,
        }
    }
}

/// True for a unique-constraint violation (SQLSTATE 23505).
pub(crate) fn is_unique_violation(e: &sqlx::Error) -> bool {
    e.as_database_error()
        .and_then(|d| d.code())
        .is_some_and(|c| c == "23505")
}

/// A transaction scoped to one tenant (`app.organization_id`, transaction-local).
pub async fn tenant_tx(
    pool: &PgPool,
    organization_id: &Uuid,
) -> Result<Transaction<'static, Postgres>, StoreError> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.organization_id', $1, true)")
        .bind(organization_id.to_string())
        .execute(&mut *tx)
        .await?;
    Ok(tx)
}

/// Parses the wire form of a run state (`ReviewState::as_str`).
pub fn parse_state(s: &str) -> Result<ReviewState, StoreError> {
    ReviewState::ALL
        .into_iter()
        .find(|st| st.as_str() == s)
        .ok_or_else(|| StoreError::Db(format!("unknown review state {s}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_state_parses_back() {
        for s in ReviewState::ALL {
            assert_eq!(parse_state(s.as_str()).unwrap(), s);
        }
        assert!(parse_state("nope").is_err());
    }

    #[test]
    fn store_error_classes() {
        assert_eq!(StoreError::Db("x".into()).class(), ErrorClass::Transient);
        assert_eq!(StoreError::NotFound.class(), ErrorClass::NotFound);
        assert_eq!(
            StoreError::Conflict("x".into()).class(),
            ErrorClass::Conflict
        );
    }
}
