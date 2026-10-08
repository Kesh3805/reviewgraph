//! `diff-engine` crate. See docs/architecture/target-architecture.md §2 and §3.6.
//!
//! * [`files`] — commit-range file diff (DIFF-002), dispositions (DIFF-004) and hunks (DIFF-003).
//! * [`hunks`] / [`lines`] — the in-process line diff.
//! * [`disposition`] — binary, large, generated, vendored, minified and lockfile files.

pub mod anchor;
pub mod disposition;
pub mod error;
pub mod files;
pub mod git;
pub mod hunks;
pub mod lines;
mod metrics;
pub mod model;
pub mod symbol_map;
pub mod testkit;

pub use disposition::{DispositionRules, FileDisposition};
pub use error::{Error, Result};
pub use files::{diff_commits, DiffError, DiffOptions};
pub use hunks::{compute_hunks, DiffHunk, HunkError, HunkOptions, HunkSet, LineStats};
pub use model::{DiffModel, FileDiff};
