//! `ProfileStore`: where profiles are persisted (PROF-001).
//!
//! The port, the in-memory adapter and the `.review/profile/` file adapter live here; the
//! PostgreSQL adapter is [`crate::pg::PgProfileStore`] (feature `pg`). Every adapter must pass
//! [`conformance::run_conformance`]. Profiles are immutable per [`ProfileKey`]: a second save of
//! the same key is a no-op, so concurrent computations converge.

pub mod file;

#[cfg(any(test, feature = "conformance"))]
pub mod conformance;

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Mutex;

use repository::store::RepoScope;
use review_core::ids::SnapshotId;

use crate::model::{ProfileKey, RepositoryProfile};

pub use file::FileProfileStore;

/// Largest serialized profile any adapter accepts.
pub const MAX_PROFILE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ProfileStoreError {
    #[error("profile is {bytes} bytes; the limit is {limit}")]
    TooLarge { bytes: usize, limit: usize },
    #[error("profile belongs to repository {profile} but the scope is {scope}")]
    ScopeMismatch { profile: String, scope: String },
    #[error("store backend: {0}")]
    Backend(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SavedProfile {
    /// False when the key was already stored: the existing profile is kept.
    pub created: bool,
}

/// Persistence of profiles, tenant-scoped and keyed by [`ProfileKey`].
pub trait ProfileStore: Send + Sync {
    /// Idempotent on the key: an existing row wins (`INSERT ... ON CONFLICT DO NOTHING`).
    fn save(
        &self,
        scope: &RepoScope,
        profile: &RepositoryProfile,
    ) -> impl Future<Output = Result<SavedProfile, ProfileStoreError>> + Send;

    fn get(
        &self,
        scope: &RepoScope,
        key: &ProfileKey,
    ) -> impl Future<Output = Result<Option<RepositoryProfile>, ProfileStoreError>> + Send;

    /// The newest profile computed for a snapshot, whatever its version and config hash.
    fn latest_for_snapshot(
        &self,
        scope: &RepoScope,
        snapshot_id: SnapshotId,
    ) -> impl Future<Output = Result<Option<RepositoryProfile>, ProfileStoreError>> + Send;
}

/// Checks every store applies before writing: scope and size. Returns the JSON bytes.
pub fn validate_for_save(
    scope: &RepoScope,
    profile: &RepositoryProfile,
) -> Result<Vec<u8>, ProfileStoreError> {
    if profile.repository_id != scope.repository_id {
        return Err(ProfileStoreError::ScopeMismatch {
            profile: profile.repository_id.to_string(),
            scope: scope.repository_id.to_string(),
        });
    }
    let bytes =
        serde_json::to_vec(profile).map_err(|e| ProfileStoreError::Backend(e.to_string()))?;
    if bytes.len() > MAX_PROFILE_BYTES {
        return Err(ProfileStoreError::TooLarge {
            bytes: bytes.len(),
            limit: MAX_PROFILE_BYTES,
        });
    }
    Ok(bytes)
}

/// An in-process store, for tests and the local CLI.
#[derive(Debug, Default)]
pub struct MemoryProfileStore {
    rows: Mutex<BTreeMap<(String, String), RepositoryProfile>>,
}

impl MemoryProfileStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn row_key(scope: &RepoScope, key: &ProfileKey) -> (String, String) {
        (
            scope.organization_id.to_string(),
            format!(
                "{}/{}/{}/{}",
                key.repository_id, key.snapshot_id, key.profile_version.0, key.config_hash
            ),
        )
    }

    /// How many profiles are stored.
    pub fn len(&self) -> usize {
        self.rows.lock().map(|r| r.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

fn poisoned<T>(_: T) -> ProfileStoreError {
    ProfileStoreError::Backend("store lock poisoned".to_owned())
}

impl ProfileStore for MemoryProfileStore {
    async fn save(
        &self,
        scope: &RepoScope,
        profile: &RepositoryProfile,
    ) -> Result<SavedProfile, ProfileStoreError> {
        validate_for_save(scope, profile)?;
        let mut rows = self.rows.lock().map_err(poisoned)?;
        let key = Self::row_key(scope, &profile.key());
        if rows.contains_key(&key) {
            return Ok(SavedProfile { created: false });
        }
        rows.insert(key, profile.clone());
        Ok(SavedProfile { created: true })
    }

    async fn get(
        &self,
        scope: &RepoScope,
        key: &ProfileKey,
    ) -> Result<Option<RepositoryProfile>, ProfileStoreError> {
        let rows = self.rows.lock().map_err(poisoned)?;
        Ok(rows.get(&Self::row_key(scope, key)).cloned())
    }

    async fn latest_for_snapshot(
        &self,
        scope: &RepoScope,
        snapshot_id: SnapshotId,
    ) -> Result<Option<RepositoryProfile>, ProfileStoreError> {
        let rows = self.rows.lock().map_err(poisoned)?;
        let org = scope.organization_id.to_string();
        Ok(rows
            .iter()
            .filter(|((o, _), p)| {
                *o == org
                    && p.repository_id == scope.repository_id
                    && p.snapshot_id == snapshot_id
            })
            .map(|(_, p)| p)
            .max_by(|a, b| {
                a.computed_at
                    .cmp(&b.computed_at)
                    .then(a.profile_version.cmp(&b.profile_version))
            })
            .cloned())
    }
}
