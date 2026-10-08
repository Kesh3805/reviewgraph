//! Shortest path between two nodes (CG-009).
//!
//! Verification needs to check claims like "A reaches endpoint E" (graph evidence, VER stage) and
//! the CLI needs `review graph path` (PRD §84). Both want the same thing: the *minimum-hop* path
//! under a direction, a kind set and a confidence floor, plus an honest answer about whether the
//! search finished or ran out of budget.
//!
//! "Not found within `max_depth`" and "budget exhausted before the target was found" are different
//! answers and are reported differently: the first is `path: None, truncated: false`, the second
//! sets `truncated`. Conflating them would let a verification step claim "no path exists" when in
//! fact the search gave up.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::node_id::NodeKey;
use crate::query::bfs::{EdgeStep, MAX_DEPTH_LIMIT, MAX_NODES_LIMIT};
use crate::query::hex128;
use crate::query::{EdgeFilter, GraphQuery, TraversalError};
use crate::{Confidence, Direction, EdgeKindSet};

/// A path search.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PathSpec {
    #[serde(with = "hex128")]
    #[schemars(with = "String")]
    pub from: NodeKey,
    #[serde(with = "hex128")]
    #[schemars(with = "String")]
    pub to: NodeKey,
    /// `Out` or `In`. `Both` is rejected: "does A reach B" and "does B reach A" are different
    /// questions, and silently answering either would be worse than refusing.
    pub direction: Direction,
    pub kinds: EdgeKindSet,
    pub max_depth: u8,
    pub max_nodes: u32,
    pub min_confidence: Confidence,
}

impl PathSpec {
    /// A spec with the default budgets: six hops and ten thousand expanded nodes.
    #[must_use]
    pub fn new(from: NodeKey, to: NodeKey, direction: Direction, kinds: EdgeKindSet) -> Self {
        Self {
            from,
            to,
            direction,
            kinds,
            max_depth: 6,
            max_nodes: 10_000,
            min_confidence: Confidence::MIN,
        }
    }

    fn filter(&self) -> EdgeFilter {
        EdgeFilter::new(self.kinds, self.min_confidence)
    }
}

/// A found path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GraphPath {
    pub steps: Vec<EdgeStep>,
    /// The weakest confidence along the path, which is the confidence of the claim as a whole.
    pub min_confidence: Confidence,
}

impl GraphPath {
    /// The number of hops, which is the length of `steps`.
    #[must_use]
    pub fn hops(&self) -> usize {
        self.steps.len()
    }

    /// The last step's target, i.e. the node the path arrives at.
    #[must_use]
    pub fn target(&self) -> Option<&NodeKey> {
        self.steps.last().map(|step| &step.target)
    }
}

/// The answer to a path search.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PathResult {
    pub path: Option<GraphPath>,
    /// True only when the budget ran out before the target was found.
    pub truncated: bool,
    /// Nodes the search expanded, for the `graph_api.path` span.
    pub nodes_visited: u32,
}

/// Why a path search could not run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PathError {
    #[error("unknown node {0}")]
    UnknownNode(NodeKey),
    #[error("a path search needs Direction::Out or Direction::In, not Both")]
    BothDirections,
    #[error(transparent)]
    Traversal(#[from] TraversalError),
}

