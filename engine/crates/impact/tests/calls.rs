//! IMP-002: callers and callees bounded expansion.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use codegraph::{EdgeKind, Graph, NodeKey, NodeKind, ResolvedBy};
use impact::graph::{GraphSide, ImpactBudget, ImpactInputs, Relation, TruncReason};
use impact::input::ChangeSet;
use proptest::prelude::*;
use review_core::change::SymbolChange;
use support::*;

fn caller_nodes(
    graph: &impact::ImpactGraph,
    seed: NodeKey,
    relation: Relation,
) -> Vec<(NodeKey, u8)> {
    graph
        .symbol(seed)
        .unwrap()
        .of(relation)
        .map(|element| (element.node, element.distance))
        .collect()
}

#[test]
fn auth_bypass_callers_updateuser_d1_controller_d2() {
    let scenario = auth_bypass();
    let graph = impact_of(&scenario.change, &scenario.head, Some(&scenario.base));
    let callers = caller_nodes(&graph, scenario.keys.authorize, Relation::Caller);
    assert!(
        callers.contains(&(scenario.keys.update_user, 1)),
        "{callers:?}"
    );
    assert!(
        callers.contains(&(scenario.keys.controller_update, 2)),
        "{callers:?}"
    );
    let controller = graph
        .symbol(scenario.keys.authorize)
        .unwrap()
        .of(Relation::Caller)
        .find(|e| e.node == scenario.keys.controller_update)
        .unwrap();
    // Path from the seed: authorize ← updateUser ← UserController.update, edges as stored.
    assert_eq!(controller.path.len(), 2);
    assert_eq!(controller.path[0].from, scenario.keys.update_user);
    assert_eq!(controller.path[0].to, scenario.keys.authorize);
    assert_eq!(controller.path[1].from, scenario.keys.controller_update);
    assert_eq!(controller.path[1].to, scenario.keys.update_user);
    assert_eq!(controller.min_confidence, conf(0.85));
    assert!(!controller.weak);
    // The caller's own table write is on the caller, not a seed relation.
    let update_user = graph
        .symbol(scenario.keys.authorize)
        .unwrap()
        .of(Relation::Caller)
        .find(|e| e.node == scenario.keys.update_user)
        .unwrap();
    assert_eq!(update_user.touches, vec!["db:public.users".to_owned()]);
}

#[test]
fn auth_bypass_removed_callee_permission_check_on_base() {
    let scenario = auth_bypass();
    let graph = impact_of(&scenario.change, &scenario.head, Some(&scenario.base));
    let impact = graph.symbol(scenario.keys.authorize).unwrap();
    let removed: Vec<_> = impact.of(Relation::RemovedCallee).collect();
    assert_eq!(removed.len(), 1);
    assert_eq!(removed[0].node, scenario.keys.permission_check);
    assert_eq!(removed[0].node_id, PERMISSION_CHECK_ID);
    assert_eq!(removed[0].path[0].graph, GraphSide::Base);
    assert_eq!(removed[0].path[0].edge, EdgeKind::Calls);
    // On head the call is gone, so it is not a callee.
    assert!(impact
        .of(Relation::Callee)
        .all(|e| e.node != scenario.keys.permission_check));
}

#[test]
fn report_service_decoy_not_a_caller() {
    let scenario = auth_bypass();
    let graph = impact_of(&scenario.change, &scenario.head, Some(&scenario.base));
    let nodes = graph.element_nodes();
    assert!(!nodes.contains(&scenario.keys.report_generate));
    assert!(!nodes.contains(&scenario.keys.authorize_header));
}

