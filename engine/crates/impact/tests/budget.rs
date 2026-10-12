//! IMP-007: impact budgets and truncation reporting.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use codegraph::{EdgeKind, Graph, NodeKey, NodeKind, ResolvedBy};
use impact::graph::{
    resolve_budget, BudgetWarning, ImpactBudget, ImpactBudgetConfig, ImpactFlags, ImpactGraph,
    ImpactStats, Relation, RunBudget, SeedTruncation, TruncReason, Truncation,
    IMPACT_SCHEMA_VERSION,
};
use impact::input::ChangeSet;
use impact::risk::RiskLevel;
use proptest::prelude::*;
use review_core::ids::SymbolKey;
use support::*;

#[test]
fn low_risk_halves_caps() {
    let defaults = ImpactBudget::default();
    let low = resolve_budget(None, RiskLevel::Low, None);
    assert!(low.warnings.is_empty());
    assert_eq!(low.multiplier, 0.5);
    assert_eq!(low.budget.max_caller_depth, 1);
    assert_eq!(low.budget.max_callers, defaults.max_callers / 2);
    assert_eq!(low.budget.max_callees, defaults.max_callees / 2);
    assert_eq!(low.budget.max_tests, defaults.max_tests / 2);
    assert_eq!(low.budget.max_endpoints, defaults.max_endpoints / 2);
    assert_eq!(
        low.budget.max_total_elements_pr,
        defaults.max_total_elements_pr / 2
    );
    let medium = resolve_budget(None, RiskLevel::Medium, None);
    assert_eq!(medium.budget, defaults);
}

#[test]
fn critical_allows_depth3() {
    let critical = resolve_budget(None, RiskLevel::Critical, None).budget;
    assert_eq!(critical.max_caller_depth, 3);
    assert_eq!(critical.max_callers, 100);
    assert_eq!(
        resolve_budget(None, RiskLevel::High, None)
            .budget
            .max_caller_depth,
        2
    );

    let mut g = Fixture::new();
    let keys: Vec<NodeKey> = (0..5)
        .map(|i| g.symbol("src/chain.ts", &format!("f{i}"), NodeKind::Function))
        .collect();
    for window in keys.windows(2) {
        g.edge(EdgeKind::Calls, window[1], window[0], ResolvedBy::Import);
    }
    let head = g.build();
    let change = ChangeSet {
        symbols: vec![changed(
            "src/chain.ts",
            "f0",
            NodeKind::Function,
            body_change(),
        )],
        ..ChangeSet::default()
    };
    let graph = impact_with(&change, &head, None, &critical);
    let deepest = graph
        .symbol(keys[0])
        .unwrap()
        .of(Relation::Caller)
        .map(|e| e.distance)
        .max();
    assert_eq!(deepest, Some(3));
}

#[test]
fn run_budget_clamps_config() {
    let config = ImpactBudgetConfig {
        max_callers: Some(150),
        max_total_elements_pr: Some(8_000),
        ..ImpactBudgetConfig::default()
    };
    // Without a run budget the hard maximum still applies: 150 × 2.0 → 200.
    let critical = resolve_budget(Some(&config), RiskLevel::Critical, None).budget;
    assert_eq!(critical.max_callers, 200);
    assert_eq!(critical.max_total_elements_pr, 16_000);

    let run = RunBudget {
        max_elements_per_relation: Some(40),
        max_total_elements_pr: Some(1_000),
    };
    let clamped = resolve_budget(Some(&config), RiskLevel::Medium, Some(&run)).budget;
    assert_eq!(clamped.max_callers, 40);
    assert_eq!(clamped.max_total_elements_pr, 1_000);
    assert!(clamped.max_tests <= 40);
}

