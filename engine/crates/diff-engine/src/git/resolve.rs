//! Commit resolution (DIFF-001).

use review_core::ids::CommitSha;

use super::{metrics, GitError, GitRepo};

impl GitRepo {
    /// Resolve a full 40- or 64-hex commit id, verifying it exists and is a commit.
    /// Refs are resolved by the caller from provider metadata.
    pub fn resolve_commit(&self, sha: &CommitSha) -> Result<gix::ObjectId, GitError> {
        let repo = self.local();
        self.resolve_commit_in(&repo, sha)
    }

    /// Whether `sha` names an existing commit object.
    pub fn commit_exists(&self, sha: &CommitSha) -> Result<bool, GitError> {
        let Ok(oid) = parse_oid(sha) else {
            return Ok(false);
        };
        let repo = self.local();
        let found = repo
            .try_find_object(oid)
            .map_err(|e| GitError::Gix(e.to_string()))?;
        Ok(found.is_some_and(|obj| obj.kind == gix::objs::Kind::Commit))
    }

    pub(crate) fn resolve_commit_in(
        &self,
        repo: &gix::Repository,
        sha: &CommitSha,
    ) -> Result<gix::ObjectId, GitError> {
        let oid = parse_oid(sha)?;
        match repo.try_find_object(oid) {
            Ok(Some(obj)) => {
                if obj.kind == gix::objs::Kind::Commit {
                    metrics::record_object_read("commit");
                    Ok(oid)
                } else {
                    Err(GitError::NotACommit {
                        sha: sha.as_str().to_owned(),
                    })
                }
            }
            Ok(None) => Err(self.missing_object(repo, oid)),
            Err(e) => Err(GitError::Gix(e.to_string())),
        }
    }
}

fn parse_oid(sha: &CommitSha) -> Result<gix::ObjectId, GitError> {
    gix::ObjectId::from_hex(sha.as_str().as_bytes())
        .map_err(|_| GitError::InvalidObjectId(sha.as_str().to_owned()))
}
