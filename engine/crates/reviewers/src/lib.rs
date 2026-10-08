//! `reviewers` crate: specialist reviewers over a bounded structured context (target-architecture
//! §4.2, ADR-009, ADR-011). Reviewers reach models only through `model_gateway::ModelGateway`.
//!
//! - [`reviewer`]: the `Reviewer` trait and its request types (REV-001).
//! - [`context`]: the reviewer input contract (a stand-in for CTX-008's `ContextPackage`).
//! - [`refs`], [`input`]: short refs and the PRD §89 `ModelReviewInput` (REV-001).
//! - [`output`]: the `reviewer_output.v1` schema and raw candidates (REV-001).
//! - [`prompts`]: the versioned prompt registry (REV-001).
//! - [`routing`], [`focus`]: reviewer routing and focus profiles (REV-002).
//! - [`correctness`]: correctness prompt v1 and reviewer (REV-C-001, REV-C-002 core).
//! - [`normalize`], [`claim_text`]: candidate normalisation (REV-C-003).
//! - [`security`]: security prompt v1 and schema (REV-S-001).

pub mod claim_text;
pub mod context;
pub mod correctness;
pub mod error;
pub mod focus;
pub mod input;
pub mod normalize;
pub mod output;
pub mod prompts;
pub mod refs;
pub mod reviewer;
pub mod routing;
pub mod security;

pub use context::{ContextSnapshot, ReviewContext};
pub use correctness::CorrectnessReviewer;
pub use error::{Error, Result, ReviewerError};
pub use focus::FocusProfile;
pub use input::{build_input, ModelReviewInput};
pub use normalize::{normalize, normalize_all, NormalizedCandidate, RefValidator, Rejection};
pub use output::{reviewer_output_schema, RawCandidate, ReviewerOutput};
pub use refs::{RefEntry, RefKind, RefTable};
pub use reviewer::{
    input_hash, Applicability, ContextBudget, ReviewRequest, Reviewer, ReviewerKind, RiskAssessment,
};
pub use routing::{plan_reviewers, ReviewerPlan};
