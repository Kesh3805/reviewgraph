//! Trust-boundary context selection (REV-S-002).
//!
//! Generic ranking optimises for correctness; security reasoning needs the entry point that
//! reaches a change, the guards and validation on that path, and the sinks the change reaches.
//! For each changed symbol `S`:
//!
//! 1. an inbound bounded BFS over `CALLS`, `HANDLED_BY` and `ROUTES_TO` (depth 4, 200 nodes)
//!    collects every `ApiEndpoint` and `QueueConsumer` that reaches `S`, with the path;
//! 2. for each entry, the `AUTHORIZES` edges into it are its guards, and the `VALIDATES` edges
//!    out of the entry and the path symbols are its validation facts;
//! 3. an outbound bounded BFS over `CALLS`, `READS`, `WRITES`, `READS_TABLE` and `WRITES_TABLE`
//!    (depth 2, 100 nodes) reaches the sinks: tables, external APIs and external dependencies.
//!
//! The result is expressed as the reviewer context's own item types, so the model sees the
//! trust boundary as ordinary neighbourhood nodes and edges. A truncated traversal sets
//! `truncated` and adds an `omitted` entry; it is never silent. No new graph edges and no taint
//! analysis: this is a bounded BFS over what NEST/IMP already produced.

use std::collections::BTreeMap;

use codegraph::{
    bounded_bfs, Confidence, Direction, EdgeFilter, EdgeKind, EdgeKindSet, GraphQuery, NodeId,
    NodeKey, NodeKind, TraversalResult, TraversalSpec,
};

use crate::context::{NeighborEdge, NeighborNode};

/// Traversal budgets of the generator (REV-S-002).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrustBoundaryLimits {
    pub in_depth: u8,
    pub in_nodes: u32,
    pub out_depth: u8,
    pub out_nodes: u32,
    /// Edges below this confidence are not followed.
    pub min_confidence: Confidence,
}

impl Default for TrustBoundaryLimits {
    fn default() -> Self {
        Self {
            in_depth: 4,
            in_nodes: 200,
            out_depth: 2,
            out_nodes: 100,
            min_confidence: Confidence::from_f32(0.6),
        }
    }
}

/// One entry point that reaches a changed symbol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryPath {
    /// Node id of the `ApiEndpoint` or `QueueConsumer`.
    pub entry_id: String,
    pub entry_kind: NodeKind,
    /// The changed symbol the path ends at.
    pub changed_id: String,
    /// Node ids from the entry to the changed symbol, both included.
    pub path: Vec<String>,
    /// Guard node ids (`AUTHORIZES` into the entry).
    pub guards: Vec<String>,
    /// Validation target node ids (`VALIDATES` out of the entry or a path symbol).
    pub validations: Vec<String>,
}

/// A sink the change reaches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sink {
    pub node_id: String,
    pub kind: NodeKind,
    pub changed_id: String,
}

/// The selected trust-boundary context.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TrustBoundary {
    pub entries: Vec<EntryPath>,
    pub sinks: Vec<Sink>,
    /// Entry, guard, validation, path and sink nodes, nearest first.
    pub nodes: Vec<NeighborNode>,
    pub edges: Vec<NeighborEdge>,
    /// A traversal hit its budget.
    pub truncated: bool,
    /// Why items are missing.
    pub omitted: Vec<String>,
}

impl TrustBoundary {
    /// Keeps at most `max_nodes` nodes (nearest first) and the edges between the kept nodes,
    /// recording what was cut.
    pub fn limit_nodes(&mut self, max_nodes: usize) {
        if self.nodes.len() <= max_nodes {
            return;
        }
        let cut = self.nodes.len() - max_nodes;
        self.nodes.truncate(max_nodes);
        let kept: Vec<&str> = self.nodes.iter().map(|n| n.symbol_id.as_str()).collect();
        self.edges
            .retain(|e| kept.contains(&e.from.as_str()) && kept.contains(&e.to.as_str()));
        self.truncated = true;
        self.omitted.push(format!(
            "{cut} trust-boundary node(s) over the symbol budget"
        ));
    }
}

fn is_entry(kind: NodeKind) -> bool {
    matches!(kind, NodeKind::ApiEndpoint | NodeKind::QueueConsumer)
}