#[test]
fn invalid_config_falls_back_to_defaults() {
    for config in [
        ImpactBudgetConfig {
            max_callers: Some(0),
            ..ImpactBudgetConfig::default()
        },
        ImpactBudgetConfig {
            max_callers: Some(500),
            ..ImpactBudgetConfig::default()
        },
        ImpactBudgetConfig {
            max_total_elements_pr: Some(50_000),
            ..ImpactBudgetConfig::default()
        },
        ImpactBudgetConfig {
            min_confidence: Some(1.5),
            ..ImpactBudgetConfig::default()
        },
    ] {
        let resolved = resolve_budget(Some(&config), RiskLevel::Medium, None);
        assert_eq!(resolved.budget, ImpactBudget::default(), "{config:?}");
        assert!(matches!(
            resolved.warnings.as_slice(),
            [BudgetWarning::ConfigInvalid { .. }]
        ));
    }
    let valid = ImpactBudgetConfig {
        max_callers: Some(10),
        min_confidence: Some(0.7),
        ..ImpactBudgetConfig::default()
    };
    let resolved = resolve_budget(Some(&valid), RiskLevel::Medium, None);
    assert!(resolved.warnings.is_empty());
    assert_eq!(resolved.budget.max_callers, 10);
    assert_eq!(resolved.budget.min_confidence, conf(0.7));
}

fn hub_graph(callers: u32) -> (Graph, NodeKey) {
    let mut g = Fixture::new();
    let hub = g.symbol("src/util.ts", "hub", NodeKind::Function);
    for i in 0..callers {
        let caller = g.symbol("src/callers.ts", &format!("c{i}"), NodeKind::Function);
        g.edge(EdgeKind::Calls, caller, hub, ResolvedBy::Import);
        let outer = g.symbol("src/outer.ts", &format!("o{i}"), NodeKind::Function);
        g.edge(EdgeKind::Calls, outer, caller, ResolvedBy::Import);
    }
    (g.build(), hub)
}

#[test]
fn every_truncation_reported_in_stats() {
    let (head, hub) = hub_graph(12);
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
        max_callers: 4,
        max_caller_depth: 1,
        ..ImpactBudget::default()
    };
    let graph = impact_with(&change, &head, None, &budget);
    let per_symbol: usize = graph.symbols.iter().map(|s| s.truncation.len()).sum();
    assert!(per_symbol >= 2, "limit and depth truncations expected");
    assert_eq!(per_symbol, graph.stats.truncations.len());
    for symbol in &graph.symbols {
        for truncation in &symbol.truncation {
            assert!(graph.stats.truncations.contains(&SeedTruncation {
                seed: symbol.seed,
                truncation: truncation.clone(),
            }));
        }
    }
    let reasons: Vec<TruncReason> = graph
        .symbol(hub)
        .unwrap()
        .truncation
        .iter()
        .map(|t| t.reason)
        .collect();
    assert!(reasons.contains(&TruncReason::Limit));
    assert!(reasons.contains(&TruncReason::Depth));
    assert!(graph
        .summary_line()
        .starts_with("Impact truncated for 1 of 1 symbol ("));
}

fn truncation(relation: Relation) -> Truncation {
    Truncation {
        relation: Some(relation),
        limit: 5,
        dropped: 1,
        reason: TruncReason::Limit,
        visited: None,
    }
}

#[test]
fn summary_line_format() {
    let seed = |n: u8| SymbolKey::from_bytes([n; 16]);
    let graph = ImpactGraph {
        schema_version: IMPACT_SCHEMA_VERSION,
        symbols: Vec::new(),
        budget: ImpactBudget::default(),
        stats: ImpactStats {
            seeds: 41,
            weak_elements: 12,
            truncations: vec![
                SeedTruncation {
                    seed: seed(1),
                    truncation: truncation(Relation::Caller),
                },
                SeedTruncation {
                    seed: seed(2),
                    truncation: truncation(Relation::Caller),
                },
                SeedTruncation {
                    seed: seed(3),
                    truncation: truncation(Relation::Test),
                },
            ],
            ..ImpactStats::default()
        },
        flags: ImpactFlags::default(),
        test_targets: Vec::new(),
        input_hash: String::new(),
    };
    assert_eq!(
        graph.summary_line(),
        "Impact truncated for 3 of 41 symbols (callers ×2, tests ×1); 12 low-confidence relations omitted."
    );

    let complete = ImpactGraph {
        stats: ImpactStats {
            seeds: 1,
            ..ImpactStats::default()
        },
        ..graph
    };
    assert_eq!(complete.summary_line(), "Impact complete for 1 symbol.");
}

