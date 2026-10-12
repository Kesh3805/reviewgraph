#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::*;
use context_engine::candidates::structural::StructuralCandidates;
use context_engine::candidates::{
    Candidate, CandidateGenerator, CandidatePool, PoolKey, RawSignals,
};
use context_engine::model::{CandidateSource, ItemKind};
use impact::Relation;
use review_core::location::DiffSide;
use review_core::reviewer_type::ReviewerType;

fn run(reviewer: ReviewerType, elements: Vec<impact::graph::ImpactElement>) -> CandidatePool {
    let graph = graph_of(elements);
    let input = input(reviewer, Some(&graph));
    let mut pool = CandidatePool::new();
    StructuralCandidates.generate(&input, &mut pool);
    pool
}

fn has(pool: &CandidatePool, id: &str, side: DiffSide) -> bool {
    pool.get_node(key(id), side).is_some()
}

#[test]
fn auth_bypass_structural_has_updateuser_controller_authprovider_permission_check_base() {
    let pool = run(ReviewerType::Correctness, auth_bypass_elements());
    assert!(has(&pool, UPDATE_USER, DiffSide::Head));
    assert!(has(&pool, CONTROLLER, DiffSide::Head));
    assert!(has(&pool, PROVIDER, DiffSide::Head));
    assert!(has(&pool, PERMISSION_CHECK, DiffSide::Base));
    assert!(!has(&pool, PERMISSION_CHECK, DiffSide::Head));
}

#[test]
fn changed_symbol_both_sides_pinned() {
    let pool = run(ReviewerType::Correctness, vec![]);
    let head = pool.get_node(key(AUTHORIZE), DiffSide::Head).unwrap();
    let base = pool.get_node(key(AUTHORIZE), DiffSide::Base).unwrap();
    for c in [head, base] {
        assert!(c.pinned);
        assert_eq!(c.kind, ItemKind::ChangedSymbol);
        assert_eq!(c.distance, Some(0));
    }
    assert_eq!(base.span.as_ref().unwrap().end_line, 34);
}

#[test]
fn class_signature_pinned() {
    let pool = run(ReviewerType::Correctness, vec![]);
    let class = pool.get_node(key(AUTH_CLASS), DiffSide::Head).unwrap();
    assert_eq!(class.kind, ItemKind::ClassSignature);
    assert!(class.pinned);
}

#[test]
fn merge_keeps_min_distance_max_confidence() {
    let mut pool = CandidatePool::new();
    let mk = |distance: u8, conf: f32, structural: f32| {
        let mut c = Candidate::node(
            key(UPDATE_USER),
            "u",
            ItemKind::RelatedSymbol,
            DiffSide::Head,
            CandidateSource::Structural,
        );
        c.distance = Some(distance);
        c.min_confidence = conf;
        c.path = vec![context_engine::model::PathStep {
            relation: "calls".into(),
            target: format!("t{distance}"),
            confidence: conf,
        }];
        c.raw = RawSignals {
            structural,
            ..RawSignals::default()
        };
        c
    };
    assert!(pool.insert(mk(2, 0.6, 0.5)));
    assert!(!pool.insert(mk(1, 0.9, 0.3)));
    let merged = pool
        .get(&PoolKey::Node(key(UPDATE_USER), DiffSide::Head))
        .unwrap();
    assert_eq!(merged.distance, Some(1));
    assert!((merged.min_confidence - 0.9).abs() < 1e-6);
    assert_eq!(merged.path[0].target, "t1");
    assert!((merged.raw.structural - 0.5).abs() < 1e-6);
}

#[test]
fn security_filter_trust_boundary_only() {
    let mut elements = auth_bypass_elements();
    // A related type that is neither on an endpoint path nor an authz symbol.
    elements.push(element(
        DECOY,
        codegraph::NodeKind::Class,
        Relation::RelatedType,
        1,
        0.9,
    ));
    let pool = run(ReviewerType::Security, elements.clone());
    assert!(
        has(&pool, UPDATE_USER, DiffSide::Head),
        "on the endpoint path"
    );
    assert!(has(&pool, CONTROLLER, DiffSide::Head));
    assert!(
        has(&pool, PERMISSION_CHECK, DiffSide::Base),
        "removed callee"
    );
    assert!(!has(&pool, DECOY, DiffSide::Head));
    assert!(
        !has(&pool, PROVIDER, DiffSide::Head),
        "no authz signal on the seed"
    );

    let graph = graph_of(elements);
    let mut input = input(ReviewerType::Security, Some(&graph));
    input.trust_nodes.insert(key(AUTHORIZE));
    let mut pool = CandidatePool::new();
    StructuralCandidates.generate(&input, &mut pool);
    assert!(
        has(&pool, PROVIDER, DiffSide::Head),
        "implementation of an authz symbol"
    );
}

#[test]
fn weak_edge_candidate_tagged() {
    let mut weak = element(
        UPDATE_USER,
        codegraph::NodeKind::Method,
        Relation::Caller,
        1,
        0.4,
    );
    weak.weak = true;
    let mut too_weak = element(DECOY, codegraph::NodeKind::Method, Relation::Caller, 1, 0.2);
    too_weak.weak = true;
    let pool = run(ReviewerType::Correctness, vec![weak, too_weak]);
    assert!(
        pool.get_node(key(UPDATE_USER), DiffSide::Head)
            .unwrap()
            .weak
    );
    assert!(!has(&pool, DECOY, DiffSide::Head), "below the 0.3 floor");
}

#[test]
fn decoy_report_service_absent() {
    let pool = run(ReviewerType::Correctness, auth_bypass_elements());
    assert!(!has(&pool, DECOY, DiffSide::Head));
}

#[test]
fn execution_path_marks_nodes_on_the_endpoint_path() {
    let pool = run(ReviewerType::Correctness, auth_bypass_elements());
    let controller = pool.get_node(key(CONTROLLER), DiffSide::Head).unwrap();
    assert!(controller.raw.execution_path > 0.9);
    let provider = pool.get_node(key(PROVIDER), DiffSide::Head).unwrap();
    assert_eq!(provider.raw.execution_path, 0.0);
}

#[test]
fn missing_impact_degrades_to_seeds_only() {
    let input = input(ReviewerType::Correctness, None);
    let mut pool = CandidatePool::new();
    let stats = StructuralCandidates.generate(&input, &mut pool);
    assert!(stats.degraded);
    assert!(pool.iter().all(|c| c.pinned));
}
