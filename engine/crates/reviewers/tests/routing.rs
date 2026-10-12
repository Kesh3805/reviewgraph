#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
//! REV-002: reviewer routing.

use std::collections::{BTreeMap, BTreeSet};

use model_gateway::{ModelTier, RiskBand};
use reviewers::routing::{
    plan_reviewers, ChangeCluster, ClusterSymbol, ConfigSource, ReviewerRegistry, ReviewersConfig,
    SkipReason,
};
use reviewers::{FocusProfile, ReviewerKind, RiskAssessment};

fn cluster(id: &str, score: f64, signals: &[&str], paths: &[(&str, bool)]) -> ChangeCluster {
    ChangeCluster {
        cluster_id: id.into(),
        risk_score: score,
        signals: signals
            .iter()
            .map(|s| (*s).to_owned())
            .collect::<BTreeSet<_>>(),
        symbols: paths
            .iter()
            .enumerate()
            .map(|(i, (p, generated))| ClusterSymbol {
                symbol_id: format!("ts:{}#S{i}/function", p.trim_end_matches(".ts")),
                path: (*p).to_owned(),
                generated: *generated,
            })
            .collect(),
        public_endpoint: false,
    }
}

fn risk(level: RiskBand) -> RiskAssessment {
    RiskAssessment {
        level,
        modules_touched: 1,
        signals: Vec::new(),
    }
}

fn all() -> ReviewerRegistry {
    ReviewerRegistry::new(ReviewerKind::ALL)
}

fn kinds(plan: &reviewers::ReviewerPlan) -> Vec<ReviewerKind> {
    plan.entries.iter().map(|e| e.reviewer).collect()
}

#[test]
fn readme_only_runs_no_reviewers() {
    let c = cluster("c1", 0.1, &["docs_only"], &[("README.md", false)]);
    let plan = plan_reviewers(
        &[c],
        Some(&risk(RiskBand::Low)),
        &ReviewersConfig::default(),
        &all(),
    );
    assert!(plan.entries.is_empty());
    assert_eq!(plan.skipped[0].reason, SkipReason::LowRiskOnly);
}

#[test]
fn migration_routes_correctness_with_database_safety_architecture_tests() {
    let c = cluster(
        "c1",
        0.7,
        &["migration_added", "database_write_changed"],
        &[("src/migrations/1700-add-col.ts", false)],
    );
    let plan = plan_reviewers(
        &[c],
        Some(&risk(RiskBand::High)),
        &ReviewersConfig::default(),
        &all(),
    );
    assert_eq!(
        kinds(&plan),
        [
            ReviewerKind::Correctness,
            ReviewerKind::Test,
            ReviewerKind::Architecture
        ]
    );
    assert_eq!(plan.entries[0].focus, [FocusProfile::DatabaseSafety]);
}

#[test]
fn auth_change_routes_correctness_security_tests_architecture() {
    let c = cluster(
        "c1",
        0.9,
        &["authorization_logic_changed", "call_removed"],
        &[("src/auth/auth.service.ts", false)],
    );
    let plan = plan_reviewers(
        &[c],
        Some(&risk(RiskBand::High)),
        &ReviewersConfig::default(),
        &all(),
    );
    assert_eq!(
        kinds(&plan),
        [
            ReviewerKind::Security,
            ReviewerKind::Correctness,
            ReviewerKind::Test,
            ReviewerKind::Architecture
        ]
    );
}

#[test]
fn generated_only_cluster_skipped() {
    let c = cluster(
        "c1",
        0.5,
        &["call_removed"],
        &[("src/gen/api.generated.ts", true)],
    );
    let plan = plan_reviewers(
        &[c],
        Some(&risk(RiskBand::Medium)),
        &ReviewersConfig::default(),
        &all(),
    );
    assert!(plan.entries.is_empty());
    assert_eq!(plan.skipped[0].reason, SkipReason::GeneratedOnly);
}

#[test]
fn generated_symbols_removed_from_mixed_cluster() {
    let c = cluster(
        "c1",
        0.5,
        &["call_removed"],
        &[("src/gen/client.ts", false), ("src/a.service.ts", false)],
    );
    let cfg = ReviewersConfig {
        generated_ignore: vec!["src/gen/**".into()],
        ..ReviewersConfig::default()
    };
    let plan = plan_reviewers(&[c], Some(&risk(RiskBand::Medium)), &cfg, &all());
    assert!(!plan.entries.is_empty());
    for e in &plan.entries {
        assert_eq!(e.symbols.len(), 1);
        assert!(e.symbols[0].contains("src/a.service"));
    }
}

