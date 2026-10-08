//! The file table (CG-004).
//!
//! Every file the graph knows about is a row here, whether it produced nodes, edges, or
//! nothing at all (a directory entry that was read but contained no symbols). `nodes` and
//! `edges_owned` are ranges into the graph's node table and its owned-edge list, which is
//! what lets an incremental update replace one file's contribution without touching the rest.

use std::fmt;
use std::ops::Range;

use review_core::language::Language;
use review_core::location::{ContentHash, RepoPath};
use serde::{Deserialize, Serialize};

use super::interner::StrId;

/// Index into [`crate::graph::Graph`]'s file table.
#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(transparent)]
pub struct FileIx(pub(crate) u32);

impl FileIx {
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    pub const fn get(self) -> u32 {
        self.0
    }
}

impl fmt::Debug for FileIx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FileIx({})", self.0)
    }
}

impl fmt::Display for FileIx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<u32> for FileIx {
    fn from(raw: u32) -> Self {
        Self(raw)
    }
}

/// What the caller knows about a file before the graph exists.
///
/// Files are declared with [`crate::graph::GraphBuilder::add_file`], but a node or an edge may
/// reference a path that was never declared: `build()` then creates a placeholder row with an
/// all-zero content hash, so inputs really can arrive in any order.
///
/// Serializable because the wire codec (CG-011) stores the file table verbatim and re-runs the
/// builder on it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileInput {
    pub path: RepoPath,
    /// `graph-storage`'s `file_versions.id` once the file has been persisted; `None` in a
    /// graph that has not been written yet.
    pub file_version_id: Option<i64>,
    pub content_hash: ContentHash,
    pub language: Language,
}

impl FileInput {
    /// Declares `path` with the defaults: unversioned, zero content hash, `Language::Other`.
    pub fn placeholder(path: RepoPath) -> Self {
        Self {
            path,
            file_version_id: None,
            content_hash: ContentHash::from_bytes([0u8; 32]),
            language: Language::Other,
        }
    }
}

/// One row of the file table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    /// Interned repository-relative path.
    pub path: StrId,
    pub file_version_id: Option<i64>,
    pub content_hash: ContentHash,
    pub language: Language,
    /// Contiguous slice of [`crate::graph::Graph`]'s node table owned by this file.
    pub nodes: Range<u32>,
    /// Slice of [`crate::graph::Graph`]'s owned-edge list: every edge whose
    /// `origin_file` is this file, in canonical edge order.
    pub edges_owned: Range<u32>,
}

impl FileEntry {
    pub fn node_count(&self) -> u32 {
        self.nodes.end - self.nodes.start
    }

    pub fn owned_edge_count(&self) -> u32 {
        self.edges_owned.end - self.edges_owned.start
    }
}
