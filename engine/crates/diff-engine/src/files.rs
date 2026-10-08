//! Commit-range file diff (DIFF-002).
//!
//! [`diff_commits`] reproduces the provider's three-dot diff: it always diffs from the
//! *merge base* of base and head, not from the provider-supplied base tip. Rename and
//! copy detection runs in-process through gix with explicitly configured thresholds, so
//! no user git configuration can alter the result.

use globset::{Glob, GlobSet, GlobSetBuilder};
use review_core::change::{ChangedFile, FileChangeStatus};
use review_core::ids::CommitSha;
use review_core::{Classify, ErrorClass};

use repository::git::{diff_trees, RawKind, RawTreeChange, TreeDiffOptions};

use crate::git::{GitError, GitRepo, MergeBase};
use crate::metrics;
use crate::model::{DiffModel, DiffStats, FileDiff};

/// Options controlling base selection, rename/copy detection, path filtering and the file cap.
#[derive(Debug, Clone, PartialEq)]
pub struct DiffOptions {
    /// Similarity in `[0, 1]` required to pair a deletion with an addition as a rename
    /// (`0.5`, like `git diff -M50%`).
    pub rename_similarity: f32,
    /// Maximum number of fuzzy rename candidates (`1000`, like git); exceeding it degrades
    /// to exact matches and sets [`DiffStats::rename_limit_hit`].
    pub rename_limit: u32,
    /// Whether copy detection runs at all (off by default, like git without `-C`).
    pub detect_copies: bool,
    /// Similarity in `[0, 1]` required for a copy (`0.9`).
    pub copy_similarity: f32,
    /// When non-empty, only paths matching at least one glob are kept.
    pub include_globs: Vec<Glob>,
    /// Paths matching any of these globs are removed (exclude wins over include).
    pub exclude_globs: Vec<Glob>,
    /// Maximum number of files in the result; further files are dropped and the model is
    /// flagged [`DiffStats::truncated`]. `0` means unlimited.
    pub max_files: u32,
}

impl Default for DiffOptions {
    fn default() -> Self {
        Self {
            rename_similarity: 0.5,
            rename_limit: 1000,
            detect_copies: false,
            copy_similarity: 0.9,
            include_globs: Vec::new(),
            exclude_globs: Vec::new(),
            max_files: 20_000,
        }
    }
}

/// Failures of [`diff_commits`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DiffError {
    /// A git read failed (missing object, shallow boundary, corrupt data).
    #[error(transparent)]
    Git(#[from] GitError),
    /// A mapped file would violate the DOM-005 invariants; this is a bug in the mapper.
    #[error("changed-file invariant violated: {0}")]
    Invariant(String),
    /// The requested cap cannot represent the change (never returned by [`diff_commits`]
    /// itself, which truncates instead; callers that pre-check a cap may surface it).
    #[error("too many changed files: {n} exceeds the cap of {cap}")]
    TooManyFiles { n: usize, cap: u32 },
}

impl Classify for DiffError {
    fn class(&self) -> ErrorClass {
        match self {
            Self::Git(e) => e.class(),
            Self::Invariant(_) => ErrorClass::Permanent,
            Self::TooManyFiles { .. } => ErrorClass::InvalidInput,
        }
    }
}

