//! Built-in convention miners.
//!
//! * PROF-002: [`layer_dependency`] — the observed layer matrix as conventions.
//! * PROF-004: [`controller_repository`], [`transaction_wrapper`], [`error_hierarchy`],
//!   [`guard_usage`].
//! * PROF-005: [`testing`], [`api`], [`queues`].

pub mod api;
pub mod controller_repository;
pub mod error_hierarchy;
pub mod guard_usage;
pub mod layer_dependency;
pub mod queues;
pub mod testing;
pub mod transaction_wrapper;

use std::ops::ControlFlow;

use codegraph::{
    Direction, EdgeFilter, EdgeKind, EdgeKindSet, EdgeRef, GraphQuery, NodeKey, NodeRef,
};

use super::miner::Sample;

/// A sample for a node (its file and first line).
pub(crate) fn node_sample(node: &NodeRef<'_>, detail: impl Into<String>) -> Option<Sample> {
    let path = node.file?;
    Some(Sample {
        path: path.to_owned(),
        line: node.range.map_or(1, |r| r.start.line),
        symbol: Some(node.id.to_owned()),
        key: Some(node.key),
        detail: detail.into(),
    })
}

/// The node itself plus every node of its file whose parent is it (methods, properties, the
/// constructor).
pub(crate) fn members(graph: &dyn GraphQuery, node: &NodeRef<'_>) -> Vec<NodeKey> {
    let mut out = vec![node.key];
    if let Some(path) = node.file {
        graph.nodes_in_file(path, &mut |member| {
            if member.attrs.parent == Some(node.key) {
                out.push(member.key);
            }
        });
    }
    out
}

/// Every edge of `key` in `dir` with one of `kinds`, in canonical order.
pub(crate) fn edges<'g>(
    graph: &'g dyn GraphQuery,
    key: NodeKey,
    dir: Direction,
    kinds: &[EdgeKind],
) -> Vec<EdgeRef<'g>> {
    let mut set = EdgeKindSet::default();
    for kind in kinds {
        set.insert(*kind);
    }
    let mut out = Vec::new();
    graph.for_each_edge(key, dir, &EdgeFilter::kinds(set), &mut |edge| {
        out.push(edge);
        ControlFlow::Continue(())
    });
    out
}

/// The `HANDLED_BY` target of an endpoint.
pub(crate) fn handler_of<'g>(graph: &'g dyn GraphQuery, endpoint: NodeKey) -> Option<NodeRef<'g>> {
    edges(graph, endpoint, Direction::Out, &[EdgeKind::HandledBy])
        .first()
        .and_then(|e| graph.node(e.target))
}

/// Every node of a kind, in canonical order.
pub(crate) fn nodes_of_kind<'g>(
    graph: &'g dyn GraphQuery,
    kinds: &[codegraph::NodeKind],
) -> Vec<NodeRef<'g>> {
    let mut out = Vec::new();
    graph.for_each_node(&mut |node| {
        if kinds.contains(&node.kind) {
            out.push(node);
        }
    });
    out
}
