//! Bounded BFS over [`GraphQuery`] (CG-008).
//!
//! Impact analysis needs multi-hop reachability — callers within two hops, endpoint
//! reachability — without unbounded work on hub nodes. Every traversal takes an explicit
//! budget ([`TraversalSpec`]) and reports truncation explicitly ([`TraversalResult`]);
//! a budget stop is never silent (target architecture §3.3, master-plan principle 4).
//!
//! The algorithm is a FIFO BFS over `for_each_edge`, so the first discovery of a node is a
//! BFS-shortest path with the deterministic tie-breaking of the query layer's edge order.

use std::collections::{HashMap, VecDeque};
use std::ops::ControlFlow;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::node_id::NodeKey;
use crate::query::hex128;
use crate::{Confidence, Direction, EdgeFilter, EdgeKind, EdgeKindSet, GraphQuery, ResolvedBy};

/// Hard ceiling on `TraversalSpec::max_depth` (the API layer clamps lower).
pub const MAX_DEPTH_LIMIT: u8 = 16;
/// Hard ceiling on `TraversalSpec::max_nodes` (the API layer clamps lower).
pub const MAX_NODES_LIMIT: u32 = 1_000_000;

/// Why a traversal stopped before its frontier ran dry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Truncation {
    /// `visits` reached `max_nodes`.
    MaxNodes,
    /// `edges_examined` reached `max_edges_examined`.
    MaxEdgesExamined,
}

/// Why a traversal could not start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TraversalError {
    /// Every seed was unknown to the graph.
    #[error("none of the requested seeds is a node in this graph")]
    NoValidSeeds,
    /// A budget is over its hard ceiling.
    #[error("traversal budget out of range: max_depth <= 16, max_nodes <= 1000000")]
    BudgetTooLarge,
}

/// One edge of a reconstructable path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EdgeStep {
    pub kind: EdgeKind,
    #[serde(with = "hex128")]
    #[schemars(with = "String")]
    pub source: NodeKey,
    #[serde(with = "hex128")]
    #[schemars(with = "String")]
    pub target: NodeKey,
    pub confidence: Confidence,
    pub resolved_by: ResolvedBy,
}

/// A visited node with its parent pointer and path confidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Visit {
    #[serde(with = "hex128")]
    #[schemars(with = "String")]
    pub key: NodeKey,
    pub depth: u8,
    /// Index of the visit that discovered this one, if any.
    pub parent: Option<u32>,
    /// The edge this node was discovered through, if any.
    pub via: Option<EdgeStep>,
    /// Minimum confidence along the path from a seed, seeds included.
    pub path_confidence: Confidence,
}

/// The budget of one traversal.
///
/// `max_edges_examined` defaults to `50 × max_nodes`; use [`TraversalSpec::new`] rather
/// than struct literals so the relationship holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TraversalSpec {
    /// Start nodes. De-duplicated, sorted, unknown seeds dropped.
    #[serde(with = "hex128::vec")]
    #[schemars(with = "Vec<String>")]
    pub seeds: Vec<NodeKey>,
    pub direction: Direction,
    pub kinds: EdgeKindSet,
    pub max_depth: u8,
    pub max_nodes: u32,
    pub max_edges_examined: u32,
    pub min_confidence: Confidence,
}

impl TraversalSpec {
    /// A spec with `max_edges_examined = 50 × max_nodes`.
    #[must_use]
    pub fn new(
        seeds: Vec<NodeKey>,
        direction: Direction,
        kinds: EdgeKindSet,
        max_depth: u8,
        max_nodes: u32,
        min_confidence: Confidence,
    ) -> Self {
        Self {
            seeds,
            direction,
            kinds,
            max_depth,
            max_nodes,
            max_edges_examined: max_nodes.saturating_mul(50),
            min_confidence,
        }
    }
}

impl Default for TraversalSpec {
    fn default() -> Self {
        Self::new(
            Vec::new(),
            Direction::Out,
            EdgeKindSet::ALL,
            2,
            10_000,
            Confidence::MIN,
        )
    }
}

/// What a traversal found, how far it got, and whether it was cut short.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TraversalResult {
    /// Seeds first (depth 0), then discoveries in BFS order.
    pub visits: Vec<Visit>,
    pub truncated: bool,
    pub truncation: Option<Truncation>,
    /// Nodes recorded at `max_depth` that still have a qualifying edge.
    pub frontier_at_max_depth: u32,
    pub edges_examined: u32,
}

impl TraversalResult {
    /// The BFS path from a seed to `key`, as the edges taken, in order.
    ///
    /// `Some(vec![])` for a seed, `None` when `key` was never visited.
    #[must_use]
    pub fn path_to(&self, key: NodeKey) -> Option<Vec<EdgeStep>> {
        let mut index = self.visits.iter().position(|visit| visit.key == key)?;
        let mut steps = Vec::new();
        loop {
            let visit = self.visits.get(index)?;
            if let Some(step) = &visit.via {
                steps.push(step.clone());
            }
            match visit.parent {
                Some(parent) => index = parent as usize,
                None => break,
            }
        }
        steps.reverse();
        Some(steps)
    }

