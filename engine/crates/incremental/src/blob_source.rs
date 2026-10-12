//! Where file bytes come from (INC-002).
//!
//! [`BlobSource`] reads the bytes of a *changed* path at the head commit. Production reads the
//! object database ([`GitBlobSource`]); tests use [`MemBlobSource`], which also records every read
//! so a test can prove that unchanged files are never touched.

use std::collections::BTreeMap;
use std::sync::Mutex;

use review_core::location::RepoPath;

/// What a read produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlobRead {
    /// The full content.
    Bytes(Vec<u8>),
    /// The blob is larger than the limit; carries its size.
    TooLarge(u64),
    /// The blob looks binary (contains a NUL byte near the start).
    Binary,
}

/// A read failed (object missing, corrupt, I/O).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct BlobError(pub String);

/// Reads blobs by path and optional object id.
pub trait BlobSource: Send + Sync {
    /// Reads at most `max_bytes` of the blob at `path`. `oid` is the hex object id from the tree
    /// diff when the caller has one; in-memory sources ignore it.
    fn read(
        &self,
        path: &RepoPath,
        oid: Option<&str>,
        max_bytes: u64,
    ) -> Result<BlobRead, BlobError>;
}

/// How many leading bytes are scanned for a NUL when classifying a blob as binary.
const BINARY_SCAN: usize = 8000;

/// Classifies raw bytes the way git does: a NUL in the first 8000 bytes means binary.
pub fn classify(bytes: Vec<u8>, max_bytes: u64) -> BlobRead {
    if bytes.len() as u64 > max_bytes {
        return BlobRead::TooLarge(bytes.len() as u64);
    }
    let scan = bytes.len().min(BINARY_SCAN);
    if bytes.iter().take(scan).any(|b| *b == 0) {
        return BlobRead::Binary;
    }
    BlobRead::Bytes(bytes)
}

/// Reads blobs from a git object database by object id.
pub struct GitBlobSource {
    repo: gix::ThreadSafeRepository,
}

impl std::fmt::Debug for GitBlobSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitBlobSource").finish_non_exhaustive()
    }
}

impl GitBlobSource {
    /// Wraps a repository handle; every read uses a thread-local clone of it.
    pub fn new(repo: gix::Repository) -> Self {
        Self {
            repo: repo.into_sync(),
        }
    }
}

impl BlobSource for GitBlobSource {
    fn read(
        &self,
        path: &RepoPath,
        oid: Option<&str>,
        max_bytes: u64,
    ) -> Result<BlobRead, BlobError> {
        let Some(oid) = oid else {
            return Err(BlobError(format!("no object id for {path}")));
        };
        let id = gix::ObjectId::from_hex(oid.as_bytes())
            .map_err(|e| BlobError(format!("{path}: bad object id: {e}")))?;
        let repo = self.repo.to_thread_local();
        let object = repo
            .find_object(id)
            .map_err(|e| BlobError(format!("{path}: {e}")))?;
        Ok(classify(object.data.clone(), max_bytes))
    }
}

/// An in-memory file map that records every path read.
#[derive(Debug, Default)]
pub struct MemBlobSource {
    files: BTreeMap<RepoPath, Vec<u8>>,
    reads: Mutex<Vec<RepoPath>>,
}

impl MemBlobSource {
    /// An empty source.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds or replaces a file.
    pub fn insert(&mut self, path: RepoPath, bytes: impl Into<Vec<u8>>) {
        self.files.insert(path, bytes.into());
    }

    /// Every path read so far, in read order.
    pub fn reads(&self) -> Vec<RepoPath> {
        self.reads
            .lock()
            .map(|reads| reads.clone())
            .unwrap_or_default()
    }
}

impl BlobSource for MemBlobSource {
    fn read(
        &self,
        path: &RepoPath,
        _oid: Option<&str>,
        max_bytes: u64,
    ) -> Result<BlobRead, BlobError> {
        if let Ok(mut reads) = self.reads.lock() {
            reads.push(path.clone());
        }
        match self.files.get(path) {
            Some(bytes) => Ok(classify(bytes.clone(), max_bytes)),
            None => Err(BlobError(format!("{path}: not in the in-memory source"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_detects_binary_and_oversize() {
        assert_eq!(
            classify(b"abc".to_vec(), 10),
            BlobRead::Bytes(b"abc".to_vec())
        );
        assert_eq!(classify(vec![b'a', 0, b'b'], 10), BlobRead::Binary);
        assert_eq!(classify(vec![b'a'; 11], 10), BlobRead::TooLarge(11));
    }
}
