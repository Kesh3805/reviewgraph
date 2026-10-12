//! The security reviewer (REV-S-002): routing predicate, budget and the `security.v1` binding.
//!
//! Trust-boundary context selection is [`super::trust_boundary`]: a bounded BFS from each
//! changed symbol to its entry points, guards, validation facts and sinks. The reviewer consumes
//! whatever [`crate::ReviewContext`] the context assembled from it.

use model_gateway::{InputSection, RiskBand, TaskType};
use serde_json::json;

use super::{security_output_schema, security_prompt, SecurityPattern};
use crate::error::ReviewerError;
use crate::reviewer::{
    Applicability, ContextBudget, ReviewRequest, ReviewerKind, RiskAssessment, RoutingInput,
};
use crate::routing::{SkipReason, NON_BEHAVIOURAL};
use crate::specialist::{always_ready, any_signal, SpecialistReviewer, SpecialistSpec};

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

/// Context budget of the security reviewer: 40 symbols, 12k tokens, 4 tests and 6 config items,
/// each scaled by 1.5 for `critical`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecurityContextBudget {
    pub max_symbols: u32,
    pub max_tokens: u32,
    pub max_tests: u32,
    pub max_configs: u32,
}

pub fn security_context_budget(risk: &RiskAssessment) -> SecurityContextBudget {
    let base = SecurityContextBudget {
        max_symbols: 40,
        max_tokens: 12_000,
        max_tests: 4,
        max_configs: 6,
    };
    if risk.level == RiskBand::Critical {
        SecurityContextBudget {
            max_symbols: base.max_symbols * 3 / 2,
            max_tokens: base.max_tokens * 3 / 2,
            max_tests: base.max_tests * 3 / 2,
            max_configs: base.max_configs * 3 / 2,
        }
    } else {
        base
    }
}

/// The model-call budget derived from [`security_context_budget`].
pub fn security_budget(risk: &RiskAssessment) -> ContextBudget {
    let base = ContextBudget::for_risk(RiskBand::Medium);
    ContextBudget {
        max_input_tokens: security_context_budget(risk).max_tokens,
        ..base
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
            ready: always_ready,
            budget: security_budget,
        },
        security_prompt()?,
        security_output_schema()?,
    ))
}
