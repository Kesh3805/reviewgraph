//! The maintainability reviewer (REV-M-001): deterministic seed metrics, the publish threshold
//! and the `maintainability` prompt binding.
//!
//! The reviewer is off by default and never calls the model without a deterministic seed: a
//! metric that crossed its threshold between base and head, or a confirmed duplicate. Metrics are
//! computed lexically from the base and head bodies (stand-in for syntax facts). The Qdrant
//! `code_chunk` duplicate search lives in the semantic layer; its confirmed candidates enter as
//! [`DuplicateCandidate`]s.

use std::sync::LazyLock;

use model_gateway::{OutputSchema, TaskType};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::context::{ChangedSymbol, DiagnosticItem, ReviewContext};
use crate::error::ReviewerError;
use crate::prompts::{prompt, Prompt};
use crate::reviewer::{
    Applicability, ContextBudget, ReviewRequest, ReviewerKind, RiskAssessment, RoutingInput,
};
use crate::routing::{SkipReason, COMPLEXITY_SIGNALS};
use crate::specialist::{any_signal, no_extra_sections, SpecialistReviewer, SpecialistSpec};

/// Semver of the maintainability reviewer code.
pub const MAINTAINABILITY_REVIEWER_VERSION: &str = "1.1.0";

pub const MAINTAINABILITY_SCHEMA_NAME: &str = "maintainability";
pub const MAINTAINABILITY_SCHEMA_VERSION: &str = "v1";
pub const MAINTAINABILITY_SCHEMA_JSON: &str =
    include_str!("../prompts/maintainability/v1.schema.json");

/// The five maintainability categories of `maintainability.v1`.
pub const MAINTAINABILITY_CATEGORIES: &[&str] = &[
    "duplicated_logic",
    "unreviewable_function",
    "abstraction_leak",
    "inconsistent_pattern",
    "unreadable_control_flow",
];

/// `tool` of a seed diagnostic: the context carries every deterministic seed as a `D#` item of
/// this tool, with the metric (or `duplicate`) as its `code`.
pub const SEED_TOOL: &str = "deterministic:maintainability";
/// `review.reviewers.maintainability` default.
pub const DEFAULT_ENABLED: bool = false;
/// Default minimum confidence to publish a maintainability finding.
pub const MINIMUM_PUBLISH: f64 = 0.90;
/// Configuration cannot lower the publish threshold below this.
pub const MINIMUM_PUBLISH_FLOOR: f64 = 0.85;
/// Qdrant similarity needed for a duplicate candidate.
pub const DUPLICATE_MIN_SIMILARITY: f64 = 0.85;
/// Token Jaccard needed to confirm a duplicate on normalised bodies.
pub const DUPLICATE_MIN_JACCARD: f64 = 0.7;

/// The publish threshold for a configured value: never below the floor.
pub fn effective_minimum_publish(configured: Option<f64>) -> f64 {
    configured.map_or(MINIMUM_PUBLISH, |c| c.max(MINIMUM_PUBLISH_FLOOR))
}

/// Complexity, size and nesting of one body.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Metrics {
    pub cyclomatic: u32,
    pub lines: u32,
    pub max_nesting: u32,
}

/// Limits a metric must exceed, base to head: a metric over its limit in the base does not
/// qualify (REV-M-001: cyclomatic > 25, lines > 150, nesting > 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetricThresholds {
    pub cyclomatic: u32,
    pub lines: u32,
    pub max_nesting: u32,
}

impl Default for MetricThresholds {
    fn default() -> Self {
        Self {
            cyclomatic: 25,
            lines: 150,
            max_nesting: 5,
        }
    }
}

/// Metrics of a body: decision points plus one, non-blank lines and brace nesting depth.
pub fn metrics_of(body: &str) -> Metrics {
    let mut cyclomatic = 1;
    let mut depth: i32 = 0;
    let mut max_nesting: i32 = 0;
    let mut lines = 0;
    for line in body.lines() {
        if line.trim().is_empty() {
            continue;
        }
        lines += 1;
        for kw in [
            "if (", "for (", "while (", "case ", "catch (", "&&", "||", " ? ",
        ] {
            cyclomatic += line.matches(kw).count() as u32;
        }
        for c in line.chars() {
            match c {
                '{' => {
                    depth += 1;
                    max_nesting = max_nesting.max(depth);
                }
                '}' => depth -= 1,
                _ => {}
            }
        }
    }
    Metrics {
        cyclomatic,
        lines,
        // The function body's own braces are nesting level one.
        max_nesting: (max_nesting - 1).max(0) as u32,
    }
}

/// A metric that crossed its limit from base to head. A metric that was already over the limit
/// in the base does not qualify.
pub fn metric_crossed(
    base: Option<Metrics>,
    head: Metrics,
    t: &MetricThresholds,
) -> Option<&'static str> {
    let crossed = |b: Option<u32>, h: u32, limit: u32| h > limit && b.is_none_or(|b| b <= limit);
    if crossed(base.map(|m| m.cyclomatic), head.cyclomatic, t.cyclomatic) {
        Some("cyclomatic_complexity")
    } else if crossed(base.map(|m| m.lines), head.lines, t.lines) {
        Some("function_length")
    } else if crossed(base.map(|m| m.max_nesting), head.max_nesting, t.max_nesting) {
        Some("nesting_depth")
    } else {
        None
    }
}

/// A duplicate candidate from the semantic layer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DuplicateCandidate {
    pub symbol_id: String,
    pub similarity: f64,
    pub token_jaccard: f64,
}

