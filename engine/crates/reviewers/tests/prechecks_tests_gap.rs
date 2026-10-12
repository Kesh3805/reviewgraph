//! REV-T-002: tests-gap pre-check.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use reviewers::context::{ChangedSymbol, ContextSnapshot, NeighborEdge, TestItem};
use reviewers::prechecks::tests_gap::{tests_precheck, RULE_ID};
use reviewers::prechecks::{PrecheckOutcome, SeverityHint};

fn symbol(id: &str, path: &str, sig: &str, body: &str) -> ChangedSymbol {
    ChangedSymbol {
        symbol_id: id.into(),
        kind: "method".into(),
        path: path.into(),
        range: [10, 20],
        base_range: None,
        file_lines: Some(50),
        change_classes: vec!["condition_changed".into()],
        signature_base: None,
        signature_head: Some(sig.into()),
        body_base: None,
        body_head: Some(body.into()),
        hunks: Vec::new(),
        generated: false,
    }
}

const PUBLIC: &str = "ts:src/orders/orders.service#OrdersService.cancel/method";
const WRAPPER: &str = "ts:src/orders/orders.facade#OrdersFacade.cancelAll/method";

fn public_ctx() -> ContextSnapshot {
    ContextSnapshot {
        changed_symbols: vec![symbol(
            PUBLIC,
            "src/orders/orders.service.ts",
            "cancel(id: string): Promise<void>",
            "async cancel(id: string) {}",
        )],
        ..ContextSnapshot::default()
    }
}

#[test]
fn untested_exported_method_flagged() {
    let out = tests_precheck(&public_ctx(), true);
    let f = out.findings();
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].rule_id, RULE_ID);
    assert_eq!(f[0].severity_hint, SeverityHint::Low);
    assert!(f[0].absence.is_some());
}

#[test]
fn tested_via_wrapper_not_flagged() {
    let mut cx = public_ctx();
    cx.edges = vec![
        NeighborEdge {
            from: WRAPPER.into(),
            to: PUBLIC.into(),
            kind: "CALLS".into(),
            confidence: 0.9,
        },
        NeighborEdge {
            from: "test:orders.facade.spec#facade".into(),
            to: WRAPPER.into(),
            kind: "TESTS".into(),
            confidence: 0.8,
        },
    ];
    assert!(tests_precheck(&cx, true).findings().is_empty());
}

#[test]
fn directly_mapped_test_not_flagged() {
    let mut cx = public_ctx();
    cx.tests = vec![TestItem {
        test_id: "test:orders.spec#cancel".into(),
        path: "src/orders/orders.service.spec.ts".into(),
        covers: vec![PUBLIC.into()],
    }];
    assert!(tests_precheck(&cx, true).findings().is_empty());
}

#[test]
fn pr_adding_test_suppresses_flag() {
    let mut cx = public_ctx();
    cx.changed_symbols.push(symbol(
        "ts:src/orders/orders.service.spec#cancel/function",
        "src/orders/orders.service.spec.ts",
        "it('cancel')",
        "it('cancel', () => service.cancel('1'))",
    ));
    assert!(tests_precheck(&cx, true).findings().is_empty());
}

#[test]
fn private_helper_not_flagged() {
    let mut cx = public_ctx();
    cx.changed_symbols[0].signature_head = Some("private cancel(id: string): void".into());
    assert!(tests_precheck(&cx, true).findings().is_empty());
}

#[test]
fn no_test_framework_reports_not_executed() {
    let out = tests_precheck(&public_ctx(), false);
    assert!(matches!(out, PrecheckOutcome::NotExecuted { .. }));
}
