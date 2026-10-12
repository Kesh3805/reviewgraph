//! Commit-range file diff (DIFF-002).
//!
//! [`diff_commits`] reproduces the provider's three-dot diff: it always diffs from the
//! *merge base* of base and head, not from the provider-supplied base tip. Rename and
//! copy detection runs in-process through gix with explicitly configured thresholds, so
//! no user git configuration can alter the result.
//!
//! After the tree walk every file is classified (DIFF-004, [`crate::disposition`]) and, when it
//! is analysable, diffed line by line (DIFF-003, [`crate::hunks`]). Files are processed in
//! parallel on the rayon pool and stay in path order.

use std::collections::{BTreeMap, HashMap};

use globset::{Glob, GlobSet, GlobSetBuilder};
use rayon::prelude::*;
use review_core::change::{ChangedFile, FileChangeStatus, Hunk};
use review_core::ids::CommitSha;
use review_core::{Classify, ErrorClass};

use repository::git::{diff_trees, RawKind, RawTreeChange, TreeDiffOptions};

use crate::disposition::{
    classify_file, CompiledRules, CoverageEntry, DispositionRules, FileDisposition, PROBE_BYTES,
};
use crate::git::{GitError, GitRepo, MergeBase, ObjectHeader};
use crate::hunks::{compute_hunks, HunkError, HunkOptions, LineStats};
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
    /// Classify files and compute hunks (`true`). `false` returns the file list only, every
    /// file left at the default `Analyze` disposition without line detail.
    pub line_detail: bool,
    /// Hunk options (context 3, histogram); `max_lines` is further capped by
    /// [`DispositionRules::max_diff_lines`].
    pub hunks: HunkOptions,
    /// File classification rules.
    pub dispositions: DispositionRules,
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
            line_detail: true,
            hunks: HunkOptions::default(),
            dispositions: DispositionRules::default(),
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
        dispositions = tracing::field::Empty,
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

    let coverage = if opts.line_detail {
        let rules = opts
            .dispositions
            .compile()
            .map_err(|e| DiffError::Invariant(format!("invalid disposition glob: {e}")))?;
        apply_line_detail(repo, &mut files, &rules, opts, &mut stats)?
    } else {
        Vec::new()
    };
    let mut dispositions: BTreeMap<&'static str, u32> = BTreeMap::new();
    for file in &files {
        *dispositions.entry(file.disposition.label()).or_insert(0) += 1;
    }
    let summary: Vec<String> = dispositions
        .iter()
        .map(|(label, n)| format!("{label}={n}"))
        .collect();
    span.record("dispositions", summary.join(",").as_str());

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
        coverage,
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

/// Classify every file, then compute hunks for the analysable ones (DIFF-003/DIFF-004).
fn apply_line_detail(
    repo: &GitRepo,
    files: &mut [FileDiff],
    rules: &CompiledRules,
    opts: &DiffOptions,
    stats: &mut DiffStats,
) -> Result<Vec<CoverageEntry>, DiffError> {
    let span = tracing::info_span!(
        "diff.hunks",
        files = files.len(),
        hunks = tracing::field::Empty,
        additions = tracing::field::Empty,
        deletions = tracing::field::Empty,
    );
    let _enter = span.enter();
    let started = std::time::Instant::now();
    let mut hunk_opts = opts.hunks;
    hunk_opts.max_lines = hunk_opts.max_lines.min(opts.dispositions.max_diff_lines);

    let outcomes: Vec<Result<bool, DiffError>> = files
        .par_iter_mut()
        .map(|file| process_file(repo, file, rules, &hunk_opts))
        .collect();
    for outcome in outcomes {
        if outcome? {
            stats.hunked_files += 1;
        }
    }
    let mut too_large = 0u64;
    let mut coverage = Vec::new();
    for file in files.iter() {
        metrics::record_disposition(file.disposition.label());
        if matches!(file.disposition, FileDisposition::TooLarge { .. }) {
            too_large += 1;
        }
        stats.hunks += file.hunks.len() as u32;
        if let Some(lines) = file.lines {
            stats.additions += lines.additions;
            stats.deletions += lines.deletions;
        }
        if !file.disposition.is_analyze() {
            coverage.push(CoverageEntry {
                path: file.file.path.clone(),
                disposition: file.disposition,
            });
        }
    }
    span.record("hunks", stats.hunks);
    span.record("additions", stats.additions);
    span.record("deletions", stats.deletions);
    metrics::record_too_large(too_large);
    metrics::record_hunks_duration(started.elapsed().as_secs_f64());
    Ok(coverage)
}

/// Content of one side, or why it was not read.
enum Side {
    Absent,
    Loaded(Vec<u8>),
    /// Larger than the git read limit: classified from the header only.
    Oversize,
    Unreadable,
}

impl Side {
    fn bytes(&self) -> &[u8] {
        match self {
            Side::Loaded(data) => data,
            Side::Absent | Side::Oversize | Side::Unreadable => &[],
        }
    }
}

