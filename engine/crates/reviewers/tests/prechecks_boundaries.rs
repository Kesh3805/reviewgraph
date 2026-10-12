//! REV-A-002: boundary-violation and cycle detector.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use reviewers::context::{ChangedSymbol, ContextSnapshot, NeighborEdge};
use reviewers::prechecks::boundaries::{
    forbidden_dependency_precheck, new_cycle_precheck, BoundaryConfig, LayerRule, RULE_FORBIDDEN,
    RULE_NEW_CYCLE,
};
use reviewers::prechecks::PrecheckOutcome;

fn sym(path: &str) -> ChangedSymbol {
    ChangedSymbol {
        symbol_id: format!("ts:{path}#X.f/method"),
        kind: "method".into(),
        path: path.into(),
        range: [1, 10],
        base_range: None,
        file_lines: Some(20),
        change_classes: vec!["call_added".into()],
        signature_base: None,
        signature_head: None,
        body_base: None,
        body_head: None,
        hunks: Vec::new(),
        generated: false,
    }
}

fn import(from: &str, to: &str) -> NeighborEdge {
    NeighborEdge {
        from: format!("ts:{from}#X.f/method"),
        to: format!("ts:{to}#Y.g/method"),
        kind: "IMPORTS".into(),
        confidence: 1.0,
    }
}

fn cfg() -> BoundaryConfig {
    BoundaryConfig {
        layers: vec![
            LayerRule {
                name: "controller".into(),
                globs: vec!["src/**/*.controller.ts".into()],
            },
            LayerRule {
                name: "service".into(),
                globs: vec!["src/**/*.service.ts".into()],
            },
            LayerRule {
                name: "repository".into(),
                globs: vec!["src/**/*.repository.ts".into()],
            },
        ],
        forbidden: vec![("controller".into(), "repository".into())],
    }
}

fn ctx(changed: &[&str], edges: Vec<NeighborEdge>) -> ContextSnapshot {
    ContextSnapshot {
        changed_symbols: changed.iter().map(|p| sym(p)).collect(),
        edges,
        ..ContextSnapshot::default()
    }
}

#[test]
fn forbidden_controller_to_repository_import_flagged() {
    let cx = ctx(
        &["src/a/a.controller.ts"],
        vec![import("src/a/a.controller.ts", "src/a/a.repository.ts")],
    );
    let out = forbidden_dependency_precheck(&cx, &[], &cfg());
    assert_eq!(out.findings().len(), 1);
    assert_eq!(out.findings()[0].rule_id, RULE_FORBIDDEN);
}

#[test]
fn allowed_layer_import_not_flagged() {
    let cx = ctx(
        &["src/a/a.controller.ts"],
        vec![import("src/a/a.controller.ts", "src/a/a.service.ts")],
    );
    assert!(forbidden_dependency_precheck(&cx, &[], &cfg())
        .findings()
        .is_empty());
}

#[test]
fn preexisting_forbidden_edge_not_flagged() {
    let e = import("src/a/a.controller.ts", "src/a/a.repository.ts");
    let cx = ctx(&["src/a/a.controller.ts"], vec![e.clone()]);
    assert!(forbidden_dependency_precheck(&cx, &[e], &cfg())
        .findings()
        .is_empty());
}

#[test]
fn no_layers_reports_not_executed() {
    let cx = ctx(&["src/a/a.controller.ts"], Vec::new());
    let out = forbidden_dependency_precheck(&cx, &[], &BoundaryConfig::default());
    assert!(matches!(out, PrecheckOutcome::NotExecuted { .. }));
}

#[test]
fn new_cycle_detected() {
    let edges = vec![
        import("src/a.ts", "src/b.ts"),
        import("src/b.ts", "src/a.ts"),
    ];
    let cx = ctx(&["src/a.ts"], edges);
    let out = new_cycle_precheck(&cx, &[]);
    assert_eq!(out.findings().len(), 1);
    assert_eq!(out.findings()[0].rule_id, RULE_NEW_CYCLE);
    assert_eq!(out.findings()[0].members, vec!["src/a.ts", "src/b.ts"]);
}

#[test]
fn preexisting_cycle_not_reported() {
    let edges = vec![
        import("src/a.ts", "src/b.ts"),
        import("src/b.ts", "src/a.ts"),
    ];
    let cx = ctx(&["src/a.ts"], edges.clone());
    assert!(new_cycle_precheck(&cx, &edges).findings().is_empty());
}

#[test]
fn acyclic_graph_reports_nothing() {
    let cx = ctx(&["src/a.ts"], vec![import("src/a.ts", "src/b.ts")]);
    assert!(new_cycle_precheck(&cx, &[]).findings().is_empty());
}
