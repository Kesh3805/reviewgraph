//! The architecture reviewer (REV-A-001): routing predicate, convention thresholds and the
//! `architecture` prompt binding.
//!
//! Profile layers and conventions (PROF-002..004) and explicit rules reach the model as
//! `repository_rules` items of the review context; this crate does not read the profile itself.

use model_gateway::TaskType;

use crate::error::ReviewerError;
use crate::output::reviewer_output_schema;
use crate::prompts::prompt;
use crate::reviewer::{Applicability, ContextBudget, ReviewerKind, RiskAssessment, RoutingInput};
use crate::routing::SkipReason;
use crate::specialist::{any_signal, no_extra_sections, SpecialistReviewer, SpecialistSpec};

/// Semver of the architecture reviewer code.
pub const ARCHITECTURE_REVIEWER_VERSION: &str = "1.0.0";

/// A convention-based finding needs at least this convention confidence (target-arch 3.10).
pub const MIN_CONVENTION_CONFIDENCE: f64 = 0.9;
/// A convention-based finding needs at least this many samples.
pub const MIN_CONVENTION_SAMPLES: u32 = 10;

/// Signals that make the architecture reviewer apply.
pub const ARCHITECTURE_SIGNALS: &[&str] = &[
    "boundary_import_changed",
    "dependency_edge_crosses_boundary",
    "module_added",
    "provider_added",
    "repository_class_added",
    "boundary_violation_hit",
    "new_cycle_hit",
    "migration_with_service_change",
];

/// Where a finding's basis comes from, strongest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum BasisKind {
    ExplicitRule,
    Documented,
    Convention,
}

/// True when a basis may ground a finding. Explicit rules and documented decisions always
/// qualify; a convention only above the confidence and sample floors.
pub fn basis_usable(kind: BasisKind, confidence: f64, samples: u32) -> bool {
    match kind {
        BasisKind::ExplicitRule | BasisKind::Documented => true,
        BasisKind::Convention => {
            confidence >= MIN_CONVENTION_CONFIDENCE && samples >= MIN_CONVENTION_SAMPLES
        }
    }
}

/// Applies when the change crosses a boundary, adds a structural unit, or a deterministic
/// detector has hits.
pub fn architecture_applies(input: &RoutingInput<'_>) -> Applicability {
    if any_signal(input, ARCHITECTURE_SIGNALS) {
        Applicability::Applies {
            focus_profiles: Vec::new(),
        }
    } else {
        Applicability::Skip {
            reason: SkipReason::NotApplicable,
        }
    }
}

pub fn architecture_budget(risk: &RiskAssessment) -> ContextBudget {
    ContextBudget::for_risk(risk.level)
}

/// Builds the architecture reviewer (prompt `architecture/v1`).
pub fn architecture_reviewer() -> Result<SpecialistReviewer, ReviewerError> {
    Ok(SpecialistReviewer::new(
        SpecialistSpec {
            kind: ReviewerKind::Architecture,
            task: TaskType::ArchitectureReview,
            version: ARCHITECTURE_REVIEWER_VERSION,
            applies: architecture_applies,
            extra: no_extra_sections,
            budget: architecture_budget,
        },
        prompt("architecture", 1)?,
        reviewer_output_schema()?,
    ))
}
