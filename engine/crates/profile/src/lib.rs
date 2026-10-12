//! `profile` crate: the repository profile and explicit repository policy
//! (target-architecture §3.10, PRD §62–§66, §122–§124).
//!
//! * [`config`] — `.review/config.yaml` schema v1, safe parsing, validation, normalization and
//!   the snapshot-time sync that binds the policy to a commit (POL-001, POL-002).

pub mod config;
pub mod error;
mod metrics;
#[cfg(feature = "pg")]
pub mod pg;

pub use error::{Error, Result};
