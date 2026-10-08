//! IMP-009: change clustering.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::collections::BTreeSet;

use codegraph::{EdgeFlags, EdgeKind, Graph, GraphQuery, NodeKey, NodeKind, ResolvedBy};
use impact::cluster::{
    cluster_changes, ClusterInputs, ClusterKind, ClusterReason, Clustering, MAX_CLUSTER_SIZE,
};
use impact::input::{ApiInput, ChangeSet, SymbolInput};
use review_core::change::ChangeClusterKey;
use review_core::location::RepoPath;
use support::*;

fn cluster(change: &ChangeSet, head: &Graph, low_files: &BTreeSet<RepoPath>) -> Clustering {
    let impact = impact_of(change, head, None);
    let none = BTreeSet::new();
    cluster_changes(&ClusterInputs {
        change,
        head: Some(head as &dyn GraphQuery),
        impact: Some(&impact),
        low_risk_symbols: &none,
        low_risk_files: low_files,
    })
}

fn change_clusters(clustering: &Clustering) -> Vec<&impact::cluster::ChangeCluster> {
    clustering
        .clusters
        .iter()
        .filter(|c| c.kind == ClusterKind::Change)
        .collect()
}

fn changes(symbols: Vec<SymbolInput>) -> ChangeSet {
    ChangeSet {
        symbols,
        ..ChangeSet::default()
    }
}

#[test]
fn auth_bypass_single_cluster_one_member() {
    let scenario = auth_bypass();
    let impact = impact_of(&scenario.change, &scenario.head, Some(&scenario.base));
    let none = BTreeSet::new();
    let low_files: BTreeSet<RepoPath> = [path(FORMAT)].into_iter().collect();
    let clustering = cluster_changes(&ClusterInputs {
        change: &scenario.change,
        head: Some(&scenario.head as &dyn GraphQuery),
        impact: Some(&impact),
        low_risk_symbols: &none,
        low_risk_files: &low_files,
    });
    let changed = change_clusters(&clustering);
    assert_eq!(changed.len(), 1);
    assert_eq!(changed[0].members, vec![scenario.keys.authorize]);
    assert_eq!(changed[0].reason, vec![ClusterReason::Singleton]);
    assert_eq!(changed[0].files, vec![path(AUTH_SERVICE)]);
    assert_eq!(
        changed[0].id,
        ChangeClusterKey::of(&[scenario.keys.authorize])
    );
    let low = clustering
        .clusters
        .iter()
        .find(|c| c.kind == ClusterKind::LowRisk)
        .unwrap();
    assert_eq!(low.files, vec![path(FORMAT)]);
    assert!(low.members.is_empty());
    assert_eq!(clustering.clusters.len(), 2);
}

#[test]
fn caller_and_callee_both_changed_same_cluster() {
    let mut g = Fixture::new();
    let caller = g.symbol("src/a/caller.ts", "caller", NodeKind::Function);
    let callee = g.symbol("src/b/callee.ts", "callee", NodeKind::Function);
    g.edge(EdgeKind::Calls, caller, callee, ResolvedBy::Import);
    let head = g.build();
    let change = changes(vec![
        changed(
            "src/a/caller.ts",
            "caller",
            NodeKind::Function,
            body_change(),
        ),
        changed(
            "src/b/callee.ts",
            "callee",
            NodeKind::Function,
            body_change(),
        ),
    ]);
    let clustering = cluster(&change, &head, &BTreeSet::new());
    let changed = change_clusters(&clustering);
    assert_eq!(changed.len(), 1);
    assert_eq!(changed[0].members.len(), 2);
    assert!(changed[0].reason.contains(&ClusterReason::CallEdge));
}

#[test]
fn same_module_unconnected_stay_separate() {
    let mut g = Fixture::new();
    g.symbol("src/users/a.ts", "alpha", NodeKind::Function);
    g.symbol("src/users/b.ts", "beta", NodeKind::Function);
    let head = g.build();
    let change = changes(vec![
        changed("src/users/a.ts", "alpha", NodeKind::Function, body_change()),
        changed("src/users/b.ts", "beta", NodeKind::Function, body_change()),
    ]);
    let clustering = cluster(&change, &head, &BTreeSet::new());
    let changed = change_clusters(&clustering);
    assert_eq!(changed.len(), 2);
    assert!(changed
        .iter()
        .all(|c| c.reason == vec![ClusterReason::Singleton]));
    assert!(changed
        .iter()
        .all(|c| c.modules == vec!["src/users".to_owned()]));
}

