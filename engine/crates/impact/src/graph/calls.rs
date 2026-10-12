//! Callers and callees (IMP-002).
//!
//! Callers are a level-by-level reverse `CALLS` search from the seed. Inside a level the
//! frontier is ordered by edge confidence (descending), then node key, so when a cap truncates
//! a hub the strongest relations are the ones kept. Interface dispatch is followed at every
//! level: if a node implements or overrides `I.m`, callers of `I.m` are callers of the node at
//! the same distance, through a synthesized `IMPLEMENTS` step of confidence ≤ 0.9 (NestJS DI
//! calls go through interfaces and tokens).
//!
//! Callees are the seed's forward `CALLS` at depth 1; removed callees are the CHG-003
//! `call_removed` targets resolved on the base graph.

use std::cmp::Reverse;
use std::collections::BTreeSet;

use codegraph::{Confidence, Direction, EdgeKind, EdgeKindSet, GraphQuery, NodeKey};

use super::builder::{collect_edges, Cx, SeedState};
use super::model::{GraphSide, Relation, TruncReason};
use super::path::{step, Extras, Trail};

/// Confidence of a synthesized interface-dispatch hop.
pub const DISPATCH_CONFIDENCE: Confidence =
    codegraph::confidence_of(codegraph::ResolvedBy::Framework);

/// The interface or supertype members `node` implements or overrides, with the hop confidence.
pub(crate) fn dispatch_targets(
    graph: &dyn GraphQuery,
    node: NodeKey,
) -> Vec<(NodeKey, Confidence)> {
    let kinds = EdgeKindSet::of(EdgeKind::Overrides).union(EdgeKindSet::of(EdgeKind::Implements));
    let mut out: Vec<(NodeKey, Confidence)> = collect_edges(graph, node, Direction::Out, kinds)
        .into_iter()
        .filter(|edge| edge.target != node)
        .map(|edge| (edge.target, edge.confidence.min(DISPATCH_CONFIDENCE)))
        .collect();
    out.sort_by_key(|(key, confidence)| (Reverse(*confidence), *key));
    out.dedup_by_key(|(key, _)| *key);
    out
}

/// One caller candidate of a level: the caller, the edge confidence that ranks it, its trail.
struct CallerCandidate {
    caller: NodeKey,
    edge_confidence: Confidence,
    trail: Trail,
}

/// Expands callers level by level from `level` (nodes at `start_depth`) up to `end_depth`.
/// Returns the last expanded level (nodes at the deepest depth reached), for the IMP-002
/// transitive pass.
pub(crate) fn expand_callers(
    cx: &Cx<'_>,
    state: &mut SeedState,
    level: Vec<NodeKey>,
    start_depth: u8,
    end_depth: u8,
) -> Vec<NodeKey> {
    let Some(graph) = cx.graph(state.side) else {
        return Vec::new();
    };
    let calls = EdgeKindSet::of(EdgeKind::Calls);
    let limit = cx.budget.max_callers;
    let mut level = level;
    let mut depth = start_depth;
    while depth < end_depth && !level.is_empty() {
        let mut candidates: Vec<CallerCandidate> = Vec::new();
        for node in &level {
            let trail = if *node == state.seed {
                Trail::seed(state.seed)
            } else {
                match state.set.trail(Relation::Caller, *node) {
                    Some(trail) => trail.clone(),
                    None => continue,
                }
            };
            for edge in collect_edges(graph, *node, Direction::In, calls) {
                if trail.visits(edge.source) {
                    continue;
                }
                let hop = step(
                    edge.source,
                    EdgeKind::Calls,
                    *node,
                    edge.confidence,
                    state.side,
                );
                candidates.push(CallerCandidate {
                    caller: edge.source,
                    edge_confidence: edge.confidence,
                    trail: trail.extend(hop, edge.source, true),
                });
            }
            for (interface, hop_confidence) in dispatch_targets(graph, *node) {
                if trail.visits(interface) {
                    continue;
                }
                let dispatch = step(
                    *node,
                    EdgeKind::Implements,
                    interface,
                    hop_confidence,
                    state.side,
                );
                let via = trail.extend(dispatch, interface, false);
                for edge in collect_edges(graph, interface, Direction::In, calls) {
                    if via.visits(edge.source) {
                        continue;
                    }
                    let hop = step(
                        edge.source,
                        EdgeKind::Calls,
                        interface,
                        edge.confidence,
                        state.side,
                    );
                    candidates.push(CallerCandidate {
                        caller: edge.source,
                        edge_confidence: edge.confidence.min(hop_confidence),
                        trail: via.extend(hop, edge.source, true),
                    });
                }
            }
        }
        candidates.sort_by(|a, b| {
            b.edge_confidence
                .cmp(&a.edge_confidence)
                .then_with(|| a.caller.cmp(&b.caller))
                .then_with(|| super::path::compare_trails(&a.trail, &b.trail))
        });

        let mut reached: BTreeSet<NodeKey> = BTreeSet::new();
        for candidate in candidates {
            if candidate.caller == state.seed {
                continue;
            }
            let Some(admitted) = cx.candidate(
                state.side,
                candidate.caller,
                candidate.trail,
                Extras::default(),
            ) else {
                continue;
            };
            if state.admit(Relation::Caller, limit, admitted)
                && state.visited.insert(candidate.caller)
            {
                reached.insert(candidate.caller);
            }
        }

        // Expand only from strong paths: weak elements are listed, never expanded.
        let mut next: Vec<(NodeKey, Confidence)> = reached
            .into_iter()
            .filter_map(|node| {
                state
                    .set
                    .trail(Relation::Caller, node)
                    .map(|trail| (node, trail.min_confidence))
            })
            .filter(|(_, confidence)| *confidence >= cx.budget.min_confidence)
            .collect();
        next.sort_by_key(|(node, confidence)| (Reverse(*confidence), *node));
        level = next.into_iter().map(|(node, _)| node).collect();
        depth += 1;
    }

    // Reaching the depth cap with callers still unexplored is a depth truncation.
    if !level.is_empty() {
        let mut beyond: BTreeSet<NodeKey> = BTreeSet::new();
        for node in &level {
            for edge in collect_edges(graph, *node, Direction::In, calls) {
                if !state.visited.contains(&edge.source) && edge.source != state.seed {
                    beyond.insert(edge.source);
                }
            }
        }
        if !beyond.is_empty() {
            state.record(
                Some(Relation::Caller),
                TruncReason::Depth,
                u32::from(end_depth),
                beyond.len() as u32,
                None,
            );
        }
    }
    level
}