#[test]
fn interface_dispatch_callers_included() {
    // A consumer calls the interface member; the implementation is the changed seed.
    let mut g = Fixture::new();
    let iface = g.symbol("src/i.ts", "Store", NodeKind::Interface);
    let iface_save = g.member("src/i.ts", iface, "Store.save", NodeKind::Method);
    let class = g.symbol("src/s.ts", "PgStore", NodeKind::Class);
    let save = g.member("src/s.ts", class, "PgStore.save", NodeKind::Method);
    g.edge(EdgeKind::Implements, class, iface, ResolvedBy::Import);
    g.edge(
        EdgeKind::Overrides,
        save,
        iface_save,
        ResolvedBy::Structural,
    );
    let consumer = g.symbol("src/c.ts", "persist", NodeKind::Function);
    g.edge(
        EdgeKind::Calls,
        consumer,
        iface_save,
        ResolvedBy::TypeAnnotation,
    );
    let head = g.build();

    let change = ChangeSet {
        symbols: vec![changed(
            "src/s.ts",
            "PgStore.save",
            NodeKind::Method,
            body_change(),
        )],
        ..ChangeSet::default()
    };
    let graph = impact_of(&change, &head, None);
    let element = graph
        .symbol(save)
        .unwrap()
        .of(Relation::Caller)
        .find(|e| e.node == consumer)
        .expect("caller through the interface");
    assert_eq!(element.distance, 1);
    assert_eq!(element.path[0].edge, EdgeKind::Implements);
    assert_eq!(element.path[0].confidence, conf(0.9));
    assert_eq!(element.path[1].edge, EdgeKind::Calls);
    assert_eq!(element.min_confidence, conf(0.8));
}

/// `f0 ← f1 ← f2 ← f3 ← f4`: a call chain.
fn chain(length: u32) -> (Graph, Vec<NodeKey>) {
    let mut g = Fixture::new();
    let keys: Vec<NodeKey> = (0..length)
        .map(|i| g.symbol("src/chain.ts", &format!("f{i}"), NodeKind::Function))
        .collect();
    for window in keys.windows(2) {
        g.edge(EdgeKind::Calls, window[1], window[0], ResolvedBy::Import);
    }
    (g.build(), keys)
}

fn chain_change() -> ChangeSet {
    ChangeSet {
        symbols: vec![changed(
            "src/chain.ts",
            "f0",
            NodeKind::Function,
            body_change(),
        )],
        ..ChangeSet::default()
    }
}

#[test]
fn depth3_only_when_budget_remains() {
    let (head, keys) = chain(5);
    let change = chain_change();

    let mut budget = ImpactBudget {
        max_caller_depth: 3,
        ..ImpactBudget::default()
    };
    let graph = impact_with(&change, &head, None, &budget);
    assert!(graph.stats.transitive_pass_ran);
    let callers = caller_nodes(&graph, keys[0], Relation::Caller);
    assert!(callers.contains(&(keys[3], 3)), "{callers:?}");
    assert!(!callers.iter().any(|(node, _)| *node == keys[4]));

    // With less than half the PR budget left after depth 2, depth 3 does not run.
    budget.max_total_elements_pr = 3;
    let graph = impact_with(&change, &head, None, &budget);
    assert!(!graph.stats.transitive_pass_ran);
    let callers = caller_nodes(&graph, keys[0], Relation::Caller);
    assert!(
        callers.iter().all(|(_, distance)| *distance <= 2),
        "{callers:?}"
    );

    // Default depth is 2.
    let graph = impact_of(&change, &head, None);
    let callers = caller_nodes(&graph, keys[0], Relation::Caller);
    assert_eq!(callers, vec![(keys[1], 1), (keys[2], 2)]);
    let depth = graph.symbol(keys[0]).unwrap().truncation.clone();
    assert!(depth
        .iter()
        .any(|t| t.relation == Some(Relation::Caller) && t.reason == TruncReason::Depth));
}

