//! The risk engine (RISK-001..RISK-006): signals, rules, detectors, scoring, effects and
//! low-risk detection.

pub mod effects;
pub mod level;
pub mod low_risk;
pub mod rules;
pub mod score;
pub mod signal;

pub use effects::{effects, EffectsConfig, RiskEffects};
pub use level::RiskLevel;
pub use rules::{render_markdown, rule_by_id, RiskRule, RISK_RULES_VERSION, RULES};
pub use score::{assess, RiskAssessment, ScoreInput, ScoredLevel};
pub use signal::{
    dedup_signals, RiskCategory, RiskSignal, RuleSource, SignalEvidence, SignalSubject,
};
