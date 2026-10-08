//! IMP-005: test mapping.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::collections::BTreeSet;

use codegraph::{EdgeKind, NodeId, NodeKey, NodeKind, ResolvedBy};
use impact::graph::Relation;
use impact::input::{ChangeSet, MockFact, TestChangeInput};
use support::*;

fn one(file: &str, qualified: &str, kind: NodeKind) -> ChangeSet {
    ChangeSet {
        symbols: vec![changed(file, qualified, kind, body_change())],
        ..ChangeSet::default()
    }
}

fn file_key(file: &str) -> NodeKey {
    NodeId::file(&path(file)).key()
}

#[test]
fn auth_bypass_authorize_spec_maps_with_invocation_and_naming() {
    let scenario = auth_bypass();
    let graph = impact_of(&scenario.change, &scenario.head, Some(&scenario.base));
    let impact = graph.symbol(scenario.keys.authorize).unwrap();
    let tests: Vec<_> = impact.of(Relation::Test).collect();
    assert_eq!(tests.len(), 1, "{tests:?}");
    let test = tests[0];
    assert_eq!(test.node, scenario.keys.test_case);
    assert_eq!(test.node_id, TEST_CASE_ID);
    let mapping = test.test.as_ref().unwrap();
    assert_eq!(mapping.signals.invocation, 1.0);
    assert_eq!(mapping.signals.tests_edge, 1.0);
    assert_eq!(mapping.signals.naming, 0.6);
    assert_eq!(mapping.signals.import, 0.8);
    assert_eq!(mapping.signals.path, 0.4);
    assert_eq!(mapping.score, 1.0);
    assert!(!mapping.mocked);
    assert_eq!(test.path[0].edge, EdgeKind::Tests);
    assert!(!impact.untested);
    assert!(!graph.flags.test_mapping_degraded);
}

#[test]
fn e2e_spec_covers_controller_via_invocation() {
    let mut g = Fixture::new();
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
    let helper = g.symbol("test/users.e2e-spec.ts", "putUser", NodeKind::Function);
    g.edge(EdgeKind::Calls, helper, update, ResolvedBy::Import);
    let case = g.test_case("test/users.e2e-spec.ts", "users (e2e)", "updates a user");
    g.edge(EdgeKind::Tests, case, helper, ResolvedBy::Framework);
    let head = g.build();

    let graph = impact_of(
        &one(
            "src/users/user.controller.ts",
            "UserController.update",
            NodeKind::Handler,
        ),
        &head,
        None,
    );
    let impact = graph.symbol(update).unwrap();
    let test = impact.of(Relation::Test).next().expect("e2e case mapped");
    assert_eq!(test.node, case);
    assert_eq!(test.distance, 2);
    let mapping = test.test.as_ref().unwrap();
    assert_eq!(mapping.signals.invocation, 1.0);
    assert_eq!(mapping.signals.tests_edge, 0.0);
    assert!(mapping.score >= 0.8);
    assert!(!impact.untested);
}

#[test]
fn mock_only_test_marked_mocked_and_untested() {
    let scenario = auth_bypass();
    let permission_service = sym_key(PERMISSION_SERVICE, "PermissionService", NodeKind::Class);
    let mut change = one(
        PERMISSION_SERVICE,
        "PermissionService.check",
        NodeKind::Method,
    );
    change.mocks = vec![MockFact {
        test: scenario.keys.test_case,
        target: permission_service,
    }];
    let graph = impact_of(&change, &scenario.head, Some(&scenario.base));
    let impact = graph.symbol(scenario.keys.permission_check).unwrap();
    let test = impact
        .of(Relation::Test)
        .next()
        .expect("mocking test listed");
    let mapping = test.test.as_ref().unwrap();
    assert!(mapping.mocked);
    assert_eq!(mapping.signals.mock, 0.5);
    assert_eq!(mapping.signals.invocation, 0.0);
    assert!(impact.untested, "a mock-only test does not clear untested");
}

