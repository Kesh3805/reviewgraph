//! REV-P-001: performance reviewer routing and prompt binding.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::collections::BTreeSet;

use reviewers::performance::{performance_applies, performance_reviewer};
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
        performance_applies(&RoutingInput {
            cluster: &c,
            risk: None
        }),
        Applicability::Applies { .. }
    )
}

#[test]
fn performance_applies_on_loop_query_or_detector_hit() {
    assert!(applies(&["query_in_loop"]));
    assert!(applies(&["io_in_loop_hit"]));
}

#[test]
fn performance_skips_without_loop_or_query_change() {
    assert!(!applies(&["condition_changed"]));
}

#[test]
fn performance_prompt_requires_concrete_path() {
    prompts::self_test().expect("prompts match the lock");
    let r = performance_reviewer().expect("reviewer");
    assert_eq!(r.kind(), ReviewerKind::Performance);
    assert_eq!(r.prompt().lock_key(), "performance/v2");
    let text = prompts::prompt("performance", 2).unwrap().render(&[]);
    assert!(text.contains("concrete execution path"));
}
