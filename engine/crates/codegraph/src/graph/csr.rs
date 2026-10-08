//! Compressed sparse row adjacency, partitioned by edge kind (CG-004).
//!
//! One `Csr` holds all outgoing (or all incoming) edges of every node as a single flat
//! `Vec<EdgeIx>`, addressed by `offsets[v]..offsets[v+1]`. Each node also carries a `u64`
//! kind mask: because `EdgeKind` has 33 variants with discriminants `0..=32`, one bit per
//! kind fits in a `u64`, so "does this node have any `CALLS` edge?" is a single AND instead
//! of a scan of the slice.
//!
//! Slices are stored in `(kind, other key)` order. A kind filter is therefore a
//! `partition_point` over the slice rather than a linear search, and a bounded BFS that only
//! follows two kinds touches two contiguous runs.

use crate::edge_kind::EdgeKind;

use super::{EdgeData, EdgeIx};

/// Forward or reverse adjacency of every node.
#[derive(Debug, Clone, Default)]
pub struct Csr {
    /// `len == node_count + 1`; `offsets[v]..offsets[v + 1]` is `v`'s slice.
    pub(crate) offsets: Vec<u32>,
    /// Edge indices, grouped by node and, inside each group, sorted by `(kind, other key)`.
    pub(crate) edges: Vec<EdgeIx>,
    /// One mask per node: bit `k` set iff the node has an edge of kind `k` in this direction.
    pub(crate) kind_mask: Vec<u64>,
}

impl Csr {
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds an empty CSR for `node_count` nodes from their degrees.
    ///
    /// `degrees[v]` is the number of edges in `v`'s slice; `edges` must already be grouped by
    /// node in ascending node order. Kind masks start at zero and are filled in with
    /// [`Self::mark`].
    pub fn from_degrees(degrees: &[u32], edges: Vec<EdgeIx>) -> Self {
        let mut offsets = Vec::with_capacity(degrees.len() + 1);
        offsets.push(0u32);
        let mut total = 0u64;
        for degree in degrees {
            total += u64::from(*degree);
            // Degrees come from a `u32`-indexed node table, so the running sum cannot pass
            // `u32::MAX` without the builder having returned `TooManyNodes` first.
            offsets.push(total as u32);
        }
        Self {
            offsets,
            edges,
            kind_mask: vec![0u64; degrees.len()],
        }
    }

    /// Number of nodes addressed by this CSR.
    pub fn node_count(&self) -> usize {
        self.offsets.len().saturating_sub(1)
    }

    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    /// `v`'s slice, in `(kind, other key)` order. Empty for an out-of-range node.
    pub fn slice(&self, v: usize) -> &[EdgeIx] {
        let (start, end) = self.bounds(v);
        &self.edges[start..end]
    }

    fn bounds(&self, v: usize) -> (usize, usize) {
        let start = match self.offsets.get(v) {
            Some(s) => *s as usize,
            None => return (0, 0),
        };
        let end = match self.offsets.get(v + 1) {
            Some(e) => *e as usize,
            None => return (start, start),
        };
        (start, end.min(self.edges.len()))
    }

    pub fn degree(&self, v: usize) -> usize {
        let (start, end) = self.bounds(v);
        end - start
    }

    /// The kind mask of node `v`; `0` for an out-of-range node.
    pub fn kinds(&self, v: usize) -> u64 {
        self.kind_mask.get(v).copied().unwrap_or(0)
    }

    pub fn has_kind(&self, v: usize, kind: EdgeKind) -> bool {
        self.kinds(v) & (1u64 << kind.as_u8()) != 0
    }

    /// Sets `kind`'s bit for `v`. Used while building; a finished `Csr` is never mutated.
    pub fn mark(&mut self, v: usize, kind: EdgeKind) {
        if let Some(mask) = self.kind_mask.get_mut(v) {
            *mask |= 1u64 << kind.as_u8();
        }
    }

    /// The run of `v`'s edges whose kind is exactly `kind`.
    ///
    /// Two binary searches over `v`'s kind-sorted slice: the index of the first edge of that
    /// kind, then the index one past the last. Both predicates are monotone over a slice
    /// sorted by kind, which is exactly what `partition_point` requires. Empty when `v` has
    /// no edge of `kind`, which the caller usually knows already from [`Self::kinds`].
    pub fn slice_of_kind(&self, v: usize, kind: EdgeKind, edges: &[EdgeData]) -> &[EdgeIx] {
        let slice = self.slice(v);
        let wanted = kind.as_u8();
        if slice.is_empty() || self.kinds(v) & (1u64 << wanted) == 0 {
            return &[];
        }
        let kind_of = |ix: &EdgeIx| kind_at(edges, ix);
        let start = slice.partition_point(|ix| kind_of(ix) < wanted);
        if start == slice.len() {
            return &[];
        }
        let end = slice.partition_point(|ix| kind_of(ix) <= wanted);
        &slice[start..end]
    }

    pub fn heap_size_bytes(&self) -> usize {
        self.offsets.capacity() * size_of::<u32>()
            + self.edges.capacity() * size_of::<EdgeIx>()
            + self.kind_mask.capacity() * size_of::<u64>()
    }
}

/// The kind of an edge, or `u8::MAX` for an index the table does not have (which can only be
/// a bug: `GraphBuilder::build` rejects dangling edges).
fn kind_at(edges: &[EdgeData], ix: &EdgeIx) -> u8 {
    edges
        .get(ix.get() as usize)
        .map_or(u8::MAX, |edge| edge.kind.as_u8())
}