fn header(repo: &GitRepo, oid: Option<gix::ObjectId>) -> Result<Option<ObjectHeader>, DiffError> {
    match oid {
        None => Ok(None),
        Some(oid) => repo.blob_header(&oid).map(Some).map_err(DiffError::Git),
    }
}

fn load(
    repo: &GitRepo,
    oid: Option<gix::ObjectId>,
    header: Option<&ObjectHeader>,
) -> Result<Side, DiffError> {
    let (Some(oid), Some(header)) = (oid, header) else {
        return Ok(Side::Absent);
    };
    if header.size > repo.limits().max_blob_bytes {
        return Ok(Side::Oversize);
    }
    match repo.read_blob_by_oid(&oid) {
        Ok(blob) => Ok(Side::Loaded(blob.data)),
        // A missing object means a stale or shallow mirror: the whole diff is unusable.
        Err(e @ (GitError::ObjectNotFound(_) | GitError::ShallowBoundary { .. })) => {
            Err(DiffError::Git(e))
        }
        Err(e) => {
            tracing::warn!(error = %e, "blob unreadable; file listed without analysis");
            Ok(Side::Unreadable)
        }
    }
}

/// Classify one file and fill its line detail. Returns whether hunks were computed.
fn process_file(
    repo: &GitRepo,
    file: &mut FileDiff,
    rules: &CompiledRules,
    hunk_opts: &HunkOptions,
) -> Result<bool, DiffError> {
    let old_header = header(repo, file.base_oid)?;
    let new_header = header(repo, file.head_oid)?;
    let old = load(repo, file.base_oid, old_header.as_ref())?;
    let new = load(repo, file.head_oid, new_header.as_ref())?;

    let unreadable = matches!(old, Side::Unreadable) || matches!(new, Side::Unreadable);
    let new_absent = matches!(new, Side::Absent);
    let primary = if new_absent { old.bytes() } else { new.bytes() };
    let probe = &primary[..primary.len().min(PROBE_BYTES)];
    let secondary: &[u8] = if new_absent {
        &[]
    } else {
        &old.bytes()[..old.bytes().len().min(PROBE_BYTES)]
    };
    let mut disposition = if unreadable {
        FileDisposition::Unreadable
    } else {
        classify_file(
            &file.file.path,
            old_header.as_ref(),
            new_header.as_ref(),
            probe,
            secondary,
            rules,
        )
    };
    let oversize = matches!(old, Side::Oversize) || matches!(new, Side::Oversize);
    if oversize && disposition.is_analyze() {
        disposition = FileDisposition::TooLarge {
            bytes: largest(old_header.as_ref(), new_header.as_ref()),
            lines: None,
        };
    }

    let mut hunked = false;
    let classified = disposition;
    match classified {
        FileDisposition::Analyze => match compute_hunks(old.bytes(), new.bytes(), hunk_opts) {
            Ok(set) => {
                file.lines = Some(set.stats);
                file.hunks = set.hunks;
                hunked = true;
            }
            Err(HunkError::TooLarge { lines, .. }) => {
                disposition = FileDisposition::TooLarge {
                    bytes: largest(old_header.as_ref(), new_header.as_ref()),
                    lines: Some(lines),
                };
            }
        },
        FileDisposition::Generated { .. }
        | FileDisposition::Vendored
        | FileDisposition::Minified
        | FileDisposition::LockfileSummary { .. } => {
            if !oversize {
                file.lines = Some(approximate_stats(old.bytes(), new.bytes()));
            }
        }
        FileDisposition::Binary
        | FileDisposition::TooLarge { .. }
        | FileDisposition::Unreadable => {}
    }
    file.disposition = disposition;

    let headers: Vec<Hunk> = file.hunks.iter().map(|h| h.header).collect();
    file.file = ChangedFile::new(
        file.file.path.clone(),
        file.file.old_path.clone(),
        file.file.status,
        disposition == FileDisposition::Binary,
        headers,
    )
    .map_err(|e| DiffError::Invariant(e.to_string()))?;
    Ok(hunked)
}

fn largest(old: Option<&ObjectHeader>, new: Option<&ObjectHeader>) -> u64 {
    old.map_or(0, |h| h.size).max(new.map_or(0, |h| h.size))
}

/// Cheap line statistics for files that are listed but not diffed: the multiset difference of
/// lines. Exact for pure additions and deletions, an approximation when lines move.
fn approximate_stats(old: &[u8], new: &[u8]) -> LineStats {
    let mut counts: HashMap<&[u8], i64> = HashMap::new();
    for line in old.split_inclusive(|b| *b == b'\n') {
        *counts.entry(line).or_insert(0) -= 1;
    }
    for line in new.split_inclusive(|b| *b == b'\n') {
        *counts.entry(line).or_insert(0) += 1;
    }
    let mut stats = LineStats::default();
    for delta in counts.values() {
        let n = u32::try_from(delta.unsigned_abs()).unwrap_or(u32::MAX);
        if *delta > 0 {
            stats.additions += n;
        } else {
            stats.deletions += n;
        }
    }
    stats
}
