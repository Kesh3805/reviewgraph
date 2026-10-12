//! REV-A-001: architecture reviewer routing, basis rules and prompt binding.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::collections::BTreeSet;

use reviewers::architecture::{
    architecture_applies, architecture_reviewer, basis_usable, BasisKind,
};
use reviewers::reviewer::RoutingInput;
use reviewers::routing::ChangeCluster;
use reviewers::{prompts, Applicability, Reviewer, ReviewerKind};

fn applies(signals: &[&str]) -> bool {
    let c = ChangeCluster {
        cluster_id: "c1".into(),
        risk_score: 0.5,
        signals: signals
            .iter()
            .map(|s| (*s).to_owned())
            .collect::<BTreeSet<_>>(),
        symbols: Vec::new(),
        public_endpoint: false,
    };
    matches!(
        architecture_applies(&RoutingInput {
            cluster: &c,
            risk: None
        }),
        Applicability::Applies { .. }
    )
}

#[test]
fn architecture_applies_on_boundary_change_and_detector_hits() {
    assert!(applies(&["boundary_import_changed"]));
    assert!(applies(&["boundary_violation_hit"]));
}

#[test]
fn architecture_skips_without_boundary_change() {
    assert!(!applies(&["condition_changed"]));
}

#[test]
fn architecture_low_confidence_convention_not_used() {
    assert!(!basis_usable(BasisKind::Convention, 0.85, 20));
    assert!(!basis_usable(BasisKind::Convention, 0.95, 9));
    assert!(basis_usable(BasisKind::Convention, 0.9, 10));
    assert!(basis_usable(BasisKind::ExplicitRule, 0.0, 0));
}

#[test]
fn architecture_explicit_rule_outranks_convention() {
    assert!(BasisKind::ExplicitRule < BasisKind::Documented);
    assert!(BasisKind::Documented < BasisKind::Convention);
}

#[test]
fn architecture_requires_basis_in_prompt() {
    prompts::self_test().expect("prompts match the lock");
    let r = architecture_reviewer().expect("reviewer");
    assert_eq!(r.kind(), ReviewerKind::Architecture);
    assert_eq!(r.prompt().lock_key(), "architecture/v1");
    let text = prompts::prompt("architecture", 1).unwrap().render(&[]);
    assert!(text.contains("A finding without a basis"));
}
