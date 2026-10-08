//! The impact graph (IMP-001..IMP-008): the derived neighbourhood of every changed symbol.
//!
//! * [`model`] — elements, relations, paths, truncation and the serialized [`ImpactGraph`].
//! * [`path`] — the best-path merge rule for elements reached more than once.
//! * [`budget`] — caps for every expansion.
//! * [`builder`] — the per-seed orchestration; [`calls`] — callers, callees, removed callees.

pub mod budget;
pub mod builder;
pub mod calls;
pub mod model;
pub mod path;

pub use budget::ImpactBudget;
pub use builder::{build_impact, ImpactInputs};
pub use model::{
    compute_input_hash, sort_elements, EndpointAttrs, EntryKind, GraphSide, ImpactElement,
    ImpactFlags, ImpactGraph, ImpactStats, PathStep, Relation, ResourceAttrs, ResourceRole,
    SeedSkip, SeedTruncation, SymbolImpact, TestMapping, TestSignals, TestTargets, TruncReason,
    Truncation, IMPACT_SCHEMA_VERSION, IMPACT_VERSION,
};
pub use path::{compare_trails, Candidate, ElementSet, Extras, Offer, Trail};