    /// The visit for `key`, if it was reached.
    #[must_use]
    pub fn visit(&self, key: NodeKey) -> Option<&Visit> {
        self.visits.iter().find(|visit| visit.key == key)
    }
}

/// BFS from `spec.seeds`, stopping at the first exhausted budget.
///
/// Complexity: O(N + X) with `N ≤ max_nodes`, `X ≤ max_edges_examined`, memory O(N).
/// Never panics on hub nodes; never does work the budget did not allow.
pub fn bounded_bfs(
    graph: &dyn GraphQuery,
    spec: &TraversalSpec,
) -> Result<TraversalResult, TraversalError> {
    if spec.max_depth > MAX_DEPTH_LIMIT || spec.max_nodes > MAX_NODES_LIMIT {
        return Err(TraversalError::BudgetTooLarge);
    }

    let mut seeds = spec.seeds.clone();
    seeds.sort_unstable();
    seeds.dedup();
    let valid: Vec<NodeKey> = seeds
        .into_iter()
        .filter(|key| graph.node(*key).is_some())
        .collect();
    if valid.is_empty() {
        return Err(TraversalError::NoValidSeeds);
    }

    let admitted = valid.len().min(spec.max_nodes as usize);
    let mut truncation = if admitted < valid.len() {
        Some(Truncation::MaxNodes)
    } else {
        None
    };
    let mut visits: Vec<Visit> = valid
        .into_iter()
        .take(admitted)
        .map(|key| Visit {
            key,
            depth: 0,
            parent: None,
            via: None,
            path_confidence: Confidence::MAX,
        })
        .collect();
    let mut seen: HashMap<NodeKey, u32> = visits
        .iter()
        .enumerate()
        .map(|(index, visit)| (visit.key, index as u32))
        .collect();
    let mut queue: VecDeque<u32> = (0..visits.len() as u32).collect();
    let filter = EdgeFilter::new(spec.kinds, spec.min_confidence);
    let mut edges_examined = 0u32;
    let mut frontier_at_max_depth = 0u32;

    'outer: while let Some(index) = queue.pop_front() {
        // A budget stop is truncation only when there is still work queued: consuming the budget
        // exactly, with an empty queue, means the traversal genuinely finished.
        if truncation.is_some() {
            break;
        }
        if visits.len() as u32 >= spec.max_nodes && !queue.is_empty() {
            truncation = Some(Truncation::MaxNodes);
            break;
        }
        if edges_examined >= spec.max_edges_examined && !queue.is_empty() {
            truncation = Some(Truncation::MaxEdgesExamined);
            break;
        }
        let depth = visits[index as usize].depth;
        let key = visits[index as usize].key;
        if depth >= spec.max_depth {
            // Recorded, not expanded: reaching max_depth is the caller's request, not
            // truncation. The frontier count is what tells them work remained.
            if graph.degree(key, spec.direction, &filter) > 0 {
                frontier_at_max_depth += 1;
            }
            continue;
        }

        let mut hit_budget: Option<Truncation> = None;
        graph.for_each_edge(key, spec.direction, &filter, &mut |edge| {
            if edges_examined >= spec.max_edges_examined {
                hit_budget = Some(Truncation::MaxEdgesExamined);
                return ControlFlow::Break(());
            }
            edges_examined += 1;
            let other = if edge.source == key {
                edge.target
            } else {
                edge.source
            };
            if seen.contains_key(&other) {
                return ControlFlow::Continue(());
            }
            if visits.len() as u32 >= spec.max_nodes {
                hit_budget = Some(Truncation::MaxNodes);
                return ControlFlow::Break(());
            }
            let parent_confidence = visits[index as usize].path_confidence;
            let path_confidence = if edge.confidence < parent_confidence {
                edge.confidence
            } else {
                parent_confidence
            };
            let visit_index = visits.len() as u32;
            seen.insert(other, visit_index);
            visits.push(Visit {
                key: other,
                depth: depth + 1,
                parent: Some(index),
                via: Some(EdgeStep {
                    kind: edge.kind,
                    source: edge.source,
                    target: edge.target,
                    confidence: edge.confidence,
                    resolved_by: edge.resolved_by,
                }),
                path_confidence,
            });
            queue.push_back(visit_index);
            ControlFlow::Continue(())
        });
        if hit_budget.is_some() {
            truncation = hit_budget;
            break 'outer;
        }
    }

    Ok(TraversalResult {
        truncated: truncation.is_some(),
        truncation,
        frontier_at_max_depth,
        edges_examined,
        visits,
    })
}
