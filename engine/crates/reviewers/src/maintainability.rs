//! The maintainability reviewer (REV-M-001): deterministic seed metrics, the publish threshold
//! and the `maintainability` prompt binding.
//!
//! The reviewer is off by default and never calls the model without a deterministic seed: a
//! metric that crossed its threshold between base and head, or a confirmed duplicate. Metrics are
//! computed lexically from the base and head bodies (stand-in for syntax facts). The Qdrant
//! `code_chunk` duplicate search lives in the semantic layer; its confirmed candidates enter as
//! [`DuplicateCandidate`]s.

use model_gateway::TaskType;
use serde::{Deserialize, Serialize};

use crate::context::{ChangedSymbol, ReviewContext};
use crate::error::ReviewerError;
use crate::output::reviewer_output_schema;
use crate::prompts::prompt;
use crate::reviewer::{Applicability, ContextBudget, ReviewerKind, RiskAssessment, RoutingInput};
use crate::routing::{SkipReason, COMPLEXITY_SIGNALS};
use crate::specialist::{any_signal, no_extra_sections, SpecialistReviewer, SpecialistSpec};

/// Semver of the maintainability reviewer code.
pub const MAINTAINABILITY_REVIEWER_VERSION: &str = "1.0.0";
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

/// Thresholds a metric must cross, base to head.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetricThresholds {
    pub cyclomatic: u32,
    pub lines: u32,
    pub max_nesting: u32,
}

impl Default for MetricThresholds {
    fn default() -> Self {
        Self {
            cyclomatic: 15,
            lines: 80,
            max_nesting: 4,
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

/// A metric that crossed its threshold from base to head. A metric that was already over the
/// threshold in the base does not qualify.
pub fn metric_crossed(
    base: Option<Metrics>,
    head: Metrics,
    t: &MetricThresholds,
) -> Option<&'static str> {
    let crossed = |b: Option<u32>, h: u32, limit: u32| h >= limit && b.is_none_or(|b| b < limit);
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

/// Builds the maintainability reviewer (prompt `maintainability/v1`).
pub fn maintainability_reviewer() -> Result<SpecialistReviewer, ReviewerError> {
    Ok(SpecialistReviewer::new(
        SpecialistSpec {
            kind: ReviewerKind::Maintainability,
            task: TaskType::MaintainabilityReview,
            version: MAINTAINABILITY_REVIEWER_VERSION,
            applies: maintainability_applies,
            extra: no_extra_sections,
            budget: maintainability_budget,
        },
        prompt("maintainability", 1)?,
        reviewer_output_schema()?,
    ))
}
