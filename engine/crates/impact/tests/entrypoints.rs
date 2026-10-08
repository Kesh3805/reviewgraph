//! IMP-004: API entrypoint reachability.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use codegraph::{EdgeKind, NodeKey, NodeKind, ResolvedBy};
use impact::graph::{EntryKind, ImpactBudget, Relation, TruncReason};
use impact::input::ChangeSet;
use support::*;

fn one(file: &str, qualified: &str, kind: NodeKind) -> ChangeSet {
    ChangeSet {
        symbols: vec![changed(file, qualified, kind, body_change())],
        ..ChangeSet::default()
    }
}

#[test]
fn auth_bypass_reaches_put_users_id_at_distance_3() {
    let scenario = auth_bypass();
    let graph = impact_of(&scenario.change, &scenario.head, Some(&scenario.base));
    let impact = graph.symbol(scenario.keys.authorize).unwrap();
    let endpoints: Vec<_> = impact.of(Relation::Endpoint).collect();
    assert_eq!(endpoints.len(), 1);
    let endpoint = endpoints[0];
    assert_eq!(endpoint.node, scenario.keys.endpoint);
    assert_eq!(endpoint.node_id, "http:PUT /users/{}");
    assert_eq!(endpoint.distance, 3);
    let attrs = endpoint.endpoint.as_ref().unwrap();
    assert_eq!(attrs.entry_kind, EntryKind::Http);
    assert_eq!(attrs.method.as_deref(), Some("PUT"));
    assert_eq!(attrs.path.as_deref(), Some("/users/:id"));
    assert!(attrs.guards.is_empty());
    // authorize ← updateUser ← UserController.update ← HANDLED_BY
    let hops: Vec<(EdgeKind, NodeKey, NodeKey)> = endpoint
        .path
        .iter()
        .map(|s| (s.edge, s.from, s.to))
        .collect();
    assert_eq!(
        hops,
        vec![
            (
                EdgeKind::Calls,
                scenario.keys.update_user,
                scenario.keys.authorize
            ),
            (
                EdgeKind::Calls,
                scenario.keys.controller_update,
                scenario.keys.update_user
            ),
            (
                EdgeKind::HandledBy,
                scenario.keys.endpoint,
                scenario.keys.controller_update
            ),
        ]
    );
    assert!(graph.stats.endpoint_search_visits >= 3);
}

#[test]
fn queue_consumer_entrypoint_found() {
    let mut g = Fixture::new();
    let seed = g.symbol("src/reports/render.ts", "render", NodeKind::Function);
    let processor = g.symbol(
        "src/reports/report.processor.ts",
        "ReportProcessor",
        NodeKind::QueueConsumer,
    );
    let handle = g.member(
        "src/reports/report.processor.ts",
        processor,
        "ReportProcessor.handle",
        NodeKind::JobHandler,
    );
    g.edge(EdgeKind::Calls, handle, seed, ResolvedBy::Import);
    let queue = g.queue("reports");
    g.edge(EdgeKind::ConsumesJob, handle, queue, ResolvedBy::Framework);
    let head = g.build();

    let graph = impact_of(
        &one("src/reports/render.ts", "render", NodeKind::Function),
        &head,
        None,
    );
    let endpoint = graph
        .symbol(seed)
        .unwrap()
        .of(Relation::Endpoint)
        .next()
        .expect("queue entrypoint");
    assert_eq!(endpoint.node, queue);
    assert_eq!(endpoint.node_id, "queue:reports");
    assert_eq!(endpoint.distance, 2);
    let attrs = endpoint.endpoint.as_ref().unwrap();
    assert_eq!(attrs.entry_kind, EntryKind::Queue);
    assert_eq!(attrs.path.as_deref(), Some("reports"));
}

#[test]
fn unreachable_internal_symbol_has_no_endpoint() {
    let scenario = auth_bypass();
    let change = one(
        PERMISSION_SERVICE,
        "PermissionService.check",
        NodeKind::Method,
    );
    let graph = impact_of(&change, &scenario.head, Some(&scenario.base));
    // On head nothing reaches PermissionService.check from an endpoint any more (only the
    // report decoy calls it, and it has no route).
    let impact = graph.symbol(scenario.keys.permission_check).unwrap();
    assert_eq!(impact.of(Relation::Endpoint).count(), 0);
    assert!(impact
        .truncation
        .iter()
        .all(|t| t.relation != Some(Relation::Endpoint)));
}