fn barrel_graph(via_barrel: bool) -> (codegraph::Graph, NodeKey, NodeKey) {
    let mut g = Fixture::new();
    g.file("src/lib/math.ts");
    g.file("src/lib/index.ts");
    g.file("test/calc.spec.ts");
    let add = g.symbol("src/lib/math.ts", "add", NodeKind::Function);
    let case = g.test_case("test/calc.spec.ts", "calc", "sums numbers");
    if via_barrel {
        g.edge(
            EdgeKind::Imports,
            file_key("src/lib/index.ts"),
            file_key("src/lib/math.ts"),
            ResolvedBy::Import,
        );
        g.edge(
            EdgeKind::Imports,
            file_key("test/calc.spec.ts"),
            file_key("src/lib/index.ts"),
            ResolvedBy::Import,
        );
    } else {
        g.edge(
            EdgeKind::Imports,
            file_key("test/calc.spec.ts"),
            file_key("src/lib/math.ts"),
            ResolvedBy::Import,
        );
    }
    (g.build(), add, case)
}

#[test]
fn barrel_import_lower_score() {
    let change = one("src/lib/math.ts", "add", NodeKind::Function);
    let (direct, add, case) = barrel_graph(false);
    let graph = impact_of(&change, &direct, None);
    let test = graph
        .symbol(add)
        .unwrap()
        .of(Relation::Test)
        .next()
        .unwrap();
    assert_eq!(test.node, case);
    assert_eq!(test.test.as_ref().unwrap().signals.import, 0.8);

    let (barrel, add, case) = barrel_graph(true);
    let graph = impact_of(&change, &barrel, None);
    let test = graph
        .symbol(add)
        .unwrap()
        .of(Relation::Test)
        .next()
        .unwrap();
    assert_eq!(test.node, case);
    let mapping = test.test.as_ref().unwrap();
    assert_eq!(mapping.signals.import, 0.6);
    assert_eq!(mapping.score, 0.6);
    assert!(graph.symbol(add).unwrap().untested);
}

#[test]
fn path_convention_only_below_threshold() {
    let mut g = Fixture::new();
    let render = g.symbol("src/a/widget.ts", "render", NodeKind::Function);
    g.test_case("src/a/other.spec.ts", "misc", "does things");
    let head = g.build();
    let graph = impact_of(
        &one("src/a/widget.ts", "render", NodeKind::Function),
        &head,
        None,
    );
    let impact = graph.symbol(render).unwrap();
    assert_eq!(impact.of(Relation::Test).count(), 0);
    assert!(impact.untested);
}

#[test]
fn untested_flag_when_no_tests() {
    let mut g = Fixture::new();
    let lonely = g.symbol("src/x/lonely.ts", "lonely", NodeKind::Function);
    let head = g.build();
    let graph = impact_of(
        &one("src/x/lonely.ts", "lonely", NodeKind::Function),
        &head,
        None,
    );
    assert!(graph.symbol(lonely).unwrap().untested);
}

#[test]
fn changed_test_maps_to_targets() {
    let scenario = auth_bypass();
    let mut change = scenario.change.clone();
    change.tests = vec![TestChangeInput {
        path: path(AUTH_SPEC),
        targets: Vec::new(),
    }];
    let graph = impact_of(&change, &scenario.head, Some(&scenario.base));
    assert_eq!(graph.test_targets.len(), 1);
    assert_eq!(graph.test_targets[0].test_path, AUTH_SPEC);
    assert_eq!(graph.test_targets[0].targets, vec![scenario.keys.authorize]);
}

#[test]
fn no_jest_adapter_degraded_mode() {
    let mut g = Fixture::new();
    g.file("src/auth/auth.service.ts");
    let spec_file = g.file("src/auth/auth.service.spec.ts");
    let class = g.symbol("src/auth/auth.service.ts", "AuthService", NodeKind::Class);
    let authorize = g.member(
        "src/auth/auth.service.ts",
        class,
        "AuthService.authorize",
        NodeKind::Method,
    );
    let head = g.build();
    let graph = impact_of(
        &one(
            "src/auth/auth.service.ts",
            "AuthService.authorize",
            NodeKind::Method,
        ),
        &head,
        None,
    );
    assert!(graph.flags.test_mapping_degraded);
    let impact = graph.symbol(authorize).unwrap();
    let test = impact
        .of(Relation::Test)
        .next()
        .expect("file-level mapping");
    assert_eq!(test.node, spec_file);
    let mapping = test.test.as_ref().unwrap();
    assert_eq!(mapping.signals.naming, 0.6);
    assert_eq!(mapping.signals.path, 0.4);
    assert_eq!(mapping.signals.invocation, 0.0);
    assert!(impact.untested, "file-level signals alone stay below 0.8");
}