#[test]
fn same_module_connected_through_impact_elements_merge() {
    let mut g = Fixture::new();
    let a = g.symbol("src/users/a.ts", "alpha", NodeKind::Function);
    let b = g.symbol("src/users/b.ts", "beta", NodeKind::Function);
    let shared = g.symbol("src/users/c.ts", "shared", NodeKind::Function);
    g.edge(EdgeKind::Calls, shared, a, ResolvedBy::Import);
    g.edge(EdgeKind::Calls, shared, b, ResolvedBy::Import);
    let head = g.build();
    let change = changes(vec![
        changed("src/users/a.ts", "alpha", NodeKind::Function, body_change()),
        changed("src/users/b.ts", "beta", NodeKind::Function, body_change()),
    ]);
    let clustering = cluster(&change, &head, &BTreeSet::new());
    let changed = change_clusters(&clustering);
    assert_eq!(changed.len(), 1);
    assert_eq!(changed[0].members, {
        let mut m = vec![a, b];
        m.sort();
        m
    });
    assert_eq!(changed[0].reason, vec![ClusterReason::SameModule]);
}

#[test]
fn dto_handler_guard_grouped_by_api() {
    let mut g = Fixture::new();
    let controller = g.symbol(
        "src/users/user.controller.ts",
        "UserController",
        NodeKind::Controller,
    );
    let handler = g.member(
        "src/users/user.controller.ts",
        controller,
        "UserController.create",
        NodeKind::Handler,
    );
    let dto = g.symbol(
        "src/dto/create-user.dto.ts",
        "CreateUserDto",
        NodeKind::Class,
    );
    let guard = g.symbol(
        "src/guards/admin.guard.ts",
        "AdminGuard",
        NodeKind::Middleware,
    );
    let head = g.build();
    let mut change = changes(vec![
        changed(
            "src/users/user.controller.ts",
            "UserController.create",
            NodeKind::Handler,
            body_change(),
        ),
        changed(
            "src/dto/create-user.dto.ts",
            "CreateUserDto",
            NodeKind::Class,
            body_change(),
        ),
        changed(
            "src/guards/admin.guard.ts",
            "AdminGuard",
            NodeKind::Middleware,
            body_change(),
        ),
    ]);
    change.apis = vec![ApiInput {
        endpoint: "http:POST /users".to_owned(),
        handler: Some(handler),
        related: vec![dto, guard],
        breaking: false,
        auth_changed: true,
    }];
    let clustering = cluster(&change, &head, &BTreeSet::new());
    let changed = change_clusters(&clustering);
    assert_eq!(changed.len(), 1);
    assert_eq!(changed[0].members.len(), 3);
    assert_eq!(changed[0].apis, vec!["http:POST /users".to_owned()]);
    assert!(changed[0].reason.contains(&ClusterReason::SameApi));
}

#[test]
fn entity_and_writer_grouped() {
    let mut g = Fixture::new();
    let table = g.table("users");
    let entity = g.symbol("src/users/user.entity.ts", "User", NodeKind::DatabaseEntity);
    g.edge_flags(
        EdgeKind::References,
        entity,
        table,
        ResolvedBy::Framework,
        EdgeFlags::MAPS_TABLE,
    );
    let writer = g.symbol("src/admin/purge.ts", "purgeUsers", NodeKind::Function);
    g.edge(EdgeKind::WritesTable, writer, table, ResolvedBy::Framework);
    let head = g.build();
    let change = changes(vec![
        changed(
            "src/users/user.entity.ts",
            "User",
            NodeKind::DatabaseEntity,
            body_change(),
        ),
        changed(
            "src/admin/purge.ts",
            "purgeUsers",
            NodeKind::Function,
            body_change(),
        ),
    ]);
    let clustering = cluster(&change, &head, &BTreeSet::new());
    let changed = change_clusters(&clustering);
    assert_eq!(changed.len(), 1);
    assert!(changed[0].reason.contains(&ClusterReason::SameEntity));
    assert_eq!(changed[0].entities, vec!["db:public.users".to_owned()]);
    assert!(changed[0].members.contains(&entity) && changed[0].members.contains(&writer));
}

#[test]
fn test_joins_target_cluster() {
    let mut g = Fixture::new();
    g.file("src/pricing/price.ts");
    let price = g.symbol("src/pricing/price.ts", "price", NodeKind::Function);
    let helper = g.symbol("src/pricing/price.spec.ts", "makeCart", NodeKind::Function);
    let case = g.test_case("src/pricing/price.spec.ts", "price", "applies discounts");
    g.edge(EdgeKind::Tests, case, price, ResolvedBy::Framework);
    let head = g.build();
    let mut test_symbol = changed(
        "src/pricing/price.spec.ts",
        "makeCart",
        NodeKind::Function,
        body_change(),
    );
    test_symbol.test = true;
    let change = changes(vec![
        changed(
            "src/pricing/price.ts",
            "price",
            NodeKind::Function,
            body_change(),
        ),
        test_symbol,
    ]);
    let clustering = cluster(&change, &head, &BTreeSet::new());
    let changed = change_clusters(&clustering);
    assert_eq!(changed.len(), 1, "{changed:?}");
    assert!(changed[0].members.contains(&price) && changed[0].members.contains(&helper));
    assert!(changed[0].reason.contains(&ClusterReason::SameTestTarget));
}