fn random_graph(n: u32, edges: &[(u32, u32, u8)]) -> Graph {
    let mut g = Fixture::new();
    let keys: Vec<NodeKey> = (0..n)
        .map(|i| g.symbol("src/r.ts", &format!("f{i}"), NodeKind::Function))
        .collect();
    let tests: Vec<NodeKey> = (0..n / 3)
        .map(|i| g.test_case("src/r.spec.ts", "r", &format!("case {i}")))
        .collect();
    for (from, to, class) in edges {
        let (from, to) = ((*from % n) as usize, (*to % n) as usize);
        if from == to {
            continue;
        }
        match class % 5 {
            0 => g.edge(EdgeKind::Calls, keys[from], keys[to], ResolvedBy::Import),
            1 => g.edge(
                EdgeKind::Calls,
                keys[from],
                keys[to],
                ResolvedBy::NameUnique,
            ),
            2 => g.edge(
                EdgeKind::Calls,
                keys[from],
                keys[to],
                ResolvedBy::NameAmbiguous,
            ),
            3 if !tests.is_empty() => g.edge(
                EdgeKind::Tests,
                tests[from % tests.len()],
                keys[to],
                ResolvedBy::Framework,
            ),
            _ => g.edge(
                EdgeKind::UsesType,
                keys[from],
                keys[to],
                ResolvedBy::TypeAnnotation,
            ),
        }
    }
    g.build()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn proptest_elements_never_exceed_effective_caps(
        n in 3u32..30,
        edges in prop::collection::vec((0u32..30, 0u32..30, 0u8..5), 0..150),
        seeds in prop::collection::vec(0u32..30, 1..6),
        level in 0usize..4,
        max_callers in 1u32..12,
        max_tests in 1u32..6,
        pr_cap in 1u32..40,
    ) {
        let head = random_graph(n, &edges);
        let mut symbols: Vec<_> = seeds
            .iter()
            .map(|i| changed("src/r.ts", &format!("f{}", i % n), NodeKind::Function, body_change()))
            .collect();
        symbols.sort_by(|a, b| a.id().cmp(b.id()));
        symbols.dedup_by(|a, b| a.key() == b.key());
        let change = ChangeSet { symbols, ..ChangeSet::default() };
        let config = ImpactBudgetConfig {
            max_callers: Some(max_callers),
            max_tests: Some(max_tests),
            max_total_elements_pr: Some(pr_cap),
            ..ImpactBudgetConfig::default()
        };
        let resolved = resolve_budget(Some(&config), RiskLevel::ALL[level], None);
        let budget = resolved.budget;
        let graph = impact_with(&change, &head, None, &budget);
        prop_assert!(graph.stats.total_elements <= budget.max_total_elements_pr);
        for symbol in &graph.symbols {
            prop_assert!(symbol.elements.len() as u32 <= budget.max_total_elements_per_symbol);
            prop_assert!(symbol.of(Relation::Caller).count() as u32 <= budget.max_callers);
            prop_assert!(symbol.of(Relation::Callee).count() as u32 <= budget.max_callees);
            prop_assert!(symbol.of(Relation::Test).count() as u32 <= budget.max_tests);
            prop_assert!(symbol.of(Relation::RelatedType).count() as u32 <= budget.max_type_relations);
            for element in symbol.of(Relation::Caller) {
                prop_assert!(element.distance <= budget.max_caller_depth);
            }
        }
    }
}