fn is_sink(kind: NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::DatabaseTable | NodeKind::ExternalApi | NodeKind::ExternalDependency
    )
}

fn node_id_of(graph: &dyn GraphQuery, key: NodeKey) -> Option<String> {
    graph.node(key).map(|n| n.id.to_owned())
}

fn neighbor_node(graph: &dyn GraphQuery, key: NodeKey, distance: u32) -> Option<NeighborNode> {
    let node = graph.node(key)?;
    Some(NeighborNode {
        symbol_id: node.id.to_owned(),
        kind: node.kind.as_str().to_owned(),
        path: node.file.unwrap_or_default().to_owned(),
        range: node.range.map_or([0, 0], |r| [r.start.line, r.end.line]),
        file_lines: None,
        distance,
        excerpt: String::new(),
    })
}

/// The chain of node keys from the traversal seed to `to`, following the BFS parent pointers.
fn chain(result: &TraversalResult, seed: NodeKey, to: NodeKey) -> Option<Vec<NodeKey>> {
    let steps = result.path_to(to)?;
    let mut keys = vec![seed];
    let mut current = seed;
    for step in &steps {
        let next = if step.source == current {
            step.target
        } else {
            step.source
        };
        keys.push(next);
        current = next;
    }
    Some(keys)
}

fn collect_edges(
    graph: &dyn GraphQuery,
    key: NodeKey,
    dir: Direction,
    kind: EdgeKind,
) -> Vec<NodeKey> {
    let mut found = Vec::new();
    graph.for_each_edge(
        key,
        dir,
        &EdgeFilter::kinds(EdgeKindSet::of(kind)),
        &mut |edge| {
            found.push(if dir == Direction::In {
                edge.source
            } else {
                edge.target
            });
            std::ops::ControlFlow::Continue(())
        },
    );
    found.sort_unstable();
    found.dedup();
    found
}

