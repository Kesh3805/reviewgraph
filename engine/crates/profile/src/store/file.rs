//! The local file adapter: `.review/profile/<snapshot>-v<version>-<config hash>.json`, plus a
//! copy of the newest profile at `.review/profile.json` for humans and tools.
//!
//! The scope is only checked for being a real (non-nil) scope: a working directory holds one
//! repository.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use repository::review_dir;
use repository::store::RepoScope;
use review_core::ids::SnapshotId;

use super::{validate_for_save, ProfileStore, ProfileStoreError, SavedProfile};
use crate::model::{ProfileKey, RepositoryProfile};

#[derive(Debug, Clone)]
pub struct FileProfileStore {
    root: PathBuf,
}

fn backend(e: impl std::fmt::Display) -> ProfileStoreError {
    ProfileStoreError::Backend(e.to_string())
}

impl FileProfileStore {
    /// `root` is the repository root (the directory that holds `.review/`).
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn dir(&self) -> PathBuf {
        review_dir::review_dir(&self.root).join("profile")
    }

    fn file_name(key: &ProfileKey) -> String {
        format!(
            "{}-v{}-{}.json",
            key.snapshot_id,
            key.profile_version.0,
            &key.config_hash.to_string()[..16]
        )
    }

    fn check_scope(scope: &RepoScope) -> Result<(), ProfileStoreError> {
        if scope.organization_id.as_uuid().is_nil() || scope.repository_id.as_uuid().is_nil() {
            return Err(ProfileStoreError::Backend(
                "scope must not be nil".to_owned(),
            ));
        }
        Ok(())
    }

    fn read(path: &Path) -> Result<Option<RepositoryProfile>, ProfileStoreError> {
        match fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(backend),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(backend(e)),
        }
    }
}

/// Write-then-rename so readers never see a partial file.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), ProfileStoreError> {
    let dir = path
        .parent()
        .ok_or_else(|| ProfileStoreError::Backend("path has no parent".to_owned()))?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir).map_err(backend)?;
    tmp.write_all(bytes).map_err(backend)?;
    tmp.persist(path).map_err(backend)?;
    Ok(())
}

impl ProfileStore for FileProfileStore {
    async fn save(
        &self,
        scope: &RepoScope,
        profile: &RepositoryProfile,
    ) -> Result<SavedProfile, ProfileStoreError> {
        Self::check_scope(scope)?;
        let bytes = validate_for_save(scope, profile)?;
        let dir = self.dir();
        fs::create_dir_all(&dir).map_err(backend)?;
        let path = dir.join(Self::file_name(&profile.key()));
        if path.exists() {
            return Ok(SavedProfile { created: false });
        }
        write_atomic(&path, &bytes)?;
        let pretty = serde_json::to_vec_pretty(profile).map_err(backend)?;
        write_atomic(&review_dir::review_dir(&self.root).join("profile.json"), &pretty)?;
        Ok(SavedProfile { created: true })
    }

    async fn get(
        &self,
        scope: &RepoScope,
        key: &ProfileKey,
    ) -> Result<Option<RepositoryProfile>, ProfileStoreError> {
        Self::check_scope(scope)?;
        let found = Self::read(&self.dir().join(Self::file_name(key)))?;
        Ok(found.filter(|p| p.repository_id == scope.repository_id && p.key() == *key))
    }

    async fn latest_for_snapshot(
        &self,
        scope: &RepoScope,
        snapshot_id: SnapshotId,
    ) -> Result<Option<RepositoryProfile>, ProfileStoreError> {
        Self::check_scope(scope)?;
        let prefix = format!("{snapshot_id}-v");
        let entries = match fs::read_dir(self.dir()) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(backend(e)),
        };
        let mut best: Option<RepositoryProfile> = None;
        let mut names: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(&prefix) && n.ends_with(".json"))
            })
            .collect();
        names.sort();
        for path in names {
            let Some(profile) = Self::read(&path)? else {
                continue;
            };
            if profile.repository_id != scope.repository_id {
                continue;
            }
            let newer = best.as_ref().is_none_or(|b| {
                (profile.computed_at, profile.profile_version)
                    > (b.computed_at, b.profile_version)
            });
            if newer {
                best = Some(profile);
            }
        }
        Ok(best)
    }
}
