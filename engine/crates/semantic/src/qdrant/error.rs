//! Qdrant client errors (SEM-003).

use review_core::{Classify, ErrorClass};

/// Failure of one Qdrant request. Bodies are Qdrant's own error text, never payload data.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum QdrantError {
    /// Connection refused, 5xx: retried.
    #[error("qdrant unavailable: {0}")]
    Unavailable(String),
    /// The request timed out: retried.
    #[error("qdrant request timed out")]
    Timeout,
    /// 400/422: our request was wrong.
    #[error("qdrant rejected the request: {body}")]
    BadRequest { body: String },
    #[error("qdrant collection or point not found")]
    NotFound,
    #[error("qdrant conflict: {0}")]
    Conflict(String),
    /// The response did not have the expected shape.
    #[error("unexpected qdrant response: {0}")]
    Protocol(String),
}

impl QdrantError {
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Unavailable(_) => "unavailable",
            Self::Timeout => "timeout",
            Self::BadRequest { .. } => "bad_request",
            Self::NotFound => "not_found",
            Self::Conflict(_) => "conflict",
            Self::Protocol(_) => "protocol",
        }
    }

    pub const fn is_retryable(&self) -> bool {
        matches!(self, Self::Unavailable(_) | Self::Timeout)
    }
}

impl Classify for QdrantError {
    fn class(&self) -> ErrorClass {
        match self {
            Self::Unavailable(_) | Self::Timeout => ErrorClass::Transient,
            Self::BadRequest { .. } => ErrorClass::InvalidInput,
            Self::NotFound => ErrorClass::NotFound,
            Self::Conflict(_) => ErrorClass::Conflict,
            Self::Protocol(_) => ErrorClass::Internal,
        }
    }
}
