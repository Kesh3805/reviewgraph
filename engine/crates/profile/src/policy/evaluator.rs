//! The `RuleEvaluator` trait and its outcome types (POL-003).
//!
//! Evaluators are pure and look **only at the pull request's change**: new edges and changed
//! symbols. A violation that already exists in the base is never reported (INV-009). Missing
//! inputs give `NotExecuted` with a reason; a rule never passes silently (INV-014).

use std::collections::BTreeMap;

use review_core::evidence::Evidence;
use serde::{Deserialize, Serialize};

use super::change::{ChangeSet, GraphPair};
use crate::config::{ReviewConfigV1, RuleSeverity};
use crate::model::RepositoryProfile;

/// Where a violation points.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRange {
    pub path: String,
    pub start_line: u32,
    pub end_line: u32,
}

/// One explicit-rule violation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleViolation {
    /// The configured rule id (`no-ctrl-repo`) or the rule key for singleton rules.
    pub rule_id: String,
    /// `forbidden_dependency`, `migrations_only`, `public_api_changes_require_tests`,
    /// `queue_deterministic_job_id`.
    pub rule_key: String,
    pub anchor: FileRange,
    pub symbol_key: Option<String>,
    pub evidence: Vec<Evidence>,
    pub severity: RuleSeverity,
    /// Template parameters (`from`, `to`, `import`, `reason`, `fix`, ...).
    pub params: BTreeMap<String, String>,
    /// The rendered comment text (POL-005 templates).
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RuleStatus {
    Executed,
    NotExecuted { reason: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleOutcome {
    pub rule_key: String,
    pub status: RuleStatus,
    pub violations: Vec<RuleViolation>,
}

impl RuleOutcome {
    pub fn executed(rule_key: &str, violations: Vec<RuleViolation>) -> Self {
        Self {
            rule_key: rule_key.to_owned(),
            status: RuleStatus::Executed,
            violations,
        }
    }

    pub fn not_executed(rule_key: &str, reason: impl Into<String>) -> Self {
        Self {
            rule_key: rule_key.to_owned(),
            status: RuleStatus::NotExecuted {
                reason: reason.into(),
            },
            violations: Vec::new(),
        }
    }

    pub fn is_executed(&self) -> bool {
        self.status == RuleStatus::Executed
    }
}

/// Everything an evaluator reads.
#[derive(Debug, Clone, Copy)]
pub struct RuleContext<'a> {
    pub change: &'a ChangeSet,
    pub graphs: GraphPair<'a>,
    pub config: &'a ReviewConfigV1,
    /// The base profile; `None` runs explicit rules only.
    pub profile: Option<&'a RepositoryProfile>,
    /// Init detected a test framework (test mapping is available).
    pub test_framework_detected: bool,
}

/// A deterministic evaluator of one rule family.
pub trait RuleEvaluator: Send + Sync {
    fn rule_key(&self) -> &'static str;
    /// Whether the config turns the rule on. Disabled rules are not evaluated at all.
    fn enabled(&self, config: &ReviewConfigV1) -> bool;
    fn evaluate(&self, cx: &RuleContext<'_>) -> RuleOutcome;
}
