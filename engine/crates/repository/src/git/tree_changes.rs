//! Path-level change detection between two trees (INC-001, consumed by DIFF-002).
//!
//! Built on gix's tree diff with rewrite (rename/copy) tracking. Identical subtrees are skipped by
//! object id, so the cost is proportional to the changed subtrees rather than to the repository
//! size. The walk never consults git configuration: rewrite options are always passed explicitly.

use review_core::location::RepoPath;

/// Options controlling rename and copy detection.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TreeDiffOptions {
    /// Similarity in `[0, 1]` required for two files to count as renamed (`0.5` = `git diff -M50%`).
    pub rename_similarity: f32,
    /// Maximum number of candidates considered for fuzzy rename matching (`1000`, like git).
    pub rename_limit: u32,
    /// Whether copy detection is performed at all (off by default, like git without `-C`).
    pub track_copies: bool,
    /// Similarity in `[0, 1]` required for a copy (`0.9`).
    pub copy_similarity: f32,
}

impl Default for TreeDiffOptions {
    fn default() -> Self {
        Self {
            rename_similarity: 0.5,
            rename_limit: 1000,
            track_copies: false,
            copy_similarity: 0.9,
        }
    }
}

/// The kind of a single changed path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RawKind {
    /// The path exists only in the new tree.
    Added,
    /// The path exists only in the old tree.
    Deleted,
    /// The path exists on both sides with different content or type.
    Modified,
    /// The path moved; `old_path` carries the source.
    Renamed,
    /// The path was copied from `old_path`.
    Copied,
}

/// One changed path, filtered to entries a reviewer can read as source.
#[derive(Debug, Clone, PartialEq)]
pub struct RawTreeChange {
    /// Path in the new tree (the old tree for deletions).
    pub path: RepoPath,
    /// Source path for renames and copies.
    pub old_path: Option<RepoPath>,
    /// What happened.
    pub kind: RawKind,
    /// Blob id before the change (`None` for additions).
    pub old_oid: Option<gix::ObjectId>,
    /// Blob id after the change (`None` for deletions).
    pub new_oid: Option<gix::ObjectId>,
    /// Entry mode before the change (`None` when the side is absent).
    pub old_mode: Option<gix::object::tree::EntryMode>,
    /// Entry mode after the change (`None` when the side is absent).
    pub new_mode: Option<gix::object::tree::EntryMode>,
    /// Similarity of a rename or copy in percent; `Some(100)` for exact rewrites.
    pub similarity: Option<u8>,
}

/// Counters describing entries the walk deliberately dropped.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TreeDiffStats {
    /// Submodule (gitlink) entries touched by the diff.
    pub skipped_submodule: u32,
    /// Symlink entries touched by the diff.
    pub skipped_symlink: u32,
    /// Entries whose path was not valid UTF-8 or not a valid repository path.
    pub skipped_paths: u32,
    /// The rename limit was reached and fuzzy matching degraded to exact matches.
    pub rename_limit_hit: bool,
}

/// Result of a tree diff.
#[derive(Debug, Clone, PartialEq)]
pub struct TreeDiffResult {
    /// Changed paths sorted by path, then deterministically by kind.
    pub changes: Vec<RawTreeChange>,
    /// Counters for skipped entries.
    pub stats: TreeDiffStats,
}

/// Failure modes of the tree walk.
#[derive(Debug, thiserror::Error)]
pub enum TreeDiffError {
    /// A commit or tree object referenced by the walk is absent.
    #[error("object not found: {0}")]
    ObjectNotFound(String),
    /// An object exists but cannot be decoded.
    #[error("object corrupt: {0}")]
    ObjectCorrupt(String),
    /// gix failed for any other reason.
    #[error("gix: {0}")]
    Gix(String),
}

/// Diff two trees, honouring `opts` for rename and copy detection.
pub fn diff_trees(
    repo: &gix::Repository,
    old_tree: &gix::Tree,
    new_tree: &gix::Tree,
    opts: &TreeDiffOptions,
) -> Result<TreeDiffResult, TreeDiffError> {
    let mut cache = repo
        .diff_resource_cache_for_tree_diff()
        .map_err(|e| TreeDiffError::Gix(e.to_string()))?;
    let mut state = gix::diff::tree::State::default();
    let rewrites = gix::diff::Rewrites {
        copies: opts.track_copies.then_some(gix::diff::rewrites::Copies {
            source: gix::diff::rewrites::CopySource::FromSetOfModifiedFiles,
            percentage: Some(opts.copy_similarity),
        }),
        percentage: Some(opts.rename_similarity),
        limit: opts.rename_limit as usize,
        track_empty: false,
    };
    let options = gix::diff::tree_with_rewrites::Options {
        location: Some(gix::diff::tree::recorder::Location::Path),
        rewrites: Some(rewrites),
    };

    let mut changes: Vec<RawTreeChange> = Vec::new();
    let mut stats = TreeDiffStats::default();
    let mut failure: Option<TreeDiffError> = None;

    let outcome = gix::diff::tree_with_rewrites(
        gix::objs::TreeRefIter::from_bytes(&old_tree.data, old_tree.id.kind()),
        gix::objs::TreeRefIter::from_bytes(&new_tree.data, new_tree.id.kind()),
        &mut cache,
        &mut state,
        &repo.objects,
        |change| {
            if failure.is_some() {
                return Ok(std::ops::ControlFlow::Break(()));
            }
            if let Err(err) = record(&change, &mut changes, &mut stats) {
                failure = Some(err);
                return Ok(std::ops::ControlFlow::Break(()));
            }
            Ok(std::ops::ControlFlow::Continue(()))
        },
        options,
    );

    if let Some(failure) = failure {
        // A break makes gix report `Error::Cancelled`; the captured cause is the real one.
        return Err(failure);
    }
    let outcome = outcome.map_err(|e| TreeDiffError::Gix(e.to_string()))?;

    emit_copy_source_modifications(repo, old_tree, &mut changes)?;

    if let Some(outcome) = outcome {
        stats.rename_limit_hit =
            outcome.num_similarity_checks_skipped_for_rename_tracking_due_to_limit > 0
                || outcome.num_similarity_checks_skipped_for_copy_tracking_due_to_limit > 0;
    }

    changes.sort_by(|a, b| {
        a.path
            .as_str()
            .cmp(b.path.as_str())
            .then_with(|| rank(a.kind).cmp(&rank(b.kind)))
    });
    Ok(TreeDiffResult { changes, stats })
}

