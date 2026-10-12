//! Deterministic risk scoring (RISK-004).
//!
//! `contrib = clamp(weight * confidence * amplifier, 0, 0.95)`; within a category the strongest
//! contribution counts (weak signals do not stack), across categories contributions combine by
//! noisy-OR. Floors raise the level and the minimum score. A PR is as risky as its riskiest
//! cluster, not as the sum of its parts.

use std::collections::{BTreeMap, BTreeSet};

use review_core::change::ChangeClusterKey;
use review_core::ids::SymbolKey;
use review_core::location::RepoPath;
use serde::Serialize;

use super::level::RiskLevel;
use super::rules::RISK_RULES_VERSION;
use super::signal::{RiskCategory, RiskSignal, RuleSource, SignalSubject};

/// Rule id of the endpoint-reachability amplifier signal (RISK-003).
pub const ENDPOINT_REACHABLE: &str = "endpoint.reachable";
/// Upper bound of one signal's contribution.
const MAX_CONTRIBUTION: f32 = 0.95;
/// Amplifier for security-flavoured categories on an HTTP-reachable symbol.
const ENDPOINT_AMPLIFIER: f32 = 1.15;
/// Dampening factor for path signals of low-risk symbols (RISK-006).
const LOW_RISK_PATH_DAMPENING: f32 = 0.3;
/// A PR touching more files than this gets a small size signal.
const LARGE_PR_FILES: usize = 50;
const PR_SIZE_WEIGHT: f32 = 0.1;

/// The members of one change cluster, for aggregation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClusterMembers {
    pub symbols: BTreeSet<SymbolKey>,
    pub files: BTreeSet<RepoPath>,
}

/// Everything scoring reads.
#[derive(Debug, Clone, Default)]
pub struct ScoreInput {
    pub signals: Vec<RiskSignal>,
    pub clusters: BTreeMap<ChangeClusterKey, ClusterMembers>,
    /// Symbols classified low-risk (RISK-006): their path signals are dampened.
    pub low_risk_symbols: BTreeSet<SymbolKey>,
    /// Files classified low-risk (RISK-006).
    pub low_risk_files: BTreeSet<RepoPath>,
    pub files_changed: usize,
}

/// A score with its level.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct ScoredLevel {
    pub score: f32,
    pub level: RiskLevel,
}

/// The risk assessment of one PR.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RiskAssessment {
    pub score: f32,
    pub level: RiskLevel,
    /// Sorted by contribution, strongest first.
    pub signals: Vec<RiskSignal>,
    pub per_symbol: BTreeMap<SymbolKey, ScoredLevel>,
    pub per_cluster: BTreeMap<ChangeClusterKey, ScoredLevel>,
    pub rules_version: u32,
}

fn round4(value: f32) -> f32 {
    (value * 10_000.0).round() / 10_000.0
}

fn is_amplified_category(category: RiskCategory) -> bool {
    matches!(
        category,
        RiskCategory::Authorization | RiskCategory::Authentication | RiskCategory::Validation
    )
}

/// Returns the signals with `contribution` filled.
fn with_contributions(input: &ScoreInput) -> Vec<RiskSignal> {
    let reachable: BTreeSet<SymbolKey> = input
        .signals
        .iter()
        .filter(|s| s.rule_id == ENDPOINT_REACHABLE)
        .filter_map(|s| match &s.subject {
            SignalSubject::Symbol(key) => Some(*key),
            _ => None,
        })
        .collect();
    input
        .signals
        .iter()
        .map(|signal| {
            let amplified = is_amplified_category(signal.category)
                && matches!(&signal.subject, SignalSubject::Symbol(key) if reachable.contains(key));
            let amplifier = if amplified { ENDPOINT_AMPLIFIER } else { 1.0 };
            let low_risk = match &signal.subject {
                SignalSubject::Symbol(key) => input.low_risk_symbols.contains(key),
                SignalSubject::File(path) => input.low_risk_files.contains(path),
                _ => false,
            };
            let dampening = if low_risk && signal.source == RuleSource::Path {
                LOW_RISK_PATH_DAMPENING
            } else {
                1.0
            };
            let mut out = signal.clone();
            out.contribution = (signal.weight * signal.confidence * amplifier * dampening)
                .clamp(0.0, MAX_CONTRIBUTION);
            out
        })
        .collect()
}

