//! IMP-003: type hierarchy relations.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use codegraph::{EdgeKind, NodeKind, ResolvedBy};
use impact::graph::{ImpactBudget, Relation, TruncReason};
use impact::input::ChangeSet;
use review_core::change::SymbolChange;
use support::*;

fn one(symbol: impact::input::SymbolInput) -> ChangeSet {
    ChangeSet {
        symbols: vec![symbol],
        ..ChangeSet::default()
    }
}

#[test]
fn auth_bypass_implements_authprovider_authorize() {
    let scenario = auth_bypass();
    let graph = impact_of(&scenario.change, &scenario.head, Some(&scenario.base));
    let impact = graph.symbol(scenario.keys.authorize).unwrap();
    let interfaces: Vec<_> = impact.of(Relation::Interface).collect();
    assert_eq!(interfaces.len(), 1);
    assert_eq!(interfaces[0].node, scenario.keys.provider_authorize);
    assert_eq!(interfaces[0].distance, 1);
    assert!(interfaces[0].min_confidence >= conf(0.9));
    assert_eq!(
        interfaces[0].node_id,
        "ts:src/auth/auth-provider.interface#AuthProvider.authorize/method"
    );
}

/// `interface Store { save }`, `class A implements Store { save }`, `class B implements Store
/// { save }` with only class-level `IMPLEMENTS` edges (member hops are synthesized).
fn two_implementations() -> (codegraph::Graph, [codegraph::NodeKey; 3]) {
    let mut g = Fixture::new();
    let store = g.symbol("src/store.ts", "Store", NodeKind::Interface);
    let store_save = g.member("src/store.ts", store, "Store.save", NodeKind::Method);
    let a = g.symbol("src/a.ts", "A", NodeKind::Class);
    let a_save = g.member("src/a.ts", a, "A.save", NodeKind::Method);
    let b = g.symbol("src/b.ts", "B", NodeKind::Class);
    let b_save = g.member("src/b.ts", b, "B.save", NodeKind::Method);
    g.edge(EdgeKind::Implements, a, store, ResolvedBy::Import);
    g.edge(EdgeKind::Implements, b, store, ResolvedBy::Import);
    (g.build(), [store_save, a_save, b_save])
}

#[test]
fn sibling_implementation_at_distance_2() {
    let (head, [store_save, a_save, b_save]) = two_implementations();
    let change = one(changed(
        "src/a.ts",
        "A.save",
        NodeKind::Method,
        body_change(),
    ));
    let graph = impact_of(&change, &head, None);
    let impact = graph.symbol(a_save).unwrap();

    let interface = impact.of(Relation::Interface).next().unwrap();
    assert_eq!(interface.node, store_save);
    assert_eq!(interface.path[0].edge, EdgeKind::Implements);
    // Synthesized from the class edge: 0.95 × 0.95.
    assert!(interface.min_confidence < conf(0.95));
    assert!(interface.min_confidence >= conf(0.9));

    let sibling = impact.of(Relation::Implementation).next().unwrap();
    assert_eq!(sibling.node, b_save);
    assert_eq!(sibling.distance, 2);
    assert_eq!(sibling.path.len(), 2);
    assert_eq!(sibling.path[0].to, store_save);
    assert_eq!(sibling.path[1].from, b_save);
    assert_eq!(sibling.path[1].to, store_save);
}