impl DuplicateCandidate {
    /// Both signals are required: embedding similarity and token overlap.
    pub fn confirmed(&self) -> bool {
        self.similarity >= DUPLICATE_MIN_SIMILARITY && self.token_jaccard >= DUPLICATE_MIN_JACCARD
    }
}

fn symbol_metric(s: &ChangedSymbol, t: &MetricThresholds) -> Option<&'static str> {
    let head = metrics_of(s.body_head.as_deref()?);
    let base = s.body_base.as_deref().map(metrics_of);
    metric_crossed(base, head, t)
}

/// The deterministic seed: symbols whose metrics crossed a threshold, plus confirmed
/// duplicates. Without a seed the model is never called.
pub fn seed(
    cx: &dyn ReviewContext,
    thresholds: &MetricThresholds,
    duplicates: &[DuplicateCandidate],
) -> Vec<String> {
    let mut out: Vec<String> = cx
        .changed_symbols()
        .iter()
        .filter(|s| !s.generated)
        .filter_map(|s| symbol_metric(s, thresholds).map(|m| format!("{m}:{}", s.symbol_id)))
        .collect();
    out.extend(
        duplicates
            .iter()
            .filter(|d| d.confirmed())
            .map(|d| format!("duplicate:{}", d.symbol_id)),
    );
    out
}

/// Applies only when a complexity or duplicate seed signal is present (the registry additionally
/// requires `review.reviewers.maintainability: true`).
pub fn maintainability_applies(input: &RoutingInput<'_>) -> Applicability {
    if any_signal(input, COMPLEXITY_SIGNALS) || any_signal(input, &["duplicate_candidate"]) {
        Applicability::Applies {
            focus_profiles: Vec::new(),
        }
    } else {
        Applicability::Skip {
            reason: SkipReason::NotApplicable,
        }
    }
}

pub fn maintainability_budget(risk: &RiskAssessment) -> ContextBudget {
    ContextBudget::for_risk(risk.level)
}

/// The seeds of [`seed`] as `D#` diagnostics, ready to join the context's
/// `deterministic_findings`. `code` is the metric name or `duplicate`.
pub fn seed_diagnostics(
    cx: &dyn ReviewContext,
    thresholds: &MetricThresholds,
    duplicates: &[DuplicateCandidate],
) -> Vec<DiagnosticItem> {
    let mut out: Vec<DiagnosticItem> = cx
        .changed_symbols()
        .iter()
        .filter(|s| !s.generated)
        .filter_map(|s| {
            symbol_metric(s, thresholds).map(|metric| DiagnosticItem {
                tool: SEED_TOOL.to_owned(),
                path: s.path.clone(),
                line: s.range[0],
                code: metric.to_owned(),
                message: format!("{metric} crossed its limit in {}", s.symbol_id),
            })
        })
        .collect();
    out.extend(
        duplicates
            .iter()
            .filter(|d| d.confirmed())
            .map(|d| DiagnosticItem {
                tool: SEED_TOOL.to_owned(),
                path: String::new(),
                line: 0,
                code: "duplicate".to_owned(),
                message: format!("duplicate of {}", d.symbol_id),
            }),
    );
    out
}

/// The model is never called without a deterministic seed in the context.
fn maintainability_ready(req: &ReviewRequest<'_>) -> Result<(), ReviewerError> {
    if req
        .context
        .diagnostics()
        .iter()
        .any(|d| d.tool == SEED_TOOL)
    {
        Ok(())
    } else {
        Err(ReviewerError::NotReady(
            "no deterministic maintainability seed in the context".into(),
        ))
    }
}

/// The fields `maintainability.v1` adds to the common candidate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaintainabilityCandidateExtension {
    pub category: String,
    /// `D#` ref of the deterministic seed the finding rests on.
    pub seed_ref: String,
}

impl MaintainabilityCandidateExtension {
    /// Reads the extension from one raw `maintainability.v1` finding.
    pub fn from_raw(finding: &Value) -> Option<Self> {
        serde_json::from_value(json!({
            "category": finding.get("category")?,
            "seed_ref": finding.get("seed_ref")?,
        }))
        .ok()
    }
}

static MAINTAINABILITY_SCHEMA: LazyLock<Result<OutputSchema, String>> = LazyLock::new(|| {
    serde_json::from_str::<Value>(MAINTAINABILITY_SCHEMA_JSON)
        .map(|v| {
            OutputSchema::new(
                MAINTAINABILITY_SCHEMA_NAME,
                MAINTAINABILITY_SCHEMA_VERSION,
                v,
            )
        })
        .map_err(|e| e.to_string())
});

/// The `maintainability.v1` output schema.
pub fn maintainability_output_schema() -> Result<OutputSchema, ReviewerError> {
    MAINTAINABILITY_SCHEMA.clone().map_err(|e| {
        ReviewerError::Prompt(format!("maintainability.v1 schema is not valid JSON: {e}"))
    })
}

/// The maintainability prompt, v2 (binds `maintainability.v1`).
pub fn maintainability_prompt() -> Result<Prompt, ReviewerError> {
    prompt("maintainability", 2)
}

/// Builds the maintainability reviewer (prompt `maintainability/v2`, schema
/// `maintainability.v1`).
pub fn maintainability_reviewer() -> Result<SpecialistReviewer, ReviewerError> {
    Ok(SpecialistReviewer::new(
        SpecialistSpec {
            kind: ReviewerKind::Maintainability,
            task: TaskType::MaintainabilityReview,
            version: MAINTAINABILITY_REVIEWER_VERSION,
            applies: maintainability_applies,
            extra: no_extra_sections,
            ready: maintainability_ready,
            budget: maintainability_budget,
        },
        maintainability_prompt()?,
        maintainability_output_schema()?,
    ))
}
