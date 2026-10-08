//! API entrypoint reachability (IMP-004).
//!
//! "Does the affected path reach a public endpoint?" can be farther than the caller depth, so
//! this is its own best-first search over reverse `CALLS` (plus interface dispatch), bounded by
//! `max_endpoint_depth` hops and `max_endpoint_visits` visited nodes per seed. Visited nodes are
//! not elements; only the entrypoints found are. The priority is the total order
//! `(Reverse(min_confidence), depth, node key)`, so the search is deterministic and the most
//! trustworthy route to an entrypoint is found first.
//!
//! Entrypoints:
//! * `http` / `cli` — an `ApiEndpoint` / `CliCommand` with a `HANDLED_BY` edge onto a visited
//!   node; the endpoint is the element, reached through the `HANDLED_BY` step.
//! * `queue` — a visited node consuming a queue (`CONSUMES_JOB`, `SUBSCRIBES`); the queue is
//!   the element.
//! * `cli` / `worker` — a visited node that is itself a `CliCommand` / `Worker`.
//!
//! Guards are recorded on the endpoint (names of `AUTHORIZES` sources), never evaluated.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

use codegraph::{Confidence, Direction, EdgeKind, EdgeKindSet, GraphQuery, NodeKey, NodeKind};

use super::builder::{collect_edges, Cx, SeedState};
use super::calls::dispatch_targets;
use super::model::{EndpointAttrs, EntryKind, Relation, TruncReason};
use super::path::{step, Extras, Trail};

type Priority = (Reverse<Confidence>, u8, NodeKey);

/// Names of the guards authorizing `endpoint`, sorted.
fn guards_of(graph: &dyn GraphQuery, endpoint: NodeKey) -> Vec<String> {
    let mut names: Vec<String> = collect_edges(
        graph,
        endpoint,
        Direction::In,
        EdgeKindSet::of(EdgeKind::Authorizes),
    )
    .into_iter()
    .filter_map(|edge| graph.node(edge.source).map(|view| view.name.to_owned()))
    .collect();
    names.sort();
    names.dedup();
    names
}

/// `"PUT /users/:id"` → `(Some("PUT"), Some("/users/:id"))`.
fn method_and_path(qualified: &str) -> (Option<String>, Option<String>) {
    match qualified.split_once(' ') {
        Some((method, route)) if !method.is_empty() && route.starts_with('/') => {
            (Some(method.to_owned()), Some(route.to_owned()))
        }
        _ => (None, None),
    }
}

/// Emits every entrypoint at `node`, reached through `trail`.
fn emit(cx: &Cx<'_>, graph: &dyn GraphQuery, state: &mut SeedState, node: NodeKey, trail: &Trail) {
    let side = state.side;
    let limit = cx.budget.max_endpoints;

    for edge in collect_edges(
        graph,
        node,
        Direction::In,
        EdgeKindSet::of(EdgeKind::HandledBy),
    ) {
        let Some(view) = graph.node(edge.source) else {
            continue;
        };
        let entry_kind = match view.kind {
            NodeKind::ApiEndpoint => EntryKind::Http,
            NodeKind::CliCommand => EntryKind::Cli,
            NodeKind::Worker => EntryKind::Worker,
            _ => continue,
        };
        let (method, path) = if entry_kind == EntryKind::Http {
            method_and_path(view.qualified_name)
        } else {
            (None, Some(view.name.to_owned()))
        };
        let attrs = EndpointAttrs {
            entry_kind,
            method,
            path,
            guards: guards_of(graph, edge.source),
        };
        let reached = trail.extend(
            step(
                edge.source,
                EdgeKind::HandledBy,
                node,
                edge.confidence,
                side,
            ),
            edge.source,
            true,
        );
        let extras = Extras {
            endpoint: Some(attrs),
            ..Extras::default()
        };
        if let Some(candidate) = cx.candidate(side, edge.source, reached, extras) {
            state.admit(Relation::Endpoint, limit, candidate);
        }
    }

    let consumes =
        EdgeKindSet::of(EdgeKind::ConsumesJob).union(EdgeKindSet::of(EdgeKind::Subscribes));
    for edge in collect_edges(graph, node, Direction::Out, consumes) {
        let Some(view) = graph.node(edge.target) else {
            continue;
        };
        let attrs = EndpointAttrs {
            entry_kind: EntryKind::Queue,
            method: None,
            path: Some(view.name.to_owned()),
            guards: Vec::new(),
        };
        let reached = trail.extend(
            step(node, edge.kind, edge.target, edge.confidence, side),
            edge.target,
            true,
        );
        let extras = Extras {
            endpoint: Some(attrs),
            ..Extras::default()
        };
        if let Some(candidate) = cx.candidate(side, edge.target, reached, extras) {
            state.admit(Relation::Endpoint, limit, candidate);
        }
    }

    if let Some(view) = graph.node(node) {
        let entry_kind = match view.kind {
            NodeKind::CliCommand => Some(EntryKind::Cli),
            NodeKind::Worker => Some(EntryKind::Worker),
            _ => None,
        };
        if let Some(entry_kind) = entry_kind {
            let attrs = EndpointAttrs {
                entry_kind,
                method: None,
                path: Some(view.name.to_owned()),
                guards: Vec::new(),
            };
            let extras = Extras {
                endpoint: Some(attrs),
                ..Extras::default()
            };
            if let Some(candidate) = cx.candidate(side, node, trail.clone(), extras) {
                state.admit(Relation::Endpoint, limit, candidate);
            }
        }
    }
}

