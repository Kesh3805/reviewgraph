//! RISK-004 scoring tests.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::{BTreeMap, BTreeSet};

use impact::risk::effects::ReviewDepth;
use impact::risk::score::{assess, ClusterMembers, ScoreInput, ENDPOINT_REACHABLE};
use impact::risk::{
    RiskCategory, RiskLevel, RiskSignal, RuleSource, SignalEvidence, SignalSubject,
};
use proptest::prelude::*;
use review_core::change::ChangeClusterKey;
use review_core::ids::SymbolKey;
use review_core::location::RepoPath;
use review_core::reviewer_type::ReviewerType;

fn key() -> SymbolKey {
    "0123456789abcdef0123456789abcdef".parse().unwrap()
}

fn signal(rule: &str, category: RiskCategory, weight: f32, floor: Option<RiskLevel>) -> RiskSignal {
    RiskSignal {
        rule_id: rule.to_owned(),
        category,
        subject: SignalSubject::Pr,
        source: RuleSource::Change,
        weight,
        confidence: 1.0,
        floor_level: floor,
        evidence: vec![SignalEvidence::detail("test")],
        explanation: String::new(),
        contribution: 0.0,
    }
}

fn input(signals: Vec<RiskSignal>) -> ScoreInput {
    ScoreInput {
        signals,
        ..ScoreInput::default()
    }
}

#[test]
fn no_signals_low_zero() {
    let a = assess(&input(Vec::new()));
    assert_eq!(a.score, 0.0);
    assert_eq!(a.level, RiskLevel::Low);
}

#[test]
fn same_category_signals_do_not_stack() {
    let a = assess(&input(vec![
        signal("a", RiskCategory::Concurrency, 0.3, None),
        signal("b", RiskCategory::Concurrency, 0.3, None),
        signal("c", RiskCategory::Concurrency, 0.3, None),
    ]));
    assert_eq!(a.score, 0.3);
}

#[test]
fn cross_category_noisy_or() {
    let a = assess(&input(vec![
        signal("a", RiskCategory::Concurrency, 0.5, None),
        signal("b", RiskCategory::Queues, 0.5, None),
    ]));
    assert_eq!(a.score, 0.75);
    assert_eq!(a.level, RiskLevel::Critical);
}

#[test]
fn floor_raises_level_and_min_score() {
    let a = assess(&input(vec![signal(
        "a",
        RiskCategory::Secrets,
        0.1,
        Some(RiskLevel::Critical),
    )]));
    assert_eq!(a.level, RiskLevel::Critical);
    assert_eq!(a.score, 0.75);
}

#[test]
fn auth_bypass_scores_critical_0_993() {
    let k = key();
    let mut authz = signal(
        "authz.call_removed",
        RiskCategory::Authorization,
        0.85,
        Some(RiskLevel::High),
    );
    authz.subject = SignalSubject::Symbol(k);
    let mut reach = signal(
        ENDPOINT_REACHABLE,
        RiskCategory::PublicApiContracts,
        0.3,
        None,
    );
    reach.subject = SignalSubject::Symbol(k);
    let mut path = signal(
        "path.configured",
        RiskCategory::Authentication,
        0.8,
        Some(RiskLevel::Critical),
    );
    path.source = RuleSource::Path;
    let a = assess(&input(vec![authz, reach, path]));
    assert_eq!(a.score, 0.993);
    assert_eq!(a.level, RiskLevel::Critical);
    assert_eq!(a.signals[0].rule_id, "authz.call_removed");
    assert_eq!(a.per_symbol[&k].level, RiskLevel::Critical);
}

#[test]
fn large_pr_size_signal_small() {
    let mut i = input(Vec::new());
    i.files_changed = 80;
    let a = assess(&i);
    assert_eq!(a.score, 0.1);
    assert_eq!(a.level, RiskLevel::Low);
}

#[test]
fn dampening_never_applies_to_change_signals() {
    let k = key();
    let mut path = signal("path.auth", RiskCategory::Authentication, 0.5, None);
    path.subject = SignalSubject::Symbol(k);
    path.source = RuleSource::Path;
    let mut change = signal(
        "authz.logic_changed",
        RiskCategory::Authorization,
        0.7,
        None,
    );
    change.subject = SignalSubject::Symbol(k);
    let mut i = input(vec![path, change]);
    i.low_risk_symbols.insert(k);
    let a = assess(&i);
    let by_rule: BTreeMap<&str, f32> = a
        .signals
        .iter()
        .map(|s| (s.rule_id.as_str(), s.contribution))
        .collect();
    assert!((by_rule["path.auth"] - 0.15).abs() < 1e-6);
    assert!((by_rule["authz.logic_changed"] - 0.7).abs() < 1e-6);
}

fn symbol_signal(
    member: SymbolKey,
    category: RiskCategory,
    weight: f32,
    floor: Option<RiskLevel>,
) -> RiskSignal {
    let mut s = signal("test.rule", category, weight, floor);
    s.subject = SignalSubject::Symbol(member);
    s
}