#[test]
fn config_disable_respected() {
    let c = cluster("c1", 0.5, &["call_removed"], &[("src/a.ts", false)]);
    let cfg = ReviewersConfig {
        toggles: BTreeMap::from([(ReviewerKind::Test, false)]),
        ..ReviewersConfig::default()
    };
    let plan = plan_reviewers(&[c], Some(&risk(RiskBand::Medium)), &cfg, &all());
    assert_eq!(kinds(&plan), [ReviewerKind::Correctness]);
    assert!(plan.skipped.iter().any(
        |s| s.reviewer == Some(ReviewerKind::Test) && s.reason == SkipReason::DisabledByConfig
    ));
}

#[test]
fn head_config_cannot_disable_reviewers() {
    let c = cluster("c1", 0.5, &["call_removed"], &[("src/a.ts", false)]);
    let cfg = ReviewersConfig {
        toggles: BTreeMap::from([(ReviewerKind::Correctness, false)]),
        source: ConfigSource::Head,
        ..ReviewersConfig::default()
    };
    let plan = plan_reviewers(&[c], Some(&risk(RiskBand::Medium)), &cfg, &all());
    assert!(kinds(&plan).contains(&ReviewerKind::Correctness));
}

#[test]
fn unimplemented_reviewers_reported_as_skipped() {
    let c = cluster(
        "c1",
        0.9,
        &["authorization_logic_changed"],
        &[("src/auth/auth.service.ts", false)],
    );
    let plan = plan_reviewers(
        &[c],
        Some(&risk(RiskBand::High)),
        &ReviewersConfig::default(),
        &ReviewerRegistry::current(),
    );
    assert_eq!(kinds(&plan), [ReviewerKind::Correctness]);
    let skipped: Vec<ReviewerKind> = plan
        .skipped
        .iter()
        .filter(|s| s.reason == SkipReason::NotImplemented)
        .filter_map(|s| s.reviewer)
        .collect();
    assert!(skipped.contains(&ReviewerKind::Security));
    assert!(skipped.contains(&ReviewerKind::Test));
    assert!(skipped.contains(&ReviewerKind::Architecture));
}

#[test]
fn plan_order_by_risk_then_precedence() {
    let low = cluster("a-low", 0.2, &["call_removed"], &[("src/a.ts", false)]);
    let high = cluster(
        "b-high",
        0.9,
        &["authorization_logic_changed"],
        &[("src/auth.ts", false)],
    );
    let plan = plan_reviewers(
        &[low, high],
        Some(&risk(RiskBand::High)),
        &ReviewersConfig::default(),
        &all(),
    );
    let order: Vec<(String, ReviewerKind)> = plan
        .entries
        .iter()
        .map(|e| (e.cluster_id.clone(), e.reviewer))
        .collect();
    assert_eq!(order[0], ("b-high".into(), ReviewerKind::Security));
    assert_eq!(order[1], ("b-high".into(), ReviewerKind::Correctness));
    assert_eq!(order.last().unwrap().0, "a-low");
}

#[test]
fn missing_risk_defaults_conservatively() {
    let c = cluster(
        "c1",
        0.5,
        &["authorization_logic_changed"],
        &[("src/auth.ts", false)],
    );
    let plan = plan_reviewers(&[c], None, &ReviewersConfig::default(), &all());
    assert_eq!(
        kinds(&plan),
        [ReviewerKind::Correctness, ReviewerKind::Test]
    );
    assert!(plan.entries[0]
        .reasons
        .contains(&"risk_unavailable".to_owned()));
    assert_eq!(plan.entries[0].tier, ModelTier::ReviewReasoner);
}

#[test]
fn deep_reasoner_requested_only_for_critical_multi_module() {
    let c = cluster("c1", 0.9, &["call_removed"], &[("src/a.ts", false)]);
    let mut r = risk(RiskBand::Critical);
    r.modules_touched = 2;
    let plan = plan_reviewers(
        std::slice::from_ref(&c),
        Some(&r),
        &ReviewersConfig::default(),
        &all(),
    );
    assert_eq!(plan.entries[0].tier, ModelTier::DeepReasoner);
    r.modules_touched = 1;
    let plan = plan_reviewers(&[c], Some(&r), &ReviewersConfig::default(), &all());
    assert_eq!(plan.entries[0].tier, ModelTier::ReviewReasoner);
}