#[test]
fn visit_cap_truncates() {
    let mut g = Fixture::new();
    let keys: Vec<NodeKey> = (0..10)
        .map(|i| g.symbol("src/chain.ts", &format!("f{i}"), NodeKind::Function))
        .collect();
    for window in keys.windows(2) {
        g.edge(EdgeKind::Calls, window[1], window[0], ResolvedBy::Import);
    }
    let head = g.build();
    let budget = ImpactBudget {
        max_endpoint_visits: 3,
        ..ImpactBudget::default()
    };
    let graph = impact_with(
        &one("src/chain.ts", "f0", NodeKind::Function),
        &head,
        None,
        &budget,
    );
    let truncation = graph
        .symbol(keys[0])
        .unwrap()
        .truncation
        .iter()
        .find(|t| t.relation == Some(Relation::Endpoint))
        .cloned()
        .unwrap();
    assert_eq!(truncation.reason, TruncReason::Depth);
    assert_eq!(truncation.visited, Some(3));
    assert_eq!(truncation.limit, 3);
    assert_eq!(graph.stats.endpoint_search_visits, 3);
}

#[test]
fn best_path_reported_when_two_routes() {
    let mut g = Fixture::new();
    let seed = g.symbol("src/core.ts", "core", NodeKind::Function);
    let strong = g.symbol("src/a.ts", "viaStrong", NodeKind::Function);
    let weak = g.symbol("src/b.ts", "viaWeak", NodeKind::Function);
    let controller = g.symbol("src/c.controller.ts", "C", NodeKind::Controller);
    let handler = g.member(
        "src/c.controller.ts",
        controller,
        "C.get",
        NodeKind::Handler,
    );
    g.edge(EdgeKind::Calls, strong, seed, ResolvedBy::Import);
    g.edge(EdgeKind::Calls, weak, seed, ResolvedBy::NameUnique);
    g.edge(EdgeKind::Calls, handler, strong, ResolvedBy::Import);
    g.edge(EdgeKind::Calls, handler, weak, ResolvedBy::Import);
    let endpoint = g.endpoint("GET", "/c");
    g.edge(
        EdgeKind::HandledBy,
        endpoint,
        handler,
        ResolvedBy::Framework,
    );
    let head = g.build();

    let graph = impact_of(&one("src/core.ts", "core", NodeKind::Function), &head, None);
    let element = graph
        .symbol(seed)
        .unwrap()
        .of(Relation::Endpoint)
        .next()
        .unwrap();
    assert_eq!(element.node, endpoint);
    assert_eq!(element.path[0].from, strong);
    assert_eq!(element.min_confidence, conf(0.9));
    assert_eq!(element.distance, 3);
}

#[test]
fn guards_recorded_on_endpoint() {
    let mut g = Fixture::new();
    let controller = g.symbol(
        "src/users/user.controller.ts",
        "UserController",
        NodeKind::Controller,
    );
    let handler = g.member(
        "src/users/user.controller.ts",
        controller,
        "UserController.remove",
        NodeKind::Handler,
    );
    let endpoint = g.endpoint("DELETE", "/users/:id");
    g.edge(
        EdgeKind::HandledBy,
        endpoint,
        handler,
        ResolvedBy::Framework,
    );
    let roles = g.symbol(
        "src/auth/roles.guard.ts",
        "RolesGuard",
        NodeKind::Middleware,
    );
    let jwt = g.symbol(
        "src/auth/jwt.guard.ts",
        "JwtAuthGuard",
        NodeKind::Middleware,
    );
    g.edge(EdgeKind::Authorizes, roles, endpoint, ResolvedBy::Framework);
    g.edge(EdgeKind::Authorizes, jwt, endpoint, ResolvedBy::Framework);
    let head = g.build();

    let graph = impact_of(
        &one(
            "src/users/user.controller.ts",
            "UserController.remove",
            NodeKind::Handler,
        ),
        &head,
        None,
    );
    let element = graph
        .symbol(handler)
        .unwrap()
        .of(Relation::Endpoint)
        .next()
        .unwrap();
    assert_eq!(element.distance, 1);
    let attrs = element.endpoint.as_ref().unwrap();
    assert_eq!(
        attrs.guards,
        vec!["JwtAuthGuard".to_owned(), "RolesGuard".to_owned()]
    );
}
