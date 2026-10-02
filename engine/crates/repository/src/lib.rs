//! `repository` crate. See docs/architecture/target-architecture.md §2.
//!
//! Repository discovery and `review init` detectors. Everything here is deterministic and never
//! executes repository code.

pub mod error;
pub mod git;
pub mod read;
pub mod remote_url;
pub mod sensitive;
pub mod walk;

pub use error::{Error, InitError, InitWarning, Result};
