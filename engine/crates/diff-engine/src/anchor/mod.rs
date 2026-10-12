//! Provider diff reconciliation and anchorable-line sets (DIFF-005).
//!
//! Inline review comments are accepted only on lines inside the diff *as the provider computed
//! it*: RIGHT (new side) context and added lines, LEFT (old side) context and deleted lines.
//! [`reconcile`] compares the local [`crate::model::DiffModel`] with the provider's changed-file
//! list and, per file, chooses the authority for anchoring:
//!
//! 1. the provider patch when present and well formed ([`AnchorSource::ProviderPatch`]);
//! 2. otherwise the local hunks (3 lines of context) when the provider listed the file without
//!    a patch and the change is small ([`AnchorSource::LocalHunks`]);
//! 3. otherwise the file is not anchorable and findings on it go to the review summary.

pub mod lineset;
pub mod patch;
pub mod reconcile;

use std::collections::BTreeMap;

use review_core::change::FileChangeStatus;
use review_core::location::{DiffSide, RepoPath};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub use lineset::LineSet;
pub use patch::{parse_patch, ParsedPatch, PatchError};
pub use reconcile::reconcile;

/// One file as the provider reported it (normalized by API-006).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProviderFileDiff {
    /// New path.
    pub path: RepoPath,
    /// Previous path for renames and copies.
    pub old_path: Option<RepoPath>,
    /// Provider status.
    pub status: FileChangeStatus,
    /// Added lines as reported.
    pub additions: u32,
    /// Deleted lines as reported.
    pub deletions: u32,
    /// Patch text; `None` when the provider omitted it (large or binary files).
    pub patch: Option<String>,
    /// The provider truncated its file list (GitHub stops at 3,000 files).
    pub truncated_list: bool,
}

/// Where a file's anchor sets came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AnchorSource {
    /// Parsed from the provider patch (authoritative).
    ProviderPatch,
    /// Derived from local hunks because the provider omitted the patch.
    LocalHunks,
    /// No usable source: the file is not anchorable.
    None,
}

/// Anchorable lines of one file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FileAnchors {
    /// New-side lines (RIGHT): context and added.
    pub right: LineSet,
    /// Old-side lines (LEFT): context and deleted, numbered in `old_path` for renames.
    pub left: LineSet,
    /// Previous path the LEFT lines refer to, for renames and copies.
    pub old_path: Option<RepoPath>,
    /// Where the sets came from.
    pub source: AnchorSource,
    /// Whether inline comments can be placed on this file at all.
    pub anchorable: bool,
}

impl FileAnchors {
    /// A file on which nothing can be anchored.
    pub fn unanchorable(old_path: Option<RepoPath>) -> Self {
        Self {
            right: LineSet::new(),
            left: LineSet::new(),
            old_path,
            source: AnchorSource::None,
            anchorable: false,
        }
    }

    fn set(&self, side: DiffSide) -> &LineSet {
        match side {
            DiffSide::Head => &self.right,
            DiffSide::Base => &self.left,
        }
    }

    /// Whether a comment can be placed on `line` of `side` (Head = RIGHT, Base = LEFT).
    pub fn can_anchor(&self, side: DiffSide, line: u32) -> bool {
        self.anchorable && self.set(side).contains(line)
    }

    /// The anchorable line of `side` nearest to `line` within `max_dist` lines.
    pub fn nearest_within(&self, side: DiffSide, line: u32, max_dist: u32) -> Option<u32> {
        if !self.anchorable {
            return None;
        }
        self.set(side).nearest_within(line, max_dist)
    }
}

/// Anchors of every file, keyed by (new) path.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AnchorMap {
    /// Per-file anchors.
    pub files: BTreeMap<RepoPath, FileAnchors>,
}

impl AnchorMap {
    /// Anchors of `path`.
    pub fn get(&self, path: &RepoPath) -> Option<&FileAnchors> {
        self.files.get(path)
    }
}

/// A difference between the local diff and the provider's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Discrepancy {
    /// The provider lists a file the local diff does not have (stale mirror: refetch).
    ProviderOnly {
        /// The file.
        path: RepoPath,
    },
    /// The local diff has a file the provider does not list.
    LocalOnly {
        /// The file.
        path: RepoPath,
    },
    /// Both list the file with different statuses.
    StatusMismatch {
        /// The file.
        path: RepoPath,
        /// Local status.
        local: FileChangeStatus,
        /// Provider status.
        provider: FileChangeStatus,
    },
    /// Both list the file with different line counts.
    StatsMismatch {
        /// The file.
        path: RepoPath,
        /// Local (additions, deletions).
        local: (u32, u32),
        /// Provider (additions, deletions).
        provider: (u32, u32),
    },
    /// Hunk boundaries differ (informational: the provider wins).
    HunkBoundaryDiffers {
        /// The file.
        path: RepoPath,
    },
    /// The provider patch is missing or malformed.
    PatchMissing {
        /// The file.
        path: RepoPath,
    },
    /// The provider truncated its file list.
    ProviderListTruncated,
}

impl Discrepancy {
    /// Stable metric label.
    pub fn label(&self) -> &'static str {
        match self {
            Self::ProviderOnly { .. } => "provider_only",
            Self::LocalOnly { .. } => "local_only",
            Self::StatusMismatch { .. } => "status_mismatch",
            Self::StatsMismatch { .. } => "stats_mismatch",
            Self::HunkBoundaryDiffers { .. } => "hunk_boundary_differs",
            Self::PatchMissing { .. } => "patch_missing",
            Self::ProviderListTruncated => "provider_list_truncated",
        }
    }

    /// The file concerned, if any.
    pub fn path(&self) -> Option<&RepoPath> {
        match self {
            Self::ProviderOnly { path }
            | Self::LocalOnly { path }
            | Self::StatusMismatch { path, .. }
            | Self::StatsMismatch { path, .. }
            | Self::HunkBoundaryDiffers { path }
            | Self::PatchMissing { path } => Some(path),
            Self::ProviderListTruncated => None,
        }
    }
}

/// Result of [`reconcile`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Reconciled {
    /// Anchor sets per file.
    pub anchors: AnchorMap,
    /// Differences, sorted by path then kind.
    pub discrepancies: Vec<Discrepancy>,
}
