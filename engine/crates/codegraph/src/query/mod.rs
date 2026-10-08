//! The query interface every consumer reads a graph through (CG-007).
//!
//! [`GraphQuery`] is dyn-compatible and visitor-based: a traversal takes an
//! [`EdgeFilter`] and a `&mut dyn FnMut`, allocates nothing, and stops the moment the visitor
//! returns [`ControlFlow::Break`]. [`Graph`] implements it today; the PR-head overlay
//! implements it in CG-010, and callers then cannot tell which one they hold.
//!
//! Iteration order is part of the contract, not an accident of layout:
//!
//! * edges of one direction come out sorted by kind discriminant, then by the other
//!   endpoint's key (the CSR slices are built in exactly that order),
//! * `Both` yields the out slice and then the in slice,
//! * [`GraphQuery::for_each_node`] walks the node table, which the builder put in
//!   `(has no file, file path, key)` order.

pub mod bfs;
mod filter;
pub(crate) mod hex128;
mod path;
mod subgraph;
mod views;

use std::ops::ControlFlow;

pub use bfs::{
    bounded_bfs, EdgeStep, TraversalError, TraversalResult, TraversalSpec, Truncation, Visit,
};
pub use filter::EdgeFilter;
pub use path::{shortest_path, GraphPath, PathError, PathResult, PathSpec};
pub use subgraph::{
    clamp_max_nodes, subgraph, Subgraph, SubgraphEdge, SubgraphNode, SubgraphSpec,
    SUBGRAPH_MAX_NODES,
};
pub use views::{EdgeRef, FileView, NodeRef};

use crate::graph::{FileIx, Graph, NodeIx};
use crate::node_id::{NodeId, NodeKey};
use crate::{Confidence, Direction, EdgeKindSet, ReverseView, UnresolvedRef};

/// Read-only access to a property graph.
///
/// Object safety is a requirement (CG-007 acceptance), so the trait has no generic methods
/// and no `Self: Sized` bounds: `&dyn GraphQuery` must compile.
pub trait GraphQuery: Send + Sync {
    /// The schema version the graph was built against.
    fn schema_version(&self) -> u32;

    /// The node behind `key`.
    fn node(&self, key: NodeKey) -> Option<NodeRef<'_>>;

    /// The node with this canonical id, for callers that hold the id string rather than its
    /// hash. Unknown ids return `None`.
    fn node_by_id(&self, id: &str) -> Option<NodeRef<'_>>;

    /// Every edge of `key` in `dir` that passes `filter`, in canonical order.
    ///
    /// A visitor returning [`ControlFlow::Break`] stops the walk immediately; the rest of the
    /// slice is untouched.
    fn for_each_edge<'a>(
        &'a self,
        key: NodeKey,
        dir: Direction,
        filter: &EdgeFilter,
        visit: &mut dyn FnMut(EdgeRef<'a>) -> ControlFlow<()>,
    );

    /// How many edges of `key` pass `filter` — a count without building a single view.
    fn degree(&self, key: NodeKey, dir: Direction, filter: &EdgeFilter) -> usize;

    fn node_count(&self) -> usize;

    fn edge_count(&self) -> usize;

