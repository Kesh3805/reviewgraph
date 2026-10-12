//! Context package model (CTX-001).

pub mod item;
pub mod model_input;
pub mod omitted;
pub mod package;

pub use item::{
    CandidateSource, ContextItem, ItemContent, ItemId, ItemKind, PathStep, SignalVector,
    SourceLocation,
};
pub use omitted::{OmitReason, OmittedItem};
pub use package::{
    ChangeSummary, ContextBudget, ContextPackage, ContextProvenance, RiskSignal,
    MAX_OMITTED_LISTED, PACKAGE_SCHEMA_VERSION,
};
