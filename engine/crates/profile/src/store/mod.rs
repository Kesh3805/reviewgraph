//! The `ProfileStore` port and its adapters (PROF-001). Profiles are immutable per natural key:
//! a second `put` of the same key is a no-op, so concurrent computations converge.

#[cfg(feature = "conformance")]
pub mod conformance;
pub mod file;

use thiserror::Error;

use crate::model::{ProfileKey, RepositoryProfile};

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum StoreError {
    #[error("io: {0}")]
    Io(String),
    #[error("serialization: {0}")]
    Serialization(String),
}

/// Synchronous profile persistence port. The PostgreSQL adapter (`pg::PgProfileStore`) offers
/// the same two operations asynchronously.
pub trait ProfileStore {
    /// Stores the profile; returns `false` when the key already existed (a no-op).
    fn put(&self, profile: &RepositoryProfile) -> Result<bool, StoreError>;
    fn get(&self, key: &ProfileKey) -> Result<Option<RepositoryProfile>, StoreError>;
}
