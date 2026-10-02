//! `review-core` crate. See docs/architecture/target-architecture.md §2.

pub mod change;
pub mod contracts;
pub mod error;
pub mod evidence;
pub mod finding;
pub mod ids;
pub mod language;
pub mod location;
pub mod provenance;
pub mod pull_request;
pub mod repository;
pub mod review;
pub mod reviewer_type;
pub(crate) mod schema;
pub mod version;

pub use error::{Classify, CoreError, ErrorClass};
