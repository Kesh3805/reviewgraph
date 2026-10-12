//! RISK-004 scoring tests.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;

use impact::risk::score::{assess, ScoreInput, ENDPOINT_REACHABLE};
use impact::risk::{
    RiskCategory, RiskLevel, RiskSignal, RuleSource, SignalEvidence, SignalSubject,
};
use review_core::ids::SymbolKey;

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
