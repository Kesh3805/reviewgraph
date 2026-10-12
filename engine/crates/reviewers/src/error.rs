//! Error types for this crate (DOM-002, REV-001). See docs/architecture/error-handling.md.

use model_gateway::GatewayError;
use review_core::{Classify, CoreError, ErrorClass};

/// Errors returned by this crate. Add domain variants as the crate gains behaviour, and keep
/// the [`Classify`] mapping exhaustive.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error(transparent)]
    Core(#[from] CoreError),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

impl Classify for Error {
    fn class(&self) -> ErrorClass {
        match self {
            Error::Core(e) => e.class(),
        }
    }
}

/// Why a reviewer call failed. Reviewers never panic on model output.
#[derive(Debug, thiserror::Error)]
pub enum ReviewerError {
    #[error("model gateway: {0}")]
    Gateway(#[from] GatewayError),
    #[error("invalid context: {0}")]
    InvalidContext(String),
    /// A prompt or schema asset is broken (caught by the startup self-test and unit tests).
    #[error("prompt asset: {0}")]
    Prompt(String),
    #[error("cancelled")]
    Cancelled,
}

impl ReviewerError {
    /// Stable class for `reviewer_runs.error_class` and metrics.
    pub fn class(&self) -> &'static str {
        match self {
            Self::Gateway(GatewayError::SchemaViolation { .. }) => "structured_output_failure",
            Self::Gateway(GatewayError::Permanent {
                kind: model_gateway::PermanentKind::ReplayMiss,
                ..
            }) => "replay_miss",
            Self::Gateway(e) => e.class(),
            Self::InvalidContext(_) => "invalid_context",
            Self::Prompt(_) => "prompt_asset",
            Self::Cancelled => "cancelled",
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
        assert_bounds::<ReviewerError>();
    }
}
