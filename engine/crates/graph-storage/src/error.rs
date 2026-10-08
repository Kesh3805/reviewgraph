//! Error type for this crate (DOM-002). See docs/architecture/error-handling.md.

use review_core::ids::SnapshotId;
use review_core::{Classify, CoreError, ErrorClass};

use crate::status::SnapshotStatus;

/// Errors returned by this crate. Add domain variants as the crate gains behaviour, and keep
/// the [`Classify`] mapping exhaustive.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error(transparent)]
    Core(#[from] CoreError),
    /// The [`GraphStore`] port failed. Kept separate so `Error` stays the crate's generic
    /// surface while adapters and callers match on the typed port errors.
    #[error(transparent)]
    Store(#[from] StoreError),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

impl Classify for Error {
    fn class(&self) -> ErrorClass {
        match self {
            Error::Core(e) => e.class(),
            Error::Store(e) => e.class(),
        }
    }
}

/// Every failure mode of the [`GraphStore`](crate::GraphStore) port (GS-001).
///
/// Adapters must never return a partially written snapshot as `Ready`: a failed `write_*`
/// leaves the snapshot in `Persisting` and the caller transitions it to `Failed`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StoreError {
    /// The id does not exist *or* belongs to another tenant. Never an existence oracle.
    #[error("snapshot {0} was not found in this scope")]
    NotFound(SnapshotId),

    #[error("snapshot {id} is {found}; expected {expected}")]
    InvalidStatus {
        id: SnapshotId,
        expected: SnapshotStatus,
        found: SnapshotStatus,
    },

    #[error("graph schema version {found} is not supported by this build (expected {expected})")]
    SchemaMismatch { found: u32, expected: u32 },

    #[error("delta chain of snapshot {id} is broken")]
    ChainBroken { id: SnapshotId },

    /// Lost a compare-and-swap or hit a duplicate-ready-fingerprint index.
    #[error("conflict: {0}")]
    Conflict(String),

    /// Stored bytes or rows do not match the manifest/schema.
    #[error("integrity: {0}")]
    Integrity(String),

    /// The request cannot be satisfied as shaped (wrong kind/base combination, bad path).
    #[error("invalid request: {0}")]
    InvalidRequest(String),

    /// Driver-level failure. Retryability follows [`Classify`]: serialization failures,
    /// deadlocks, statement timeouts and connection errors are `Transient`, everything else
    /// is `Internal`.
    ///
    /// Boxed rather than `anyhow::Error`: library crates keep typed errors (DOM-002), and the
    /// boxed source still lets [`Classify`] downcast to the driver error.
    #[error("backend: {0}")]
    Backend(#[source] BackendError),
}

/// The type-erased driver error carried by [`StoreError::Backend`].
pub type BackendError = Box<dyn std::error::Error + Send + Sync + 'static>;

impl StoreError {
    /// Wraps any driver-level error as [`StoreError::Backend`].
    pub fn backend(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        StoreError::Backend(Box::new(error))
    }

    /// A backend failure described only by a message.
    pub fn backend_msg(message: impl Into<String>) -> Self {
        StoreError::Backend(message.into().into())
    }
}

impl From<sqlx::Error> for StoreError {
    fn from(error: sqlx::Error) -> Self {
        StoreError::backend(error)
    }
}

impl Classify for StoreError {
    fn class(&self) -> ErrorClass {
        match self {
            StoreError::NotFound(_) => ErrorClass::NotFound,
            StoreError::InvalidStatus { .. } => ErrorClass::Conflict,
            StoreError::SchemaMismatch { .. } => ErrorClass::Permanent,
            StoreError::ChainBroken { .. } => ErrorClass::Permanent,
            StoreError::Conflict(_) => ErrorClass::Conflict,
            StoreError::Integrity(_) => ErrorClass::Permanent,
            StoreError::InvalidRequest(_) => ErrorClass::InvalidInput,
            StoreError::Backend(e) => {
                if is_retryable_backend(e) {
                    ErrorClass::Transient
                } else {
                    ErrorClass::Internal
                }
            }
        }
    }
}

/// SQLSTATEs worth retrying: serialization failures, deadlocks, statement timeouts and
/// connection-class errors.
fn is_retryable_backend(error: &BackendError) -> bool {
    let Some(sqlx) = error.downcast_ref::<sqlx::Error>() else {
        return false;
    };
    match sqlx {
        sqlx::Error::Io(_) | sqlx::Error::PoolTimedOut | sqlx::Error::PoolClosed => true,
        sqlx::Error::Database(db) => db.code().is_some_and(|code| is_retryable_code(&code)),
        _ => false,
    }
}

/// `40001` serialization failure, `40P01` deadlock, `57014` statement timeout, `08***`
/// connection errors.
fn is_retryable_code(code: &str) -> bool {
    code == "40001" || code == "40P01" || code == "57014" || code.starts_with("08")
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
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
        assert_bounds::<StoreError>();
    }

    #[test]
    fn store_error_classes_are_stable() {
        let id = SnapshotId::new();
        let cases = [
            (StoreError::NotFound(id), ErrorClass::NotFound),
            (
                StoreError::InvalidStatus {
                    id,
                    expected: SnapshotStatus::Persisting,
                    found: SnapshotStatus::Ready,
                },
                ErrorClass::Conflict,
            ),
            (
                StoreError::SchemaMismatch {
                    found: 2,
                    expected: 1,
                },
                ErrorClass::Permanent,
            ),
            (StoreError::ChainBroken { id }, ErrorClass::Permanent),
            (
                StoreError::Conflict("duplicate fingerprint".into()),
                ErrorClass::Conflict,
            ),
            (
                StoreError::Integrity("hash mismatch".into()),
                ErrorClass::Permanent,
            ),
            (
                StoreError::InvalidRequest("bad kind".into()),
                ErrorClass::InvalidInput,
            ),
        ];
        for (error, expected) in cases {
            assert_eq!(error.class(), expected, "{error}");
        }
        assert_eq!(
            Error::from(StoreError::NotFound(id)).class(),
            ErrorClass::NotFound
        );
    }

    #[test]
    fn backend_errors_classify_by_sqlstate() {
        for code in ["40001", "40P01", "57014", "08006", "08001"] {
            assert!(is_retryable_code(code), "{code} must be retryable");
        }
        for code in ["23505", "23503", "42601", "53300"] {
            assert!(!is_retryable_code(code), "{code} must not be retryable");
        }
        let boxed = |e: sqlx::Error| -> BackendError { Box::new(e) };
        assert!(is_retryable_backend(&boxed(sqlx::Error::PoolTimedOut)));
        assert!(!is_retryable_backend(&boxed(sqlx::Error::RowNotFound)));
        assert!(!is_retryable_backend(&BackendError::from(
            "not a driver error"
        )));
        let classified = StoreError::backend(sqlx::Error::PoolTimedOut).class();
        assert_eq!(classified, ErrorClass::Transient);
        assert_eq!(
            StoreError::backend_msg("boom").class(),
            ErrorClass::Internal
        );
    }

    #[test]
    fn store_error_display_is_readable() {
        let id = SnapshotId::new();
        assert_eq!(
            StoreError::NotFound(id).to_string(),
            format!("snapshot {id} was not found in this scope")
        );
        assert_eq!(
            StoreError::InvalidStatus {
                id,
                expected: SnapshotStatus::Ready,
                found: SnapshotStatus::Pending,
            }
            .to_string(),
            format!("snapshot {id} is pending; expected ready")
        );
    }
}
