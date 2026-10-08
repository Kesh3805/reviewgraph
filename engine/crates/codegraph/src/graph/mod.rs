//! The immutable, `Arc`-shared in-memory graph (CG-004).
//!
//! # Shape
//!
//! `GraphBuilder` accepts files, nodes, edges and unresolved references in any order and
//! produces a `Graph` whose layout is a pure function of the *set* of inputs:
//!
//! * nodes are sorted by `(has no file, file path, key)`, so every file owns a contiguous
//!   slice of the node table and synthetic nodes (repository, queue, env var) come last;
//! * edges are sorted by `(source key, kind, target key)`;
//! * the file table is sorted by path, and strings are interned in that deterministic order.
//!
//! Two graphs built from the same multiset of inputs are therefore identical down to the
//! byte, which is what lets CG-011 assert a serialization hash and what makes an incremental
//! rebuild comparable to a from-scratch one.
//!
//! # Adjacency
//!
//! Forward and reverse CSR share one flat `Vec<EdgeIx>` per direction, grouped by node and
//! inside each group sorted by kind. Combined with a `u64` per-node kind mask, a kind-filtered
//! neighbour lookup is a binary search over one small run instead of a scan.
//!
//! # Sharing
//!
//! `Graph` has no interior mutability and is freely shared as `Arc<Graph>` across tokio tasks
//! and rayon threads. Everything that changes it goes through the builder, which is
//! single-threaded by design: the linker computes per-file edge vectors in parallel and feeds
//! them back in file order.

mod builder;
mod csr;
mod file;
mod interner;
mod node;
mod unresolved;

pub use builder::{GraphBuildError, GraphBuilder, NodeInput, NodeInputAttrs};
pub use csr::Csr;
pub use file::{FileEntry, FileInput, FileIx};
pub use interner::{Interner, StrId};
pub use node::{NodeAttrs, NodeData, NodeFlags};
pub use unresolved::{UnresolvedReason, UnresolvedRef};

use std::collections::HashMap;
use std::fmt;
use std::ops::Range;

use review_core::location::RepoPath;

use crate::confidence::Confidence;
use crate::edge::{EdgeFlags, Provenance, ResolvedBy};
use crate::edge_kind::EdgeKind;
use crate::node_id::{NodeId, NodeKey};

/// Index into the node table.
#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(transparent)]
pub struct NodeIx(u32);

impl NodeIx {
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    pub const fn get(self) -> u32 {
        self.0
    }
}

impl fmt::Debug for NodeIx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "NodeIx({})", self.0)
    }
}

impl fmt::Display for NodeIx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<u32> for NodeIx {
    fn from(raw: u32) -> Self {
        Self(raw)
    }
}

impl TryFrom<usize> for NodeIx {
    type Error = std::num::TryFromIntError;
    fn try_from(value: usize) -> Result<Self, Self::Error> {
        u32::try_from(value).map(Self)
    }
}

/// Index into the edge table.
#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(transparent)]
pub struct EdgeIx(u32);

impl EdgeIx {
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    pub const fn get(self) -> u32 {
        self.0
    }
}

impl fmt::Debug for EdgeIx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "EdgeIx({})", self.0)
    }
}

impl fmt::Display for EdgeIx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<u32> for EdgeIx {
    fn from(raw: u32) -> Self {
        Self(raw)
    }
}

impl TryFrom<usize> for EdgeIx {
    type Error = std::num::TryFromIntError;
    fn try_from(value: usize) -> Result<Self, Self::Error> {
        u32::try_from(value).map(Self)
    }
}

/// One row of the edge table.
///
/// `line`/`col` are `0` when the edge carries no location; when they are set, `origin_file` is
/// the file the location points at, which keeps the codec's round-trip exact without storing
/// the path twice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EdgeData {
    pub source: NodeIx,
    pub target: NodeIx,
    pub kind: EdgeKind,
    pub resolved_by: ResolvedBy,
    pub provenance: Provenance,
    pub flags: EdgeFlags,
    pub confidence: Confidence,
    pub origin_file: Option<FileIx>,
    pub line: u32,
    pub col: u32,
    pub occurrences: u32,
}

