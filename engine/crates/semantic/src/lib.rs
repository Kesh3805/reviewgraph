//! `semantic` crate: embedding providers, the tenant-scoped Qdrant index, embedding units and
//! incremental embedding sync (ADR-008). See docs/architecture/target-architecture.md §3.9.
//!
//! The raw Qdrant client is private. [`SemanticIndex`] is the only way to search or write
//! points, and every method takes a [`TenantScope`]:
//!
//! ```compile_fail
//! // compile_fail_raw_client_access: the raw client is not reachable from outside the crate.
//! use semantic::qdrant::QdrantClient;
//! ```
//!
//! ```compile_fail
//! // compile_fail_search_without_scope: a search needs a TenantScope.
//! async fn f(index: &semantic::SemanticIndex, q: &semantic::SemanticQuery) {
//!     let _ = index.search(q).await;
//! }
//! ```
//!
//! ```no_run
//! // The scoped form compiles.
//! async fn f(
//!     index: &semantic::SemanticIndex,
//!     scope: &semantic::TenantScope,
//!     q: &semantic::SemanticQuery,
//! ) {
//!     let _ = index.search(scope, q).await;
//! }
//! ```

#[cfg(feature = "audit")]
pub mod audit;
pub mod bench;
pub mod collections;
pub mod embedding;
pub mod error;
pub mod filter;
pub mod index;
pub mod invalidation;
pub mod metrics;
pub mod point_id;
mod qdrant;
pub mod redact;
pub mod sync;
pub mod tenant;
pub mod units;

pub use collections::{
    bootstrap, bootstrap_until_ready, CollectionRecord, CollectionRegistry, CollectionStatus,
    MemoryRegistry,
};
pub use embedding::{EmbeddingProvider, EmbeddingSpace, ProviderName};
pub use error::{Error, Result, SemanticError};
pub use index::{
    CollectionTargets, DeleteSelector, QueryVector, SemanticHit, SemanticIndex, SemanticQuery,
    UpsertReport,
};
pub use qdrant::{CollectionState, HnswCfg, Payload, QdrantConfig, QdrantError};
pub use sync::{SyncOptions, SyncReport, SyncRequest};
pub use tenant::{ExtraFilter, NonEmptyVec, TenantScope};
pub use units::{EmbeddingUnit, UnitKind};
