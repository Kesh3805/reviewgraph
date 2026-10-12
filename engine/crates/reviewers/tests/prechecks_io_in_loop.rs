//! REV-P-002: I/O-in-loop detector.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use reviewers::context::{ChangedSymbol, ContextSnapshot, Hunk, NeighborEdge, NeighborNode};
use reviewers::prechecks::io_in_loop::{io_in_loop_precheck, RULE_FAN_OUT, RULE_IO_IN_LOOP};

const SYM: &str = "ts:src/orders/orders.service#OrdersService.load/method";

fn ctx(body: &str, hunks: Vec<Hunk>) -> ContextSnapshot {
    ContextSnapshot {
        changed_symbols: vec![ChangedSymbol {
            symbol_id: SYM.into(),
            kind: "method".into(),
            path: "src/orders/orders.service.ts".into(),
            range: [10, 30],
            base_range: None,
            file_lines: Some(60),
            change_classes: vec!["call_added".into()],
            signature_base: None,
            signature_head: None,
            body_base: None,
            body_head: Some(body.into()),
            hunks,
            generated: false,
        }],
        ..ContextSnapshot::default()
    }
}

#[test]
fn await_repository_find_in_for_of_flagged() {
    let cx = ctx(
        "for (const id of ids) {\n  const u = await this.repo.findOne(id);\n}",
        Vec::new(),
    );
    let out = io_in_loop_precheck(&cx);
    assert_eq!(out.findings().len(), 1);
    assert_eq!(out.findings()[0].rule_id, RULE_IO_IN_LOOP);
}

#[test]
fn batched_find_in_not_flagged() {
    let cx = ctx(
        "const rows = await this.repo.find({ where: { id: In(ids) } });\nfor (const r of rows) {\n  total += r.amount;\n}",
        Vec::new(),
    );
    assert!(io_in_loop_precheck(&cx).findings().is_empty());
}

#[test]
fn literal_small_array_not_flagged() {
    let cx = ctx(
        "for (const k of ['a', 'b']) {\n  await this.repo.findOne(k);\n}",
        Vec::new(),
    );
    assert!(io_in_loop_precheck(&cx).findings().is_empty());
}

#[test]
fn io_two_hops_deep_flagged() {
    let mut cx = ctx(
        "for (const id of ids) {\n  await this.helper(id);\n}",
        Vec::new(),
    );
    cx.nodes = vec![
        NeighborNode {
            symbol_id: "ts:src/orders/h#H.helper/method".into(),
            kind: "method".into(),
            path: "src/orders/h.ts".into(),
            range: [1, 5],
            file_lines: None,
            distance: 1,
            excerpt: "return this.inner(id);".into(),
        },
        NeighborNode {
            symbol_id: "ts:src/orders/h#H.inner/method".into(),
            kind: "method".into(),
            path: "src/orders/h.ts".into(),
            range: [6, 9],
            file_lines: None,
            distance: 2,
            excerpt: "return await this.repo.findOne(id);".into(),
        },
    ];
    cx.edges = vec![NeighborEdge {
        from: "ts:src/orders/h#H.helper/method".into(),
        to: "ts:src/orders/h#H.inner/method".into(),
        kind: "CALLS".into(),
        confidence: 0.9,
    }];
    assert_eq!(io_in_loop_precheck(&cx).findings().len(), 1);
}

#[test]
fn unchanged_loop_not_flagged() {
    let cx = ctx(
        "for (const id of ids) {\n  await this.repo.findOne(id);\n}",
        vec![Hunk {
            old: [1, 2],
            new: [1, 2],
        }],
    );
    assert!(io_in_loop_precheck(&cx).findings().is_empty());
}

#[test]
fn promise_all_fan_out_is_a_lower_severity_hint() {
    let cx = ctx(
        "await Promise.all(ids.map(async (id) => {\n  return await this.repo.findOne(id);\n}));",
        Vec::new(),
    );
    let out = io_in_loop_precheck(&cx);
    assert_eq!(out.findings().len(), 1);
    assert_eq!(out.findings()[0].rule_id, RULE_FAN_OUT);
}