impl EdgeData {
    /// `line != 0` is the "has a location" bit; the builder never writes line `0`.
    pub const fn has_location(&self) -> bool {
        self.line != 0
    }
}

/// Counts a graph's size, for cache budgets, spans and the CG-004 memory assertions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GraphStats {
    pub nodes: usize,
    pub edges: usize,
    pub files: usize,
    pub unresolved: usize,
    pub strings: usize,
    pub heap_bytes: usize,
}

/// The built graph. Construct one with [`GraphBuilder`].
#[derive(Clone)]
pub struct Graph {
    pub(crate) schema_version: u32,
    pub(crate) nodes: Vec<NodeData>,
    pub(crate) by_key: HashMap<NodeKey, NodeIx>,
    pub(crate) edges: Vec<EdgeData>,
    pub(crate) fwd: Csr,
    pub(crate) rev: Csr,
    pub(crate) owned_edges: Vec<EdgeIx>,
    pub(crate) files: Vec<FileEntry>,
    pub(crate) by_path: HashMap<StrId, FileIx>,
    pub(crate) strings: Interner,
    pub(crate) unresolved: Vec<UnresolvedRef>,
    pub(crate) unresolved_by_name: HashMap<StrId, Vec<u32>>,
}

impl fmt::Debug for Graph {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Graph")
            .field("schema_version", &self.schema_version)
            .field("nodes", &self.nodes.len())
            .field("edges", &self.edges.len())
            .field("files", &self.files.len())
            .field("unresolved", &self.unresolved.len())
            .field("strings", &self.strings.len())
            .field("heap_bytes", &self.heap_size_bytes())
            .finish()
    }
}