/// Minimum-hop path from `spec.from` to `spec.to`.
///
/// Complexity `O(N + X)` inside the budget. `from == to` yields an empty path.
///
/// # Errors
///
/// [`PathError::UnknownNode`] when either endpoint is not in the graph, [`PathError::BothDirections`]
/// when the spec asks for `Both`, and [`TraversalError::BudgetTooLarge`] past the hard caps.
pub fn shortest_path(graph: &dyn GraphQuery, spec: &PathSpec) -> Result<PathResult, PathError> {
    if spec.direction == Direction::Both {
        return Err(PathError::BothDirections);
    }
    if spec.max_depth > MAX_DEPTH_LIMIT || spec.max_nodes > MAX_NODES_LIMIT {
        return Err(PathError::Traversal(TraversalError::BudgetTooLarge));
    }
    if graph.node(spec.from).is_none() {
        return Err(PathError::UnknownNode(spec.from));
    }
    if graph.node(spec.to).is_none() {
        return Err(PathError::UnknownNode(spec.to));
    }
    if spec.from == spec.to {
        return Ok(PathResult {
            path: Some(GraphPath {
                steps: Vec::new(),
                min_confidence: Confidence::MAX,
            }),
            truncated: false,
            nodes_visited: 1,
        });
    }

    let filter = spec.filter();
    let target = spec.to;
    // The visit list doubles as the BFS queue: index order is discovery order, so walking it
    // forward is breadth-first. Each visit keeps the edge it was discovered through, so the path
    // is reconstructed exactly rather than re-queried.
    let mut visits: Vec<PathVisit> = vec![PathVisit {
        key: spec.from,
        parent: None,
        via: None,
        depth: 0,
    }];
    let mut frontier = 0usize;
    let mut nodes_visited = 0u32;
    let mut truncated = false;

    while frontier < visits.len() {
        let index = frontier;
        let depth = visits[index].depth;
        let key = visits[index].key;
        frontier += 1;
        nodes_visited += 1;
        if depth >= spec.max_depth {
            continue;
        }
        if visits.len() as u32 >= spec.max_nodes {
            truncated = true;
            break;
        }

        let mut reached: Option<usize> = None;
        let mut budget_hit = false;
        graph.for_each_edge(key, spec.direction, &filter, &mut |edge| {
            let other = match spec.direction {
                Direction::Out => edge.target,
                _ => edge.source,
            };
            if other == target {
                // The target may already have been discovered on an earlier edge of this same
                // node: use its existing visit rather than pointing at the newest one.
                let found = match visits.iter().position(|visit| visit.key == other) {
                    Some(existing) => existing,
                    None => {
                        visits.push(PathVisit {
                            key: other,
                            parent: Some(index),
                            via: Some(EdgeStep {
                                kind: edge.kind,
                                source: edge.source,
                                target: edge.target,
                                confidence: edge.confidence,
                                resolved_by: edge.resolved_by,
                            }),
                            depth: depth.saturating_add(1),
                        });
                        visits.len() - 1
                    }
                };
                reached = Some(found);
                return std::ops::ControlFlow::Break(());
            }
            if visits.iter().any(|visit| visit.key == other) {
                return std::ops::ControlFlow::Continue(());
            }
            if visits.len() as u32 >= spec.max_nodes {
                budget_hit = true;
                return std::ops::ControlFlow::Break(());
            }
            visits.push(PathVisit {
                key: other,
                parent: Some(index),
                via: Some(EdgeStep {
                    kind: edge.kind,
                    source: edge.source,
                    target: edge.target,
                    confidence: edge.confidence,
                    resolved_by: edge.resolved_by,
                }),
                depth: depth.saturating_add(1),
            });
            std::ops::ControlFlow::Continue(())
        });

        if budget_hit {
            truncated = true;
            break;
        }
        if let Some(found) = reached {
            let steps = reconstruct(&visits, found);
            let weakest = min_confidence(&steps);
            return Ok(PathResult {
                path: Some(GraphPath {
                    steps,
                    min_confidence: weakest,
                }),
                truncated: false,
                nodes_visited,
            });
        }
    }

    Ok(PathResult {
        path: None,
        truncated,
        nodes_visited,
    })
}

/// One node the path search discovered.
#[derive(Debug, Clone)]
struct PathVisit {
    key: NodeKey,
    parent: Option<usize>,
    via: Option<EdgeStep>,
    depth: u8,
}

/// The steps from the seed to `index`, in order.
fn reconstruct(visits: &[PathVisit], index: usize) -> Vec<EdgeStep> {
    let mut steps = Vec::new();
    let mut cursor = index;
    while let Some(step) = visits.get(cursor).and_then(|visit| visit.via.clone()) {
        steps.push(step);
        match visits.get(cursor).and_then(|visit| visit.parent) {
            Some(parent) => cursor = parent,
            None => break,
        }
    }
    steps.reverse();
    steps
}

/// The weakest confidence along a path; `1.0` for an empty path.
#[must_use]
pub fn min_confidence(steps: &[EdgeStep]) -> Confidence {
    steps.iter().fold(Confidence::MAX, |weakest, step| {
        if step.confidence < weakest {
            step.confidence
        } else {
            weakest
        }
    })
}
