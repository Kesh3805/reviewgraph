//! Embedding error taxonomy (SEM-001).

use review_core::{Classify, ErrorClass};

/// Failure of one embedding call. Messages never contain input text or credentials.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EmbedError {
    /// Timeouts, connection failures and 5xx: worth retrying.
    #[error("transient embedding failure: {0}")]
    Transient(String),
    /// HTTP 429; `retry_after_ms` is the provider's hint (0 when absent).
    #[error("embedding provider rate limited (retry after {retry_after_ms} ms)")]
    RateLimited { retry_after_ms: u64 },
    /// Authentication, invalid request and other 4xx failures: never retried.
    #[error("permanent embedding failure: {0}")]
    Permanent(String),
    /// Input `index` exceeds the provider's token limit. Unit builders truncate; this is a bug
    /// signal, never retried.
    #[error("embedding input {index} exceeds the provider token limit")]
    InputTooLong { index: usize },
    /// A returned vector does not have the space's dimensionality.
    #[error("embedding dimension mismatch: expected {expected}, got {got}")]
    DimensionMismatch { expected: u16, got: usize },
}

impl EmbedError {
    /// Metric label for `embedding_errors_total{kind}`.
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Transient(_) => "transient",
            Self::RateLimited { .. } => "rate_limited",
            Self::Permanent(_) => "permanent",
            Self::InputTooLong { .. } => "input_too_long",
            Self::DimensionMismatch { .. } => "dimension_mismatch",
        }
    }

    pub const fn is_retryable(&self) -> bool {
        matches!(self, Self::Transient(_) | Self::RateLimited { .. })
    }
}

impl Classify for EmbedError {
    fn class(&self) -> ErrorClass {
        match self {
            Self::Transient(_) => ErrorClass::Transient,
            Self::RateLimited { .. } => ErrorClass::RateLimited,
            Self::Permanent(_) | Self::DimensionMismatch { .. } => ErrorClass::Permanent,
            Self::InputTooLong { .. } => ErrorClass::InvalidInput,
        }
    }
}
