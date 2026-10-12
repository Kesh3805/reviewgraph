//! `manifest.json` of the file adapter (format 1): the source of truth for which snapshots exist,
//! their status and metadata, and the file-version id map. Snapshot payload files are only
//! reachable through it.

use chrono::{DateTime, Utc};
use repository::store::RepoScope;
use review_core::ids::{CommitSha, OrganizationId, RepositoryId, SnapshotId};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::status::SnapshotStatus;
use crate::types::{
    hex_lower, SnapshotKind, SnapshotMeta, SnapshotPurpose, SnapshotStats, SnapshotVersions,
};
use crate::StoreError;

/// The on-disk format this build reads and writes.
pub const MANIFEST_FORMAT: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: u32,
    /// Monotonic insertion counter; breaks `created_at` ties in `find_ready`.
    pub seq: u64,
    pub next_file_version: i64,
    pub snapshots: Vec<SnapshotEntry>,
    pub file_versions: Vec<FileVersionEntry>,
}

impl Default for Manifest {
    fn default() -> Self {
        Self {
            format: MANIFEST_FORMAT,
            seq: 0,
            next_file_version: 0,
            snapshots: Vec::new(),
            file_versions: Vec::new(),
        }
    }
}

/// One snapshot. `file`, `bytes` and `blake3` describe the payload once a `write_*` committed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotEntry {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub repository_id: Uuid,
    pub commit_sha: String,
    pub kind: SnapshotKind,
    pub base: Option<Uuid>,
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
    pub file: Option<String>,
    pub bytes: u64,
    pub blake3: Option<String>,
}

impl SnapshotEntry {
    pub fn scope(&self) -> RepoScope {
        RepoScope {
            organization_id: OrganizationId::from_uuid(self.organization_id),
            repository_id: RepositoryId::from_uuid(self.repository_id),
        }
    }

    pub fn in_scope(&self, scope: &RepoScope) -> bool {
        self.organization_id == *scope.organization_id.as_uuid()
            && self.repository_id == *scope.repository_id.as_uuid()
    }

    pub fn meta(&self) -> Result<SnapshotMeta, StoreError> {
        Ok(SnapshotMeta {
            id: SnapshotId::from_uuid(self.id),
            scope: self.scope(),
            commit_sha: self
                .commit_sha
                .parse::<CommitSha>()
                .map_err(|_| StoreError::Integrity("manifest commit sha is invalid".to_owned()))?,
            kind: self.kind,
            base: self.base.map(SnapshotId::from_uuid),
            chain_depth: self.chain_depth,
            purpose: self.purpose,
            status: self.status,
            versions: self.versions.clone(),
            stats: self.stats.clone(),
            error: self.error.clone(),
            created_at: self.created_at,
            updated_at: self.updated_at,
            completed_at: self.completed_at,
        })
    }
}

/// One content-addressed file version and its store-local id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileVersionEntry {
    pub organization_id: Uuid,
    pub repository_id: Uuid,
    /// `<path> NUL <hash hex> NUL <analyzer>` (`FileVersionKey::manifest_key`).
    pub key: String,
    pub id: i64,
}

/// Lowercase hex of a payload hash.
pub fn digest(bytes: &[u8]) -> String {
    hex_lower(blake3::hash(bytes).as_bytes())
}
