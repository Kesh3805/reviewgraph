//! The blob-store port used to spill large stage outputs (PIPE-005). The object-store adapter
//! belongs to the SEC/DEV tasks; this crate only needs the trait and an in-memory double.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;

use super::StoreError;

/// Minimal object store. Keys are tenant-prefixed (`org/{org}/runs/{run}/...`).
#[async_trait]
pub trait BlobStore: Send + Sync + std::fmt::Debug {
    async fn put(&self, key: &str, bytes: Vec<u8>) -> Result<(), StoreError>;
    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, StoreError>;
}

/// In-memory blob store for tests and local runs.
#[derive(Debug, Default)]
pub struct MemoryBlobStore {
    blobs: Mutex<HashMap<String, Vec<u8>>>,
}

impl MemoryBlobStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.blobs.lock().map(|b| b.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Drops a blob (simulates a lost object).
    pub fn remove(&self, key: &str) {
        if let Ok(mut b) = self.blobs.lock() {
            b.remove(key);
        }
    }
}

#[async_trait]
impl BlobStore for MemoryBlobStore {
    async fn put(&self, key: &str, bytes: Vec<u8>) -> Result<(), StoreError> {
        self.blobs
            .lock()
            .map_err(|_| StoreError::Blob("blob store lock poisoned".into()))?
            .insert(key.to_owned(), bytes);
        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, StoreError> {
        Ok(self
            .blobs
            .lock()
            .map_err(|_| StoreError::Blob("blob store lock poisoned".into()))?
            .get(key)
            .cloned())
    }
}
