//! Error type for this crate (DOM-002). See docs/architecture/error-handling.md.

use review_core::ids::SymbolKey;
use review_core::{Classify, CoreError, ErrorClass};

/// Errors returned by this crate. Add domain variants as the crate gains behaviour, and keep
/// the [`Classify`] mapping exhaustive.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error(transparent)]
    Core(#[from] CoreError),
    /// The graph an operation needs (head, or base for removed symbols) was not supplied.
    #[error("the {0} graph is required but was not supplied")]
    GraphMissing(&'static str),
    /// A requested seed is not a node of the graph.
    #[error("seed {0} is not a node of the graph")]
    SeedNotFound(SymbolKey),
    /// A risk policy or budget configuration value is invalid.
    #[error("invalid configuration: {0}")]
    InvalidConfig(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

impl Classify for Error {
    fn class(&self) -> ErrorClass {
        match self {
            Error::Core(e) => e.class(),
            Error::GraphMissing(_) | Error::InvalidConfig(_) => ErrorClass::InvalidInput,
            Error::SeedNotFound(_) => ErrorClass::NotFound,
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
    fn domain_errors_are_classified() {
        assert_eq!(
            Error::GraphMissing("base").class(),
            ErrorClass::InvalidInput
        );
        assert_eq!(
            Error::SeedNotFound(SymbolKey::from_bytes([1; 16])).class(),
            ErrorClass::NotFound
        );
        assert_eq!(
            Error::InvalidConfig("x".into()).class(),
            ErrorClass::InvalidInput
        );
    }

    #[test]
    fn error_is_send_sync_static() {
        fn assert_bounds<T: Send + Sync + 'static>() {}
        assert_bounds::<Error>();
    }
}
