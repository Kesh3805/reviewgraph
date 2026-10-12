//! The test reviewer (REV-T-001): routing predicate, budget and the `tests` prompt binding.
//!
//! Evidence from the REV-T-002 pre-check reaches the model through the context's
//! `deterministic_findings` section. Findings carry the common `reviewer_output.v1` schema with
//! `category = "tests"`; a dedicated `tests/v1.schema.json` is not part of this crate yet.

use model_gateway::TaskType;

use crate::error::ReviewerError;
use crate::output::reviewer_output_schema;
use crate::prompts::prompt;
use crate::reviewer::{Applicability, ContextBudget, ReviewerKind, RiskAssessment, RoutingInput};
use crate::routing::SkipReason;
use crate::specialist::{any_signal, no_extra_sections, SpecialistReviewer, SpecialistSpec};

/// Semver of the test reviewer code.
pub const TESTS_REVIEWER_VERSION: &str = "1.0.0";

/// Minimum confidence to publish a test finding (default threshold override).
pub const TESTS_MIN_PUBLISH_CONFIDENCE: f64 = 0.80;

/// Behavioural change classes (CHG-002..005) that make the test reviewer apply.
pub const TESTS_BEHAVIOUR_CLASSES: &[&str] = &[
    "condition_changed",
    "return_changed",
    "exception_changed",
    "throw_changed",
    "call_added",
    "call_removed",
    "signature_changed",
    "exported_signature_changed",
];

/// Applies when a behavioural change class is present; skips tests-only, rename-only and
/// formatting-only clusters.
pub fn tests_applies(input: &RoutingInput<'_>) -> Applicability {
    if any_signal(input, TESTS_BEHAVIOUR_CLASSES) && !input.cluster.signals.contains("tests_only") {
        Applicability::Applies {
            focus_profiles: Vec::new(),
        }
    } else {
        Applicability::Skip {
            reason: SkipReason::NotApplicable,
        }
    }
}

pub fn tests_budget(risk: &RiskAssessment) -> ContextBudget {
    ContextBudget::for_risk(risk.level)
}

/// Builds the test reviewer (prompt `tests/v1`).
pub fn tests_reviewer() -> Result<SpecialistReviewer, ReviewerError> {
    Ok(SpecialistReviewer::new(
        SpecialistSpec {
            kind: ReviewerKind::Test,
            task: TaskType::TestReview,
            version: TESTS_REVIEWER_VERSION,
            applies: tests_applies,
            extra: no_extra_sections,
            budget: tests_budget,
        },
        prompt("tests", 1)?,
        reviewer_output_schema()?,
    ))
}