    /// Every node, in the graph's canonical layout order.
    fn for_each_node<'a>(&'a self, visit: &mut dyn FnMut(NodeRef<'a>));

    /// Every node of a file, in node-table order. Unknown paths visit nothing.
    fn nodes_in_file<'a>(&'a self, path: &str, visit: &mut dyn FnMut(NodeRef<'a>));

    /// Every edge a file owns (its `origin_file`), in canonical edge order.
    fn edges_owned_by<'a>(&'a self, path: &str, visit: &mut dyn FnMut(EdgeRef<'a>));

    /// The unresolved references whose name is `name`, in `(file, ordinal)` order.
    fn unresolved_named<'a>(&'a self, name: &str, visit: &mut dyn FnMut(&'a UnresolvedRef));

    /// Every unresolved reference, in `(file, ordinal, name)` order.
    ///
    /// CG-012's `compare` needs the whole list, not a name-filtered view, so the traversal is
    /// part of the interface rather than a `Graph`-only extra: an overlay's replaced lists and its
    /// base's leftovers have to be compared as one sequence.
    fn for_each_unresolved<'a>(&'a self, visit: &mut dyn FnMut(&'a UnresolvedRef));

    /// File metadata by repository-relative path.
    fn file(&self, path: &str) -> Option<FileView<'_>>;
}

/// Collecting helpers over [`GraphQuery`], for callers that would rather have a `Vec`.
pub trait GraphQueryExt: GraphQuery {
    /// Out-edges of `k` restricted to `kinds`.
    fn out_edges(&self, key: NodeKey, kinds: EdgeKindSet) -> Vec<EdgeRef<'_>> {
        let mut out = Vec::new();
        self.for_each_edge(
            key,
            Direction::Out,
            &EdgeFilter::kinds(kinds),
            &mut |edge| {
                out.push(edge);
                ControlFlow::Continue(())
            },
        );
        out
    }

    /// In-edges of `k` restricted to `kinds`.
    fn in_edges(&self, key: NodeKey, kinds: EdgeKindSet) -> Vec<EdgeRef<'_>> {
        let mut out = Vec::new();
        self.for_each_edge(key, Direction::In, &EdgeFilter::kinds(kinds), &mut |edge| {
            out.push(edge);
            ControlFlow::Continue(())
        });
        out
    }

    /// The distinct nodes on the other side of the matching edges, sorted by key.
    ///
    /// A node reachable through two kinds appears once; a self-loop contributes `k` itself.
    fn neighbors(
        &self,
        key: NodeKey,
        dir: Direction,
        kinds: EdgeKindSet,
        min_confidence: Confidence,
    ) -> Vec<NodeKey> {
        let filter = EdgeFilter::new(kinds, min_confidence);
        let mut seen: Vec<NodeKey> = Vec::new();
        self.for_each_edge(key, dir, &filter, &mut |edge| {
            match dir {
                Direction::Out => seen.push(edge.target),
                Direction::In => seen.push(edge.source),
                Direction::Both => {
                    seen.push(edge.target);
                    seen.push(edge.source);
                }
            }
            ControlFlow::Continue(())
        });
        seen.sort_unstable();
        seen.dedup();
        seen
    }

    /// The in-edges behind a reverse view: `CALLED_BY(k)` is `in_edges(k, CALLS)`.
    fn view(&self, key: NodeKey, view: ReverseView) -> Vec<EdgeRef<'_>> {
        self.in_edges(key, EdgeKindSet::of(view.underlying()))
    }
}

impl<T: GraphQuery + ?Sized> GraphQueryExt for T {}

impl GraphQuery for Graph {
    fn schema_version(&self) -> u32 {
        Graph::schema_version(self)
    }

    fn node(&self, key: NodeKey) -> Option<NodeRef<'_>> {
        let index = Graph::index_of_key(self, &key)?;
        Graph::node(self, index).map(|node| NodeRef::new(self, node))
    }

    fn node_by_id(&self, id: &str) -> Option<NodeRef<'_>> {
        <Self as GraphQuery>::node(self, NodeId::from_canonical(id).key())
    }

    fn for_each_edge<'a>(
        &'a self,
        key: NodeKey,
        dir: Direction,
        filter: &EdgeFilter,
        visit: &mut dyn FnMut(EdgeRef<'a>) -> ControlFlow<()>,
    ) {
        let Some(index) = self.index_of_key(&key) else {
            return;
        };
        match dir {
            Direction::Out => {
                walk(self, index, false, filter, visit);
            }
            Direction::In => {
                walk(self, index, true, filter, visit);
            }
            Direction::Both => {
                if walk(self, index, false, filter, visit) {
                    walk(self, index, true, filter, visit);
                }
            }
        }
    }

    fn degree(&self, key: NodeKey, dir: Direction, filter: &EdgeFilter) -> usize {
        let Some(index) = Graph::index_of_key(self, &key) else {
            return 0;
        };
        let forward = matches!(dir, Direction::Out | Direction::Both);
        let reverse = matches!(dir, Direction::In | Direction::Both);
        let mut count = 0;
        if forward {
            count += count_walk(self, index, false, filter);
        }
        if reverse {
            count += count_walk(self, index, true, filter);
        }
        count
    }

    fn node_count(&self) -> usize {
        self.nodes().len()
    }

    fn edge_count(&self) -> usize {
        self.edges().len()
    }

