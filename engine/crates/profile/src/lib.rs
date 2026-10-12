//! `profile` crate: the repository profile and explicit repository policy
//! (target-architecture §3.10, PRD §62–§66, §122–§124).
//!
//! * [`config`] — `.review/config.yaml` schema v1, safe parsing, validation, normalization and
//!   the snapshot-time sync that binds the policy to a commit (POL-001, POL-002).
//! * [`model`], [`store`] — the typed `RepositoryProfile` and its stores (PROF-001).
//! * [`infer`] — roles, layers, modules and the layer matrix inferred from the graph (PROF-002).
//! * [`conventions`] — the miner framework and the built-in miners (PROF-003..005).
//! * [`version`], [`cache`] — `profile_version`, reuse of the base profile, the LRU (PROF-006).
//! * [`knowledge`] — documentation rules from vaults and ADRs (PROF-007).
//! * [`policy`] — rule evaluators, precedence, violations as evidence, suppressions
//!   (POL-003..006).
//! * [`prechecks`] — deterministic boundary, cycle and test-gap checks (REV-A-002, REV-T-002).
//!
//! The crate is pure over its inputs except for the stores; PostgreSQL adapters live behind the
//! `pg` feature.

pub mod cache;
pub mod compute;
pub mod config;
pub mod conventions;
pub mod error;
pub mod facts;
pub mod infer;
pub mod knowledge;
mod metrics;
pub mod model;
#[cfg(feature = "pg")]
pub mod pg;
pub mod policy;
pub mod prechecks;
pub mod store;
pub mod version;

pub use compute::compute_profile;
pub use error::{Error, Result};
pub use model::RepositoryProfile;
