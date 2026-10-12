//! REV-M-001: maintainability seed metrics, threshold floor and prompt binding.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use reviewers::context::{ChangedSymbol, ContextSnapshot};
use reviewers::maintainability::{
    effective_minimum_publish, maintainability_reviewer, metric_crossed, metrics_of, seed,
    DuplicateCandidate, MetricThresholds, DEFAULT_ENABLED,
};
use reviewers::{prompts, Reviewer, ReviewerKind};

fn branchy(n: usize) -> String {
    let mut s = String::from("function f(x) {\n");
    for i in 0..n {
        s.push_str(&format!("  if (x === {i}) {{ return {i}; }}\n"));
    }
    s.push_str("}\n");
    s
}

fn symbol(base: &str, head: &str) -> ChangedSymbol {
    ChangedSymbol {
        symbol_id: "ts:src/a#f/function".into(),
        kind: "function".into(),
        path: "src/a.ts".into(),
        range: [1, 100],
        base_range: None,
        file_lines: None,
        change_classes: vec!["condition_changed".into()],
        signature_base: None,
        signature_head: None,
        body_base: Some(base.into()),
        body_head: Some(head.into()),
        hunks: Vec::new(),
        generated: false,
    }
}

#[test]
#[allow(clippy::assertions_on_constants)]
fn maintainability_disabled_by_default() {
    assert!(!DEFAULT_ENABLED);
}

#[test]
fn maintainability_threshold_floor_085() {
    assert!((effective_minimum_publish(None) - 0.90).abs() < f64::EPSILON);
    assert!((effective_minimum_publish(Some(0.5)) - 0.85).abs() < f64::EPSILON);
    assert!((effective_minimum_publish(Some(0.95)) - 0.95).abs() < f64::EPSILON);
}

#[test]
fn complexity_crossed_threshold_qualifies() {
    let cx = ContextSnapshot {
        changed_symbols: vec![symbol(&branchy(3), &branchy(30))],
        ..ContextSnapshot::default()
    };
    assert_eq!(seed(&cx, &MetricThresholds::default(), &[]).len(), 1);
}

#[test]
fn complexity_preexisting_not_qualify() {
    let cx = ContextSnapshot {
        changed_symbols: vec![symbol(&branchy(28), &branchy(30))],
        ..ContextSnapshot::default()
    };
    assert!(seed(&cx, &MetricThresholds::default(), &[]).is_empty());
    let head = metrics_of(&branchy(30));
    let base = metrics_of(&branchy(28));
    assert!(metric_crossed(Some(base), head, &MetricThresholds::default()).is_none());
}

#[test]
fn duplicate_requires_two_signals() {
    let cx = ContextSnapshot::default();
    let weak = DuplicateCandidate {
        symbol_id: "ts:src/b#g/function".into(),
        similarity: 0.95,
        token_jaccard: 0.5,
    };
    let strong = DuplicateCandidate {
        token_jaccard: 0.8,
        ..weak.clone()
    };
    assert!(seed(&cx, &MetricThresholds::default(), &[weak]).is_empty());
    assert_eq!(seed(&cx, &MetricThresholds::default(), &[strong]).len(), 1);
}

#[test]
fn no_model_call_without_seed() {
    let cx = ContextSnapshot {
        changed_symbols: vec![symbol(&branchy(3), &branchy(4))],
        ..ContextSnapshot::default()
    };
    assert!(seed(&cx, &MetricThresholds::default(), &[]).is_empty());
}

#[test]
fn maintainability_prompt_binds_lock() {
    prompts::self_test().expect("prompts match the lock");
    let r = maintainability_reviewer().expect("reviewer");
    assert_eq!(r.kind(), ReviewerKind::Maintainability);
    assert_eq!(r.prompt().lock_key(), "maintainability/v2");
}