/// A labelled mini repository (`test-mapping`): precision and recall of the included,
/// non-mocked mappings against the labels must both be ≥ 0.9.
#[test]
fn labelled_fixture_precision_and_recall() {
    let mut g = Fixture::new();
    for file in [
        "src/orders/order.service.ts",
        "src/orders/order.service.spec.ts",
        "src/billing/invoice.ts",
        "src/billing/__tests__/invoice.ts",
        "src/shipping/label.ts",
        "test/shipping/label.e2e-spec.ts",
        "src/util/slug.ts",
        "src/util/misc.spec.ts",
    ] {
        g.file(file);
    }
    let order_service = g.symbol(
        "src/orders/order.service.ts",
        "OrderService",
        NodeKind::Class,
    );
    let place = g.member(
        "src/orders/order.service.ts",
        order_service,
        "OrderService.place",
        NodeKind::Method,
    );
    let cancel = g.member(
        "src/orders/order.service.ts",
        order_service,
        "OrderService.cancel",
        NodeKind::Method,
    );
    let invoice = g.symbol("src/billing/invoice.ts", "buildInvoice", NodeKind::Function);
    let label = g.symbol("src/shipping/label.ts", "printLabel", NodeKind::Function);
    let slug = g.symbol("src/util/slug.ts", "slugify", NodeKind::Function);

    let place_case = g.test_case(
        "src/orders/order.service.spec.ts",
        "OrderService",
        "places an order",
    );
    g.edge(EdgeKind::Tests, place_case, place, ResolvedBy::Framework);
    let cancel_case = g.test_case(
        "src/orders/order.service.spec.ts",
        "OrderService",
        "cancels an order",
    );
    g.edge(EdgeKind::Tests, cancel_case, cancel, ResolvedBy::Framework);
    let invoice_case = g.test_case(
        "src/billing/__tests__/invoice.ts",
        "buildInvoice",
        "totals lines",
    );
    g.edge(
        EdgeKind::Imports,
        file_key("src/billing/__tests__/invoice.ts"),
        file_key("src/billing/invoice.ts"),
        ResolvedBy::Import,
    );
    let label_case = g.test_case(
        "test/shipping/label.e2e-spec.ts",
        "labels",
        "prints a label",
    );
    g.edge(EdgeKind::Tests, label_case, label, ResolvedBy::Framework);
    // A misc spec in util that never touches slugify: path convention only.
    g.test_case("src/util/misc.spec.ts", "misc", "formats dates");
    let head = g.build();

    let change = ChangeSet {
        symbols: vec![
            changed(
                "src/orders/order.service.ts",
                "OrderService.place",
                NodeKind::Method,
                body_change(),
            ),
            changed(
                "src/orders/order.service.ts",
                "OrderService.cancel",
                NodeKind::Method,
                body_change(),
            ),
            changed(
                "src/billing/invoice.ts",
                "buildInvoice",
                NodeKind::Function,
                body_change(),
            ),
            changed(
                "src/shipping/label.ts",
                "printLabel",
                NodeKind::Function,
                body_change(),
            ),
            changed(
                "src/util/slug.ts",
                "slugify",
                NodeKind::Function,
                body_change(),
            ),
        ],
        ..ChangeSet::default()
    };
    let graph = impact_of(&change, &head, None);

    let labels: BTreeSet<(NodeKey, NodeKey)> = [
        (place, place_case),
        (cancel, cancel_case),
        (invoice, invoice_case),
        (label, label_case),
    ]
    .into_iter()
    .collect();
    let mut predicted: BTreeSet<(NodeKey, NodeKey)> = BTreeSet::new();
    for symbol in &graph.symbols {
        for element in symbol.of(Relation::Test) {
            let mapping = element.test.as_ref().unwrap();
            if !mapping.mocked && mapping.score >= 0.8 {
                predicted.insert((symbol.seed, element.node));
            }
        }
    }
    let hits = predicted.intersection(&labels).count() as f64;
    let precision = if predicted.is_empty() {
        0.0
    } else {
        hits / predicted.len() as f64
    };
    let recall = hits / labels.len() as f64;
    assert!(precision >= 0.9, "precision {precision}: {predicted:?}");
    assert!(recall >= 0.9, "recall {recall}");
    assert!(graph.symbol(slug).unwrap().untested);
}
