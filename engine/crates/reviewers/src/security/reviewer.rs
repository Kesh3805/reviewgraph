//! The security reviewer (REV-S-002): routing predicate, budget and the `security.v1` binding.
//!
//! Trust-boundary context selection itself (`context-engine` candidate generators and the
//! `security` weight profile) is not part of this crate; the reviewer consumes whatever
//! [`crate::ReviewContext`] the context engine assembled.

use model_gateway::{InputSection, RiskBand, TaskType};
use serde_json::json;

use super::{security_output_schema, security_prompt, SecurityPattern};
use crate::error::ReviewerError;
use crate::reviewer::{
    Applicability, ContextBudget, ReviewRequest, ReviewerKind, RiskAssessment, RoutingInput,
};
use crate::routing::{SkipReason, NON_BEHAVIOURAL};
use crate::specialist::{any_signal, SpecialistReviewer, SpecialistSpec};

/// Semver of the security reviewer code (`security@1`).
pub const SECURITY_REVIEWER_VERSION: &str = "1.0.0";

/// Risk or change signals that make the security reviewer apply.
pub const SECURITY_SIGNALS: &[&str] = &[
    "auth_path",
    "auth_path_high_risk",
    "api_contract_changed",
    "guard_changed",
    "authorization_logic_changed",
    "authentication_changed",
    "permission_changed",
    "input_parsing_changed",
    "query_construction_changed",
    "crypto_or_secret",
    "file_path_io",
    "outbound_http",
    "dependency_manifest_changed",
];

/// Signals meaning a changed symbol is reachable from an `APIEndpoint` within depth 4, or a
/// configured risk path maps to `critical`.
pub const SECURITY_REACHABILITY_SIGNALS: &[&str] =
    &["endpoint_reachable_depth_4", "risk_path_critical"];

/// Security-specific routing: applies on security signals or endpoint reachability, never on
/// docs-only, test-only or generated-only clusters.
pub fn security_applies(input: &RoutingInput<'_>) -> Applicability {
    let cluster = input.cluster;
    let skip = |reason| Applicability::Skip { reason };
    if !cluster.symbols.is_empty() && cluster.symbols.iter().all(|s| s.generated) {
        return skip(SkipReason::GeneratedOnly);
    }
    let signals = &cluster.signals;
    if !signals.is_empty()
        && (signals
            .iter()
            .all(|s| NON_BEHAVIOURAL.contains(&s.as_str()))
            || signals.contains("tests_only"))
    {
        return skip(SkipReason::NotApplicable);
    }
    if any_signal(input, SECURITY_SIGNALS)
        || any_signal(input, SECURITY_REACHABILITY_SIGNALS)
        || cluster.public_endpoint
    {
        Applicability::Applies {
            focus_profiles: Vec::new(),
        }
    } else {
        skip(SkipReason::NotApplicable)
    }
}

/// `{ max_tokens: 12_000, .. }`, scaled by 1.5 for `critical`.
pub fn security_budget(risk: &RiskAssessment) -> ContextBudget {
    let base = ContextBudget::for_risk(RiskBand::Medium);
    if risk.level == RiskBand::Critical {
        ContextBudget {
            max_input_tokens: base.max_input_tokens * 3 / 2,
            ..base
        }
    } else {
        base
    }
}

fn security_sections(_req: &ReviewRequest<'_>) -> Vec<InputSection> {
    let patterns: Vec<SecurityPattern> = Vec::new();
    vec![InputSection::new(
        "security_patterns",
        json!({ "untrusted_data": true, "patterns": patterns }),
    )]
}

/// Builds the security reviewer (prompt v1, `security.v1`).
pub fn security_reviewer() -> Result<SpecialistReviewer, ReviewerError> {
    Ok(SpecialistReviewer::new(
        SpecialistSpec {
            kind: ReviewerKind::Security,
            task: TaskType::SecurityReview,
            version: SECURITY_REVIEWER_VERSION,
            applies: security_applies,
            extra: security_sections,
            budget: security_budget,
        },
        security_prompt()?,
        security_output_schema()?,
    ))
}
