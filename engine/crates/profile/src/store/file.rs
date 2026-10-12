//! Local-file adapter: one JSON file per profile key under a directory (default `.review/`).

use std::io::Write;
use std::path::PathBuf;

use super::{ProfileStore, StoreError};
use crate::model::{ProfileKey, RepositoryProfile};

#[derive(Debug, Clone)]
pub struct FileProfileStore {
    dir: PathBuf,
}

impl FileProfileStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    fn path(&self, key: &ProfileKey) -> PathBuf {
        self.dir.join(format!(
            "profile-{}-{}-v{}-{}.json",
            key.repository_id, key.snapshot_id, key.profile_version, key.config_hash
        ))
    }
}

impl ProfileStore for FileProfileStore {
    fn put(&self, profile: &RepositoryProfile) -> Result<bool, StoreError> {
        std::fs::create_dir_all(&self.dir).map_err(|e| StoreError::Io(e.to_string()))?;
        let path = self.path(&profile.key());
        let bytes = serde_json::to_vec_pretty(profile)
            .map_err(|e| StoreError::Serialization(e.to_string()))?;
        // create_new makes concurrent writers of the same key converge on the first writer.
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut f) => {
                f.write_all(&bytes)
                    .map_err(|e| StoreError::Io(e.to_string()))?;
                Ok(true)
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
            Err(e) => Err(StoreError::Io(e.to_string())),
        }
    }

    fn get(&self, key: &ProfileKey) -> Result<Option<RepositoryProfile>, StoreError> {
        match std::fs::read(self.path(key)) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|e| StoreError::Serialization(e.to_string())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(StoreError::Io(e.to_string())),
        }
    }
}
