//! `pipeline` crate. See docs/architecture/target-architecture.md §2.

pub mod budget;
pub mod cache;
pub mod error;
pub mod jobs;

pub use error::{Error, Result};
pub mod index;
