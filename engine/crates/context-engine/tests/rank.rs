#![allow(clippy::unwrap_used, clippy::expect_used)]

use context_engine::model::{ItemKind, OmitReason};
use context_engine::ranking::*;
use review_core::reviewer_type::ReviewerType;

#[test]
fn weights_rows_sum_to_one_and_max_035() {
    for r in ReviewerType::ALL {
        Weights::for_reviewer(r).validate().unwrap();
    }
}

#[test]
fn override_with_040_rejected() {
    let mut w = Weights::for_reviewer(ReviewerType::Correctness);
    w.structural = 0.40;
    w.proximity = 0.05;
    assert!(matches!(w.validate(), Err(WeightError::OutOfRange { .. })));
}

fn strong(n: usize) -> Signals {
    let mut s = Signals::default();
    let fields = [
        &mut s.structural,
        &mut s.execution_path,
        &mut s.test_relationship,
    ];
    for f in fields.into_iter().take(n) {
        *f = 0.9;
    }
    s
}

#[test]
fn distance_gate_table() {
    let none = Signals::default();
    assert_eq!(
        gate(Some(0), 0.0, &none, ItemKind::ChangedSymbol),
        GateOutcome::Pass
    );
    assert_eq!(
        gate(Some(1), 0.2, &none, ItemKind::RelatedSymbol),
        GateOutcome::Pass
    );
    assert!(matches!(
        gate(Some(1), 0.19, &none, ItemKind::RelatedSymbol),
        GateOutcome::Reject(_)
    ));
    assert_eq!(
        gate(Some(2), 0.35, &none, ItemKind::RelatedSymbol),
        GateOutcome::Pass
    );
    assert!(matches!(
        gate(Some(2), 0.34, &none, ItemKind::RelatedSymbol),
        GateOutcome::Reject(_)
    ));
    assert_eq!(
        gate(Some(3), 0.5, &strong(2), ItemKind::RelatedSymbol),
        GateOutcome::Pass
    );
    assert!(matches!(
        gate(Some(3), 0.5, &strong(1), ItemKind::RelatedSymbol),
        GateOutcome::Reject(_)
    ));
    assert!(matches!(
        gate(Some(5), 0.9, &none, ItemKind::RelatedSymbol),
        GateOutcome::Reject(OmitReason::DistanceRule { distance: 5 })
    ));
}

#[test]
fn d4_endpoint_path_exception() {
    let s = Signals {
        execution_path: 1.0,
        ..Signals::default()
    };
    assert_eq!(
        gate(Some(4), 0.1, &s, ItemKind::RelatedSymbol),
        GateOutcome::Pass
    );
    assert_eq!(
        gate(Some(6), 0.1, &Signals::default(), ItemKind::Test),
        GateOutcome::Pass
    );
}

#[test]
fn lexical_only_treated_as_d3() {
    assert!(matches!(
        gate(None, 0.4, &strong(2), ItemKind::RelatedSymbol),
        GateOutcome::Reject(_)
    ));
    assert_eq!(
        gate(None, 0.6, &strong(2), ItemKind::RelatedSymbol),
        GateOutcome::Pass
    );
}

#[test]
fn weak_edge_penalty() {
    let w = Weights::for_reviewer(ReviewerType::Correctness);
    let s = Signals {
        structural: 1.0,
        proximity: 0.8,
        ..Signals::default()
    };
    let strong_edge = score(&s, &w, 1.0, false);
    let weak_edge = score(&s, &w, 1.0, true);
    assert!(weak_edge < strong_edge);
    assert!((weak_edge - (strong_edge * 0.7)).abs() < 1e-5);
}

#[test]
fn confidence_lowers_score() {
    let w = Weights::for_reviewer(ReviewerType::Security);
    let s = Signals {
        structural: 1.0,
        execution_path: 1.0,
        ..Signals::default()
    };
    assert!(score(&s, &w, 0.2, false) < score(&s, &w, 1.0, false));
}

#[test]
fn proximity_heuristic() {
    assert_eq!(proximity(Some(0)), 1.0);
    assert_eq!(proximity(Some(1)), 0.8);
    assert_eq!(proximity(Some(2)), 0.5);
    assert_eq!(proximity(Some(3)), 0.2);
    assert_eq!(proximity(Some(4)), 0.0);
}
