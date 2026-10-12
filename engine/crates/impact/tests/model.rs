//! IMP-001: the impact graph model, its merge rule, ordering, serialization and input hash.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;

use codegraph::{Confidence, EdgeKind, NodeKey, NodeKind};
use impact::graph::{
    compute_input_hash, Candidate, ElementSet, Extras, GraphSide, ImpactBudget, ImpactFlags,
    ImpactGraph, ImpactStats, Offer, PathStep, Relation, SymbolImpact, Trail,
    IMPACT_SCHEMA_VERSION,
};
use review_core::ids::SymbolKey;

fn key(n: u8) -> NodeKey {
    SymbolKey::from_bytes([n; 16])
}

fn conf(value: f32) -> Confidence {
    Confidence::from_f32(value)
}

fn calls(from: NodeKey, to: NodeKey, confidence: f32) -> PathStep {
    PathStep {
        from,
        edge: EdgeKind::Calls,
        to,
        confidence: conf(confidence),
        graph: GraphSide::Head,
    }
}

/// A caller trail `seed ← via… ← end`, one hop per node.
fn caller_trail(seed: NodeKey, via: &[(NodeKey, f32)]) -> Trail {
    let mut trail = Trail::seed(seed);
    let mut previous = seed;
    for (node, confidence) in via {
        trail = trail.extend(calls(*node, previous, *confidence), *node, true);
        previous = *node;
    }
    trail
}

fn candidate(node: NodeKey, trail: Trail) -> Candidate {
    Candidate {
        node,
        node_id: format!("ts:src/n{}#f/function", node.as_bytes()[0]),
        kind: NodeKind::Function,
        trail,
        extras: Extras::default(),
    }
}

#[test]
fn best_path_prefers_higher_min_confidence() {
    let seed = key(1);
    let target = key(9);
    let mut set = ElementSet::new();
    // Short but weak: one hop at 0.6.
    let weak = caller_trail(seed, &[(target, 0.6)]);
    // Longer but stronger: two hops at 0.95.
    let strong = caller_trail(seed, &[(key(5), 0.95), (target, 0.95)]);
    assert_eq!(
        set.offer(Relation::Caller, candidate(target, weak)),
        Offer::Inserted
    );
    assert_eq!(
        set.offer(Relation::Caller, candidate(target, strong.clone())),
        Offer::Improved
    );
    let elements = set.into_elements(&ImpactBudget::default());
    assert_eq!(elements.len(), 1);
    assert_eq!(elements[0].min_confidence, conf(0.95));
    assert_eq!(elements[0].distance, 2);
    assert_eq!(elements[0].alt_paths, 1);
    assert_eq!(elements[0].path, strong.steps);
}

#[test]
fn tie_breaks_by_distance_then_lexicographic() {
    let seed = key(1);
    let target = key(9);

    // Same confidence: the shorter path wins regardless of offer order.
    for order in [false, true] {
        let mut set = ElementSet::new();
        let short = caller_trail(seed, &[(target, 0.9)]);
        let long = caller_trail(seed, &[(key(4), 0.9), (target, 0.9)]);
        let (first, second) = if order {
            (short.clone(), long)
        } else {
            (long, short.clone())
        };
        set.offer(Relation::Caller, candidate(target, first));
        set.offer(Relation::Caller, candidate(target, second));
        let elements = set.into_elements(&ImpactBudget::default());
        assert_eq!(elements[0].distance, 1, "order {order}");
        assert_eq!(elements[0].path, short.steps);
    }

    // Same confidence and distance: the lexicographically smaller node sequence wins.
    for order in [false, true] {
        let mut set = ElementSet::new();
        let via_low = caller_trail(seed, &[(key(3), 0.9), (target, 0.9)]);
        let via_high = caller_trail(seed, &[(key(7), 0.9), (target, 0.9)]);
        let (first, second) = if order {
            (via_low.clone(), via_high)
        } else {
            (via_high, via_low.clone())
        };
        set.offer(Relation::Caller, candidate(target, first));
        set.offer(Relation::Caller, candidate(target, second));
        let elements = set.into_elements(&ImpactBudget::default());
        assert_eq!(elements[0].path, via_low.steps, "order {order}");
        assert_eq!(elements[0].alt_paths, 1);
    }
}

