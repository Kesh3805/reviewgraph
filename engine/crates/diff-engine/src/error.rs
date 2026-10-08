//! Error type for this crate (DOM-002). See docs/architecture/error-handling.md.

use review_core::{Classify, CoreError, ErrorClass};

use crate::git::GitError;

/// Errors returned by this crate. Each domain error is wrapped in its own variant so callers can
/// match on the stage that failed; the [`Classify`] mapping stays exhaustive.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error(transparent)]
    Core(#[from] CoreError),
    #[error(transparent)]
    Git(#[from] GitError),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

impl Classify for Error {
    fn class(&self) -> ErrorClass {
        match self {
            Error::Core(e) => e.class(),
            Error::Git(e) => e.class(),
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
            value: "x".into(),
        });
        assert_eq!(e.class(), ErrorClass::InvalidInput);
    }

    #[test]
    fn git_errors_classify() {
        let e = Error::from(GitError::ObjectNotFound("abc".to_owned()));
        assert_eq!(e.class(), ErrorClass::NotFound);
    }

    #[test]
    fn error_is_send_sync_static() {
        fn assert_bounds<T: Send + Sync + 'static>() {}
        assert_bounds::<Error>();
    }
}
