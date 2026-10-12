//! `manifest.json`: the source of truth of the file adapter (GS-006).
//!
//! The manifest is rewritten atomically after the snapshot file it describes is durable, so a
//! crash leaves either the old manifest or the new one, never one that points at a missing file.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use chrono::{DateTime, Utc};
use repository::store::RepoScope;
use review_core::ids::{CommitSha, OrganizationId, RepositoryId, SnapshotId};
use serde::{Deserialize, Serialize};

use super::atomic;
use super::container::integrity;
use crate::status::SnapshotStatus;
use crate::types::{SnapshotKind, SnapshotMeta, SnapshotPurpose, SnapshotStats, SnapshotVersions};
use crate::StoreError;

pub(crate) const MANIFEST_NAME: &str = "manifest.json";
pub(crate) const SNAPSHOT_DIR: &str = "snapshots";
pub(crate) const LOCK_NAME: &str = ".lock";
pub(crate) const MANIFEST_FORMAT: u32 = 1;

/// Guard against a self-referential chain while walking `base` links.
const MAX_CHAIN_DEPTH: usize = 10_000;

/// The whole manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Manifest {
    pub format: u32,
    /// Informational: a stable hex digest of the first scope that wrote here.
    pub repository_key: Option<String>,
    /// Monotonic sequence that orders `find_ready` ties deterministically.
    pub seq: u64,
    /// Last file-version id handed out.
    pub next_file_version: i64,
    pub snapshots: Vec<SnapshotEntry>,
    /// Newest `Ready` snapshot per `"{scope}:{purpose}"`; retention never deletes these.
    pub heads: BTreeMap<String, SnapshotId>,
    /// Per scope, `FileVersionKey::manifest_key` to the local id.
    pub file_versions: BTreeMap<String, BTreeMap<String, i64>>,
}

impl Default for Manifest {
    fn default() -> Self {
        Self {
            format: MANIFEST_FORMAT,
            repository_key: None,
            seq: 0,
            next_file_version: 0,
            snapshots: Vec::new(),
            heads: BTreeMap::new(),
            file_versions: BTreeMap::new(),
        }
    }
}

/// One snapshot, as the manifest records it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SnapshotEntry {
    pub id: SnapshotId,
    pub organization_id: OrganizationId,
    pub repository_id: RepositoryId,
    pub commit_sha: CommitSha,
    pub kind: SnapshotKind,
    pub base: Option<SnapshotId>,
    pub chain_depth: u16,
    pub purpose: SnapshotPurpose,
    pub status: SnapshotStatus,
    pub versions: SnapshotVersions,
    pub stats: SnapshotStats,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub seq: u64,
    /// File name inside `snapshots/`, derived from `id`; `None` until a `write_*` call.
    pub file: Option<String>,
    /// Size of the snapshot file, verified before decoding.
    pub bytes: u64,
    /// Lowercase hex blake3 of the whole snapshot file, verified before decoding.
    pub blake3: Option<String>,
}

impl SnapshotEntry {
    pub fn scope(&self) -> RepoScope {
        RepoScope {
            organization_id: self.organization_id,
            repository_id: self.repository_id,
        }
    }

    pub fn meta(&self) -> SnapshotMeta {
        SnapshotMeta {
            id: self.id,
            scope: self.scope(),
            commit_sha: self.commit_sha.clone(),
            kind: self.kind,
            base: self.base,
            chain_depth: self.chain_depth,
            purpose: self.purpose,
            status: self.status,
            versions: self.versions.clone(),
            stats: self.stats.clone(),
            error: self.error.clone(),
            created_at: self.created_at,
            updated_at: self.updated_at,
            completed_at: self.completed_at,
        }
    }
}

/// `"{organization}/{repository}"`, the key of the per-scope maps.
pub(crate) fn scope_key(scope: &RepoScope) -> String {
    format!("{}/{}", scope.organization_id, scope.repository_id)
}

impl Manifest {
    /// The entry of `id`, `NotFound` when it does not exist *or* belongs to another tenant.
    pub fn entry(&self, scope: &RepoScope, id: SnapshotId) -> Result<&SnapshotEntry, StoreError> {
        self.snapshots
            .iter()
            .find(|entry| entry.id == id && entry.scope() == *scope)
            .ok_or(StoreError::NotFound(id))
    }

    pub fn entry_mut(
        &mut self,
        scope: &RepoScope,
        id: SnapshotId,
    ) -> Result<&mut SnapshotEntry, StoreError> {
        self.snapshots
            .iter_mut()
            .find(|entry| entry.id == id && entry.scope() == *scope)
            .ok_or(StoreError::NotFound(id))
    }