/// Peel `base` and `head` to their trees and diff them (the INC-001 entry point).
pub fn tree_changes(
    repo: &gix::Repository,
    base: &gix::ObjectId,
    head: &gix::ObjectId,
    opts: &TreeDiffOptions,
) -> Result<TreeDiffResult, TreeDiffError> {
    let base_commit = repo
        .find_commit(*base)
        .map_err(|_| TreeDiffError::ObjectNotFound(base.to_string()))?;
    let head_commit = repo
        .find_commit(*head)
        .map_err(|_| TreeDiffError::ObjectNotFound(head.to_string()))?;
    let base_tree = base_commit
        .tree()
        .map_err(|e| TreeDiffError::ObjectCorrupt(format!("{base}: {e}")))?;
    let head_tree = head_commit
        .tree()
        .map_err(|e| TreeDiffError::ObjectCorrupt(format!("{head}: {e}")))?;
    diff_trees(repo, &base_tree, &head_tree, opts)
}

fn rank(kind: RawKind) -> u8 {
    match kind {
        RawKind::Deleted => 0,
        RawKind::Added => 1,
        RawKind::Modified => 2,
        RawKind::Renamed => 3,
        RawKind::Copied => 4,
    }
}

fn record(
    change: &gix::diff::tree_with_rewrites::ChangeRef<'_>,
    out: &mut Vec<RawTreeChange>,
    stats: &mut TreeDiffStats,
) -> Result<(), TreeDiffError> {
    use gix::diff::tree_with_rewrites::ChangeRef;

    match change {
        ChangeRef::Addition {
            location,
            entry_mode,
            id,
            ..
        } => {
            if entry_mode.is_tree() {
                return Ok(());
            }
            if entry_mode.is_commit() {
                stats.skipped_submodule += 1;
                return Ok(());
            }
            if entry_mode.is_link() {
                stats.skipped_symlink += 1;
                return Ok(());
            }
            let Some(path) = to_path(location, stats) else {
                return Ok(());
            };
            out.push(RawTreeChange {
                path,
                old_path: None,
                kind: RawKind::Added,
                old_oid: None,
                new_oid: Some(*id),
                old_mode: None,
                new_mode: Some(*entry_mode),
                similarity: None,
            });
        }
        ChangeRef::Deletion {
            location,
            entry_mode,
            id,
            ..
        } => {
            if entry_mode.is_tree() {
                return Ok(());
            }
            if entry_mode.is_commit() {
                stats.skipped_submodule += 1;
                return Ok(());
            }
            if entry_mode.is_link() {
                stats.skipped_symlink += 1;
                return Ok(());
            }
            let Some(path) = to_path(location, stats) else {
                return Ok(());
            };
            out.push(RawTreeChange {
                path,
                old_path: None,
                kind: RawKind::Deleted,
                old_oid: Some(*id),
                new_oid: None,
                old_mode: Some(*entry_mode),
                new_mode: None,
                similarity: None,
            });
        }
        ChangeRef::Modification {
            location,
            previous_entry_mode,
            previous_id,
            entry_mode,
            id,
            ..
        } => {
            if previous_entry_mode.is_tree() && entry_mode.is_tree() {
                return Ok(());
            }
            if previous_entry_mode.is_commit() || entry_mode.is_commit() {
                stats.skipped_submodule += 1;
                return Ok(());
            }
            let previous_kind = previous_entry_mode.kind();
            let kind = entry_mode.kind();
            if previous_kind == kind {
                if previous_id == id {
                    return Ok(());
                }
                if entry_mode.is_link() {
                    stats.skipped_symlink += 1;
                    return Ok(());
                }
                let Some(path) = to_path(location, stats) else {
                    return Ok(());
                };
                out.push(RawTreeChange {
                    path,
                    old_path: None,
                    kind: RawKind::Modified,
                    old_oid: Some(*previous_id),
                    new_oid: Some(*id),
                    old_mode: Some(*previous_entry_mode),
                    new_mode: Some(*entry_mode),
                    similarity: None,
                });
                return Ok(());
            }
            // Type change: the sides are no longer the same kind of entry.
            let Some(path) = to_path(location, stats) else {
                return Ok(());
            };
            out.push(RawTreeChange {
                path: path.clone(),
                old_path: None,
                kind: RawKind::Deleted,
                old_oid: Some(*previous_id),
                new_oid: None,
                old_mode: Some(*previous_entry_mode),
                new_mode: None,
                similarity: None,
            });
            if kind == gix::objs::tree::EntryKind::Blob {
                out.push(RawTreeChange {
                    path,
                    old_path: None,
                    kind: RawKind::Added,
                    old_oid: None,
                    new_oid: Some(*id),
                    old_mode: None,
                    new_mode: Some(*entry_mode),
                    similarity: None,
                });
            } else {
                stats.skipped_symlink += 1;
            }
        }
        ChangeRef::Rewrite {
            location,
            source_location,
            source_entry_mode,
            source_id,
            entry_mode,
            id,
            diff,
            copy,
            ..
        } => {
            if source_entry_mode.is_tree() || entry_mode.is_tree() {
                return Ok(());
            }
            if source_entry_mode.is_commit() || entry_mode.is_commit() {
                stats.skipped_submodule += 1;
                return Ok(());
            }
            if source_entry_mode.is_link() || entry_mode.is_link() {
                stats.skipped_symlink += 1;
                return Ok(());
            }
            let Some(path) = to_path(location, stats) else {
                return Ok(());
            };
            let Some(old_path) = to_path(source_location, stats) else {
                return Ok(());
            };
            let similarity = match diff {
                None => 100,
                Some(line_stats) => (line_stats.similarity * 100.0).round().clamp(0.0, 100.0) as u8,
            };
            out.push(RawTreeChange {
                path,
                old_path: Some(old_path),
                kind: if *copy {
                    RawKind::Copied
                } else {
                    RawKind::Renamed
                },
                old_oid: Some(*source_id),
                new_oid: Some(*id),
                old_mode: Some(*source_entry_mode),
                new_mode: Some(*entry_mode),
                similarity: Some(similarity),
            });
        }
    }
    Ok(())
}