/// Diff `head` against the merge base of `base` and `head` (three-dot semantics).
///
/// The old side falls back to `base`'s own tree when the commits are unrelated, in which
/// case [`DiffModel::merge_base`] is `None` and a warning is logged.
pub fn diff_commits(
    repo: &GitRepo,
    base: &CommitSha,
    head: &CommitSha,
    opts: &DiffOptions,
) -> Result<DiffModel, DiffError> {
    let span = tracing::info_span!(
        "diff_analysis",
        base = base.as_str(),
        head = head.as_str(),
        files = tracing::field::Empty,
        renamed = tracing::field::Empty,
        copied = tracing::field::Empty,
        filtered = tracing::field::Empty,
        truncated = tracing::field::Empty,
    );
    let _enter = span.enter();
    let started = std::time::Instant::now();

    let merge_base = match repo.merge_base(base, head)? {
        MergeBase::Found { sha, .. } => Some(sha),
        MergeBase::None => {
            tracing::warn!(
                base = base.as_str(),
                head = head.as_str(),
                "no merge base; diffing against the base tree directly"
            );
            None
        }
    };
    let old_commit = merge_base.clone().unwrap_or_else(|| base.clone());
    let local = repo.local();
    let old_tree = find_tree(repo, &local, &old_commit)?;
    let new_tree = find_tree(repo, &local, head)?;

    let tree_opts = TreeDiffOptions {
        rename_similarity: opts.rename_similarity,
        rename_limit: opts.rename_limit,
        track_copies: opts.detect_copies,
        copy_similarity: opts.copy_similarity,
    };
    let walked = diff_trees(&local, &old_tree, &new_tree, &tree_opts)
        .map_err(|e| GitError::Gix(e.to_string()))?;

    let include = build_globset(&opts.include_globs)?;
    let exclude = build_globset(&opts.exclude_globs)?;
    let cap = if opts.max_files == 0 {
        usize::MAX
    } else {
        opts.max_files as usize
    };

    let mut stats = DiffStats {
        rename_limit_hit: walked.stats.rename_limit_hit,
        skipped_symlink: walked.stats.skipped_symlink,
        skipped_submodule: walked.stats.skipped_submodule,
        ..DiffStats::default()
    };
    let mut files = Vec::with_capacity(walked.changes.len().min(cap));
    for change in &walked.changes {
        if !accept(change.path.as_str(), &include, &exclude) {
            stats.filtered += 1;
            continue;
        }
        if files.len() >= cap {
            stats.truncated = true;
            break;
        }
        let file = map_change(change)?;
        match file.file.status {
            FileChangeStatus::Added => stats.added += 1,
            FileChangeStatus::Modified => stats.modified += 1,
            FileChangeStatus::Deleted => stats.deleted += 1,
            FileChangeStatus::Renamed => stats.renamed += 1,
            FileChangeStatus::Copied => stats.copied += 1,
        }
        files.push(file);
    }
    files.sort_by(|a, b| a.file.path.as_str().cmp(b.file.path.as_str()));
    stats.files = files.len() as u32;

    span.record("files", stats.files);
    span.record("renamed", stats.renamed);
    span.record("copied", stats.copied);
    span.record("filtered", stats.filtered);
    span.record("truncated", stats.truncated);
    metrics::record_files_duration(started.elapsed().as_secs_f64());
    if stats.rename_limit_hit {
        metrics::record_rename_limit_hit();
    }

    Ok(DiffModel {
        base: base.clone(),
        head: head.clone(),
        merge_base,
        files,
        stats,
    })
}

fn find_tree<'repo>(
    repo: &GitRepo,
    local: &'repo gix::Repository,
    commit: &CommitSha,
) -> Result<gix::Tree<'repo>, GitError> {
    let oid = repo.tree_of(commit)?;
    local.find_tree(oid).map_err(|e| {
        if local.is_shallow() {
            GitError::ShallowBoundary {
                missing: oid.to_string(),
            }
        } else {
            GitError::Gix(e.to_string())
        }
    })
}

fn build_globset(globs: &[Glob]) -> Result<Option<GlobSet>, DiffError> {
    if globs.is_empty() {
        return Ok(None);
    }
    let mut builder = GlobSetBuilder::new();
    for glob in globs {
        builder.add(glob.clone());
    }
    builder
        .build()
        .map(Some)
        .map_err(|e| DiffError::Invariant(format!("invalid path glob: {e}")))
}

fn accept(path: &str, include: &Option<GlobSet>, exclude: &Option<GlobSet>) -> bool {
    if let Some(exclude) = exclude {
        if exclude.is_match(path) {
            return false;
        }
    }
    match include {
        Some(include) => include.is_match(path),
        None => true,
    }
}

fn map_change(change: &RawTreeChange) -> Result<FileDiff, DiffError> {
    let (status, old_path) = match change.kind {
        RawKind::Added => (FileChangeStatus::Added, None),
        RawKind::Deleted => (FileChangeStatus::Deleted, None),
        RawKind::Modified => (FileChangeStatus::Modified, None),
        RawKind::Renamed => (FileChangeStatus::Renamed, change.old_path.clone()),
        RawKind::Copied => (FileChangeStatus::Copied, change.old_path.clone()),
    };
    let similarity = match change.kind {
        RawKind::Renamed | RawKind::Copied => change.similarity,
        _ => None,
    };
    let file = ChangedFile::new(change.path.clone(), old_path, status, false, Vec::new())
        .map_err(|e| DiffError::Invariant(e.to_string()))?;
    Ok(FileDiff::new(
        file,
        change.old_oid,
        change.new_oid,
        similarity,
    ))
}
