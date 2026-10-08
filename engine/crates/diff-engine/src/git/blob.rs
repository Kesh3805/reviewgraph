//! Blob reads with size limits (DIFF-001).

use review_core::ids::CommitSha;
use review_core::location::RepoPath;

use super::{metrics, GitError, GitRepo};

/// Blob content read from the object database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blob {
    /// Object id of the blob.
    pub oid: gix::ObjectId,
    /// Size of the decoded content in bytes.
    pub size: u64,
    /// Raw content.
    pub data: Vec<u8>,
}

/// Object header information obtained without inflating the object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjectHeader {
    /// Size of the fully decoded object in bytes.
    pub size: u64,
}

impl GitRepo {
    /// Read a blob from `commit`'s tree. Returns `Ok(None)` when the path does not exist,
    /// [`GitError::NotABlob`] when it exists but is not a regular blob, and
    /// [`GitError::BlobTooLarge`] when the size header exceeds the configured limit.
    pub fn read_blob(&self, commit: &CommitSha, path: &RepoPath) -> Result<Option<Blob>, GitError> {
        let span = tracing::debug_span!("git.read_blob", bytes = tracing::field::Empty);
        let _enter = span.enter();
        let started = std::time::Instant::now();
        let result = self.read_blob_inner(commit, path);
        metrics::record_blob_duration(started.elapsed().as_secs_f64());
        if let Ok(Some(blob)) = &result {
            span.record("bytes", blob.size);
        }
        result
    }

    /// Read blob content directly by object id, enforcing the size limit.
    pub fn read_blob_by_oid(&self, oid: &gix::ObjectId) -> Result<Blob, GitError> {
        let repo = self.local();
        self.load_blob(&repo, *oid, oid.to_string())
    }

    /// Object size without inflating the object (loose/packed header).
    pub fn blob_header(&self, oid: &gix::ObjectId) -> Result<ObjectHeader, GitError> {
        let repo = self.local();
        let header = match repo.try_find_header(*oid) {
            Ok(Some(header)) => header,
            Ok(None) => return Err(self.missing_object(&repo, *oid)),
            Err(e) => return Err(GitError::Gix(e.to_string())),
        };
        Ok(ObjectHeader {
            size: header.size(),
        })
    }

    /// Object size in bytes without inflating the object.
    pub fn blob_size(&self, oid: &gix::ObjectId) -> Result<u64, GitError> {
        Ok(self.blob_header(oid)?.size)
    }

    fn read_blob_inner(
        &self,
        commit: &CommitSha,
        path: &RepoPath,
    ) -> Result<Option<Blob>, GitError> {
        let repo = self.local();
        let c_oid = self.resolve_commit_in(&repo, commit)?;
        let commit_obj = repo
            .find_commit(c_oid)
            .map_err(|e| GitError::Corrupt(format!("commit {c_oid}: {e}")))?;
        let mut tree = commit_obj
            .tree()
            .map_err(|e| GitError::Corrupt(format!("commit {c_oid}: {e}")))?;
        let label = format!("{commit}:{path}");
        let components: Vec<&str> = path.as_str().split('/').collect();
        for (index, component) in components.iter().enumerate() {
            let Some(entry) = tree.find_entry(component.as_bytes()) else {
                return Ok(None);
            };
            let mode = entry.mode();
            if index + 1 == components.len() {
                if !mode.is_blob() {
                    return Err(GitError::NotABlob { object: label });
                }
                let oid = entry.oid().to_owned();
                return self.load_blob(&repo, oid, label).map(Some);
            }
            if !mode.is_tree() {
                return Ok(None);
            }
            let oid = entry.oid().to_owned();
            tree = repo
                .find_tree(oid)
                .map_err(|e| GitError::Corrupt(format!("tree {oid}: {e}")))?;
            metrics::record_object_read("tree");
        }
        Ok(None)
    }

    fn load_blob(
        &self,
        repo: &gix::Repository,
        oid: gix::ObjectId,
        object: String,
    ) -> Result<Blob, GitError> {
        let header = match repo.try_find_header(oid) {
            Ok(Some(header)) => header,
            Ok(None) => return Err(self.missing_object(repo, oid)),
            Err(e) => return Err(GitError::Gix(e.to_string())),
        };
        if header.kind() != gix::objs::Kind::Blob {
            return Err(GitError::NotABlob { object });
        }
        let size = header.size();
        if size > self.limits.max_blob_bytes {
            return Err(GitError::BlobTooLarge {
                oid: oid.to_string(),
                size,
                limit: self.limits.max_blob_bytes,
            });
        }
        let mut obj = repo.find_object(oid).map_err(|e| {
            if repo.is_shallow() {
                self.missing_object(repo, oid)
            } else {
                GitError::Corrupt(format!("blob {oid}: {e}"))
            }
        })?;
        metrics::record_object_read("blob");
        let data = std::mem::take(&mut obj.data);
        Ok(Blob {
            oid,
            size: data.len() as u64,
            data,
        })
    }
}
