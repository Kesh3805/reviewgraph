//! `review-core` crate. See docs/architecture/target-architecture.md §2.

pub mod contracts;
pub mod error;
pub mod ids;
pub mod language;
pub mod location;
pub mod provenance;
pub mod repository;
pub(crate) mod schema;
pub mod version;

pub use error::{Classify, CoreError, ErrorClass};