/// Collects the trust-boundary context of `changed` (node ids of the changed symbols).
pub fn trust_boundary(
    graph: &dyn GraphQuery,
    changed: &[String],
    limits: &TrustBoundaryLimits,
) -> TrustBoundary {
    let in_kinds = EdgeKindSet::of(EdgeKind::Calls)
        .union(EdgeKindSet::of(EdgeKind::HandledBy))
        .union(EdgeKindSet::of(EdgeKind::RoutesTo));
    let out_kinds = EdgeKindSet::of(EdgeKind::Calls)
        .union(EdgeKindSet::of(EdgeKind::Reads))
        .union(EdgeKindSet::of(EdgeKind::Writes))
        .union(EdgeKindSet::of(EdgeKind::ReadsTable))
        .union(EdgeKindSet::of(EdgeKind::WritesTable));

    let mut out = TrustBoundary::default();
    let mut nodes: BTreeMap<String, NeighborNode> = BTreeMap::new();
    let mut edges: BTreeMap<(String, String, String), NeighborEdge> = BTreeMap::new();

    for changed_id in changed {
        let seed = NodeId::from_canonical(changed_id.as_str()).key();
        if graph.node(seed).is_none() {
            out.omitted
                .push(format!("{changed_id} is not in the graph"));
            continue;
        }

        let inbound = TraversalSpec::new(
            vec![seed],
            Direction::In,
            in_kinds,
            limits.in_depth,
            limits.in_nodes,
            limits.min_confidence,
        );
        if let Ok(result) = bounded_bfs(graph, &inbound) {
            if result.truncated {
                out.truncated = true;
                out.omitted.push(format!(
                    "inbound traversal from {changed_id} hit its budget"
                ));
            }
            for visit in &result.visits {
                if let Some(step) = &visit.via {
                    if let (Some(from), Some(to)) = (
                        node_id_of(graph, step.source),
                        node_id_of(graph, step.target),
                    ) {
                        edges
                            .entry((from.clone(), to.clone(), step.kind.as_str().to_owned()))
                            .or_insert_with(|| NeighborEdge {
                                from,
                                to,
                                kind: step.kind.as_str().to_owned(),
                                confidence: f64::from(step.confidence.as_f32()),
                            });
                    }
                }
                let Some(node) = graph.node(visit.key) else {
                    continue;
                };
                if !is_entry(node.kind) {
                    continue;
                }
                let Some(keys) = chain(&result, seed, visit.key) else {
                    continue;
                };
                let mut path_keys = keys;
                path_keys.reverse();
                let path: Vec<String> = path_keys
                    .iter()
                    .filter_map(|k| node_id_of(graph, *k))
                    .collect();
                let guard_keys =
                    collect_edges(graph, visit.key, Direction::In, EdgeKind::Authorizes);
                let mut validation_keys: Vec<NodeKey> = Vec::new();
                for k in &path_keys {
                    validation_keys.extend(collect_edges(
                        graph,
                        *k,
                        Direction::Out,
                        EdgeKind::Validates,
                    ));
                }
                validation_keys.sort_unstable();
                validation_keys.dedup();

                for (k, depth) in path_keys
                    .iter()
                    .enumerate()
                    .map(|(i, k)| (*k, (path_keys.len() - 1 - i) as u32))
                    .chain(guard_keys.iter().map(|k| (*k, u32::from(visit.depth))))
                    .chain(validation_keys.iter().map(|k| (*k, u32::from(visit.depth))))
                {
                    if let Some(n) = neighbor_node(graph, k, depth) {
                        nodes.entry(n.symbol_id.clone()).or_insert(n);
                    }
                }
                for guard in &guard_keys {
                    if let (Some(from), Some(to)) =
                        (node_id_of(graph, *guard), node_id_of(graph, visit.key))
                    {
                        edges
                            .entry((from.clone(), to.clone(), "AUTHORIZES".to_owned()))
                            .or_insert_with(|| NeighborEdge {
                                from,
                                to,
                                kind: "AUTHORIZES".to_owned(),
                                confidence: 1.0,
                            });
                    }
                }
                out.entries.push(EntryPath {
                    entry_id: node.id.to_owned(),
                    entry_kind: node.kind,
                    changed_id: changed_id.clone(),
                    path,
                    guards: guard_keys
                        .iter()
                        .filter_map(|k| node_id_of(graph, *k))
                        .collect(),
                    validations: validation_keys
                        .iter()
                        .filter_map(|k| node_id_of(graph, *k))
                        .collect(),
                });
            }
        }

        let outbound = TraversalSpec::new(
            vec![seed],
            Direction::Out,
            out_kinds,
            limits.out_depth,
            limits.out_nodes,
            limits.min_confidence,
        );
        if let Ok(result) = bounded_bfs(graph, &outbound) {
            if result.truncated {
                out.truncated = true;
                out.omitted.push(format!(
                    "outbound traversal from {changed_id} hit its budget"
                ));
            }
            for visit in &result.visits {
                if let Some(step) = &visit.via {
                    if let (Some(from), Some(to)) = (
                        node_id_of(graph, step.source),
                        node_id_of(graph, step.target),
                    ) {
                        edges
                            .entry((from.clone(), to.clone(), step.kind.as_str().to_owned()))
                            .or_insert_with(|| NeighborEdge {
                                from,
                                to,
                                kind: step.kind.as_str().to_owned(),
                                confidence: f64::from(step.confidence.as_f32()),
                            });
                    }
                }
                let Some(node) = graph.node(visit.key) else {
                    continue;
                };
                if visit.depth > 0 && is_sink(node.kind) {
                    out.sinks.push(Sink {
                        node_id: node.id.to_owned(),
                        kind: node.kind,
                        changed_id: changed_id.clone(),
                    });
                    if let Some(n) = neighbor_node(graph, visit.key, u32::from(visit.depth)) {
                        nodes.entry(n.symbol_id.clone()).or_insert(n);
                    }
                }
            }
        }
    }

    let mut ordered: Vec<NeighborNode> = nodes.into_values().collect();
    ordered.sort_by(|a, b| {
        a.distance
            .cmp(&b.distance)
            .then_with(|| a.symbol_id.cmp(&b.symbol_id))
    });
    out.nodes = ordered;
    out.edges = edges.into_values().collect();
    out.entries.sort_by(|a, b| {
        (&a.entry_id, &a.changed_id, &a.path).cmp(&(&b.entry_id, &b.changed_id, &b.path))
    });
    out.entries.dedup();
    out.sinks
        .sort_by(|a, b| (&a.node_id, &a.changed_id).cmp(&(&b.node_id, &b.changed_id)));
    out.sinks.dedup();
    out
}
