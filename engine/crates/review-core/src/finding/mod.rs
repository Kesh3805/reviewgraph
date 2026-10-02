//! Finding entities and lifecycle (DOM-006, PRD §49 and §150).
//!
//! The typed chain `CandidateFinding -> VerifiedFinding -> PublishedFinding` enforces
//! Invariant 2: LLM output never directly becomes an external finding. Every suppression is
//! persisted with its reason. [`state`] holds the authoritative lifecycle table.

pub mod candidate;
pub mod published;
pub mod severity;
pub mod state;
pub mod verified;

pub use candidate::{
    ArtifactKind, CandidateDraft, CandidateFinding, FindingArtifact, FindingFingerprint,
    Suppression, SuppressionReason,
};
pub use published::{Placement, PublishedFinding};
pub use severity::{Confidence, FindingCategory, Severity};
pub use state::FindingState;
pub use verified::{PublicationBand, StageOutcome, StageOutcomeRecord, VerifiedFinding};

pub use crate::reviewer_type::ReviewerType;