#[test]
fn override_chain() {
    let mut g = Fixture::new();
    let parent = g.symbol("src/p.ts", "Parent", NodeKind::Class);
    let parent_m = g.member("src/p.ts", parent, "Parent.render", NodeKind::Method);
    let child = g.symbol("src/c.ts", "Child", NodeKind::Class);
    let child_m = g.member("src/c.ts", child, "Child.render", NodeKind::Method);
    let grand = g.symbol("src/g.ts", "Grand", NodeKind::Class);
    let grand_m = g.member("src/g.ts", grand, "Grand.render", NodeKind::Method);
    g.edge(EdgeKind::Extends, child, parent, ResolvedBy::Import);
    g.edge(EdgeKind::Extends, grand, child, ResolvedBy::Import);
    g.edge(
        EdgeKind::Overrides,
        child_m,
        parent_m,
        ResolvedBy::Structural,
    );
    g.edge(
        EdgeKind::Overrides,
        grand_m,
        child_m,
        ResolvedBy::Structural,
    );
    let head = g.build();

    let change = one(changed(
        "src/c.ts",
        "Child.render",
        NodeKind::Method,
        body_change(),
    ));
    let graph = impact_of(&change, &head, None);
    let impact = graph.symbol(child_m).unwrap();
    let overrides: Vec<_> = impact.of(Relation::Override).map(|e| e.node).collect();
    assert_eq!(overrides, vec![parent_m]);
    let overridden: Vec<_> = impact.of(Relation::OverriddenBy).map(|e| e.node).collect();
    assert_eq!(overridden, vec![grand_m]);
    assert_eq!(impact.of(Relation::Interface).count(), 0);
}

#[test]
fn interface_signature_change_pulls_all_impls() {
    let (head, [store_save, a_save, b_save]) = two_implementations();
    let change = one(changed(
        "src/store.ts",
        "Store.save",
        NodeKind::Method,
        SymbolChange::modified(true, false, false).unwrap(),
    ));
    let graph = impact_of(&change, &head, None);
    let impact = graph.symbol(store_save).unwrap();
    let implementations: Vec<_> = impact
        .of(Relation::Implementation)
        .map(|e| (e.node, e.distance))
        .collect();
    assert_eq!(implementations.len(), 2);
    assert!(implementations.contains(&(a_save, 1)));
    assert!(implementations.contains(&(b_save, 1)));
}

fn dto_with_handlers(
    handlers: usize,
) -> (
    codegraph::Graph,
    codegraph::NodeKey,
    Vec<codegraph::NodeKey>,
) {
    let mut g = Fixture::new();
    let dto = g.symbol(
        "src/users/create-user.dto.ts",
        "CreateUserDto",
        NodeKind::Class,
    );
    let controller = g.symbol(
        "src/users/user.controller.ts",
        "UserController",
        NodeKind::Controller,
    );
    let keys = (0..handlers)
        .map(|i| {
            let handler = g.member(
                "src/users/user.controller.ts",
                controller,
                &format!("UserController.create{i}"),
                NodeKind::Handler,
            );
            g.edge(
                EdgeKind::AcceptsType,
                handler,
                dto,
                ResolvedBy::TypeAnnotation,
            );
            handler
        })
        .collect();
    (g.build(), dto, keys)
}

#[test]
fn dto_change_pulls_handlers_as_related_type() {
    let (head, dto, handlers) = dto_with_handlers(2);
    let change = one(changed(
        "src/users/create-user.dto.ts",
        "CreateUserDto",
        NodeKind::Class,
        body_change(),
    ));
    let graph = impact_of(&change, &head, None);
    let related: Vec<_> = graph
        .symbol(dto)
        .unwrap()
        .of(Relation::RelatedType)
        .map(|e| e.node)
        .collect();
    assert_eq!(related.len(), 2);
    for handler in handlers {
        assert!(related.contains(&handler));
    }
}

#[test]
fn type_relations_capped_and_reported() {
    let (head, dto, _) = dto_with_handlers(8);
    let change = one(changed(
        "src/users/create-user.dto.ts",
        "CreateUserDto",
        NodeKind::Class,
        body_change(),
    ));
    let budget = ImpactBudget {
        max_type_relations: 3,
        ..ImpactBudget::default()
    };
    let graph = impact_with(&change, &head, None, &budget);
    let impact = graph.symbol(dto).unwrap();
    assert_eq!(impact.of(Relation::RelatedType).count(), 3);
    let truncation = impact
        .truncation
        .iter()
        .find(|t| t.relation == Some(Relation::RelatedType))
        .unwrap();
    assert_eq!(truncation.reason, TruncReason::Limit);
    assert_eq!(truncation.limit, 3);
    assert_eq!(truncation.dropped, 5);
}