#[test]
fn per_symbol_caller_cap_truncates_and_reports() {
    let mut g = Fixture::new();
    let hub = g.symbol("src/util.ts", "hub", NodeKind::Function);
    for i in 0..12 {
        let caller = g.symbol("src/callers.ts", &format!("c{i:02}"), NodeKind::Function);
        let by = if i < 3 {
            ResolvedBy::Import
        } else {
            ResolvedBy::NameUnique
        };
        g.edge(EdgeKind::Calls, caller, hub, by);
    }
    let head = g.build();
    let change = ChangeSet {
        symbols: vec![changed(
            "src/util.ts",
            "hub",
            NodeKind::Function,
            body_change(),
        )],
        ..ChangeSet::default()
    };
    let budget = ImpactBudget {
        max_callers: 5,
        ..ImpactBudget::default()
    };
    let graph = impact_with(&change, &head, None, &budget);
    let impact = graph.symbol(hub).unwrap();
    let callers: Vec<_> = impact.of(Relation::Caller).collect();
    assert_eq!(callers.len(), 5);
    // The three high-confidence callers are kept first.
    assert_eq!(
        callers
            .iter()
            .filter(|e| e.min_confidence == conf(0.95))
            .count(),
        3
    );
    let truncation = impact
        .truncation
        .iter()
        .find(|t| t.relation == Some(Relation::Caller) && t.reason == TruncReason::Limit)
        .unwrap();
    assert_eq!(truncation.limit, 5);
    assert_eq!(truncation.dropped, 7);
    assert!(graph
        .stats
        .truncations
        .iter()
        .any(|t| t.seed == hub && t.truncation == *truncation));
}

#[test]
fn low_confidence_edge_excluded_but_weak_listed() {
    let mut g = Fixture::new();
    let seed = g.symbol("src/a.ts", "target", NodeKind::Function);
    let ambiguous = g.symbol("src/b.ts", "maybeCaller", NodeKind::Function);
    let beyond = g.symbol("src/c.ts", "outer", NodeKind::Function);
    g.edge(EdgeKind::Calls, ambiguous, seed, ResolvedBy::NameAmbiguous);
    g.edge(EdgeKind::Calls, beyond, ambiguous, ResolvedBy::Import);
    let head = g.build();
    let change = ChangeSet {
        symbols: vec![changed(
            "src/a.ts",
            "target",
            NodeKind::Function,
            body_change(),
        )],
        ..ChangeSet::default()
    };
    let graph = impact_of(&change, &head, None);
    let callers: Vec<_> = graph.symbol(seed).unwrap().of(Relation::Caller).collect();
    assert_eq!(callers.len(), 1, "a weak caller is not expanded");
    assert_eq!(callers[0].node, ambiguous);
    assert!(callers[0].weak);
    assert_eq!(callers[0].min_confidence, conf(0.3));
    assert_eq!(graph.stats.weak_elements, 1);
}

#[test]
fn removed_symbol_callers_from_base_graph() {
    let mut base = Fixture::new();
    let gone = base.symbol("src/a.ts", "legacy", NodeKind::Function);
    let kept = base.symbol("src/b.ts", "kept", NodeKind::Function);
    let deleted_too = base.symbol("src/c.ts", "deletedToo", NodeKind::Function);
    base.edge(EdgeKind::Calls, kept, gone, ResolvedBy::Import);
    base.edge(EdgeKind::Calls, deleted_too, gone, ResolvedBy::Import);
    let base = base.build();
    let mut head = Fixture::new();
    head.symbol("src/b.ts", "kept", NodeKind::Function);
    let head = head.build();

    let change = ChangeSet {
        symbols: vec![changed(
            "src/a.ts",
            "legacy",
            NodeKind::Function,
            SymbolChange::Removed,
        )],
        ..ChangeSet::default()
    };
    let graph = impact_of(&change, &head, Some(&base));
    let impact = graph.symbol(gone).unwrap();
    assert_eq!(impact.side, GraphSide::Base);
    let callers: Vec<_> = impact.of(Relation::Caller).collect();
    assert_eq!(callers.len(), 2);
    for caller in callers {
        assert_eq!(caller.path[0].graph, GraphSide::Base);
        assert_eq!(caller.missing_on_head, caller.node == deleted_too);
    }

    // Without a base graph the removed seed is reported missing, not silently empty.
    let graph = impact_of(&change, &head, None);
    let impact = graph.symbol(gone).unwrap();
    assert!(impact.elements.is_empty());
    assert_eq!(impact.truncation[0].reason, TruncReason::SeedMissing);
    assert_eq!(graph.stats.seeds_without_graph, 1);
}

