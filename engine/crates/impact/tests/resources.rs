//! IMP-006: config, DB, queue and external API relations.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use codegraph::{EdgeKind, NodeKind, ResolvedBy};
use impact::graph::{GraphSide, ImpactBudget, Relation, ResourceRole, TruncReason};
use impact::input::ChangeSet;
use support::*;

fn one(file: &str, qualified: &str, kind: NodeKind) -> ChangeSet {
    ChangeSet {
        symbols: vec![changed(file, qualified, kind, body_change())],
        ..ChangeSet::default()
    }
}

#[test]
fn produced_job_pulls_consumer_at_distance_2() {
    let mut g = Fixture::new();
    let producer = g.symbol(
        "src/orders/order.service.ts",
        "placeOrder",
        NodeKind::Function,
    );
    let queue = g.queue("emails");
    g.edge(
        EdgeKind::ProducesJob,
        producer,
        queue,
        ResolvedBy::Framework,
    );
    let consumer = g.symbol(
        "src/mail/mail.processor.ts",
        "sendMail",
        NodeKind::JobHandler,
    );
    g.edge(
        EdgeKind::ConsumesJob,
        consumer,
        queue,
        ResolvedBy::Framework,
    );
    let head = g.build();

    let graph = impact_of(
        &one(
            "src/orders/order.service.ts",
            "placeOrder",
            NodeKind::Function,
        ),
        &head,
        None,
    );
    let impact = graph.symbol(producer).unwrap();
    let produced = impact.of(Relation::QueueProducer).next().unwrap();
    assert_eq!(produced.node, queue);
    assert_eq!(produced.distance, 1);
    assert_eq!(produced.resource.unwrap().role, ResourceRole::Produces);
    let consumed = impact.of(Relation::QueueConsumer).next().unwrap();
    assert_eq!(consumed.node, consumer);
    assert_eq!(consumed.distance, 2);
    assert_eq!(consumed.resource.unwrap().role, ResourceRole::Consumer);
    assert!(graph.flags.resource_facts_available);
}

#[test]
fn table_write_lists_co_writers_capped() {
    let mut g = Fixture::new();
    let table = g.table("orders");
    let seed = g.symbol("src/orders/a.ts", "writeA", NodeKind::Function);
    g.edge(EdgeKind::WritesTable, seed, table, ResolvedBy::Framework);
    for i in 0..7 {
        let writer = g.symbol(
            "src/orders/others.ts",
            &format!("write{i}"),
            NodeKind::Function,
        );
        g.edge(EdgeKind::WritesTable, writer, table, ResolvedBy::Framework);
    }
    let head = g.build();
    let graph = impact_of(
        &one("src/orders/a.ts", "writeA", NodeKind::Function),
        &head,
        None,
    );
    let impact = graph.symbol(seed).unwrap();
    let tables: Vec<_> = impact.of(Relation::DbTable).collect();
    let own: Vec<_> = tables.iter().filter(|e| e.distance == 1).collect();
    assert_eq!(own.len(), 1);
    assert_eq!(own[0].node_id, "db:public.orders");
    assert_eq!(own[0].resource.unwrap().role, ResourceRole::Writes);
    let co_writers: Vec<_> = tables
        .iter()
        .filter(|e| e.resource.unwrap().role == ResourceRole::CoWriter)
        .collect();
    assert_eq!(co_writers.len(), 5);
    assert!(co_writers.iter().all(|e| e.distance == 2));
    let truncation = impact
        .truncation
        .iter()
        .find(|t| t.relation == Some(Relation::DbTable))
        .unwrap();
    assert_eq!(truncation.reason, TruncReason::Limit);
    assert_eq!(
        truncation.limit,
        ImpactBudget::default().max_resource_other_side
    );
    assert_eq!(truncation.dropped, 2);
}

