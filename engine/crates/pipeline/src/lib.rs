//! `pipeline` crate. See docs/architecture/target-architecture.md §2.

pub mod budget;
pub mod error;
pub mod index;
pub mod jobs;
pub mod store;

pub use error::{Error, Result};
