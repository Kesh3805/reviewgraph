//! The `Reviewer` trait (REV-001).
//!
//! A reviewer receives a bounded [`ReviewContext`] and a `&dyn ModelGateway`. It never sees raw
//! files, never opens a provider client and never sets confidence (ADR-011). Reviewers are
//! stateless and shared as `Arc<dyn Reviewer>`.

use async_trait::async_trait;
use model_gateway::{
    CallBudget, ModelGateway, ModelTier, PrivacyClass, RiskBand, TenantScope, TraceContext,
};
use review_core::reviewer_type::ReviewerType;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::context::ReviewContext;
use crate::error::ReviewerError;
use crate::focus::FocusProfile;
use crate::output::ReviewerOutput;
use crate::prompts::PromptRef;
use crate::routing::{ChangeCluster, SkipReason};

/// The six reviewers.
pub type ReviewerKind = ReviewerType;

/// Risk of the change, as far as reviewers need it.
///
/// **Stand-in for RISK-005**: the risk engine does not exist yet. `level` drives tier and budget;
/// `modules_touched` drives the deep-reasoner request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RiskAssessment {
    pub level: RiskBand,
    #[serde(default)]
    pub modules_touched: u32,
    #[serde(default)]
    pub signals: Vec<String>,
}

impl Default for RiskAssessment {
    fn default() -> Self {
        Self {
            level: RiskBand::Medium,
            modules_touched: 1,
            signals: Vec::new(),
        }
    }
}

/// Context and output budget of one reviewer call (CTX-006 defaults).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextBudget {
    pub max_input_tokens: u32,
    pub max_output_tokens: u32,
    pub max_candidates: u32,
}

impl ContextBudget {
    /// low 6k, medium 12k, high 24k, critical 40k input tokens; 4k output tokens.
    pub const fn for_risk(level: RiskBand) -> Self {
        let max_input_tokens = match level {
            RiskBand::Low => 6_000,
            RiskBand::Medium => 12_000,
            RiskBand::High => 24_000,
            RiskBand::Critical => 40_000,
        };
        Self {
            max_input_tokens,
            max_output_tokens: 4_000,
            max_candidates: 5,
        }
    }
}

/// Whether a reviewer applies to a cluster.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Applicability {
    Applies { focus_profiles: Vec<FocusProfile> },
    Skip { reason: SkipReason },
}

/// Routing facts a reviewer may inspect in [`Reviewer::applies`].
#[derive(Debug, Clone, Copy)]
pub struct RoutingInput<'a> {
    pub cluster: &'a ChangeCluster,
    pub risk: Option<&'a RiskAssessment>,
}

/// Everything one reviewer call needs.
#[derive(Clone, Copy)]
pub struct ReviewRequest<'a> {
    pub context: &'a dyn ReviewContext,
    pub risk: &'a RiskAssessment,
    pub cluster_id: &'a str,
    pub focus: &'a [FocusProfile],
    pub tenant: TenantScope,
    pub budget: CallBudget,
    pub trace: &'a TraceContext,
    /// From repository policy. Callers must fail closed (never default to `Standard` when the
    /// policy is unknown).
    pub privacy: PrivacyClass,
}

impl std::fmt::Debug for ReviewRequest<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReviewRequest")
            .field("cluster_id", &self.cluster_id)
            .field("focus", &self.focus)
            .field("package_hash", &self.context.package_hash())
            .finish_non_exhaustive()
    }
}

#[async_trait]
pub trait Reviewer: Send + Sync {
    fn kind(&self) -> ReviewerKind;
    /// Semver of the reviewer code.
    fn version(&self) -> &'static str;
    fn prompt(&self) -> &PromptRef;
    fn applies(&self, input: &RoutingInput<'_>) -> Applicability;
    fn budget(&self, risk: &RiskAssessment) -> ContextBudget;
    fn tier(&self, risk: &RiskAssessment) -> ModelTier;
    async fn review(
        &self,
        req: ReviewRequest<'_>,
        gw: &dyn ModelGateway,
        cancel: CancellationToken,
    ) -> Result<ReviewerOutput, ReviewerError>;
}

/// The reviewer stage key (PRD §76): `blake3(context_package_hash ‖ reviewer_kind ‖
/// reviewer_version ‖ prompt_sha ‖ focus_profiles ‖ route.table_hash)`. Each part is
/// length-prefixed so concatenations cannot collide.
pub fn input_hash(
    context_package_hash: &str,
    kind: ReviewerKind,
    reviewer_version: &str,
    prompt_sha: &str,
    focus: &[FocusProfile],
    route_table_hash: &str,
) -> String {
    let mut focus_names: Vec<&str> = focus.iter().map(|f| f.as_str()).collect();
    focus_names.sort_unstable();
    focus_names.dedup();
    let focus_joined = focus_names.join(",");
    let mut h = blake3::Hasher::new();
    for part in [
        context_package_hash,
        kind.as_str(),
        reviewer_version,
        prompt_sha,
        focus_joined.as_str(),
        route_table_hash,
    ] {
        h.update(&(part.len() as u64).to_le_bytes());
        h.update(part.as_bytes());
    }
    h.finalize().to_hex().to_string()
}
