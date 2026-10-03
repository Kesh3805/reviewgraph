//! Error types for this crate (DOM-002). See docs/architecture/error-handling.md.

use std::path::PathBuf;

use review_core::location::RepoPath;
use review_core::{Classify, CoreError, ErrorClass};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Errors returned by this crate. Add domain variants as the crate gains behaviour, and keep
/// the [`Classify`] mapping exhaustive.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error(transparent)]
    Core(#[from] CoreError),
    #[error(transparent)]
    Init(#[from] InitError),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

impl Classify for Error {
    fn class(&self) -> ErrorClass {
        match self {
            Error::Core(e) => e.class(),
            Error::Init(e) => e.class(),
        }
    }
}

/// Failures of `review init` and its detectors. One malformed file never produces one of these;
/// it produces an [`InitWarning`] instead.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum InitError {
    /// The path is not inside a git repository.
    #[error("{} is not inside a git repository", .0.display())]
    NotAGitRepository(PathBuf),
    /// Init always runs on a worktree; workers create one.
    #[error("{} is a bare repository", .0.display())]
    BareRepository(PathBuf),
    #[error("io error at {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("git operation `{op}` failed: {message}")]
    Git { op: &'static str, message: String },
    /// More files than `WalkOptions::max_files`. The fix is configuration, never silent truncation.
    #[error("repository has more than {limit} files; raise max_files or add ignore rules")]
    TooManyFiles { limit: u64 },
    /// A value a detector produced was not representable (for example a path that is not a
    /// valid `RepoPath`).
    #[error(transparent)]
    Core(#[from] CoreError),
    /// A persistence adapter failed (INIT-013).
    #[error("repository facts store: {0}")]
    Store(String),
    /// Facts could not be serialized or deserialized.
    #[error("repository facts serialization: {0}")]
    Serde(String),
}

impl InitError {
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }

    pub fn git(op: &'static str, source: impl std::fmt::Display) -> Self {
        Self::Git {
            op,
            message: source.to_string(),
        }
    }
}

impl Classify for InitError {
    fn class(&self) -> ErrorClass {
        match self {
            Self::NotAGitRepository(_) | Self::BareRepository(_) | Self::TooManyFiles { .. } => {
                ErrorClass::InvalidInput
            }
            Self::Core(e) => e.class(),
            Self::Io { .. } | Self::Git { .. } | Self::Store(_) | Self::Serde(_) => {
                ErrorClass::Internal
            }
        }
    }
}

/// How serious a warning is.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Default,
    Serialize,
    Deserialize,
    JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum WarningSeverity {
    Info,
    #[default]
    Warning,
    High,
}

/// A non-fatal observation made while initializing a repository.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
pub struct InitWarning {
    /// Stable snake_case code, for example `non_utf8_path` or `symlink_escapes_root`.
    pub code: String,
    #[serde(default)]
    pub severity: WarningSeverity,
    pub path: Option<RepoPath>,
    pub message: String,
}

impl InitWarning {
    pub fn new(code: &str, path: Option<RepoPath>, message: impl Into<String>) -> Self {
        Self {
            code: code.to_owned(),
            severity: WarningSeverity::Warning,
            path,
            message: message.into(),
        }
    }

    pub fn with_severity(mut self, severity: WarningSeverity) -> Self {
        self.severity = severity;
        self
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
        assert_bounds::<InitError>();
    }

    #[test]
    fn init_errors_are_classified() {
        assert_eq!(
            InitError::NotAGitRepository("x".into()).class(),
            ErrorClass::InvalidInput
        );
        assert_eq!(
            InitError::git("status", "boom").class(),
            ErrorClass::Internal
        );
    }
}
