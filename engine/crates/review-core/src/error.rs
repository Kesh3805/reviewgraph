//! Errors for the pure domain crate. DOM-002 consolidates and classifies them.

/// Errors produced while constructing or parsing domain values.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CoreError {
    /// A typed identifier could not be parsed or validated.
    #[error("invalid {kind}: {reason}")]
    InvalidId { kind: &'static str, reason: String },
    /// A commit SHA was neither 40 nor 64 hexadecimal characters.
    #[error("invalid commit sha: {0}")]
    InvalidCommitSha(String),
}
