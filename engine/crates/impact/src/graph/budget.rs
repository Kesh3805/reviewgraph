//! Impact budgets (IMP-001 defaults, IMP-007 resolution).
//!
//! Every expansion takes an explicit budget and reports when it stopped short; nothing in the
//! impact graph is unbounded (master plan principle 4).
//!
//! # Resolution (IMP-007)
//!
//! `effective = clamp(base × risk_multiplier, min = base / 2, max = run budget)`, where `base`
//! is the configured value (`review.budgets.impact.*`, POL-001) or the default, the multiplier
//! is `low 0.5, medium 1.0, high 1.5, critical 2.0`, and the run budget (PIPE-006) or, absent
//! one, the hard maxima cap the result. Caller depth is `1` for low, `2` for medium/high and
//! `3` for critical (still subject to the IMP-002 remaining-budget rule). Invalid configuration
//! never yields an unbounded build: it falls back to the defaults with a `config_invalid`
//! warning.

use codegraph::Confidence;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::risk::RiskLevel;

/// Hard ceiling on `max_callers`, whatever the configuration says.
pub const HARD_MAX_CALLERS: u32 = 200;
/// Hard ceiling on `max_total_elements_pr`.
pub const HARD_MAX_TOTAL_ELEMENTS_PR: u32 = 20_000;
/// Hard ceiling on every other per-relation cap.
pub const HARD_MAX_RELATION: u32 = 1_000;

/// Caps for one impact build. Element caps are per seed unless named `_pr`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImpactBudget {
    /// Reverse `CALLS` depth. Depth 3 additionally requires half the PR budget to remain
    /// after every seed's depth-2 pass (IMP-002).
    pub max_caller_depth: u8,
    pub max_callers: u32,
    pub max_callees: u32,
    /// Removed callees, implementations, interfaces, overrides, sub/supertypes and related types.
    pub max_type_relations: u32,
    pub max_endpoints: u32,
    pub max_endpoint_depth: u8,
    /// Nodes the endpoint search may visit per seed (visited nodes are not elements).
    pub max_endpoint_visits: u32,
    pub max_tests: u32,
    /// Resource elements at distance 1 (tables, queues, config, env vars, external APIs).
    pub max_resources: u32,
    /// "Other side" resource elements per resource (co-writers, other readers, consumers).
    pub max_resource_other_side: u32,
    pub max_total_elements_per_symbol: u32,
    pub max_total_elements_pr: u32,
    /// Paths weaker than this are listed as `weak` and never expanded further.
    pub min_confidence: Confidence,
}

impl Default for ImpactBudget {
    fn default() -> Self {
        Self {
            max_caller_depth: 2,
            max_callers: 50,
            max_callees: 30,
            max_type_relations: 30,
            max_endpoints: 10,
            max_endpoint_depth: 6,
            max_endpoint_visits: 2_000,
            max_tests: 20,
            max_resources: 30,
            max_resource_other_side: 5,
            max_total_elements_per_symbol: 200,
            max_total_elements_pr: 5_000,
            min_confidence: Confidence::from_f32(0.5),
        }
    }
}

/// `review.budgets.impact.*` from `.review/config.yaml` (schema owned by POL-001).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImpactBudgetConfig {
    pub max_callers: Option<u32>,
    pub max_callees: Option<u32>,
    pub max_tests: Option<u32>,
    pub max_endpoints: Option<u32>,
    pub max_total_elements_pr: Option<u32>,
    pub min_confidence: Option<f32>,
}

/// Ceilings from the per-run budget manager (PIPE-006). `None` means "no run ceiling": the
/// hard maxima still apply.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunBudget {
    pub max_elements_per_relation: Option<u32>,
    pub max_total_elements_pr: Option<u32>,
}

/// A non-fatal problem found while resolving the budget.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum BudgetWarning {
    /// The configuration was invalid; the defaults were used instead.
    ConfigInvalid { reason: String },
}

/// The budget an impact build should use, with how it was derived.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResolvedBudget {
    pub budget: ImpactBudget,
    pub multiplier: f32,
    pub warnings: Vec<BudgetWarning>,
}

/// The element-cap multiplier of a risk level (RISK-005 `impact ×`).
pub const fn risk_multiplier(level: RiskLevel) -> f32 {
    match level {
        RiskLevel::Low => 0.5,
        RiskLevel::Medium => 1.0,
        RiskLevel::High => 1.5,
        RiskLevel::Critical => 2.0,
    }
}

/// The caller depth of a risk level.
pub const fn caller_depth(level: RiskLevel) -> u8 {
    match level {
        RiskLevel::Low => 1,
        RiskLevel::Medium | RiskLevel::High => 2,
        RiskLevel::Critical => 3,
    }
}