    /// `[root, ..., id]`, oldest first, every entry `Ready`.
    pub fn chain_ids(
        &self,
        scope: &RepoScope,
        id: SnapshotId,
    ) -> Result<Vec<SnapshotId>, StoreError> {
        let mut ids = Vec::new();
        let mut seen = BTreeSet::new();
        let mut cursor = id;
        loop {
            if !seen.insert(cursor) || ids.len() > MAX_CHAIN_DEPTH {
                return Err(StoreError::ChainBroken { id });
            }
            let entry = self.entry(scope, cursor)?;
            if !entry.status.is_readable() {
                return Err(StoreError::InvalidStatus {
                    id: cursor,
                    expected: SnapshotStatus::Ready,
                    found: entry.status,
                });
            }
            ids.push(cursor);
            match entry.base {
                Some(base) => cursor = base,
                None => break,
            }
        }
        ids.reverse();
        let Some(&root) = ids.first() else {
            return Err(StoreError::ChainBroken { id });
        };
        if self.entry(scope, root)?.kind != SnapshotKind::Full {
            return Err(StoreError::ChainBroken { id });
        }
        Ok(ids)
    }

    /// Snapshot ids retention must keep: the newest `keep_latest` `Ready` snapshots, every head,
    /// every snapshot that is not `Ready` yet, and the base chain of all of those.
    pub fn retained(&self, keep_latest: usize) -> BTreeSet<SnapshotId> {
        let mut ready: Vec<&SnapshotEntry> = self
            .snapshots
            .iter()
            .filter(|entry| entry.status == SnapshotStatus::Ready)
            .collect();
        ready.sort_by_key(|entry| std::cmp::Reverse((entry.created_at, entry.seq)));
        let mut keep: BTreeSet<SnapshotId> = ready
            .iter()
            .take(keep_latest)
            .map(|entry| entry.id)
            .collect();
        keep.extend(self.heads.values().copied());
        keep.extend(
            self.snapshots
                .iter()
                .filter(|entry| entry.status != SnapshotStatus::Ready)
                .map(|entry| entry.id),
        );
        let mut frontier: Vec<SnapshotId> = keep.iter().copied().collect();
        while let Some(id) = frontier.pop() {
            let base = self
                .snapshots
                .iter()
                .find(|entry| entry.id == id)
                .and_then(|entry| entry.base);
            if let Some(base) = base {
                if keep.insert(base) {
                    frontier.push(base);
                }
            }
        }
        keep
    }
}

/// Reads the manifest of `root`; a missing manifest is an empty store.
pub(crate) fn read(root: &Path) -> Result<Manifest, StoreError> {
    let bytes = match std::fs::read(root.join(MANIFEST_NAME)) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Manifest::default());
        }
        Err(error) => return Err(StoreError::backend(error)),
    };
    let manifest: Manifest = serde_json::from_slice(&bytes).map_err(|error| {
        integrity(format!(
            "manifest.json is corrupt ({error}); run `review graph rebuild`"
        ))
    })?;
    if manifest.format != MANIFEST_FORMAT {
        return Err(integrity(format!(
            "manifest.json has format {}, this build reads {MANIFEST_FORMAT}; run `review graph rebuild`",
            manifest.format
        )));
    }
    Ok(manifest)
}

/// Writes the manifest of `root` atomically.
pub(crate) fn write(root: &Path, manifest: &Manifest) -> Result<(), StoreError> {
    let bytes = serde_json::to_vec_pretty(manifest).map_err(StoreError::backend)?;
    atomic::write_atomic(root, MANIFEST_NAME, &bytes).map_err(StoreError::backend)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn missing_manifest_is_an_empty_store() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = read(dir.path()).unwrap();
        assert!(manifest.snapshots.is_empty());
        assert_eq!(manifest.format, MANIFEST_FORMAT);
    }

    #[test]
    fn manifest_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = Manifest {
            seq: 3,
            next_file_version: 7,
            ..Manifest::default()
        };
        write(dir.path(), &manifest).unwrap();
        let back = read(dir.path()).unwrap();
        assert_eq!(back.seq, 3);
        assert_eq!(back.next_file_version, 7);
    }

    #[test]
    fn corrupt_manifest_is_an_integrity_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(MANIFEST_NAME), b"{ not json").unwrap();
        match read(dir.path()) {
            Err(StoreError::Integrity(message)) => {
                assert!(message.contains("review graph rebuild"))
            }
            other => panic!("expected an integrity error, got {other:?}"),
        }
    }
}
