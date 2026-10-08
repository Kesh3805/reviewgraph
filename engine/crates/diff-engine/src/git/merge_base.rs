//! Merge base computation (DIFF-001).

use std::str::FromStr;

use review_core::ids::CommitSha;

use super::{GitError, GitRepo};

/// Result of [`GitRepo::merge_base`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum MergeBase {
    /// The commits share no common ancestor (unrelated histories); the caller diffs against the
    /// base commit directly and warns.
    None,
    /// A common ancestor was found. `candidates` is the number of merge bases that were
    /// returned; when there is more than one, the reported `sha` was chosen deterministically
    /// by (committer time descending, object id ascending).
    Found { sha: CommitSha, candidates: u8 },
}

impl GitRepo {
    /// Compute the merge base of two commits.
    pub fn merge_base(&self, a: &CommitSha, b: &CommitSha) -> Result<MergeBase, GitError> {
        let span = tracing::debug_span!("git.merge_base", candidates = tracing::field::Empty);
        let _enter = span.enter();
        let repo = self.local();
        let a_oid = self.resolve_commit_in(&repo, a)?;
        let b_oid = self.resolve_commit_in(&repo, b)?;
        self.ensure_parents_present(&repo, a_oid)?;
        self.ensure_parents_present(&repo, b_oid)?;
        let bases = repo
            .merge_bases_many(a_oid, &[b_oid])
            .map_err(|e| walk_error(&repo, e.to_string()))?;
        if bases.is_empty() {
            span.record("candidates", 0);
            return Ok(MergeBase::None);
        }
        let mut keyed: Vec<(i64, gix::ObjectId)> = Vec::with_capacity(bases.len());
        for base in bases {
            let oid = base.detach();
            let commit = repo
                .find_commit(oid)
                .map_err(|e| GitError::Corrupt(format!("commit {oid}: {e}")))?;
            let seconds = commit
                .time()
                .map(|time| time.seconds)
                .map_err(|e| GitError::Corrupt(format!("commit {oid}: {e}")))?;
            keyed.push((seconds, oid));
        }
        keyed.sort_by(|x, y| y.0.cmp(&x.0).then_with(|| x.1.cmp(&y.1)));
        let candidates = u8::try_from(keyed.len()).unwrap_or(u8::MAX);
        let Some((_, best)) = keyed.first().copied() else {
            span.record("candidates", 0);
            return Ok(MergeBase::None);
        };
        let sha =
            CommitSha::from_str(&best.to_string()).map_err(|e| GitError::Corrupt(e.to_string()))?;
        span.record("candidates", candidates);
        Ok(MergeBase::Found { sha, candidates })
    }

    fn ensure_parents_present(
        &self,
        repo: &gix::Repository,
        oid: gix::ObjectId,
    ) -> Result<(), GitError> {
        let commit = repo
            .find_commit(oid)
            .map_err(|e| GitError::Corrupt(format!("commit {oid}: {e}")))?;
        for parent in commit.parent_ids() {
            let parent_oid = parent.detach();
            match repo.try_find_object(parent_oid) {
                Ok(None) => return Err(self.missing_object(repo, parent_oid)),
                Ok(Some(_)) => {}
                Err(e) => return Err(GitError::Gix(e.to_string())),
            }
        }
        Ok(())
    }
}

fn walk_error(repo: &gix::Repository, message: String) -> GitError {
    if repo.is_shallow() {
        GitError::ShallowBoundary { missing: message }
    } else {
        GitError::Gix(message)
    }
}