#[test]
fn env_var_read_added_lists_other_readers() {
    let mut g = Fixture::new();
    let env = g.env("JWT_SECRET");
    let seed = g.symbol("src/auth/jwt.ts", "signToken", NodeKind::Function);
    let other = g.symbol("src/auth/verify.ts", "verifyToken", NodeKind::Function);
    g.edge(EdgeKind::ReadsConfig, seed, env, ResolvedBy::Framework);
    g.edge(EdgeKind::ReadsConfig, other, env, ResolvedBy::Framework);
    let head = g.build();
    let graph = impact_of(
        &one("src/auth/jwt.ts", "signToken", NodeKind::Function),
        &head,
        None,
    );
    let impact = graph.symbol(seed).unwrap();
    let vars: Vec<_> = impact
        .of(Relation::EnvVar)
        .map(|e| (e.node, e.distance))
        .collect();
    assert_eq!(vars, vec![(env, 1), (other, 2)]);
    let reader = impact
        .of(Relation::EnvVar)
        .find(|e| e.node == other)
        .unwrap();
    assert_eq!(reader.resource.unwrap().role, ResourceRole::CoReader);
}

#[test]
fn removed_write_reported_from_base() {
    let build = |with_write: bool| {
        let mut g = Fixture::new();
        let table = g.table("audit_log");
        let seed = g.symbol("src/audit/audit.ts", "record", NodeKind::Function);
        if with_write {
            g.edge(EdgeKind::WritesTable, seed, table, ResolvedBy::Framework);
        }
        (g.build(), seed, table)
    };
    let (base, _, _) = build(true);
    let (head, seed, table) = build(false);
    let graph = impact_of(
        &one("src/audit/audit.ts", "record", NodeKind::Function),
        &head,
        Some(&base),
    );
    let element = graph
        .symbol(seed)
        .unwrap()
        .of(Relation::DbTable)
        .next()
        .unwrap();
    assert_eq!(element.node, table);
    let attrs = element.resource.unwrap();
    assert!(attrs.removed);
    assert_eq!(attrs.role, ResourceRole::Writes);
    assert_eq!(element.path[0].graph, GraphSide::Base);
}

#[test]
fn auth_bypass_admin_updateuser_writes_users_table_not_seed_relation() {
    let scenario = auth_bypass();
    let graph = impact_of(&scenario.change, &scenario.head, Some(&scenario.base));
    let impact = graph.symbol(scenario.keys.authorize).unwrap();
    assert_eq!(impact.of(Relation::DbTable).count(), 0);
    assert!(impact
        .elements
        .iter()
        .all(|e| e.node != scenario.keys.users_table));
    let caller = impact
        .of(Relation::Caller)
        .find(|e| e.node == scenario.keys.update_user)
        .unwrap();
    assert_eq!(caller.touches, vec!["db:public.users".to_owned()]);
}

#[test]
fn injected_entity_of_container_is_db_entity() {
    let mut g = Fixture::new();
    let table = g.table("users");
    let entity = g.symbol("src/users/user.entity.ts", "User", NodeKind::DatabaseEntity);
    g.edge(EdgeKind::References, entity, table, ResolvedBy::Framework);
    let service = g.symbol("src/users/user.service.ts", "UserService", NodeKind::Class);
    let ctor = g.member(
        "src/users/user.service.ts",
        service,
        "UserService.constructor",
        NodeKind::Constructor,
    );
    g.edge(
        EdgeKind::AcceptsType,
        ctor,
        entity,
        ResolvedBy::TypeAnnotation,
    );
    let rename = g.member(
        "src/users/user.service.ts",
        service,
        "UserService.rename",
        NodeKind::Method,
    );
    let head = g.build();
    let graph = impact_of(
        &one(
            "src/users/user.service.ts",
            "UserService.rename",
            NodeKind::Method,
        ),
        &head,
        None,
    );
    let element = graph
        .symbol(rename)
        .unwrap()
        .of(Relation::DbEntity)
        .next()
        .unwrap();
    assert_eq!(element.node, entity);
    assert_eq!(element.distance, 1);
    assert_eq!(element.resource.unwrap().role, ResourceRole::Injected);
}
