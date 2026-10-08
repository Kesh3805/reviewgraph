//! Config-isolated, read-only git access (DIFF-001).
//!
//! [`GitRepo`] wraps a [`gix::ThreadSafeRepository`] opened with an isolated configuration:
//! no system/global/user config, no includes, no hooks, filters, attributes or environment
//! (`GIT_DIR`, `GIT_CONFIG_*`, `HOME` are not consulted). The engine never shells out to `git`,
//! so user configuration cannot alter results.

mod blob;
mod error;
mod merge_base;
mod metrics;
mod open;
mod resolve;
mod tree;

pub use blob::{Blob, ObjectHeader};
pub use error::GitError;
pub use merge_base::MergeBase;
pub use tree::{TreeEntry, TreeEntryKind};

use std::fmt;

/// Read limits enforced by [`GitRepo`] so a hostile or corrupt mirror cannot exhaust memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadLimits {
    /// Maximum blob content size in bytes; larger blobs fail with [`GitError::BlobTooLarge`].
    pub max_blob_bytes: u64,
    /// Maximum number of entries [`GitRepo::list_tree`] will return before
    /// [`GitError::TreeTooLarge`].
    pub max_tree_entries: u32,
}

impl Default for ReadLimits {
    fn default() -> Self {
        Self {
            max_blob_bytes: 8 * 1024 * 1024,
            max_tree_entries: 200_000,
        }
    }
}

/// A handle to a bare mirror or worktree repository, opened in isolation and safe to share
/// across threads. All operations are pure reads.
pub struct GitRepo {
    inner: gix::ThreadSafeRepository,
    limits: ReadLimits,
}

impl fmt::Debug for GitRepo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GitRepo")
            .field("git_dir", &self.inner.git_dir())
            .field("limits", &self.limits)
            .finish()
    }
}

impl Clone for GitRepo {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            limits: self.limits,
        }
    }
}

impl GitRepo {
    /// The limits this handle was opened with.
    pub fn limits(&self) -> ReadLimits {
        self.limits
    }

    /// Cheap thread-local handle for rayon workers, with a 64 MiB object cache enabled.
    pub fn local(&self) -> gix::Repository {
        let mut repo = self.inner.to_thread_local();
        repo.object_cache_size_if_unset(64 * 1024 * 1024);
        repo
    }

    pub(crate) fn missing_object(&self, repo: &gix::Repository, oid: gix::ObjectId) -> GitError {
        if repo.is_shallow() {
            GitError::ShallowBoundary {
                missing: oid.to_string(),
            }
        } else {
            GitError::ObjectNotFound(oid.to_string())
        }
    }
}

/// Number of git objects read through [`GitRepo`] in this process (`git_object_reads_total`).
pub fn object_reads_total() -> u64 {
    metrics::object_reads_total()
}
