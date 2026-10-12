//! Error type for this crate (DOM-002). See docs/architecture/error-handling.md.

use review_core::{Classify, CoreError, ErrorClass};

use crate::embedding::EmbedError;
use crate::qdrant::QdrantError;

/// Errors returned by this crate. Keep the [`Classify`] mapping exhaustive.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error(transparent)]
    Core(#[from] CoreError),
    #[error(transparent)]
    Embed(#[from] EmbedError),
    #[error(transparent)]
    Qdrant(#[from] QdrantError),
    /// Invalid configuration (provider, dims, missing key).
    #[error("invalid semantic configuration: {0}")]
    Config(String),
    /// A request argument is out of range.
    #[error("invalid semantic request: {0}")]
    InvalidInput(String),
    /// A write or read fell outside the caller's [`crate::TenantScope`]. A bug signal.
    #[error("tenant scope violation: {0}")]
    ScopeViolation(String),
    /// An existing collection has different dimensions than the configured space.
    #[error("collection {collection} has {actual} dims, the embedding space has {expected}")]
    SpaceMismatch {
        collection: String,
        expected: u16,
        actual: u64,
    },
    /// No collection is active yet, or the registry refused a transition.
    #[error("collection registry: {0}")]
    Registry(String),
    /// The collection registry database failed.
    #[error("collection registry database: {0}")]
    Database(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Alias used by the public search API (SEM-005).
pub type SemanticError = Error;

impl Classify for Error {
    fn class(&self) -> ErrorClass {
        match self {
            Error::Core(e) => e.class(),
            Error::Embed(e) => e.class(),
            Error::Qdrant(e) => e.class(),
            Error::Config(_) | Error::InvalidInput(_) => ErrorClass::InvalidInput,
            Error::ScopeViolation(_) => ErrorClass::Internal,
            Error::SpaceMismatch { .. } => ErrorClass::Permanent,
            Error::Registry(_) => ErrorClass::Conflict,
            Error::Database(_) => ErrorClass::Transient,
        }
    }
}

#[cfg(feature = "pg")]
impl From<sqlx::Error> for Error {
    fn from(e: sqlx::Error) -> Self {
        Error::Database(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_library_error_wraps_core_error() {
        let e = Error::from(CoreError::OutOfRange {
            field: "x",
            value: "1".into(),
        });
        assert_eq!(e.class(), ErrorClass::InvalidInput);
    }

    #[test]
    fn error_is_send_sync_static() {
        fn assert_bounds<T: Send + Sync + 'static>() {}
        assert_bounds::<Error>();
    }

    #[test]
    fn embed_errors_keep_their_retry_class() {
        assert!(Error::from(EmbedError::Transient("x".into()))
            .class()
            .is_retryable());
        assert!(!Error::from(EmbedError::Permanent("x".into()))
            .class()
            .is_retryable());
    }
}