#[test]
fn cosmetic_and_generated_seeds_listed_without_elements() {
    let scenario = auth_bypass();
    let mut change = scenario.change.clone();
    change.symbols[0].cosmetic = true;
    let graph = impact_of(&change, &scenario.head, Some(&scenario.base));
    let impact = graph.symbol(scenario.keys.authorize).unwrap();
    assert!(impact.elements.is_empty());
    assert_eq!(impact.skipped, Some(impact::graph::SeedSkip::Cosmetic));
}

/// A random call graph over `n` functions: edge `(from, to, confidence class)` triples.
fn random_graph(n: u32, edges: &[(u32, u32, u8)]) -> Graph {
    let mut g = Fixture::new();
    let keys: Vec<NodeKey> = (0..n)
        .map(|i| g.symbol("src/r.ts", &format!("f{i}"), NodeKind::Function))
        .collect();
    for (from, to, class) in edges {
        let (from, to) = (*from % n, *to % n);
        if from == to {
            continue;
        }
        let by = match class % 4 {
            0 => ResolvedBy::Import,
            1 => ResolvedBy::DiConstructor,
            2 => ResolvedBy::NameUnique,
            _ => ResolvedBy::NameAmbiguous,
        };
        g.edge(EdgeKind::Calls, keys[from as usize], keys[to as usize], by);
    }
    g.build()
}

fn random_change(n: u32, seeds: &[u32]) -> ChangeSet {
    let mut symbols: Vec<_> = seeds
        .iter()
        .map(|i| {
            changed(
                "src/r.ts",
                &format!("f{}", i % n),
                NodeKind::Function,
                body_change(),
            )
        })
        .collect();
    symbols.sort_by(|a, b| a.id().cmp(b.id()));
    symbols.dedup_by(|a, b| a.key() == b.key());
    ChangeSet {
        symbols,
        ..ChangeSet::default()
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn parallel_result_equals_sequential(
        n in 3u32..40,
        edges in prop::collection::vec((0u32..40, 0u32..40, 0u8..4), 0..160),
        seeds in prop::collection::vec(0u32..40, 1..6),
        depth in 1u8..4,
    ) {
        let head = random_graph(n, &edges);
        let change = random_change(n, &seeds);
        let budget = ImpactBudget {
            max_caller_depth: depth,
            max_callers: 7,
            max_total_elements_pr: 25,
            ..ImpactBudget::default()
        };
        let run = |parallel: bool| {
            impact::build_impact(&ImpactInputs {
                change: &change,
                head: &head,
                base: None,
                budget: &budget,
                priority: None,
                parallel,
            })
        };
        let sequential = run(false);
        let parallel = run(true);
        prop_assert_eq!(
            serde_json::to_string(&sequential).unwrap(),
            serde_json::to_string(&parallel).unwrap()
        );
    }

    #[test]
    fn bfs_never_exceeds_budget(
        n in 3u32..40,
        edges in prop::collection::vec((0u32..40, 0u32..40, 0u8..4), 0..200),
        seeds in prop::collection::vec(0u32..40, 1..8),
        max_callers in 1u32..10,
        per_symbol in 1u32..20,
        pr_cap in 1u32..60,
        depth in 1u8..4,
    ) {
        let head = random_graph(n, &edges);
        let change = random_change(n, &seeds);
        let budget = ImpactBudget {
            max_caller_depth: depth,
            max_callers,
            max_total_elements_per_symbol: per_symbol,
            max_total_elements_pr: pr_cap,
            ..ImpactBudget::default()
        };
        let graph = impact_with(&change, &head, None, &budget);
        prop_assert!(graph.stats.total_elements <= pr_cap);
        for symbol in &graph.symbols {
            prop_assert!(symbol.elements.len() as u32 <= per_symbol);
            prop_assert!(symbol.of(Relation::Caller).count() as u32 <= max_callers);
            for element in &symbol.elements {
                prop_assert!(element.distance <= depth.max(1));
            }
        }
    }
}