impl ImpactBudgetConfig {
    /// Re-checks the POL-001 constraints: positive caps within the hard maxima and a
    /// confidence floor in `[0, 1]`.
    pub fn validate(&self) -> Result<(), String> {
        let checks: [(&str, Option<u32>, u32); 5] = [
            ("max_callers", self.max_callers, HARD_MAX_CALLERS),
            ("max_callees", self.max_callees, HARD_MAX_RELATION),
            ("max_tests", self.max_tests, HARD_MAX_RELATION),
            ("max_endpoints", self.max_endpoints, HARD_MAX_RELATION),
            (
                "max_total_elements_pr",
                self.max_total_elements_pr,
                HARD_MAX_TOTAL_ELEMENTS_PR,
            ),
        ];
        for (name, value, hard_max) in checks {
            if let Some(value) = value {
                if value == 0 || value > hard_max {
                    return Err(format!("{name} = {value} is outside 1..={hard_max}"));
                }
            }
        }
        if let Some(floor) = self.min_confidence {
            if !floor.is_finite() || !(0.0..=1.0).contains(&floor) {
                return Err(format!("min_confidence = {floor} is outside 0..=1"));
            }
        }
        Ok(())
    }
}

/// `clamp(base × multiplier, min = base / 2, max = ceiling)`.
fn scale(base: u32, multiplier: f32, ceiling: u32) -> u32 {
    let scaled = (f64::from(base) * f64::from(multiplier)).round();
    let scaled = if scaled >= f64::from(u32::MAX) {
        u32::MAX
    } else {
        scaled as u32
    };
    scaled.max(base / 2).min(ceiling)
}

/// Resolves the effective budget: defaults → configuration → risk multiplier → run clamp.
/// Pure; computed once per run before expansion.
pub fn resolve_budget(
    config: Option<&ImpactBudgetConfig>,
    level: RiskLevel,
    run: Option<&RunBudget>,
) -> ResolvedBudget {
    let defaults = ImpactBudget::default();
    let mut warnings = Vec::new();
    let config = match config.map(|c| (c, c.validate())) {
        Some((config, Ok(()))) => config.clone(),
        Some((_, Err(reason))) => {
            warnings.push(BudgetWarning::ConfigInvalid { reason });
            ImpactBudgetConfig::default()
        }
        None => ImpactBudgetConfig::default(),
    };
    let multiplier = risk_multiplier(level);
    let relation_ceiling = run
        .and_then(|run| run.max_elements_per_relation)
        .unwrap_or(HARD_MAX_RELATION)
        .min(HARD_MAX_RELATION);
    let callers_ceiling = relation_ceiling.min(HARD_MAX_CALLERS);
    let pr_ceiling = run
        .and_then(|run| run.max_total_elements_pr)
        .unwrap_or(HARD_MAX_TOTAL_ELEMENTS_PR)
        .min(HARD_MAX_TOTAL_ELEMENTS_PR);

    let budget = ImpactBudget {
        max_caller_depth: caller_depth(level),
        max_callers: scale(
            config.max_callers.unwrap_or(defaults.max_callers),
            multiplier,
            callers_ceiling,
        ),
        max_callees: scale(
            config.max_callees.unwrap_or(defaults.max_callees),
            multiplier,
            relation_ceiling,
        ),
        max_type_relations: scale(defaults.max_type_relations, multiplier, relation_ceiling),
        max_endpoints: scale(
            config.max_endpoints.unwrap_or(defaults.max_endpoints),
            multiplier,
            relation_ceiling,
        ),
        max_endpoint_depth: defaults.max_endpoint_depth,
        max_endpoint_visits: defaults.max_endpoint_visits,
        max_tests: scale(
            config.max_tests.unwrap_or(defaults.max_tests),
            multiplier,
            relation_ceiling,
        ),
        max_resources: scale(defaults.max_resources, multiplier, relation_ceiling),
        max_resource_other_side: scale(
            defaults.max_resource_other_side,
            multiplier,
            relation_ceiling,
        ),
        max_total_elements_per_symbol: scale(
            defaults.max_total_elements_per_symbol,
            multiplier,
            pr_ceiling,
        ),
        max_total_elements_pr: scale(
            config
                .max_total_elements_pr
                .unwrap_or(defaults.max_total_elements_pr),
            multiplier,
            pr_ceiling,
        ),
        min_confidence: config
            .min_confidence
            .map_or(defaults.min_confidence, Confidence::from_f32),
    };
    let span = tracing::info_span!(
        "impact_budget",
        budget_multiplier = f64::from(multiplier),
        effective_total_cap = u64::from(budget.max_total_elements_pr),
        config_invalid = !warnings.is_empty(),
    );
    let _entered = span.enter();
    ResolvedBudget {
        budget,
        multiplier,
        warnings,
    }
}
