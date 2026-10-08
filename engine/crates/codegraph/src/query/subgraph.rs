//! Bounded, serializable subgraphs for the graph explorer (CG-009).
//!
//! The UI shows at most [`SUBGRAPH_MAX_NODES`] nodes per view, and that clamp lives here rather
//! than in the API layer: a caller who asks for more gets 500 and a `truncated: true`, so a
//! malicious or careless caller cannot turn one request into a hundred-megabyte payload.
//!
//! The subgraph is *induced*: every edge between two collected nodes is present, whether or not
//! the traversal followed it. That is what makes it a picture of a neighbourhood rather than a
//! path with dangling ends.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::node_id::NodeKey;
use crate::node_kind::NodeKind;
use crate::query::bfs::{bounded_bfs, MAX_DEPTH_LIMIT};
use crate::query::hex128;
use crate::query::{EdgeFilter, GraphQuery, TraversalError, TraversalSpec};
use crate::{Confidence, Direction, EdgeKindSet};

/// The hard clamp on subgraph size, enforced in the engine and not only in the UI.
pub const SUBGRAPH_MAX_NODES: u32 = 500;

/// The requested shape of a subgraph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SubgraphSpec {
    #[serde(with = "hex128::vec")]
    #[schemars(with = "Vec<String>")]
    pub seeds: Vec<NodeKey>,
    pub depth: u8,
    pub kinds: EdgeKindSet,
    pub direction: Direction,
    /// Clamped to [`SUBGRAPH_MAX_NODES`]; the clamp is reported through `truncated`.
    pub max_nodes: u32,
    pub min_confidence: Confidence,
}

impl SubgraphSpec {
    /// A spec with `max_nodes` already at the clamp.
    #[must_use]
    pub fn new(seeds: Vec<NodeKey>, depth: u8, direction: Direction, kinds: EdgeKindSet) -> Self {
        Self {
            seeds,
            depth,
            kinds,
            direction,
            max_nodes: SUBGRAPH_MAX_NODES,
            min_confidence: Confidence::MIN,
        }
    }

    /// The node budget actually used: `max_nodes` clamped to [`SUBGRAPH_MAX_NODES`].
    #[must_use]
    pub fn clamped_max_nodes(&self) -> u32 {
        self.max_nodes.min(SUBGRAPH_MAX_NODES)
    }
}

/// One node of a subgraph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SubgraphNode {
    #[serde(with = "hex128")]
    #[schemars(with = "String")]
    pub key: NodeKey,
    pub id: String,
    pub kind: NodeKind,
    pub name: String,
    pub qualified_name: String,
    /// Repository-relative path, or `None` for a synthetic node.
    pub file: Option<String>,
    pub range: Option<review_core::location::SourceRange>,
    /// Hops from the nearest seed.
    pub depth: u8,
}

/// One edge of a subgraph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SubgraphEdge {
    pub kind: crate::EdgeKind,
    #[serde(with = "hex128")]
    #[schemars(with = "String")]
    pub source: NodeKey,
    #[serde(with = "hex128")]
    #[schemars(with = "String")]
    pub target: NodeKey,
    pub confidence: Confidence,
    pub resolved_by: crate::ResolvedBy,
}

/// A bounded, induced, serializable view of part of the graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Subgraph {
    /// Sorted by key.
    pub nodes: Vec<SubgraphNode>,
    /// Sorted by identity.
    pub edges: Vec<SubgraphEdge>,
    /// True when the node budget or the edge budget stopped the collection.
    pub truncated: bool,
    /// The seeds that were actually in the graph, sorted.
    pub seeds: Vec<NodeKey>,
}

impl Subgraph {
    /// Number of nodes, for the `graph_api.subgraph` span.
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }
}

/// Collects the nodes around `spec.seeds` and every edge among them.
///
/// # Errors
///
/// [`TraversalError::NoValidSeeds`] when no seed is in the graph, and
/// [`TraversalError::BudgetTooLarge`] past the hard depth/node caps.
pub fn subgraph(graph: &dyn GraphQuery, spec: &SubgraphSpec) -> Result<Subgraph, TraversalError> {
    if spec.depth > MAX_DEPTH_LIMIT {
        return Err(TraversalError::BudgetTooLarge);
    }
    let max_nodes = spec.clamped_max_nodes();
    let traversal = bounded_bfs(
        graph,
        &TraversalSpec::new(
            spec.seeds.clone(),
            spec.direction,
            spec.kinds,
            spec.depth,
            max_nodes,
            spec.min_confidence,
        ),
    )?;

    let mut depths: Vec<(NodeKey, u8)> = traversal
        .visits
        .iter()
        .map(|visit| (visit.key, visit.depth))
        .collect();
    depths.sort();
    depths.dedup();

    let mut nodes: Vec<SubgraphNode> = Vec::with_capacity(depths.len());
    for (key, depth) in &depths {
        let Some(node) = graph.node(*key) else {
            continue;
        };
        nodes.push(SubgraphNode {
            key: *key,
            id: node.id.to_owned(),
            kind: node.kind,
            name: node.name.to_owned(),
            qualified_name: node.qualified_name.to_owned(),
            file: node.file.map(str::to_owned),
            range: node.range,
            depth: *depth,
        });
    }
    nodes.sort_by_key(|a| a.key);

    // Induced edges: for every collected node, its out-edges whose target is also collected.
    // `O(Σ deg)` over the collected nodes, and the member test is a binary search on the sorted
    // key list.
    let members: Vec<NodeKey> = depths.iter().map(|(key, _)| *key).collect();
    let filter = EdgeFilter::new(spec.kinds, spec.min_confidence);
    let mut edges: Vec<SubgraphEdge> = Vec::new();
    for (key, _) in &depths {
        graph.for_each_edge(*key, Direction::Out, &filter, &mut |edge| {
            if members.binary_search(&edge.target).is_err() {
                return std::ops::ControlFlow::Continue(());
            }
            edges.push(SubgraphEdge {
                kind: edge.kind,
                source: edge.source,
                target: edge.target,
                confidence: edge.confidence,
                resolved_by: edge.resolved_by,
            });
            std::ops::ControlFlow::Continue(())
        });
    }
    edges.sort_by(|a, b| {
        a.source
            .cmp(&b.source)
            .then_with(|| a.kind.cmp(&b.kind))
            .then_with(|| a.target.cmp(&b.target))
    });
    edges.dedup();

    Ok(Subgraph {
        nodes,
        edges,
        truncated: traversal.truncated,
        seeds: traversal
            .visits
            .iter()
            .filter(|visit| visit.depth == 0)
            .map(|visit| visit.key)
            .collect(),
    })
}

/// Node budgets above the clamp are clamped rather than rejected, so a caller that asks for "as
/// much as you have" gets 500 nodes and a `truncated` flag instead of an error.
#[must_use]
pub fn clamp_max_nodes(requested: u32) -> u32 {
    requested.min(SUBGRAPH_MAX_NODES)
}