/// `f0 → f1 → … → f14`, all changed, with one weak link `f7 → f8`.
fn long_chain() -> (Graph, ChangeSet, Vec<NodeKey>) {
    let mut g = Fixture::new();
    let keys: Vec<NodeKey> = (0..15)
        .map(|i| {
            g.symbol(
                "src/flow/steps.ts",
                &format!("step{i:02}"),
                NodeKind::Function,
            )
        })
        .collect();
    for i in 0..14 {
        let by = if i == 7 {
            ResolvedBy::NameUnique
        } else {
            ResolvedBy::Import
        };
        g.edge(EdgeKind::Calls, keys[i], keys[i + 1], by);
    }
    let change = changes(
        (0..15)
            .map(|i| {
                changed(
                    "src/flow/steps.ts",
                    &format!("step{i:02}"),
                    NodeKind::Function,
                    body_change(),
                )
            })
            .collect(),
    );
    (g.build(), change, keys)
}

#[test]
fn oversized_cluster_split_deterministically() {
    let (head, change, keys) = long_chain();
    let clustering = cluster(&change, &head, &BTreeSet::new());
    assert_eq!(clustering.splits, 1);
    let changed = change_clusters(&clustering);
    assert_eq!(changed.len(), 2);
    assert!(changed.iter().all(|c| c.members.len() <= MAX_CLUSTER_SIZE));
    let origin = changed[0].split_from.unwrap();
    assert!(changed.iter().all(|c| c.split_from == Some(origin)));
    let mut all = keys.clone();
    all.sort();
    assert_eq!(origin, ChangeClusterKey::of(&all));
    let first: BTreeSet<NodeKey> = keys[..8].iter().copied().collect();
    let second: BTreeSet<NodeKey> = keys[8..].iter().copied().collect();
    let parts: Vec<BTreeSet<NodeKey>> = changed
        .iter()
        .map(|c| c.members.iter().copied().collect())
        .collect();
    assert!(parts.contains(&first), "{parts:?}");
    assert!(parts.contains(&second));
    assert_eq!(clustering, cluster(&change, &head, &BTreeSet::new()));
}

#[test]
fn cluster_id_stable_across_runs() {
    let scenario = auth_bypass();
    let first = cluster(&scenario.change, &scenario.head, &BTreeSet::new());
    let mut reordered = scenario.change.clone();
    reordered.files.reverse();
    let second = cluster(&reordered, &scenario.head, &BTreeSet::new());
    let ids = |c: &Clustering| c.clusters.iter().map(|c| c.id).collect::<Vec<_>>();
    assert_eq!(ids(&first), ids(&second));
    let core = change_clusters(&first)[0].to_core();
    assert_eq!(core.key, change_clusters(&first)[0].id);
}

#[test]
fn cosmetic_into_low_risk_pseudo_cluster() {
    let mut g = Fixture::new();
    let tidy = g.symbol("src/x/tidy.ts", "tidy", NodeKind::Function);
    let real = g.symbol("src/x/real.ts", "real", NodeKind::Function);
    let head = g.build();
    let mut cosmetic = changed("src/x/tidy.ts", "tidy", NodeKind::Function, body_change());
    cosmetic.cosmetic = true;
    let change = changes(vec![
        cosmetic,
        changed("src/x/real.ts", "real", NodeKind::Function, body_change()),
    ]);
    let clustering = cluster(&change, &head, &BTreeSet::new());
    let low = clustering
        .clusters
        .iter()
        .find(|c| c.kind == ClusterKind::LowRisk)
        .unwrap();
    assert_eq!(low.members, vec![tidy]);
    assert_eq!(low.reason, vec![ClusterReason::LowRisk]);
    let changed = change_clusters(&clustering);
    assert_eq!(changed.len(), 1);
    assert_eq!(changed[0].members, vec![real]);
}

#[test]
fn degraded_without_graph_groups_by_file() {
    let change = changes(vec![
        changed("src/a.ts", "one", NodeKind::Function, body_change()),
        changed("src/a.ts", "two", NodeKind::Function, body_change()),
        changed("src/b.ts", "three", NodeKind::Function, body_change()),
    ]);
    let none = BTreeSet::new();
    let clustering = cluster_changes(&ClusterInputs {
        change: &change,
        head: None,
        impact: None,
        low_risk_symbols: &none,
        low_risk_files: &none_files(),
    });
    assert!(clustering.degraded);
    let sizes: Vec<usize> = change_clusters(&clustering)
        .iter()
        .map(|c| c.members.len())
        .collect();
    let mut sorted = sizes.clone();
    sorted.sort();
    assert_eq!(sorted, vec![1, 2]);
}

fn none_files() -> BTreeSet<RepoPath> {
    BTreeSet::new()
}
