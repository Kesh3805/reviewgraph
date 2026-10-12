//! The performance reviewer (REV-P-001): routing predicate and the `performance` prompt binding.
//!
//! The "concrete execution path" rule is enforced by the prompt and by the ref validator that
//! checks every cited ref; the per-hop confidence check belongs to verification.

use model_gateway::TaskType;

use crate::error::ReviewerError;
use crate::output::reviewer_output_schema;
use crate::prompts::prompt;
use crate::reviewer::{Applicability, ContextBudget, ReviewerKind, RiskAssessment, RoutingInput};
use crate::routing::{SkipReason, PERFORMANCE_SIGNALS};
use crate::specialist::{any_signal, no_extra_sections, SpecialistReviewer, SpecialistSpec};

/// Semver of the performance reviewer code.
pub const PERFORMANCE_REVIEWER_VERSION: &str = "1.0.0";

/// Extra signals: a REV-P-002 hit and query or collection changes.
pub const PERFORMANCE_EXTRA_SIGNALS: &[&str] = &[
    "io_in_loop_hit",
    "query_changed",
    "pagination_removed",
    "collection_scan_added",
];

/// Applies on a loop, query or I/O change in a hot path, or when REV-P-002 has hits.
pub fn performance_applies(input: &RoutingInput<'_>) -> Applicability {
    if any_signal(input, PERFORMANCE_SIGNALS) || any_signal(input, PERFORMANCE_EXTRA_SIGNALS) {
        Applicability::Applies {
            focus_profiles: Vec::new(),
        }
    } else {
        Applicability::Skip {
            reason: SkipReason::NotApplicable,
        }
    }
}

pub fn performance_budget(risk: &RiskAssessment) -> ContextBudget {
    ContextBudget::for_risk(risk.level)
}

/// Builds the performance reviewer (prompt `performance/v1`).
pub fn performance_reviewer() -> Result<SpecialistReviewer, ReviewerError> {
    Ok(SpecialistReviewer::new(
        SpecialistSpec {
            kind: ReviewerKind::Performance,
            task: TaskType::PerformanceReview,
            version: PERFORMANCE_REVIEWER_VERSION,
            applies: performance_applies,
            extra: no_extra_sections,
            budget: performance_budget,
        },
        prompt("performance", 1)?,
        reviewer_output_schema()?,
    ))
}
