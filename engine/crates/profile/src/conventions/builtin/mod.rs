//! The built-in convention miners (PROF-004, PROF-005). Generic: nothing here is specific to one
//! consumer repository.

pub mod api;
pub mod controller_repository;
pub mod error_hierarchy;
pub mod guard_usage;
pub mod layer_dependency;
pub mod queues;
pub mod testing;
pub mod transaction_wrapper;

use codegraph::query::GraphQuery;
use codegraph::{EdgeKindSet, GraphQueryExt, NodeFlags, NodeKey, NodeKind, NodeRef};

use super::registry::MinerRegistry;

/// An owned snapshot of the node fields miners need.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(crate) struct Found {
    pub key: NodeKey,
    pub id: String,
    pub name: String,
    pub qualified_name: String,
    pub kind: NodeKind,
    pub flags: NodeFlags,
    pub file: Option<String>,
}

/// Every node `keep` accepts, in the graph's canonical order.
pub(crate) fn collect(g: &dyn GraphQuery, keep: impl Fn(&NodeRef<'_>) -> bool) -> Vec<Found> {
    let mut out = Vec::new();
    g.for_each_node(&mut |n| {
        if keep(&n) {
            out.push(Found {
                key: n.key,
                id: n.id.to_owned(),
                name: n.name.to_owned(),
                qualified_name: n.qualified_name.to_owned(),
                kind: n.kind,
                flags: n.attrs.flags,
                file: n.file.map(str::to_owned),
            });
        }
    });
    out
}

/// The nodes on the far side of `key`'s out-edges of `kinds`.
pub(crate) fn out_targets(g: &dyn GraphQuery, key: NodeKey, kinds: EdgeKindSet) -> Vec<NodeKey> {
    g.out_edges(key, kinds).iter().map(|e| e.target).collect()
}

/// The nodes on the near side of `key`'s in-edges of `kinds`.
pub(crate) fn in_sources(g: &dyn GraphQuery, key: NodeKey, kinds: EdgeKindSet) -> Vec<NodeKey> {
    g.in_edges(key, kinds).iter().map(|e| e.source).collect()
}

/// Names of the guard symbols (framework `Middleware` nodes), sorted and deduplicated.
pub fn guard_names(g: &dyn super::GraphView) -> Vec<String> {
    let mut names: Vec<String> = collect(g.graph(), |n| n.kind == NodeKind::Middleware)
        .into_iter()
        .map(|f| f.name)
        .collect();
    names.sort();
    names.dedup();
    names
}

impl MinerRegistry {
    /// A registry with every built-in miner.
    pub fn builtin() -> Self {
        let mut r = Self::new();
        r.register(Box::new(controller_repository::ControllerRepository));
        r.register(Box::new(transaction_wrapper::TransactionWrapper));
        r.register(Box::new(error_hierarchy::ErrorHierarchy));
        r.register(Box::new(guard_usage::GuardUsage));
        r.register(Box::new(testing::PublicServicesHaveUnitTests));
        r.register(Box::new(api::EndpointsUseDtoValidation));
        for (id, from, to) in layer_dependency::PAIRS {
            r.register(Box::new(layer_dependency::LayerDependency::new(
                id, from, to,
            )));
        }
        r.register(Box::new(queues::JobsHaveDeterministicId));
        r
    }
}