    fn for_each_node<'a>(&'a self, visit: &mut dyn FnMut(NodeRef<'a>)) {
        for node in self.nodes() {
            visit(NodeRef::new(self, node));
        }
    }

    fn nodes_in_file<'a>(&'a self, path: &str, visit: &mut dyn FnMut(NodeRef<'a>)) {
        let Some(file) = file_ix(self, path) else {
            return;
        };
        for node in self.nodes_of_file(file) {
            visit(NodeRef::new(self, node));
        }
    }

    fn edges_owned_by<'a>(&'a self, path: &str, visit: &mut dyn FnMut(EdgeRef<'a>)) {
        let Some(file) = file_ix(self, path) else {
            return;
        };
        for index in Graph::edges_owned_by(self, file) {
            if let Some(edge) = self.edge(*index) {
                visit(EdgeRef::new(self, edge));
            }
        }
    }

    fn unresolved_named<'a>(&'a self, name: &str, visit: &mut dyn FnMut(&'a UnresolvedRef)) {
        let Some(indices) = self.unresolved_indices_by_name(name) else {
            return;
        };
        for index in indices {
            if let Some(reference) = self.unresolved().get(*index as usize) {
                visit(reference);
            }
        }
    }

    fn for_each_unresolved<'a>(&'a self, visit: &mut dyn FnMut(&'a UnresolvedRef)) {
        for reference in self.unresolved() {
            visit(reference);
        }
    }

    fn file(&self, path: &str) -> Option<FileView<'_>> {
        let index = file_ix(self, path)?;
        let entry = Graph::file(self, index)?;
        Some(FileView::new(self, entry))
    }
}

/// Resolves a wire path to a file index. An unparseable path is simply not in the graph.
fn file_ix(graph: &Graph, path: &str) -> Option<FileIx> {
    review_core::location::RepoPath::new(path)
        .ok()
        .and_then(|path| graph.file_by_path(&path))
}

/// The runs of one node's slice that a filter wants, in visit order: the whole slice when
/// the filter asks for every kind the node has (the common case), otherwise one kind-ordered
/// run per requested kind, so a mask whose kinds are not adjacent in the discriminant space
/// is still answered exactly.
fn for_each_run(
    graph: &Graph,
    index: NodeIx,
    reverse: bool,
    filter: &EdgeFilter,
    mut f: impl FnMut(&[crate::graph::EdgeIx]),
) {
    let csr = if reverse {
        graph.in_csr()
    } else {
        graph.out_csr()
    };
    let raw = index.get() as usize;
    let node_mask = csr.kinds(raw);
    let wanted = filter.kinds.bits() & node_mask;
    if wanted == 0 {
        return;
    }
    if wanted == node_mask {
        f(csr.slice(raw));
        return;
    }
    for kind in filter.kinds.iter() {
        if wanted & (1u64 << kind.as_u8()) == 0 {
            continue;
        }
        f(csr.slice_of_kind(raw, kind, graph.edges()));
    }
}

/// Walks one direction of one node's slice, stopping early when the visitor breaks.
///
/// Returns `false` when the visitor broke, so [`Direction::Both`] can skip the reverse half.
fn walk<'a>(
    graph: &'a Graph,
    index: NodeIx,
    reverse: bool,
    filter: &EdgeFilter,
    visit: &mut dyn FnMut(EdgeRef<'a>) -> ControlFlow<()>,
) -> bool {
    let mut proceed = true;
    for_each_run(graph, index, reverse, filter, |run| {
        if !proceed {
            return;
        }
        for edge_index in run {
            let Some(edge) = graph.edge(*edge_index) else {
                continue;
            };
            if edge.confidence < filter.min_confidence {
                continue;
            }
            if matches!(visit(EdgeRef::new(graph, edge)), ControlFlow::Break(_)) {
                proceed = false;
                return;
            }
        }
    });
    proceed
}

/// [`GraphQuery::degree`] without building a view per edge: same runs, same confidence
/// floor, no string resolution.
fn count_walk(graph: &Graph, index: NodeIx, reverse: bool, filter: &EdgeFilter) -> usize {
    let mut count = 0;
    for_each_run(graph, index, reverse, filter, |run| {
        count += run
            .iter()
            .filter(|edge_index| {
                graph
                    .edge(**edge_index)
                    .is_some_and(|edge| edge.confidence >= filter.min_confidence)
            })
            .count();
    });
    count
}