impl Graph {
    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }

    pub fn nodes(&self) -> &[NodeData] {
        &self.nodes
    }

    pub fn edges(&self) -> &[EdgeData] {
        &self.edges
    }

    pub fn files(&self) -> &[FileEntry] {
        &self.files
    }

    pub fn unresolved(&self) -> &[UnresolvedRef] {
        &self.unresolved
    }

    pub fn strings(&self) -> &Interner {
        &self.strings
    }

    /// The interned string behind `id`, or `""` for an unknown id.
    pub fn str(&self, id: StrId) -> &str {
        self.strings.resolve(id)
    }

    pub fn node(&self, ix: NodeIx) -> Option<&NodeData> {
        self.nodes.get(ix.0 as usize)
    }

    pub fn file(&self, ix: FileIx) -> Option<&FileEntry> {
        self.files.get(ix.0 as usize)
    }

    pub fn edge(&self, ix: EdgeIx) -> Option<&EdgeData> {
        self.edges.get(ix.0 as usize)
    }

    /// Node index of a key, the hot path for edge construction and for `NodeId` lookups.
    pub fn index_of_key(&self, key: &NodeKey) -> Option<NodeIx> {
        self.by_key.get(key).copied()
    }

    /// Node index of a canonical node id.
    pub fn index_of(&self, id: &NodeId) -> Option<NodeIx> {
        self.index_of_key(&id.key())
    }

    /// The canonical id of a node, as interned.
    pub fn node_id(&self, ix: NodeIx) -> Option<StrId> {
        self.node(ix).map(|n| n.id)
    }

    /// File index of a repository-relative path.
    pub fn file_by_path(&self, path: &RepoPath) -> Option<FileIx> {
        self.strings
            .lookup(path.as_str())
            .and_then(|id| self.by_path.get(&id).copied())
    }

    /// Every node a file owns, in node-table order.
    pub fn nodes_of_file(&self, file: FileIx) -> &[NodeData] {
        match self.file(file) {
            Some(entry) => &self.nodes[range_usize(&entry.nodes)],
            None => &[],
        }
    }

    /// The edges whose `origin_file` is `file`, in canonical edge order. Used by INC-005/006
    /// to replace one file's contribution without touching the rest of the graph.
    pub fn edges_owned_by(&self, file: FileIx) -> &[EdgeIx] {
        match self.file(file) {
            Some(entry) => &self.owned_edges[range_usize(&entry.edges_owned)],
            None => &[],
        }
    }

    pub fn out_csr(&self) -> &Csr {
        &self.fwd
    }

    pub fn in_csr(&self) -> &Csr {
        &self.rev
    }

    /// Outgoing edge indices of `v`, sorted by `(kind, target key)`.
    pub fn out_edges(&self, v: NodeIx) -> &[EdgeIx] {
        self.fwd.slice(v.0 as usize)
    }

    /// Incoming edge indices of `v`, sorted by `(kind, source key)`.
    pub fn in_edges(&self, v: NodeIx) -> &[EdgeIx] {
        self.rev.slice(v.0 as usize)
    }

    /// Does `v` have an outgoing edge of `kind`? One mask bit, no slice walk.
    pub fn has_out_kind(&self, v: NodeIx, kind: EdgeKind) -> bool {
        self.fwd.has_kind(v.0 as usize, kind)
    }

    pub fn has_in_kind(&self, v: NodeIx, kind: EdgeKind) -> bool {
        self.rev.has_kind(v.0 as usize, kind)
    }

    /// Unresolved reference positions whose name is `name`, sorted by `(file, ordinal)`.
    pub fn unresolved_indices_by_name(&self, name: &str) -> Option<&[u32]> {
        self.strings
            .lookup(name)
            .and_then(|id| self.unresolved_by_name.get(&id))
            .map(Vec::as_slice)
    }

    /// The unresolved references whose name is `name`.
    pub fn unresolved_by_name(&self, name: &str) -> Vec<&UnresolvedRef> {
        self.unresolved_indices_by_name(name)
            .unwrap_or(&[])
            .iter()
            .filter_map(|i| self.unresolved.get(*i as usize))
            .collect()
    }

    pub fn stats(&self) -> GraphStats {
        GraphStats {
            nodes: self.nodes.len(),
            edges: self.edges.len(),
            files: self.files.len(),
            unresolved: self.unresolved.len(),
            strings: self.strings.len(),
            heap_bytes: self.heap_size_bytes(),
        }
    }

    /// Heap held by the graph, accurate to roughly 15% (asserted against a counting allocator
    /// by `tests/graph_build.rs`).
    ///
    /// Counts `Vec`/`HashMap` capacity rather than `len`, because capacity is what the
    /// process actually pays for.
    pub fn heap_size_bytes(&self) -> usize {
        let mut total = 0usize;
        total += self.nodes.capacity() * size_of::<NodeData>();
        total += self.by_key.capacity() * (size_of::<NodeKey>() + size_of::<NodeIx>() + 8);
        total += self.edges.capacity() * size_of::<EdgeData>();
        total += self.fwd.heap_size_bytes();
        total += self.rev.heap_size_bytes();
        total += self.owned_edges.capacity() * size_of::<EdgeIx>();
        total += self.files.capacity() * size_of::<FileEntry>();
        total += self.by_path.capacity() * (size_of::<StrId>() + size_of::<FileIx>() + 8);
        total += self.strings.heap_size_bytes();
        let unresolved_bytes: usize = self
            .unresolved
            .iter()
            .map(UnresolvedRef::heap_size_bytes)
            .sum();
        total += unresolved_bytes + self.unresolved.capacity() * size_of::<UnresolvedRef>();
        let index_entries: usize = self.unresolved_by_name.values().map(Vec::len).sum();
        total += self.unresolved_by_name.capacity() * (size_of::<StrId>() + 40);
        total += index_entries * size_of::<u32>();
        total
    }

    /// The contiguous `(start, end)` node range a file owns, for callers that want indices
    /// rather than a slice.
    pub fn file_node_range(&self, file: FileIx) -> Range<u32> {
        self.file(file)
            .map(|entry| entry.nodes.clone())
            .unwrap_or(0..0)
    }
}

/// `u32` ranges are the wire representation; slices index by `usize`.
fn range_usize(range: &Range<u32>) -> std::ops::Range<usize> {
    range.start as usize..range.end as usize
}
