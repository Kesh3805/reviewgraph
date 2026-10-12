//! Error type for this crate (DOM-002). See docs/architecture/error-handling.md.

use review_core::{Classify, CoreError, ErrorClass};

/// Errors returned by this crate. Add domain variants as the crate gains behaviour, and keep
/// the [`Classify`] mapping exhaustive.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error(transparent)]
    Core(#[from] CoreError),
    /// Change detection between two commits failed (INC-001).
    #[error(transparent)]
    ChangeSet(#[from] crate::changeset::ChangeSetError),
    /// A blob of a changed path could not be read; a partial head graph is worse than none.
    #[error("blob read failed for {path}: {reason}")]
    BlobRead { path: String, reason: String },
    /// Two distinct symbols produced the same node key; only a full rebuild can recover.
    #[error("node key collision: {0}")]
    KeyCollision(String),
    /// An unchanged file's base object is gone, so it cannot be re-linked.
    #[error("base object missing: {0}")]
    BaseObjectMissing(String),
    /// The assembled delta failed local validation. This is a bug, not data.
    #[error("invalid delta: {0}")]
    InvalidDelta(String),
    /// The graph store failed.
    #[error("graph store: {0}")]
    Store(String),
    /// The head snapshot cannot be materialized because it is not ready.
    #[error("snapshot not ready: {0}")]
    SnapshotNotReady(String),
}

/// Name used by the task specs for this crate's error type.
pub type IncError = Error;

pub type Result<T, E = Error> = std::result::Result<T, E>;

impl Classify for Error {
    fn class(&self) -> ErrorClass {
        match self {
            Error::Core(e) => e.class(),
            Error::ChangeSet(crate::changeset::ChangeSetError::CommitNotFound(_))
            | Error::ChangeSet(crate::changeset::ChangeSetError::Io(_))
            | Error::Store(_)
            | Error::SnapshotNotReady(_) => ErrorClass::Transient,
            Error::ChangeSet(_) | Error::BlobRead { .. } | Error::BaseObjectMissing(_) => {
                ErrorClass::Permanent
            }
            Error::KeyCollision(_) | Error::InvalidDelta(_) => ErrorClass::Internal,
        }
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
}
