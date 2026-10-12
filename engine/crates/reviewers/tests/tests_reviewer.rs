//! REV-T-001: test reviewer routing and prompt binding.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::collections::BTreeSet;

use reviewers::reviewer::RoutingInput;
use reviewers::routing::ChangeCluster;
use reviewers::tests_reviewer::{tests_applies, tests_reviewer};
use reviewers::{prompts, Applicability, Reviewer, ReviewerKind};

fn cluster(signals: &[&str]) -> ChangeCluster {
    ChangeCluster {
        cluster_id: "c1".into(),
        risk_score: 0.5,
        signals: signals
            .iter()
            .map(|s| (*s).to_owned())
            .collect::<BTreeSet<_>>(),
        symbols: Vec::new(),
        public_endpoint: false,
    }
}

fn applies(signals: &[&str]) -> bool {
    let c = cluster(signals);
    matches!(
        tests_applies(&RoutingInput {
            cluster: &c,
            risk: None
        }),
        Applicability::Applies { .. }
    )
}

#[test]
fn tests_reviewer_applies_on_behavioural_change() {
    assert!(applies(&["condition_changed"]));
}

#[test]
fn tests_reviewer_skips_test_only_change() {
    assert!(!applies(&["condition_changed", "tests_only"]));
}

#[test]
fn tests_reviewer_skips_rename_and_formatting() {
    assert!(!applies(&["rename_pure"]));
    assert!(!applies(&["formatting"]));
}

#[test]
fn tests_reviewer_binds_locked_prompt() {
    prompts::self_test().expect("prompts match the lock");
    let r = tests_reviewer().expect("reviewer");
    assert_eq!(r.kind(), ReviewerKind::Test);
    assert_eq!(r.prompt().lock_key(), "tests/v2");
}
