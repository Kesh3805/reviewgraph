//! `repository` crate. See docs/architecture/target-architecture.md §2.
//!
//! Repository discovery and `review init` detectors. Everything here is deterministic and never
//! executes repository code.

pub mod build_systems;
pub mod dirs;
pub mod error;
pub mod frameworks;
pub mod git;
pub mod jsonc;
pub mod language;
pub mod manifests;
pub mod read;
pub mod remote_url;
pub mod sensitive;
pub mod walk;
pub mod workspaces;

pub use dirs::RepoDir;
pub use error::{Error, InitError, InitWarning, Result};
