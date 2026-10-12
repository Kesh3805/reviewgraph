//! IMP-008 / RISK-004: golden impact, cluster and risk expectations for the PRD §151
//! auth-bypass pull request, label checks (`must_include` / `must_exclude`) and synthetic
//! scenarios for the other change shapes.
//!
//! Golden files live in `fixtures/pull-requests/auth-bypass/expected/`. Run with
//! `REVIEWGRAPH_BLESS=1` to rewrite them; a mismatch otherwise prints the actual document.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::path::PathBuf;

use codegraph::{EdgeKind, NodeKind, ResolvedBy};
use impact::cluster::PlanConfig;
use impact::graph::{ImpactBudget, ImpactGraph, Relation, ResourceRole};
use impact::input::{tags, ChangeClass, ChangeSet, ClassInput};
use impact::risk::{PathRule, RiskLevel, RiskPolicy};
use review_core::change::SymbolChange;
use review_core::ids::SymbolId;
use serde::{Deserialize, Serialize};
use support::*;

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../fixtures/pull-requests/auth-bypass/expected")
}

fn assert_golden<T: Serialize>(name: &str, value: &T) {
    let actual = format!("{}\n", serde_json::to_string_pretty(value).unwrap());
    let path = fixture_dir().join(name);
    if std::env::var("REVIEWGRAPH_BLESS").is_ok() {
        std::fs::write(&path, &actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_default()
        .replace("\r\n", "\n");
    if expected != actual {
        println!("----- BEGIN {name} -----");
        print!("{actual}");
        println!("----- END {name} -----");
        panic!(
            "{} does not match; rerun with REVIEWGRAPH_BLESS=1 or copy the document printed above",
            path.display()
        );
    }
}

#[derive(Debug, Deserialize)]
struct Include {
    relation: Relation,
    node: String,
    #[serde(default)]
    distance: Option<u8>,
}

#[derive(Debug, Deserialize)]
struct Exclude {
    node: String,
}

#[derive(Debug, Deserialize)]
struct Labels {
    must_include: Vec<Include>,
    must_exclude: Vec<Exclude>,
}

/// Every unmet label, with a message naming it.
fn label_failures(graph: &ImpactGraph, labels: &Labels) -> Vec<String> {
    let mut failures = Vec::new();
    for include in &labels.must_include {
        let found = graph.symbols.iter().any(|symbol| {
            symbol.of(include.relation).any(|element| {
                element.node_id == include.node
                    && include.distance.is_none_or(|d| d == element.distance)
            })
        });
        if !found {
            failures.push(format!(
                "missing {:?} {} (distance {:?})",
                include.relation, include.node, include.distance
            ));
        }
    }
    for exclude in &labels.must_exclude {
        let hit = graph.symbols.iter().find_map(|symbol| {
            symbol
                .elements
                .iter()
                .find(|element| element.node_id.contains(&exclude.node))
        });
        if let Some(element) = hit {
            failures.push(format!(
                "{} must be excluded but is a {:?}",
                element.node_id, element.relation
            ));
        }
    }
    failures
}

fn load_labels() -> Labels {
    let raw = std::fs::read_to_string(fixture_dir().join("impact-labels.yaml")).unwrap();
    serde_yaml::from_str(&raw).unwrap()
}

fn src_auth_policy() -> RiskPolicy {
    RiskPolicy::default().with_paths(vec![PathRule::new(
        "src/auth/**",
        RiskLevel::Critical,
        None,
    )
    .unwrap()])
}

#[test]
fn golden_impact_auth_bypass() {
    let scenario = auth_bypass();
    let graph = impact_of(&scenario.change, &scenario.head, Some(&scenario.base));
    let failures = label_failures(&graph, &load_labels());
    assert!(failures.is_empty(), "{failures:#?}");
    assert_golden("impact.json", &graph);

    let pipeline = run_pipeline(
        &scenario.change,
        &scenario.head,
        Some(&scenario.base),
        &src_auth_policy(),
        &PlanConfig::default(),
    );
    assert_eq!(pipeline.impact, graph);
    assert_golden("clusters.json", &pipeline.clustering);
}

#[test]
fn golden_risk_all_fixtures() {
    let scenario = auth_bypass();
    let pipeline = run_pipeline(
        &scenario.change,
        &scenario.head,
        Some(&scenario.base),
        &src_auth_policy(),
        &PlanConfig::default(),
    );
    let assessment = &pipeline.assessment;
    assert_eq!(assessment.level, RiskLevel::Critical);
    assert!(assessment.score >= 0.95, "{}", assessment.score);
    assert_eq!(assessment.signals[0].rule_id, "authz.call_removed");
    assert_eq!(assessment.input_hash.len(), 64);
    assert_golden("risk.json", assessment);

    // The assessment round-trips through its own wire form.
    let json = serde_json::to_string(assessment).unwrap();
    let back: impact::risk::RiskAssessment = serde_json::from_str(&json).unwrap();
    assert_eq!(&back, assessment);
}

#[test]
fn labels_must_include_and_exclude() {
    let scenario = auth_bypass();
    let graph = impact_of(&scenario.change, &scenario.head, Some(&scenario.base));
    // The harness reports a missing element and a decoy that should have been excluded.
    let wrong = Labels {
        must_include: vec![Include {
            relation: Relation::Caller,
            node: "ts:src/reports/report.service#ReportService.generate/method".to_owned(),
            distance: None,
        }],
        must_exclude: vec![Exclude {
            node: "ts:src/admin/admin.service#AdminService.updateUser".to_owned(),
        }],
    };
    let failures = label_failures(&graph, &wrong);
    assert_eq!(failures.len(), 2, "{failures:#?}");
    assert!(failures[0].starts_with("missing Caller"));
    assert!(failures[1].contains("must be excluded"));
}

fn change_of(symbols: Vec<impact::input::SymbolInput>) -> ChangeSet {
    ChangeSet {
        input_hash: "golden".to_owned(),
        head_snapshot: "head".to_owned(),
        base_snapshot: "base".to_owned(),
        symbols,
        ..ChangeSet::default()
    }
}

#[test]
fn golden_impact_api_contract_break() {
    // A DTO loses a field: every handler accepting it is related.
    let mut g = Fixture::new();
    let dto = g.symbol("src/users/dto.ts", "UpdateUserDto", NodeKind::Class);
    let controller = g.symbol(
        "src/users/user.controller.ts",
        "UserController",
        NodeKind::Controller,
    );
    let update = g.member(
        "src/users/user.controller.ts",
        controller,
        "UserController.update",
        NodeKind::Handler,
    );
    let patch = g.member(
        "src/users/user.controller.ts",
        controller,
        "UserController.patch",
        NodeKind::Handler,
    );
    g.edge(
        EdgeKind::AcceptsType,
        update,
        dto,
        ResolvedBy::TypeAnnotation,
    );
    g.edge(
        EdgeKind::AcceptsType,
        patch,
        dto,
        ResolvedBy::TypeAnnotation,
    );
    let head = g.build();

    let mut seed = changed(
        "src/users/dto.ts",
        "UpdateUserDto",
        NodeKind::Class,
        body_change(),
    );
    seed.exported = true;
    seed.classes = vec![ClassInput::new(ChangeClass::ApiContractChanged).tag(tags::BREAKING)];
    let graph = impact_of(&change_of(vec![seed]), &head, None);
    let impact = graph
        .symbol(sym_key(
            "src/users/dto.ts",
            "UpdateUserDto",
            NodeKind::Class,
        ))
        .unwrap();
    let related: Vec<_> = impact.of(Relation::RelatedType).map(|e| e.node).collect();
    assert!(related.contains(&update), "{related:?}");
    assert!(related.contains(&patch), "{related:?}");
}

#[test]
fn golden_impact_transaction_removed() {
    // `save` stops opening a transaction: its caller and the table it writes are in the impact.
    let mut g = Fixture::new();
    let table = g.table("accounts");
    let service = g.symbol(
        "src/accounts/account.service.ts",
        "AccountService",
        NodeKind::Class,
    );
    let save = g.member(
        "src/accounts/account.service.ts",
        service,
        "AccountService.save",
        NodeKind::Method,
    );
    g.edge(EdgeKind::WritesTable, save, table, ResolvedBy::Framework);
    let caller = g.symbol("src/accounts/transfer.ts", "transfer", NodeKind::Function);
    g.edge(EdgeKind::Calls, caller, save, ResolvedBy::DiConstructor);
    let head = g.build();

    let mut seed = changed(
        "src/accounts/account.service.ts",
        "AccountService.save",
        NodeKind::Method,
        body_change(),
    );
    seed.classes = vec![ClassInput::new(ChangeClass::TransactionBoundaryChanged)];
    let graph = impact_of(&change_of(vec![seed]), &head, None);
    let impact = graph.symbol(save).unwrap();
    assert!(impact.of(Relation::Caller).any(|e| e.node == caller));
    let written = impact
        .of(Relation::DbTable)
        .find(|e| e.node == table)
        .expect("the written table");
    assert_eq!(written.resource.unwrap().role, ResourceRole::Writes);
}

#[test]
fn golden_impact_pure_rename() {
    // A rename with unchanged behaviour: callers are linked to the new name, nothing was
    // removed.
    let mut g = Fixture::new();
    let renamed = g.symbol("src/a.ts", "newName", NodeKind::Function);
    let caller = g.symbol("src/b.ts", "useIt", NodeKind::Function);
    g.edge(EdgeKind::Calls, caller, renamed, ResolvedBy::Import);
    let head = g.build();

    let change = SymbolChange::Renamed {
        from: SymbolId::from_canonical_unchecked("ts:src/a#oldName/function".to_owned()),
        similarity: 1.0,
    };
    let mut seed = changed("src/a.ts", "newName", NodeKind::Function, change);
    seed.exported = true;
    let graph = impact_of(&change_of(vec![seed]), &head, None);
    let impact = graph.symbol(renamed).unwrap();
    assert!(impact.of(Relation::Caller).any(|e| e.node == caller));
    assert_eq!(impact.of(Relation::RemovedCallee).count(), 0);
}

#[test]
fn golden_impact_large_mixed() {
    // One hub with more callers than the budget allows next to a quiet symbol: the stop is
    // reported per seed and in the summary line.
    let mut g = Fixture::new();
    let hub = g.symbol("src/util.ts", "hub", NodeKind::Function);
    for i in 0..12 {
        let caller = g.symbol("src/callers.ts", &format!("c{i:02}"), NodeKind::Function);
        g.edge(EdgeKind::Calls, caller, hub, ResolvedBy::Import);
    }
    let quiet = g.symbol("src/quiet.ts", "quiet", NodeKind::Function);
    let head = g.build();
    let change = change_of(vec![
        changed("src/util.ts", "hub", NodeKind::Function, body_change()),
        changed("src/quiet.ts", "quiet", NodeKind::Function, body_change()),
    ]);
    let budget = ImpactBudget {
        max_callers: 5,
        ..ImpactBudget::default()
    };
    let graph = impact_with(&change, &head, None, &budget);
    assert_eq!(graph.symbol(hub).unwrap().of(Relation::Caller).count(), 5);
    assert!(graph.symbol(quiet).unwrap().truncation.is_empty());
    assert_eq!(graph.stats.truncations.len(), 1);
    assert_eq!(graph.stats.truncations[0].seed, hub);
    assert!(
        graph
            .summary_line()
            .starts_with("Impact truncated for 1 of 2 symbols"),
        "{}",
        graph.summary_line()
    );
}
