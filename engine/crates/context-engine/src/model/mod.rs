//! Context package model (CTX-001).

pub mod item;
pub mod model_input;
pub mod omitted;
pub mod package;

pub use item::{
    CandidateSource, CompressionLevel, ContextItem, ElidedLines, ItemContent, ItemId, ItemKind,
    PathStep, Segment, SignalVector, SourceLocation,
};
pub use omitted::{OmitReason, OmittedItem};
pub use package::{
    ChangeSummary, ContextBudget, ContextPackage, ContextProvenance, RiskSignal, SemanticStatus,
    MAX_OMITTED_LISTED, PACKAGE_SCHEMA_VERSION,
};
