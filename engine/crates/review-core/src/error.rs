//! Error taxonomy (DOM-002). See docs/architecture/error-handling.md.
//!
//! Every library crate defines its own `Error` enum that implements [`Classify`]. The
//! [`ErrorClass`] it reports drives three decisions: retry or not, HTTP status, and which
//! `FAILED_*` run state is entered.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Maximum number of characters of a rejected repository path echoed in an error message.
const MAX_PATH_ECHO: usize = 256;

/// Coarse failure category with a stable snake_case wire form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorClass {
    InvalidInput,
    NotFound,
    Conflict,
    Transient,
    RateLimited,
    Permanent,
    Cancelled,
    Internal,
}

impl ErrorClass {
    /// Wire name, used as the `error.class` span attribute and the `error_class` metric label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidInput => "invalid_input",
            Self::NotFound => "not_found",
            Self::Conflict => "conflict",
            Self::Transient => "transient",
            Self::RateLimited => "rate_limited",
            Self::Permanent => "permanent",
            Self::Cancelled => "cancelled",
            Self::Internal => "internal",
        }
    }

    /// The only input to the automatic-retry decision. `Conflict` is never retried: retrying a
    /// lost compare-and-swap would re-run a stage somebody else already advanced.
    pub const fn is_retryable(self) -> bool {
        matches!(self, Self::Transient | Self::RateLimited)
    }
}

/// Implemented by every library error type.
pub trait Classify {
    fn class(&self) -> ErrorClass;
}

/// Errors produced while constructing, parsing or transitioning domain values.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CoreError {
    /// A typed identifier could not be parsed or validated.
    #[error("invalid {kind}: {reason}")]
    InvalidId { kind: &'static str, reason: String },
    /// A repository path was rejected. The echoed path is capped at 256 characters.
    #[error("invalid repository path {path:?}: {reason}")]
    InvalidRepoPath { path: String, reason: &'static str },
    /// A commit SHA was neither 40 nor 64 hexadecimal characters.
    #[error("invalid commit sha: {0}")]
    InvalidCommitSha(String),
    /// A state machine refused a transition.
    #[error("invalid {entity} transition {from} -> {to}")]
    InvalidTransition {
        entity: &'static str,
        from: &'static str,
        to: &'static str,
    },
    /// A numeric or enumerated value was outside its allowed range.
    #[error("{field} out of range: {value}")]
    OutOfRange { field: &'static str, value: String },
    /// A version string or version component was invalid.
    #[error("invalid version {kind}: {reason}")]
    InvalidVersion { kind: &'static str, reason: String },
}

impl CoreError {
    /// Builds [`CoreError::InvalidRepoPath`], truncating the echoed path to 256 characters.
    pub fn invalid_repo_path(path: &str, reason: &'static str) -> Self {
        Self::InvalidRepoPath {
            path: path.chars().take(MAX_PATH_ECHO).collect(),
            reason,
        }
    }
}

impl Classify for CoreError {
    fn class(&self) -> ErrorClass {
        match self {
            Self::InvalidTransition { .. } => ErrorClass::Conflict,
            Self::InvalidId { .. }
            | Self::InvalidRepoPath { .. }
            | Self::InvalidCommitSha(_)
            | Self::OutOfRange { .. }
            | Self::InvalidVersion { .. } => ErrorClass::InvalidInput,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [ErrorClass; 8] = [
        ErrorClass::InvalidInput,
        ErrorClass::NotFound,
        ErrorClass::Conflict,
        ErrorClass::Transient,
        ErrorClass::RateLimited,
        ErrorClass::Permanent,
        ErrorClass::Cancelled,
        ErrorClass::Internal,
    ];

    #[test]
    fn error_class_wire_names_stable() {
        let names: Vec<(&str, String)> = ALL
            .iter()
            .map(|c| (c.as_str(), serde_json::to_string(c).unwrap()))
            .collect();
        insta::assert_yaml_snapshot!(names);
        for c in ALL {
            assert_eq!(
                serde_json::to_string(&c).unwrap(),
                format!("\"{}\"", c.as_str())
            );
        }
    }

    #[test]
    fn only_transient_and_rate_limited_retryable() {
        for c in ALL {
            let expected = matches!(c, ErrorClass::Transient | ErrorClass::RateLimited);
            assert_eq!(c.is_retryable(), expected, "{c:?}");
        }
    }

    #[test]
    fn invalid_transition_is_conflict() {
        let e = CoreError::InvalidTransition {
            entity: "ReviewRun",
            from: "queued",
            to: "published",
        };
        assert_eq!(e.class(), ErrorClass::Conflict);
        assert_eq!(
            e.to_string(),
            "invalid ReviewRun transition queued -> published"
        );
    }

    #[test]
    fn other_core_errors_are_invalid_input() {
        let errors = [
            CoreError::InvalidId {
                kind: "RepositoryId",
                reason: "x".into(),
            },
            CoreError::invalid_repo_path("../x", "escapes the repository"),
            CoreError::InvalidCommitSha("zz".into()),
            CoreError::OutOfRange {
                field: "confidence",
                value: "2".into(),
            },
            CoreError::InvalidVersion {
                kind: "prompt",
                reason: "x".into(),
            },
        ];
        for e in errors {
            assert_eq!(e.class(), ErrorClass::InvalidInput, "{e}");
        }
    }

    #[test]
    fn repo_path_echo_is_capped() {
        let long = "a/".repeat(500);
        match CoreError::invalid_repo_path(&long, "too long") {
            CoreError::InvalidRepoPath { path, .. } => assert_eq!(path.chars().count(), 256),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn core_error_is_send_sync_static() {
        fn assert_bounds<T: Send + Sync + 'static>() {}
        assert_bounds::<CoreError>();
        assert_bounds::<ErrorClass>();
    }
}
