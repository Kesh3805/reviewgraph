//! `verification` crate. See docs/architecture/target-architecture.md §2.

pub mod dedup;
pub mod error;
pub mod store;

pub use error::{Error, Result};
