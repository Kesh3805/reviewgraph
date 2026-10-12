//! `context-engine` crate. See docs/architecture/target-architecture.md §2.
//!
//! Context selection for reviewers: candidate generation (structural, test, config, API, rule,
//! lexical, semantic), ranking, compression, budgeting and the deterministic package hash.

pub mod assemble;
pub mod budget;
pub mod cache;
pub mod candidates;
pub mod compress;
pub mod error;
pub mod hash;
pub mod lexical;
pub mod model;
pub mod ranking;

pub use error::{Error, Result};
