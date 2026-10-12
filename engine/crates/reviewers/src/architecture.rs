//! The architecture reviewer (REV-A-001): routing predicate, convention thresholds, the
//! `architecture` prompt binding, the `architecture.v1` schema and candidate extension.
//!
//! Profile layers and conventions (PROF-002..004) and explicit rules reach the model as
//! `repository_rules` items of the review context; this crate does not read the profile itself.
//! Every finding states its `basis`; [`crate::gates`] drop a finding whose basis is not in the
//! provided rules (`unknown_basis`) or whose convention is below the confidence and sample floors.

use std::sync::LazyLock;

use model_gateway::{OutputSchema, TaskType};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::ReviewerError;
use crate::prompts::{prompt, Prompt};
use crate::reviewer::{Applicability, ContextBudget, ReviewerKind, RiskAssessment, RoutingInput};
use crate::routing::SkipReason;
use crate::specialist::{
    always_ready, any_signal, no_extra_sections, SpecialistReviewer, SpecialistSpec,
};

/// Semver of the architecture reviewer code.
pub const ARCHITECTURE_REVIEWER_VERSION: &str = "1.1.0";

/// A convention-based finding needs at least this convention confidence (target-arch 3.10).
pub const MIN_CONVENTION_CONFIDENCE: f64 = 0.9;
/// A convention-based finding needs at least this many samples.
pub const MIN_CONVENTION_SAMPLES: u32 = 10;

pub const ARCHITECTURE_SCHEMA_NAME: &str = "architecture";
pub const ARCHITECTURE_SCHEMA_VERSION: &str = "v1";
pub const ARCHITECTURE_SCHEMA_JSON: &str = include_str!("../prompts/architecture/v1.schema.json");

/// The eight architecture categories of `architecture.v1`.
pub const ARCHITECTURE_CATEGORIES: &[&str] = &[
    "boundary_violation",
    "forbidden_dependency",
    "cross_layer_coupling",
    "ownership_violation",
    "abstraction_bypass",
    "cycle_risk",
    "duplicate_architecture",
    "state_ownership",
];

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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

/// The basis every `architecture.v1` finding carries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Basis {
    pub kind: BasisKind,
    pub id: String,
    /// `R#` ref of the rule in the context.
    pub rule_ref: String,
    pub confidence: f64,
    pub samples: u32,
}

/// The fields `architecture.v1` adds to the common candidate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchitectureCandidateExtension {
    pub category: String,
    pub basis: Basis,
}

impl ArchitectureCandidateExtension {
    /// Reads the extension from one raw `architecture.v1` finding.
    pub fn from_raw(finding: &Value) -> Option<Self> {
        serde_json::from_value(json!({
            "category": finding.get("category")?,
            "basis": finding.get("basis")?,
        }))
        .ok()
    }
}

static ARCHITECTURE_SCHEMA: LazyLock<Result<OutputSchema, String>> = LazyLock::new(|| {
    serde_json::from_str::<Value>(ARCHITECTURE_SCHEMA_JSON)
        .map(|v| OutputSchema::new(ARCHITECTURE_SCHEMA_NAME, ARCHITECTURE_SCHEMA_VERSION, v))
        .map_err(|e| e.to_string())
});

/// The `architecture.v1` output schema.
pub fn architecture_output_schema() -> Result<OutputSchema, ReviewerError> {
    ARCHITECTURE_SCHEMA.clone().map_err(|e| {
        ReviewerError::Prompt(format!("architecture.v1 schema is not valid JSON: {e}"))
    })
}

/// The architecture prompt, v2 (binds `architecture.v1`).
pub fn architecture_prompt() -> Result<Prompt, ReviewerError> {
    prompt("architecture", 2)
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

/// Builds the architecture reviewer (prompt `architecture/v2`, schema `architecture.v1`).
pub fn architecture_reviewer() -> Result<SpecialistReviewer, ReviewerError> {
    Ok(SpecialistReviewer::new(
        SpecialistSpec {
            kind: ReviewerKind::Architecture,
            task: TaskType::ArchitectureReview,
            version: ARCHITECTURE_REVIEWER_VERSION,
            applies: architecture_applies,
            extra: no_extra_sections,
            ready: always_ready,
            budget: architecture_budget,
        },
        architecture_prompt()?,
        architecture_output_schema()?,
    ))
}
