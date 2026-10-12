//! The performance reviewer (REV-P-001): routing predicate, the `performance` prompt binding,
//! the `performance.v1` schema and candidate extension.
//!
//! The "concrete execution path" rule is structural: every finding carries an `execution_path`
//! and a `cost_driver`, and [`crate::gates`] drop a finding whose path has a hop that is not a
//! `CALLS` or `HANDLED_BY` edge of confidence >= [`MIN_PATH_EDGE_CONFIDENCE`] in the context
//! (`unproven_path`). Verification re-checks the hops on the full graph.

use std::sync::LazyLock;

use model_gateway::{OutputSchema, TaskType};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::ReviewerError;
use crate::prompts::{prompt, Prompt};
use crate::reviewer::{Applicability, ContextBudget, ReviewerKind, RiskAssessment, RoutingInput};
use crate::routing::{SkipReason, PERFORMANCE_SIGNALS};
use crate::specialist::{
    always_ready, any_signal, no_extra_sections, SpecialistReviewer, SpecialistSpec,
};

/// Semver of the performance reviewer code.
pub const PERFORMANCE_REVIEWER_VERSION: &str = "1.1.0";

/// Minimum confidence of every edge on a claimed execution path.
pub const MIN_PATH_EDGE_CONFIDENCE: f64 = 0.6;

pub const PERFORMANCE_SCHEMA_NAME: &str = "performance";
pub const PERFORMANCE_SCHEMA_VERSION: &str = "v1";
pub const PERFORMANCE_SCHEMA_JSON: &str = include_str!("../prompts/performance/v1.schema.json");

/// The cost drivers of `performance.v1`.
pub const COST_DRIVERS: &[&str] = &[
    "loop_over_collection",
    "per_item_query",
    "sync_io",
    "unbounded_query",
    "cache_bypass",
    "repeated_computation",
];

/// Extra signals: a REV-P-002 hit, query or collection changes, and a hot-path risk signal.
pub const PERFORMANCE_EXTRA_SIGNALS: &[&str] = &[
    "io_in_loop_hit",
    "query_changed",
    "pagination_removed",
    "collection_scan_added",
    "loop_changed",
    "await_added",
    "hot_path",
];

/// What makes a path expensive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CostDriver {
    LoopOverCollection,
    PerItemQuery,
    SyncIo,
    UnboundedQuery,
    CacheBypass,
    RepeatedComputation,
}

impl CostDriver {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LoopOverCollection => "loop_over_collection",
            Self::PerItemQuery => "per_item_query",
            Self::SyncIo => "sync_io",
            Self::UnboundedQuery => "unbounded_query",
            Self::CacheBypass => "cache_bypass",
            Self::RepeatedComputation => "repeated_computation",
        }
    }
}

/// The fields `performance.v1` adds to the common candidate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerformanceCandidateExtension {
    /// Refs from the entry point (or loop) to the expensive operation.
    pub execution_path: Vec<String>,
    pub cost_driver: CostDriver,
}

impl PerformanceCandidateExtension {
    /// Reads the extension from one raw `performance.v1` finding.
    pub fn from_raw(finding: &Value) -> Option<Self> {
        serde_json::from_value(json!({
            "execution_path": finding.get("execution_path")?,
            "cost_driver": finding.get("cost_driver")?,
        }))
        .ok()
    }
}

static PERFORMANCE_SCHEMA: LazyLock<Result<OutputSchema, String>> = LazyLock::new(|| {
    serde_json::from_str::<Value>(PERFORMANCE_SCHEMA_JSON)
        .map(|v| OutputSchema::new(PERFORMANCE_SCHEMA_NAME, PERFORMANCE_SCHEMA_VERSION, v))
        .map_err(|e| e.to_string())
});

/// The `performance.v1` output schema.
pub fn performance_output_schema() -> Result<OutputSchema, ReviewerError> {
    PERFORMANCE_SCHEMA
        .clone()
        .map_err(|e| ReviewerError::Prompt(format!("performance.v1 schema is not valid JSON: {e}")))
}

/// The performance prompt, v2 (binds `performance.v1`).
pub fn performance_prompt() -> Result<Prompt, ReviewerError> {
    prompt("performance", 2)
}

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

/// Builds the performance reviewer (prompt `performance/v2`, schema `performance.v1`).
pub fn performance_reviewer() -> Result<SpecialistReviewer, ReviewerError> {
    Ok(SpecialistReviewer::new(
        SpecialistSpec {
            kind: ReviewerKind::Performance,
            task: TaskType::PerformanceReview,
            version: PERFORMANCE_REVIEWER_VERSION,
            applies: performance_applies,
            extra: no_extra_sections,
            ready: always_ready,
            budget: performance_budget,
        },
        performance_prompt()?,
        performance_output_schema()?,
    ))
}
