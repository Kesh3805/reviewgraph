//! `diff-engine` crate. See docs/architecture/target-architecture.md §2.

pub mod disposition;
pub mod error;
pub mod files;
pub mod git;
pub mod hunks;
mod metrics;
pub mod model;
pub mod testkit;

pub use error::{Error, Result};
pub use files::{diff_commits, DiffError, DiffOptions};