/// Scores signals whose `contribution` is filled: per-category maximum, noisy-OR, floors.
pub fn score_signals<'a>(signals: impl IntoIterator<Item = &'a RiskSignal>) -> ScoredLevel {
    let mut by_category: BTreeMap<RiskCategory, f32> = BTreeMap::new();
    let mut floor: Option<RiskLevel> = None;
    for signal in signals {
        let slot = by_category.entry(signal.category).or_insert(0.0);
        *slot = slot.max(signal.contribution);
        floor = floor.max(signal.floor_level);
    }
    let remaining: f32 = by_category.values().map(|c| 1.0 - c).product();
    let mut score = (1.0 - remaining).clamp(0.0, 1.0);
    let mut level = RiskLevel::from_score(score);
    if let Some(floor) = floor {
        if floor > level {
            level = floor;
        }
        score = score.max(level.min_score());
    }
    ScoredLevel {
        score: round4(score),
        level,
    }
}

fn belongs_to_cluster(
    subject: &SignalSubject,
    key: &ChangeClusterKey,
    members: &ClusterMembers,
) -> bool {
    match subject {
        SignalSubject::Symbol(symbol) => members.symbols.contains(symbol),
        SignalSubject::File(path) => members.files.contains(path),
        SignalSubject::Cluster(cluster) => cluster == key,
        SignalSubject::Pr => false,
    }
}

/// Scores the PR, its clusters and its symbols.
pub fn assess(input: &ScoreInput) -> RiskAssessment {
    let mut signals = with_contributions(input);
    signals.sort_by(|a, b| {
        b.contribution
            .total_cmp(&a.contribution)
            .then_with(|| a.rule_id.cmp(&b.rule_id))
            .then_with(|| a.subject.cmp(&b.subject))
    });

    let symbols: BTreeSet<SymbolKey> = signals
        .iter()
        .filter_map(|s| match &s.subject {
            SignalSubject::Symbol(key) => Some(*key),
            _ => None,
        })
        .collect();
    let per_symbol: BTreeMap<SymbolKey, ScoredLevel> = symbols
        .into_iter()
        .map(|key| {
            let scored = score_signals(
                signals
                    .iter()
                    .filter(|s| matches!(&s.subject, SignalSubject::Symbol(k) if *k == key)),
            );
            (key, scored)
        })
        .collect();

    let per_cluster: BTreeMap<ChangeClusterKey, ScoredLevel> = input
        .clusters
        .iter()
        .map(|(key, members)| {
            let scored = score_signals(
                signals
                    .iter()
                    .filter(|s| belongs_to_cluster(&s.subject, key, members)),
            );
            (*key, scored)
        })
        .collect();

    // PR score: the riskiest cluster (or PR-level signals), never a sum over clusters.
    let mut best = if per_cluster.is_empty() {
        score_signals(signals.iter())
    } else {
        score_signals(signals.iter().filter(|s| s.subject == SignalSubject::Pr))
    };
    for scored in per_cluster.values() {
        best = ScoredLevel {
            score: best.score.max(scored.score),
            level: best.level.max(scored.level),
        };
    }
    if input.files_changed > LARGE_PR_FILES {
        let score = round4(1.0 - (1.0 - best.score) * (1.0 - PR_SIZE_WEIGHT));
        best = ScoredLevel {
            score,
            level: best.level.max(RiskLevel::from_score(score)),
        };
    }

    RiskAssessment {
        score: best.score,
        level: best.level,
        signals,
        per_symbol,
        per_cluster,
        rules_version: RISK_RULES_VERSION,
    }
}