/// Best-first reverse reachability from the seed to entrypoints. Returns the nodes visited.
pub(crate) fn expand_endpoints(cx: &Cx<'_>, state: &mut SeedState) -> u32 {
    let Some(graph) = cx.graph(state.side) else {
        return 0;
    };
    let side = state.side;
    let seed = state.seed;
    let max_depth = cx.budget.max_endpoint_depth;
    let max_visits = cx.budget.max_endpoint_visits;
    let calls = EdgeKindSet::of(EdgeKind::Calls);

    let mut queue: BTreeSet<Priority> = BTreeSet::new();
    let mut best: BTreeMap<NodeKey, Trail> = BTreeMap::new();
    let mut visited: BTreeSet<NodeKey> = BTreeSet::new();
    queue.insert((Reverse(Confidence::MAX), 0, seed));
    best.insert(seed, Trail::seed(seed));
    let mut visits = 0u32;

    while let Some((_, depth, node)) = queue.pop_first() {
        if visited.contains(&node) {
            continue;
        }
        if visits >= max_visits {
            let pending = queue
                .iter()
                .filter(|(_, _, key)| !visited.contains(key))
                .count() as u32
                + 1;
            state.record(
                Some(Relation::Endpoint),
                TruncReason::Depth,
                max_visits,
                pending,
                Some(visits),
            );
            break;
        }
        visited.insert(node);
        visits += 1;
        let Some(trail) = best.get(&node).cloned() else {
            continue;
        };
        emit(cx, graph, state, node, &trail);

        if depth >= max_depth || (node != seed && trail.min_confidence < cx.budget.min_confidence) {
            continue;
        }

        let mut next: Vec<(NodeKey, Trail)> = Vec::new();
        for edge in collect_edges(graph, node, Direction::In, calls) {
            if trail.visits(edge.source) {
                continue;
            }
            let hop = step(edge.source, EdgeKind::Calls, node, edge.confidence, side);
            next.push((edge.source, trail.extend(hop, edge.source, true)));
        }
        for (interface, hop_confidence) in dispatch_targets(graph, node) {
            if trail.visits(interface) {
                continue;
            }
            let via = trail.extend(
                step(node, EdgeKind::Implements, interface, hop_confidence, side),
                interface,
                false,
            );
            for edge in collect_edges(graph, interface, Direction::In, calls) {
                if via.visits(edge.source) {
                    continue;
                }
                let hop = step(
                    edge.source,
                    EdgeKind::Calls,
                    interface,
                    edge.confidence,
                    side,
                );
                next.push((edge.source, via.extend(hop, edge.source, true)));
            }
        }
        for (caller, candidate) in next {
            if visited.contains(&caller) {
                continue;
            }
            let better = match best.get(&caller) {
                Some(existing) => {
                    super::path::compare_trails(&candidate, existing) == std::cmp::Ordering::Less
                }
                None => true,
            };
            if better {
                if let Some(existing) = best.get(&caller) {
                    queue.remove(&(Reverse(existing.min_confidence), existing.distance, caller));
                }
                queue.insert((
                    Reverse(candidate.min_confidence),
                    candidate.distance,
                    caller,
                ));
                best.insert(caller, candidate);
            }
        }
    }
    visits
}