#[test]
fn elements_sorted_deterministically() {
    let seed = key(1);
    let offers = vec![
        (
            Relation::Test,
            key(20),
            caller_trail(seed, &[(key(20), 1.0)]),
        ),
        (
            Relation::Caller,
            key(12),
            caller_trail(seed, &[(key(12), 0.6)]),
        ),
        (
            Relation::Caller,
            key(11),
            caller_trail(seed, &[(key(11), 0.95)]),
        ),
        (
            Relation::Caller,
            key(13),
            caller_trail(seed, &[(key(11), 0.95), (key(13), 0.95)]),
        ),
        (
            Relation::Callee,
            key(30),
            caller_trail(seed, &[(key(30), 0.8)]),
        ),
    ];
    let mut forward = ElementSet::new();
    for (relation, node, trail) in offers.clone() {
        forward.offer(relation, candidate(node, trail));
    }
    let mut backward = ElementSet::new();
    for (relation, node, trail) in offers.into_iter().rev() {
        backward.offer(relation, candidate(node, trail));
    }
    let budget = ImpactBudget::default();
    let a = forward.into_elements(&budget);
    let b = backward.into_elements(&budget);
    assert_eq!(a, b);
    let order: Vec<(Relation, NodeKey)> = a.iter().map(|e| (e.relation, e.node)).collect();
    assert_eq!(
        order,
        vec![
            (Relation::Caller, key(11)),
            (Relation::Caller, key(12)),
            (Relation::Caller, key(13)),
            (Relation::Callee, key(30)),
            (Relation::Test, key(20)),
        ]
    );
    // Every path is at or above the default 0.5 floor, so nothing is weak here.
    assert!(a.iter().all(|e| !e.weak));
}

#[test]
fn weak_elements_are_flagged_against_the_budget() {
    let seed = key(1);
    let mut set = ElementSet::new();
    set.offer(
        Relation::Caller,
        candidate(key(2), caller_trail(seed, &[(key(2), 0.3)])),
    );
    let elements = set.into_elements(&ImpactBudget::default());
    assert!(elements[0].weak);
}

fn sample_graph() -> ImpactGraph {
    let seed = key(1);
    let mut set = ElementSet::new();
    set.offer(
        Relation::Caller,
        candidate(key(2), caller_trail(seed, &[(key(2), 0.95)])),
    );
    let budget = ImpactBudget::default();
    let mut by_relation = BTreeMap::new();
    by_relation.insert(Relation::Caller, 1);
    ImpactGraph {
        schema_version: IMPACT_SCHEMA_VERSION,
        symbols: vec![SymbolImpact {
            seed,
            seed_id: "ts:src/a#f/function".to_owned(),
            side: GraphSide::Head,
            skipped: None,
            untested: true,
            elements: set.into_elements(&budget),
            truncation: Vec::new(),
        }],
        input_hash: compute_input_hash("change", "head", "base", &budget),
        budget,
        stats: ImpactStats {
            seeds: 1,
            total_elements: 1,
            elements_by_relation: by_relation,
            ..ImpactStats::default()
        },
        flags: ImpactFlags::default(),
        test_targets: Vec::new(),
    }
}

#[test]
fn serde_roundtrip_and_schema() {
    let graph = sample_graph();
    let json = serde_json::to_string_pretty(&graph).unwrap();
    let back: ImpactGraph = serde_json::from_str(&json).unwrap();
    assert_eq!(back, graph);
    assert_eq!(serde_json::to_string_pretty(&back).unwrap(), json);
    // Relations and sides are snake_case on the wire; keys are 32 hex characters.
    assert!(json.contains("\"relation\": \"caller\""));
    assert!(json.contains("\"graph\": \"head\""));
    assert!(json.contains(&key(2).to_string()));

    let schema = serde_json::to_value(ImpactGraph::json_schema()).unwrap();
    let properties = schema
        .pointer("/properties")
        .and_then(|p| p.as_object())
        .unwrap();
    for field in ["schema_version", "symbols", "budget", "stats", "input_hash"] {
        assert!(properties.contains_key(field), "schema lacks {field}");
    }
    let unknown = json.replacen(
        "\"schema_version\"",
        "\"surprise\": 1, \"schema_version\"",
        1,
    );
    assert!(serde_json::from_str::<ImpactGraph>(&unknown).is_err());
}

#[test]
fn input_hash_includes_budget() {
    let budget = ImpactBudget::default();
    let base = compute_input_hash("change", "head", "base", &budget);
    assert_eq!(base.len(), 64);
    assert_eq!(base, compute_input_hash("change", "head", "base", &budget));
    let mut tighter = budget.clone();
    tighter.max_callers -= 1;
    assert_ne!(base, compute_input_hash("change", "head", "base", &tighter));
    assert_ne!(base, compute_input_hash("change2", "head", "base", &budget));
    assert_ne!(base, compute_input_hash("change", "head2", "base", &budget));
    assert_ne!(base, compute_input_hash("change", "head", "base2", &budget));
    // Length prefixes keep field boundaries meaningful.
    assert_ne!(
        compute_input_hash("ab", "c", "", &budget),
        compute_input_hash("a", "bc", "", &budget)
    );
}
