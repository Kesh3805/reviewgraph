//! Commit trees and tree listings (DIFF-001).

use review_core::ids::CommitSha;
use review_core::location::RepoPath;

use super::{metrics, GitError, GitRepo};

/// Kind of a non-tree entry listed by [`GitRepo::list_tree`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TreeEntryKind {
    /// Regular file.
    Blob,
    /// Symbolic link.
    Link,
    /// Git submodule gitlink entry.
    Submodule,
}

/// One non-tree entry reachable from a commit's tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeEntry {
    /// Repository-relative path of the entry.
    pub path: RepoPath,
    /// Entry kind.
    pub kind: TreeEntryKind,
    /// Object id the entry points at.
    pub oid: gix::ObjectId,
}

impl GitRepo {
    /// Tree object id recorded in `commit`.
    pub fn tree_of(&self, commit: &CommitSha) -> Result<gix::ObjectId, GitError> {
        let repo = self.local();
        let c_oid = self.resolve_commit_in(&repo, commit)?;
        let commit_obj = repo
            .find_commit(c_oid)
            .map_err(|e| GitError::Corrupt(format!("commit {c_oid}: {e}")))?;
        commit_obj
            .tree_id()
            .map(|tree| tree.detach())
            .map_err(|e| GitError::Corrupt(format!("commit {c_oid}: {e}")))
    }

    /// Recursively list every non-tree entry reachable from `commit`, optionally restricted to
    /// `prefix`. A missing or non-tree prefix yields an empty listing. Results are sorted by
    /// path. Fails with [`GitError::TreeTooLarge`] when more than `max_tree_entries` entries
    /// exist.
    pub fn list_tree(
        &self,
        commit: &CommitSha,
        prefix: Option<&RepoPath>,
    ) -> Result<Vec<TreeEntry>, GitError> {
        let repo = self.local();
        let c_oid = self.resolve_commit_in(&repo, commit)?;
        let commit_obj = repo
            .find_commit(c_oid)
            .map_err(|e| GitError::Corrupt(format!("commit {c_oid}: {e}")))?;
        let mut tree = commit_obj
            .tree()
            .map_err(|e| GitError::Corrupt(format!("commit {c_oid}: {e}")))?;
        let mut root_oid = commit_obj
            .tree_id()
            .map(|tree| tree.detach())
            .map_err(|e| GitError::Corrupt(format!("commit {c_oid}: {e}")))?;
        if let Some(prefix) = prefix {
            for component in prefix.as_str().split('/') {
                let Some(entry) = tree.find_entry(component.as_bytes()) else {
                    return Ok(Vec::new());
                };
                if !entry.mode().is_tree() {
                    return Ok(Vec::new());
                }
                root_oid = entry.oid().to_owned();
                tree = repo
                    .find_tree(root_oid)
                    .map_err(|e| GitError::Corrupt(format!("tree {root_oid}: {e}")))?;
                metrics::record_object_read("tree");
            }
        }
        let mut out = Vec::new();
        let base = prefix.map(|p| p.as_str().to_string()).unwrap_or_default();
        let mut stack: Vec<(gix::ObjectId, String)> = vec![(root_oid, base)];
        while let Some((tree_oid, dir)) = stack.pop() {
            let tree = repo
                .find_tree(tree_oid)
                .map_err(|e| GitError::Corrupt(format!("tree {tree_oid}: {e}")))?;
            metrics::record_object_read("tree");
            for entry in tree.iter() {
                let entry =
                    entry.map_err(|e| GitError::Corrupt(format!("tree {tree_oid}: {e}")))?;
                let name = entry.filename().to_string();
                let path = if dir.is_empty() {
                    name
                } else {
                    format!("{dir}/{name}")
                };
                let mode = entry.mode();
                if mode.is_tree() {
                    stack.push((entry.oid().to_owned(), path));
                    continue;
                }
                if out.len() as u32 >= self.limits.max_tree_entries {
                    return Err(GitError::TreeTooLarge {
                        limit: self.limits.max_tree_entries,
                    });
                }
                let path = RepoPath::new(&path).map_err(|e| GitError::Corrupt(format!("{e}")))?;
                let kind = if mode.is_link() {
                    TreeEntryKind::Link
                } else if mode.is_commit() {
                    TreeEntryKind::Submodule
                } else {
                    TreeEntryKind::Blob
                };
                out.push(TreeEntry {
                    path,
                    kind,
                    oid: entry.oid().to_owned(),
                });
            }
        }
        out.sort_by(|a, b| a.path.as_str().cmp(b.path.as_str()));
        Ok(out)
    }
}
