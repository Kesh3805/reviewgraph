//! Error type for this crate (DOM-002). See docs/architecture/error-handling.md.

use review_core::{Classify, CoreError, ErrorClass};

/// Errors returned by this crate.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error(transparent)]
    Core(#[from] CoreError),
    /// `init` was already called in this process.
    #[error("telemetry is already initialized in this process")]
    AlreadyInitialized,
    /// A configuration value was malformed. The message never contains header values.
    #[error("invalid telemetry config {field}: {reason}")]
    InvalidConfig { field: &'static str, reason: String },
    /// An OTLP exporter or provider could not be constructed.
    #[error("telemetry exporter setup failed: {0}")]
    Exporter(String),
    /// The global tracing subscriber was already set by someone else.
    #[error("global tracing subscriber is already set")]
    SubscriberAlreadySet,
}

/// Alias used by the public API (`TelemetryError::AlreadyInitialized`).
pub type TelemetryError = Error;

pub type Result<T, E = Error> = std::result::Result<T, E>;

impl Classify for Error {
    fn class(&self) -> ErrorClass {
        match self {
            Error::Core(e) => e.class(),
            Error::AlreadyInitialized | Error::SubscriberAlreadySet => ErrorClass::Conflict,
            Error::InvalidConfig { .. } => ErrorClass::InvalidInput,
            Error::Exporter(_) => ErrorClass::Internal,
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

    #[test]
    fn already_initialized_is_a_conflict() {
        assert_eq!(Error::AlreadyInitialized.class(), ErrorClass::Conflict);
    }
}
