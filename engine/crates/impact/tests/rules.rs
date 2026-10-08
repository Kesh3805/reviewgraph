//! RISK-001: the risk signal model and rule table.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::path::PathBuf;

use impact::risk::{
    dedup_signals, render_markdown, rule_by_id, RiskCategory, RiskLevel, RiskSignal,
    SignalEvidence, SignalSubject, RULES,
};
use review_core::ids::SymbolKey;

#[test]
fn all_18_categories_have_weight() {
    assert_eq!(RiskCategory::ALL.len(), 18);
    let distinct: BTreeSet<RiskCategory> = RiskCategory::ALL.into_iter().collect();
    assert_eq!(distinct.len(), 18);
    for category in RiskCategory::ALL {
        let weight = category.base_weight();
        assert!(weight > 0.0 && weight <= 1.0, "{category}: {weight}");
    }
    assert_eq!(RiskCategory::Authorization.base_weight(), 0.70);
    assert_eq!(RiskCategory::DependencyUpdates.base_weight(), 0.30);
}

#[test]
fn weights_in_unit_interval() {
    for rule in RULES {
        assert!(
            rule.weight > 0.0 && rule.weight <= 1.0,
            "{} has weight {}",
            rule.id,
            rule.weight
        );
        assert!(!rule.description.is_empty());
    }
    assert_eq!(rule_by_id("authz.call_removed").unwrap().weight, 0.85);
    assert_eq!(
        rule_by_id("authz.call_removed").unwrap().floor_level,
        Some(RiskLevel::High)
    );
    assert_eq!(
        rule_by_id("secrets.literal_added").unwrap().floor_level,
        Some(RiskLevel::Critical)
    );
    assert_eq!(
        rule_by_id("migration.destructive").unwrap().floor_level,
        Some(RiskLevel::High)
    );
    // A rule without an override takes its category's weight.
    assert_eq!(
        rule_by_id("authz.logic_changed").unwrap().weight,
        RiskCategory::Authorization.base_weight()
    );
}

#[test]
fn rule_ids_unique() {
    let ids: BTreeSet<&str> = RULES.iter().map(|rule| rule.id).collect();
    assert_eq!(ids.len(), RULES.len());
    for id in ids {
        assert!(id.contains('.'), "{id} is not namespaced");
    }
}

#[test]
fn dedup_merges_evidence_keeps_max_confidence() {
    let rule = rule_by_id("authz.call_removed").unwrap();
    let subject = SignalSubject::Symbol(SymbolKey::from_bytes([7; 16]));
    let a = RiskSignal::from_rule(
        rule,
        subject.clone(),
        0.6,
        vec![SignalEvidence::detail("removed PermissionService.check")],
    );
    let b = RiskSignal::from_rule(
        rule,
        subject.clone(),
        0.9,
        vec![
            SignalEvidence::detail("removed PermissionService.check"),
            SignalEvidence::detail("removed guard"),
        ],
    );
    let other = RiskSignal::from_rule(
        rule_by_id("authz.direct_role_check").unwrap(),
        subject,
        0.5,
        Vec::new(),
    );
    let merged = dedup_signals(vec![a, other, b]);
    assert_eq!(merged.len(), 2);
    let call_removed = merged
        .iter()
        .find(|s| s.rule_id == "authz.call_removed")
        .unwrap();
    assert_eq!(call_removed.confidence, 0.9);
    assert_eq!(call_removed.evidence.len(), 2);
    assert_eq!(call_removed.floor_level, Some(RiskLevel::High));

    let json = serde_json::to_string(call_removed).unwrap();
    let back: RiskSignal = serde_json::from_str(&json).unwrap();
    assert_eq!(&back, call_removed);
    assert!(json.contains("\"kind\":\"symbol\""));
}

#[test]
fn risk_rules_doc_up_to_date() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../docs/reviewers/risk-rules.md");
    let generated = render_markdown();
    let committed = std::fs::read_to_string(&path)
        .unwrap_or_default()
        .replace("\r\n", "\n");
    if committed != generated {
        println!("----- BEGIN generated risk-rules.md -----");
        print!("{generated}");
        println!("----- END generated risk-rules.md -----");
        panic!(
            "{} is out of date; regenerate it from impact::risk::rules::render_markdown()",
            path.display()
        );
    }
}