fn cluster_input(
    clusters: &[(u8, SymbolKey)],
    signals: Vec<RiskSignal>,
) -> (ScoreInput, Vec<ChangeClusterKey>) {
    let mut i = input(signals);
    let mut ids = Vec::new();
    for (id, member) in clusters {
        let key = ChangeClusterKey::from_bytes([*id; 16]);
        i.clusters.insert(
            key,
            ClusterMembers {
                symbols: [*member].into_iter().collect(),
                files: BTreeSet::new(),
                untested: false,
                non_cosmetic: true,
            },
        );
        ids.push(key);
    }
    (i, ids)
}

#[test]
fn pr_score_is_max_of_clusters() {
    let (k1, k2) = (
        SymbolKey::from_bytes([1; 16]),
        SymbolKey::from_bytes([2; 16]),
    );
    let (i, ids) = cluster_input(
        &[(1, k1), (2, k2)],
        vec![
            symbol_signal(k1, RiskCategory::Concurrency, 0.5, None),
            symbol_signal(k2, RiskCategory::Queues, 0.4, None),
        ],
    );
    let a = assess(&i);
    assert_eq!(a.per_cluster[&ids[0]].score, 0.5);
    assert_eq!(a.per_cluster[&ids[1]].score, 0.4);
    // Not 1 - 0.5 * 0.6 = 0.7: a PR is as risky as its riskiest cluster.
    assert_eq!(a.score, 0.5);
    assert_eq!(a.level, RiskLevel::High);
    assert_eq!(a.cluster_effects.len(), 2);
    assert_eq!(a.cluster_effects[&ids[1]].depth, ReviewDepth::Standard);
}

#[test]
fn assessment_carries_effects_and_a_stable_input_hash() {
    let k = SymbolKey::from_bytes([3; 16]);
    let signals = vec![symbol_signal(
        k,
        RiskCategory::Authorization,
        0.85,
        Some(RiskLevel::High),
    )];
    let (mut i, _) = cluster_input(&[(7, k)], signals);
    i.change_input_hash = "change".to_owned();
    i.policy_hash = "policy".to_owned();
    let a = assess(&i);
    assert_eq!(a.effects.depth, ReviewDepth::Deep);
    assert!(a.effects.reviewers.contains(&ReviewerType::Security));
    assert_eq!(a.input_hash.len(), 64);
    assert_eq!(a.input_hash, assess(&i).input_hash);

    let mut other = i.clone();
    other.change_input_hash = "other change".to_owned();
    assert_ne!(a.input_hash, assess(&other).input_hash);
    let mut other = i.clone();
    other.policy_hash = "other policy".to_owned();
    assert_ne!(a.input_hash, assess(&other).input_hash);
    let mut other = i.clone();
    other.effects_config.deep_reasoner_enabled = true;
    assert_ne!(a.input_hash, assess(&other).input_hash);

    let json = serde_json::to_string(&a).unwrap();
    let back: impact::risk::RiskAssessment = serde_json::from_str(&json).unwrap();
    assert_eq!(back, a);
    let schema =
        serde_json::to_string(&schemars::schema_for!(impact::risk::RiskAssessment)).unwrap();
    assert!(schema.contains("input_hash"));
}

#[test]
fn file_signals_apply_to_the_symbols_of_the_file() {
    let k = SymbolKey::from_bytes([4; 16]);
    let file = RepoPath::new("src/auth/a.ts").unwrap();
    let mut s = signal(
        "path.configured",
        RiskCategory::Authentication,
        0.8,
        Some(RiskLevel::Critical),
    );
    s.subject = SignalSubject::File(file.clone());
    s.source = RuleSource::Path;
    let mut i = input(vec![s]);
    i.symbol_files.insert(k, file);
    let a = assess(&i);
    assert_eq!(a.per_symbol[&k].level, RiskLevel::Critical);
}

proptest! {
    #[test]
    fn proptest_score_monotone_in_signal_weight(
        low in 0u32..=100,
        bump in 0u32..=100,
        other in 0u32..=100,
        floor_high in any::<bool>(),
    ) {
        let high = (low + bump).min(100);
        let build = |weight: u32| {
            let mut signals = vec![
                signal("a", RiskCategory::Authorization, weight as f32 / 100.0, None),
                signal("b", RiskCategory::Queues, other as f32 / 100.0, None),
            ];
            if floor_high {
                signals[1].floor_level = Some(RiskLevel::High);
            }
            assess(&input(signals))
        };
        let (weaker, stronger) = (build(low), build(high));
        prop_assert!(stronger.score >= weaker.score, "{} < {}", stronger.score, weaker.score);
        prop_assert!(stronger.level >= weaker.level);
        prop_assert!((0.0..=1.0).contains(&stronger.score));
    }
}
