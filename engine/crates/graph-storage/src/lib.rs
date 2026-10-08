//! `graph-storage` crate. See docs/architecture/target-architecture.md §2.
//!
//! Persistence adapters for the code graph: the `GraphStore` port ([`port`]), its in-memory
//! reference implementation ([`mem`]), the PostgreSQL adapter ([`pg`]) and the file adapter
//! (`file`). The shared conformance suite lives in [`conformance`] (feature `conformance`).

pub mod cache;
pub mod error;
pub mod kinds;
pub mod mem;
pub mod model;
pub mod pg;
pub mod port;
pub mod status;
pub mod types;

#[cfg(feature = "conformance")]
pub mod conformance;

pub use error::{Error, Result, StoreError};
pub use port::GraphStore;
