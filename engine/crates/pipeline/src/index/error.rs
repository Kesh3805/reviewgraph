//! Errors of an index run (IDX-001). Messages carry paths and counts only, never source text.

use graph_storage::convert::ConvertError;
use graph_storage::StoreError;

use super::tolerance::ToleranceExceeded;

/// Why a full index failed. Every variant except [`IndexError::Store`] is terminal.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum IndexError {
    #[error("repository walk failed: {0}")]
    Walk(String),
    #[error("graph store failed: {0}")]
    Store(#[from] StoreError),
    #[error("graph could not be built: {0}")]
    Build(String),
    #[error("graph is inconsistent: {errors} validation errors")]
    Inconsistent { errors: usize },
    #[error(transparent)]
    ToleranceExceeded(#[from] ToleranceExceeded),
    #[error("graph conversion failed: {0}")]
    Convert(#[from] ConvertError),
    #[error("index cancelled")]
    Cancelled,
    #[error("worker failed: {0}")]
    Worker(String),
}

impl IndexError {
    /// True when a job runner should retry with backoff; everything else goes straight to the
    /// dead-letter state (IDX-004 failure behaviour).
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::Store(StoreError::Backend(_) | StoreError::Conflict(_))
        )
    }

    /// Short stable code recorded on the failed snapshot.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Walk(_) => "walk_failed",
            Self::Store(_) => "store_failed",
            Self::Build(_) => "build_failed",
            Self::Inconsistent { .. } => "inconsistent",
            Self::ToleranceExceeded(_) => "tolerance_exceeded",
            Self::Convert(_) => "convert_failed",
            Self::Cancelled => "cancelled",
            Self::Worker(_) => "worker_failed",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_backend_and_conflict_store_errors_retry() {
        assert!(IndexError::Store(StoreError::Conflict("x".into())).is_retryable());
        assert!(!IndexError::Store(StoreError::InvalidRequest("x".into())).is_retryable());
        assert!(!IndexError::Cancelled.is_retryable());
        assert!(!IndexError::ToleranceExceeded(ToleranceExceeded {
            failed: 1,
            total: 2
        })
        .is_retryable());
    }

    #[test]
    fn cancelled_has_the_documented_code() {
        assert_eq!(IndexError::Cancelled.code(), "cancelled");
    }
}
