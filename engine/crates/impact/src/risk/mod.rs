//! The risk engine (RISK-001..RISK-006): signals, rules, detectors, scoring, effects and
//! low-risk detection.

pub mod detect_change;
pub mod detect_graph;
pub mod detect_manifest;
pub mod detect_path;
pub mod detect_secrets;
pub mod effects;
pub mod level;
pub mod low_risk;
pub mod policy;
pub mod rules;
pub mod score;
pub mod signal;

use review_core::location::RepoPath;
use serde::Serialize;

use crate::graph::ImpactGraph;
use crate::input::ChangeSet;
use crate::metrics;

pub use detect_change::detect_change_signals;
pub use detect_graph::{detect_graph_signals, GraphDetection};
pub use detect_manifest::detect_manifest_signals;
pub use detect_path::detect_path_signals;
pub use detect_secrets::{detect_secret_signals, scan_line, SecretDetection, SecretFinding};
pub use effects::{effects, EffectsConfig, RiskEffects};
pub use level::RiskLevel;
pub use policy::{infer_category, level_weight, PathRule, RiskPolicy};
pub use rules::{render_markdown, rule_by_id, RiskRule, RISK_RULES_VERSION, RULES};
pub use score::{assess, RiskAssessment, ScoreInput, ScoredLevel};
pub use signal::{
    dedup_signals, RiskCategory, RiskSignal, RuleSource, SignalEvidence, SignalSubject,
};

/// A condition the risk detectors could not resolve; surfaced in the review summary.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct RiskWarning {
    /// e.g. `secrets_scan_incomplete`.
    pub code: &'static str,
    pub path: Option<RepoPath>,
}

/// A signal of the table rule `rule_id`, or `None` for an unknown rule id.
pub(crate) fn new_signal(
    rule_id: &str,
    subject: signal::SignalSubject,
    confidence: f32,
    evidence: Vec<signal::SignalEvidence>,
) -> Option<signal::RiskSignal> {
    let rule = rules::rule_by_id(rule_id)?;
    Some(signal::RiskSignal::from_rule(
        rule, subject, confidence, evidence,
    ))
}

/// Everything the detectors found.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RiskDetection {
    /// Deduplicated and sorted by `(rule_id, subject)`.
    pub signals: Vec<RiskSignal>,
    pub warnings: Vec<RiskWarning>,
    /// No impact graph was supplied: change-class rules ran, graph rules were skipped.
    pub graph_degraded: bool,
}

/// Runs every detector over the change set (and the impact graph, when there is one).
pub fn detect_all(
    changes: &ChangeSet,
    impact: Option<&ImpactGraph>,
    policy: &RiskPolicy,
) -> RiskDetection {
    let mut signals = detect_path_signals(changes, policy);
    signals.extend(detect_manifest_signals(changes));
    let secrets = detect_secret_signals(changes);
    signals.extend(secrets.signals);
    signals.extend(detect_change_signals(changes, policy));
    let graph = detect_graph_signals(changes, impact, policy);
    signals.extend(graph.signals);
    let signals = dedup_signals(signals);
    for signal in &signals {
        metrics::record_signal(signal.category.as_str(), signal.source.as_str());
    }
    let mut warnings = secrets.warnings;
    warnings.sort();
    RiskDetection {
        signals,
        warnings,
        graph_degraded: graph.degraded,
    }
}
