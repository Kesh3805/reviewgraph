//! Qdrant REST adapter (SEM-003). Private to the crate: see [`crate::SemanticIndex`].

pub(crate) mod client;
pub(crate) mod error;
pub(crate) mod types;

pub(crate) use client::QdrantClient;
pub use client::QdrantConfig;
pub use error::QdrantError;
pub use types::{CollectionState, HnswCfg, Payload};
