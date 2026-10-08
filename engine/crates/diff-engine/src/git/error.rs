//! Git-layer error type (DIFF-001). See docs/planning/tasks/P09-P14 §"Failure behavior".

use std::path::PathBuf;

use review_core::{Classify, ErrorClass};

/// Failures produced while opening or reading a git repository.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum GitError {
    #[error("not a git repository: {0}")]
    NotARepository(PathBuf),
    #[error("invalid object id: {0}")]
    InvalidObjectId(String),
    #[error("object not found: {0}")]
    ObjectNotFound(String),
    #[error("object {sha} is not a commit")]
    NotACommit { sha: String },
    #[error("repository is shallow; missing object {missing}")]
    ShallowBoundary { missing: String },
    #[error("blob {oid} is {size} bytes, over the {limit} byte limit")]
    BlobTooLarge { oid: String, size: u64, limit: u64 },
    #[error("path {path} not found at commit {commit}")]
    PathNotFound { commit: String, path: String },
    #[error("object {object} is not a blob")]
    NotABlob { object: String },
    #[error("tree listing exceeds the limit of {limit} entries")]
    TreeTooLarge { limit: u32 },
    #[error("repository path escapes its root through a symlink: {0}")]
    SymlinkEscape(PathBuf),
    #[error("corrupt git object: {0}")]
    Corrupt(String),
    #[error("i/o error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("git backend error: {0}")]
    Gix(String),
}

impl Classify for GitError {
    fn class(&self) -> ErrorClass {
        match self {
            Self::ObjectNotFound(_) => ErrorClass::NotFound,
            Self::ShallowBoundary { .. } => ErrorClass::Conflict,
            Self::Io { .. } => ErrorClass::Transient,
            Self::NotARepository(_)
            | Self::InvalidObjectId(_)
            | Self::NotACommit { .. }
            | Self::BlobTooLarge { .. }
            | Self::PathNotFound { .. }
            | Self::NotABlob { .. }
            | Self::TreeTooLarge { .. }
            | Self::SymlinkEscape(_) => ErrorClass::InvalidInput,
            Self::Corrupt(_) | Self::Gix(_) => ErrorClass::Permanent,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classes_follow_the_taxonomy() {
        assert_eq!(
            GitError::ObjectNotFound("abc".to_owned()).class(),
            ErrorClass::NotFound
        );
        assert_eq!(
            GitError::ShallowBoundary {
                missing: "abc".to_owned()
            }
            .class(),
            ErrorClass::Conflict
        );
        assert_eq!(
            GitError::Io {
                path: PathBuf::from("x"),
                source: std::io::Error::other("boom"),
            }
            .class(),
            ErrorClass::Transient
        );
        assert_eq!(
            GitError::SymlinkEscape(PathBuf::from("x")).class(),
            ErrorClass::InvalidInput
        );
        assert_eq!(
            GitError::Corrupt("bad pack".to_owned()).class(),
            ErrorClass::Permanent
        );
    }
}