/// Forward `CALLS` at depth 1 on the seed's graph.
pub(crate) fn expand_callees(cx: &Cx<'_>, state: &mut SeedState) {
    let Some(graph) = cx.graph(state.side) else {
        return;
    };
    let mut edges = collect_edges(
        graph,
        state.seed,
        Direction::Out,
        EdgeKindSet::of(EdgeKind::Calls),
    );
    edges.sort_by_key(|edge| (Reverse(edge.confidence), edge.target));
    let seed_trail = Trail::seed(state.seed);
    for edge in edges {
        if edge.target == state.seed {
            continue;
        }
        let hop = step(
            state.seed,
            EdgeKind::Calls,
            edge.target,
            edge.confidence,
            state.side,
        );
        let trail = seed_trail.extend(hop, edge.target, true);
        if let Some(candidate) = cx.candidate(state.side, edge.target, trail, Extras::default()) {
            state.admit(Relation::Callee, cx.budget.max_callees, candidate);
        }
    }
}

/// CHG-003 `call_removed` targets, resolved on the base graph.
pub(crate) fn expand_removed_callees(cx: &Cx<'_>, state: &mut SeedState, removed: &[NodeKey]) {
    let Some(base) = cx.base else {
        return;
    };
    let base_calls = collect_edges(
        base,
        state.seed,
        Direction::Out,
        EdgeKindSet::of(EdgeKind::Calls),
    );
    let mut targets: Vec<(NodeKey, Confidence)> = removed
        .iter()
        .map(|target| {
            let confidence = base_calls
                .iter()
                .find(|edge| edge.target == *target)
                .map_or(Confidence::MAX, |edge| edge.confidence);
            (*target, confidence)
        })
        .collect();
    targets.sort_by_key(|(key, confidence)| (Reverse(*confidence), *key));
    targets.dedup_by_key(|(key, _)| *key);
    let seed_trail = Trail::seed(state.seed);
    for (target, confidence) in targets {
        let hop = step(
            state.seed,
            EdgeKind::Calls,
            target,
            confidence,
            GraphSide::Base,
        );
        let trail = seed_trail.extend(hop, target, true);
        if let Some(candidate) = cx.candidate(GraphSide::Base, target, trail, Extras::default()) {
            state.admit(Relation::RemovedCallee, cx.budget.max_callees, candidate);
        }
    }
}
