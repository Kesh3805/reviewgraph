//! Convention mining (PROF-003..PROF-005): one framework, uniform confidence semantics.
//!
//! See `docs/reviewers/conventions.md` for the formula and the built-in miners.

pub mod builtin;
pub mod confidence;
pub mod miner;
pub mod registry;
pub mod scope;

pub use confidence::{is_enforceable, ENFORCEABLE_CONFIDENCE, ENFORCEABLE_SAMPLES};
pub use miner::{mine, Conformance, ConventionMiner, MiningContext, Sample};
pub use registry::ConventionRegistry;
