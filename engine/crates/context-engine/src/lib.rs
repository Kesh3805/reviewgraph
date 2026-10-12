//! `context-engine` crate. See docs/architecture/target-architecture.md §2.

pub mod budget;
pub mod error;
pub mod model;
pub mod ranking;

pub use error::{Error, Result};
