//! `graph-storage` crate. See docs/architecture/target-architecture.md §2.
//!
//! Persistence adapters. Today: the PostgreSQL adapter of the repository facts store (INIT-013).

pub mod error;
pub mod pg;

pub use error::{Error, Result};