fn to_path(location: &gix::bstr::BStr, stats: &mut TreeDiffStats) -> Option<RepoPath> {
    let text = std::str::from_utf8(location).ok()?;
    match RepoPath::new(text) {
        Ok(path) => Some(path),
        Err(_) => {
            stats.skipped_paths += 1;
            None
        }
    }
}

/// gix marks *both* sides of a matched copy as emitted, which silently drops the source
/// file's own modification event; git prints `M <source>` next to `C100 <source> <dest>`.
/// This recovers the missing `Modified` entry from the old tree.
fn emit_copy_source_modifications(
    repo: &gix::Repository,
    old_tree: &gix::Tree,
    changes: &mut Vec<RawTreeChange>,
) -> Result<(), TreeDiffError> {
    let mut recovered: Vec<RawTreeChange> = Vec::new();
    for change in changes.iter().filter(|c| c.kind == RawKind::Copied) {
        let Some(source) = change.old_path.clone() else {
            continue;
        };
        if changes.iter().any(|c| c.path == source) || recovered.iter().any(|c| c.path == source) {
            continue;
        }
        let Some((old_oid, old_mode)) = lookup_entry(repo, old_tree, &source)? else {
            continue;
        };
        let (Some(new_oid), Some(new_mode)) = (change.old_oid, change.old_mode) else {
            continue;
        };
        if old_oid == new_oid {
            continue;
        }
        recovered.push(RawTreeChange {
            path: source,
            old_path: None,
            kind: RawKind::Modified,
            old_oid: Some(old_oid),
            new_oid: Some(new_oid),
            old_mode: Some(old_mode),
            new_mode: Some(new_mode),
            similarity: None,
        });
    }
    changes.extend(recovered);
    Ok(())
}

/// Resolve a single path inside `root`, component by component.
fn lookup_entry(
    repo: &gix::Repository,
    root: &gix::Tree,
    path: &RepoPath,
) -> Result<Option<(gix::ObjectId, gix::object::tree::EntryMode)>, TreeDiffError> {
    let parts: Vec<&str> = path.as_str().split('/').filter(|p| !p.is_empty()).collect();
    let mut current = root.clone();
    for (idx, part) in parts.iter().enumerate() {
        let found = current
            .find_entry(*part)
            .map(|e| (e.oid().to_owned(), e.mode()));
        let Some((oid, mode)) = found else {
            return Ok(None);
        };
        if idx + 1 == parts.len() {
            return Ok(Some((oid, mode)));
        }
        if !mode.is_tree() {
            return Ok(None);
        }
        current = repo
            .find_tree(oid)
            .map_err(|e| TreeDiffError::ObjectCorrupt(format!("tree {oid}: {e}")))?;
    }
    Ok(None)
}
