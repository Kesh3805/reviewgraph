//! `DiffModel`: the file-level change model produced by [`crate::files::diff_commits`].
//!
//! DIFF-003 fills `lines`/`hunks`, DIFF-004 replaces the default `disposition`, DIFF-005
//! reconciles the model with provider input and DIFF-006 maps symbols onto it.

use review_core::change::ChangedFile;
use review_core::ids::CommitSha;

use crate::disposition::{CoverageEntry, FileDisposition};
use crate::hunks::{DiffHunk, LineStats};

/// Summary counters of one [`DiffModel`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DiffStats {
    /// Number of files after filtering and truncation (`files.len()`).
    pub files: u32,
    /// Files with status `Added`.
    pub added: u32,
    /// Files with status `Modified`.
    pub modified: u32,
    /// Files with status `Deleted`.
    pub deleted: u32,
    /// Files with status `Renamed`.
    pub renamed: u32,
    /// Files with status `Copied`.
    pub copied: u32,
    /// Files removed by include/exclude path filters.
    pub filtered: u32,
    /// The diff stopped at the file cap (see `DiffOptions::max_files`); `files` is partial.
    pub truncated: bool,
    /// Rename detection hit the configured limit and degraded (exact matches only).
    pub rename_limit_hit: bool,
    /// Symlink entries encountered and skipped by the tree walk.
    pub skipped_symlink: u32,
    /// Submodule (gitlink) entries encountered and skipped by the tree walk.
    pub skipped_submodule: u32,
    /// Files whose hunks were computed (disposition `Analyze`).
    pub hunked_files: u32,
    /// Hunks over all files.
    pub hunks: u32,
    /// Added lines over all files with line statistics.
    pub additions: u32,
    /// Deleted lines over all files with line statistics.
    pub deletions: u32,
}

/// One changed file with everything DIFF-003..006 attach to it.
#[derive(Debug, Clone, PartialEq)]
pub struct FileDiff {
    /// The DOM-005 file record: path, old path, status, binary flag, hunk headers.
    pub file: ChangedFile,
    /// Blob id of the old side (`None` for additions).
    pub base_oid: Option<gix::ObjectId>,
    /// Blob id of the new side (`None` for deletions).
    pub head_oid: Option<gix::ObjectId>,
    /// Similarity in percent for renames/copies (`Some(100)` for exact rewrites).
    pub similarity: Option<u8>,
    /// How the file should be processed; `Analyze` until DIFF-004 classifies it.
    pub disposition: FileDisposition,
    /// Line statistics; filled by DIFF-003 (`None` until then).
    pub lines: Option<LineStats>,
    /// Full hunk detail; filled by DIFF-003 (empty until then).
    pub hunks: Vec<DiffHunk>,
}

impl FileDiff {
    /// A freshly mapped file with the default `Analyze` disposition and no line detail yet.
    pub fn new(
        file: ChangedFile,
        base_oid: Option<gix::ObjectId>,
        head_oid: Option<gix::ObjectId>,
        similarity: Option<u8>,
    ) -> Self {
        Self {
            file,
            base_oid,
            head_oid,
            similarity,
            disposition: FileDisposition::Analyze,
            lines: None,
            hunks: Vec::new(),
        }
    }
}

/// Result of [`crate::files::diff_commits`]: commits, files and summary stats.
#[derive(Debug, Clone, PartialEq)]
pub struct DiffModel {
    /// The provider-reported base commit the diff was requested for.
    pub base: CommitSha,
    /// The head commit.
    pub head: CommitSha,
    /// The merge base actually diffed from; `None` when the commits are unrelated (the
    /// old side then falls back to `base` itself).
    pub merge_base: Option<CommitSha>,
    /// Changed files sorted by path.
    pub files: Vec<FileDiff>,
    /// Summary counters.
    pub stats: DiffStats,
    /// Files listed but not analysed (disposition other than `Analyze`), with the reason, so
    /// reviewers and the summary can state what was not covered.
    pub coverage: Vec<CoverageEntry>,
}

impl DiffModel {
    /// The file at `path`, if it changed.
    pub fn file(&self, path: &str) -> Option<&FileDiff> {
        self.files
            .binary_search_by(|f| f.file.path.as_str().cmp(path))
            .ok()
            .and_then(|i| self.files.get(i))
    }
}
