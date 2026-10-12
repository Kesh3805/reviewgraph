//! The graph access the change classifiers need (CHG-003..006), behind a small trait so the
//! classifiers do not depend on how a graph is stored. [`CodeGraphView`] adapts any
//! [`codegraph::GraphQuery`] (a built `Graph` or the PR-head `GraphOverlay`).

use std::ops::ControlFlow;

use codegraph::{Direction, EdgeFilter, EdgeKind, EdgeKindSet, GraphQuery, NodeFlags, NodeKind};

/// Edge kinds the classifiers read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Rel {
    /// `CALLS`.
    Calls,
    /// `THROWS`.
    Throws,
    /// `CATCHES`.
    Catches,
    /// `AUTHORIZES`.
    Authorizes,
    /// `VALIDATES`.
    Validates,
    /// `HANDLED_BY` (endpoint -> handler).
    HandledBy,
    /// `WRITES_TABLE`.
    WritesTable,
    /// `TESTS`.
    Tests,
    /// `READS_CONFIG`.
    ReadsConfig,
    /// `USES_TYPE`.
    UsesType,
}

impl Rel {
    fn edge_kind(self) -> EdgeKind {
        match self {
            Self::Calls => EdgeKind::Calls,
            Self::Throws => EdgeKind::Throws,
            Self::Catches => EdgeKind::Catches,
            Self::Authorizes => EdgeKind::Authorizes,
            Self::Validates => EdgeKind::Validates,
            Self::HandledBy => EdgeKind::HandledBy,
            Self::WritesTable => EdgeKind::WritesTable,
            Self::Tests => EdgeKind::Tests,
            Self::ReadsConfig => EdgeKind::ReadsConfig,
            Self::UsesType => EdgeKind::UsesType,
        }
    }
}

/// Edge direction relative to the queried node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    /// Edges leaving the node.
    Out,
    /// Edges entering the node.
    In,
}

/// A node as the classifiers see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeInfo {
    /// Canonical node id.
    pub id: String,
    /// Terminal name.
    pub name: String,
    /// Enclosing symbol id (class of a method), if any.
    pub parent: Option<String>,
    /// The node is an external package or API.
    pub external: bool,
    /// The node is an `async` callable.
    pub is_async: bool,
}

/// One edge from the queried node to `other`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EdgeInfo {
    /// Id of the node at the other end.
    pub other: String,
    /// Source line of the edge, when recorded.
    pub line: Option<u32>,
}

/// Read access to one side's graph.
pub trait GraphView: Send + Sync {
    /// The node with this id.
    fn node(&self, id: &str) -> Option<NodeInfo>;
    /// Edges of `rel` in `dir` from the node `id`, in canonical order.
    fn edges(&self, id: &str, rel: Rel, dir: Dir) -> Vec<EdgeInfo>;
}

/// Base and head graphs; either may be absent (local runs without a graph store).
#[derive(Clone, Copy, Default)]
pub struct GraphPair<'a> {
    /// Base-side graph.
    pub base: Option<&'a dyn GraphView>,
    /// Head-side graph.
    pub head: Option<&'a dyn GraphView>,
}

impl std::fmt::Debug for GraphPair<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GraphPair")
            .field("base", &self.base.is_some())
            .field("head", &self.head.is_some())
            .finish()
    }
}

impl<'a> GraphPair<'a> {
    /// No graphs.
    pub fn none() -> Self {
        Self::default()
    }

    /// The graph of one side.
    pub fn side(&self, side: review_core::location::DiffSide) -> Option<&'a dyn GraphView> {
        match side {
            review_core::location::DiffSide::Base => self.base,
            review_core::location::DiffSide::Head => self.head,
        }
    }
}

/// [`GraphView`] over a codegraph [`GraphQuery`].
pub struct CodeGraphView<'g>(pub &'g dyn GraphQuery);

impl std::fmt::Debug for CodeGraphView<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodeGraphView")
            .field("nodes", &self.0.node_count())
            .finish()
    }
}

impl GraphView for CodeGraphView<'_> {
    fn node(&self, id: &str) -> Option<NodeInfo> {
        let node = self.0.node_by_id(id)?;
        let parent = node
            .attrs
            .parent
            .and_then(|key| self.0.node(key))
            .map(|p| p.id.to_owned());
        Some(NodeInfo {
            id: node.id.to_owned(),
            name: node.name.to_owned(),
            parent,
            external: matches!(
                node.kind,
                NodeKind::ExternalDependency | NodeKind::ExternalApi | NodeKind::Package
            ),
            is_async: node.attrs.flags.contains(NodeFlags::ASYNC),
        })
    }

    fn edges(&self, id: &str, rel: Rel, dir: Dir) -> Vec<EdgeInfo> {
        let Some(node) = self.0.node_by_id(id) else {
            return Vec::new();
        };
        let key = node.key;
        let direction = match dir {
            Dir::Out => Direction::Out,
            Dir::In => Direction::In,
        };
        let filter = EdgeFilter::kinds(EdgeKindSet::of(rel.edge_kind()));
        let mut others = Vec::new();
        self.0.for_each_edge(key, direction, &filter, &mut |edge| {
            let other = if edge.source == key {
                edge.target
            } else {
                edge.source
            };
            others.push((other, edge.location.map(|(_, line, _)| line)));
            ControlFlow::Continue(())
        });
        others
            .into_iter()
            .filter_map(|(other, line)| {
                self.0.node(other).map(|n| EdgeInfo {
                    other: n.id.to_owned(),
                    line,
                })
            })
            .collect()
    }
}
