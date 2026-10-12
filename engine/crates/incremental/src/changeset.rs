//! Path-level change detection between two commits (INC-001).
//!
//! A thin, deterministic layer over [`repository::git::tree_changes`]: it turns raw tree-diff
//! entries into a [`ChangeSet`] sorted by path. No blob content is read and no hashing happens
//! here (that is INC-002), so the cost stays proportional to the changed subtrees.
//!
//! The diff is taken between the *trees of the two commits given*, never against a merge base:
//! the graph snapshot being updated belongs to the base commit.

use repository::git::{tree_changes, RawKind, RawTreeChange, TreeDiffError, TreeDiffOptions};
use review_core::ids::CommitSha;
use review_core::location::RepoPath;
use serde::Serialize;

/// What happened to one path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PathChangeKind {
    /// The path exists only at the head commit.
    Added,
    /// The path exists at both commits with different content.
    Modified,
    /// The path exists only at the base commit.
    Deleted,
    /// The path moved; [`PathChange::renamed_from`] carries the old path.
    Renamed,
}

/// One changed path. Object ids are hex strings so the type stays `Serialize` for debugging.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PathChange {
    /// Path at the head commit (at the base commit for deletions).
    pub path: RepoPath,
    /// What happened.
    pub kind: PathChangeKind,
    /// Source path of a rename.
    pub renamed_from: Option<RepoPath>,
    /// Blob id at the base commit.
    pub base_oid: Option<String>,
    /// Blob id at the head commit.
    pub head_oid: Option<String>,
    /// Similarity of a rename in percent.
    pub similarity: Option<u8>,
}

/// Counters describing the detection run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct ChangeSetStats {
    /// Number of entries in the change set.
    pub entries: u32,
    /// How many of them are renames.
    pub renamed: u32,
    /// Submodule entries dropped.
    pub skipped_submodule: u32,
    /// Symlink entries dropped.
    pub skipped_symlink: u32,
    /// Entries whose path was not valid UTF-8 or not a valid repository path.
    pub skipped_non_utf8: u32,
    /// Fuzzy rename matching was cut short; renames degrade to delete + add pairs.
    pub rename_limit_hit: bool,
}

/// The paths that differ between two commits, sorted by path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChangeSet {
    /// Base commit.
    pub base: CommitSha,
    /// Head commit.
    pub head: CommitSha,
    /// Changed paths, sorted by path bytes.
    pub entries: Vec<PathChange>,
    /// Run counters.
    pub stats: ChangeSetStats,
}

impl ChangeSet {
    /// The first entry for `path`, if the path changed.
    pub fn get(&self, path: &RepoPath) -> Option<&PathChange> {
        self.entries.iter().find(|entry| &entry.path == path)
    }

    /// `(old, new)` path pairs of every rename, the hint the symbol matcher consumes.
    pub fn rename_pairs(&self) -> Vec<(&RepoPath, &RepoPath)> {
        self.entries
            .iter()
            .filter_map(|entry| entry.renamed_from.as_ref().map(|from| (from, &entry.path)))
            .collect()
    }
}

/// Failure modes of [`detect_changes`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ChangeSetError {
    /// A commit is missing from the object database. Retryable after a fetch; never an empty set.
    #[error("commit not found: {0}")]
    CommitNotFound(CommitSha),
    /// An object exists but cannot be decoded.
    #[error("object corrupt: {0}")]
    ObjectCorrupt(String),
    /// The walk reached the boundary of a shallow clone.
    #[error("shallow boundary reached: {0}")]
    ShallowBoundary(String),
    /// Any other repository failure.
    #[error("repository access failed: {0}")]
    Io(String),
}

/// Detects the changed paths between `base` and `head`.
pub fn detect_changes(
    repo: &gix::Repository,
    base: &CommitSha,
    head: &CommitSha,
    opts: &TreeDiffOptions,
) -> Result<ChangeSet, ChangeSetError> {
    let base_oid = object_id(base)?;
    let head_oid = object_id(head)?;
    for (sha, oid) in [(base, &base_oid), (head, &head_oid)] {
        if repo.find_commit(*oid).is_err() {
            return Err(ChangeSetError::CommitNotFound(sha.clone()));
        }
    }

    let span = tracing::info_span!(
        "incremental.changeset",
        base = base.short(),
        head = head.short(),
        entries = tracing::field::Empty,
        renamed = tracing::field::Empty,
    );
    let _guard = span.enter();

    let raw = tree_changes(repo, &base_oid, &head_oid, opts).map_err(|err| match err {
        TreeDiffError::ObjectNotFound(message) => ChangeSetError::ShallowBoundary(message),
        TreeDiffError::ObjectCorrupt(message) => ChangeSetError::ObjectCorrupt(message),
        TreeDiffError::Gix(message) => ChangeSetError::Io(message),
    })?;

    let mut entries: Vec<PathChange> = raw.changes.iter().map(path_change).collect();
    entries.sort_by(|a, b| {
        a.path
            .as_str()
            .cmp(b.path.as_str())
            .then_with(|| a.kind.cmp(&b.kind))
    });

    let renamed = entries
        .iter()
        .filter(|entry| entry.kind == PathChangeKind::Renamed)
        .count() as u32;
    let stats = ChangeSetStats {
        entries: entries.len() as u32,
        renamed,
        skipped_submodule: raw.stats.skipped_submodule,
        skipped_symlink: raw.stats.skipped_symlink,
        skipped_non_utf8: raw.stats.skipped_paths,
        rename_limit_hit: raw.stats.rename_limit_hit,
    };
    span.record("entries", stats.entries);
    span.record("renamed", stats.renamed);
    Ok(ChangeSet {
        base: base.clone(),
        head: head.clone(),
        entries,
        stats,
    })
}

fn object_id(sha: &CommitSha) -> Result<gix::ObjectId, ChangeSetError> {
    gix::ObjectId::from_hex(sha.as_str().as_bytes())
        .map_err(|_| ChangeSetError::CommitNotFound(sha.clone()))
}

fn path_change(raw: &RawTreeChange) -> PathChange {
    // Copies are not tracked by default; a copy is a new file as far as the graph is concerned.
    let kind = match raw.kind {
        RawKind::Added | RawKind::Copied => PathChangeKind::Added,
        RawKind::Deleted => PathChangeKind::Deleted,
        RawKind::Modified => PathChangeKind::Modified,
        RawKind::Renamed => PathChangeKind::Renamed,
    };
    let renamed_from = if kind == PathChangeKind::Renamed {
        raw.old_path.clone()
    } else {
        None
    };
    PathChange {
        path: raw.path.clone(),
        kind,
        renamed_from,
        base_oid: raw.old_oid.map(|oid| oid.to_string()),
        head_oid: raw.new_oid.map(|oid| oid.to_string()),
        similarity: raw.similarity,
    }
}
