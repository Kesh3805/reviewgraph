//! `GraphBuilder`: turns unordered inputs into the deterministic `Graph` layout (CG-004).

use std::collections::HashMap;
use std::ops::Range;

use analysis_ir::symbol::Visibility;
use review_core::location::{RepoPath, SourceRange};
use review_core::symbol::Hash128;
use serde::{Deserialize, Serialize};

use crate::edge::{Edge, EdgeIdentity};
use crate::node_id::{NodeId, NodeKey};
use crate::node_kind::NodeKind;
use crate::schema::SCHEMA_VERSION;

use super::file::{FileEntry, FileInput, FileIx};
use super::interner::Interner;
use super::node::{NodeAttrs, NodeData, NodeFlags};
use super::unresolved::UnresolvedRef;
use super::{Csr, EdgeData, EdgeIx, Graph, NodeIx};

/// Attribute block as the caller supplies it: strings instead of [`super::StrId`]s, because
/// nothing has been interned yet.
///
/// Serializable because a [`crate::GraphDelta`] carries nodes in exactly this shape (CG-010) and
/// the wire codec (CG-011) re-runs the builder on it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeInputAttrs {
    pub visibility: Visibility,
    pub flags: NodeFlags,
    /// Display signature, already truncated to at most 512 characters by the analyzer.
    pub signature: Option<String>,
    pub body_hash: Option<Hash128>,
    pub signature_hash: Option<Hash128>,
    pub parent: Option<NodeKey>,
    /// Analyzer-specific key/value pairs; the builder sorts them by `(key, value)` so the
    /// resulting graph does not depend on insertion order.
    pub extra: Vec<(String, String)>,
}

impl Default for NodeInputAttrs {
    fn default() -> Self {
        Self {
            visibility: Visibility::Public,
            flags: NodeFlags::EMPTY,
            signature: None,
            body_hash: None,
            signature_hash: None,
            parent: None,
            extra: Vec::new(),
        }
    }
}

/// A node before it has a position in the table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeInput {
    pub id: NodeId,
    pub kind: NodeKind,
    pub name: String,
    pub qualified_name: String,
    /// Owning file. `None` marks a synthetic node (repository, queue, env var, package).
    pub file: Option<RepoPath>,
    pub range: Option<SourceRange>,
    pub attrs: NodeInputAttrs,
}

impl NodeInput {
    /// A node with the default attributes, a name equal to its qualified name and no file.
    pub fn new(id: NodeId, kind: NodeKind, name: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            qualified_name: name.clone(),
            id,
            kind,
            name,
            file: None,
            range: None,
            attrs: NodeInputAttrs::default(),
        }
    }

    pub fn qualified_name(mut self, qualified: impl Into<String>) -> Self {
        self.qualified_name = qualified.into();
        self
    }

    pub fn in_file(mut self, path: RepoPath) -> Self {
        self.file = Some(path);
        self
    }

    pub fn with_range(mut self, range: SourceRange) -> Self {
        self.range = Some(range);
        self
    }

    pub fn with_attrs(mut self, attrs: NodeInputAttrs) -> Self {
        self.attrs = attrs;
        self
    }
}

/// Why `GraphBuilder::build` refused the inputs.
///
/// Every variant is a hard failure: the graph is never built with a silently repaired shape,
/// because a repaired graph is one the incremental stages cannot reproduce.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GraphBuildError {
    /// Two different node ids hashed to the same 128-bit key.
    #[error("node key collision: {key} is claimed by both {id_a:?} and {id_b:?}")]
    KeyCollision {
        key: NodeKey,
        id_a: String,
        id_b: String,
    },
    /// An edge points at a key that is not in the node table.
    #[error("edge {identity} has an endpoint that is not a node")]
    DanglingEdge { identity: EdgeIdentity },
    /// The node table would not fit in `u32` indices.
    #[error("graph would need more than {} nodes", u32::MAX - 1)]
    TooManyNodes,
    /// The edge table would not fit in `u32` indices.
    #[error("graph would need more than {} edges", u32::MAX - 1)]
    TooManyEdges,
    /// The string table would not fit in `u32` ids.
    #[error("graph would need more than {} distinct strings", u32::MAX)]
    TooManyStrings,
    /// The caller asked for a schema this build does not speak.
    #[error("schema version {found} requested, this build speaks {expected}")]
    SchemaVersionMismatch { expected: u32, found: u32 },
}

/// Accumulates files, nodes, edges and unresolved references in any order and freezes them
/// into a [`Graph`].
///
/// The builder is single-threaded by design (CG-004): the linker computes per-file edge
/// vectors in parallel and feeds them back in file order, which keeps the merge trivially
/// deterministic.
#[derive(Debug)]
pub struct GraphBuilder {
    schema_version: u32,
    files: HashMap<RepoPath, FileInput>,
    nodes: Vec<NodeInput>,
    node_index: HashMap<NodeKey, usize>,
    edges: Vec<Edge>,
    edge_index: HashMap<EdgeIdentity, usize>,
    unresolved: Vec<UnresolvedRef>,
}

impl GraphBuilder {
    /// `schema_version` must be [`SCHEMA_VERSION`]; anything else fails at `build()`.
    pub fn new(schema_version: u32) -> Self {
        Self {
            schema_version,
            files: HashMap::new(),
            nodes: Vec::new(),
            node_index: HashMap::new(),
            edges: Vec::new(),
            edge_index: HashMap::new(),
            unresolved: Vec::new(),
        }
    }

    /// Declares a file. Re-declaring a path keeps the first declaration, so a repository that
    /// lists the same file twice still builds the same graph.
    pub fn add_file(&mut self, file: FileInput) -> Result<(), GraphBuildError> {
        self.files.entry(file.path.clone()).or_insert(file);
        Ok(())
    }

    /// Adds a node, rejecting a blake3-128 collision between two different ids.
    ///
    /// Adding the same id twice is idempotent: the first declaration wins.
    pub fn add_node(&mut self, node: NodeInput) -> Result<(), GraphBuildError> {
        self.insert_node(node, None)
    }

    fn insert_node(
        &mut self,
        node: NodeInput,
        key_override: Option<NodeKey>,
    ) -> Result<(), GraphBuildError> {
        let key = key_override.unwrap_or_else(|| node.id.key());
        if let Some(&existing) = self.node_index.get(&key) {
            let id_a = self.nodes[existing].id.as_str();
            let id_b = node.id.as_str();
            if id_a != id_b {
                return Err(GraphBuildError::KeyCollision {
                    key,
                    id_a: id_a.to_owned(),
                    id_b: id_b.to_owned(),
                });
            }
            return Ok(());
        }
        self.node_index.insert(key, self.nodes.len());
        self.nodes.push(node);
        Ok(())
    }

    /// Adds an edge. A second edge with the same `(source, kind, target)` identity is merged
    /// with [`Edge::merge_occurrence`] instead of creating a parallel row.
    pub fn add_edge(&mut self, edge: Edge) {
        let identity = edge.identity();
        match self.edge_index.get(&identity) {
            Some(&index) => self.edges[index].merge_occurrence(&edge),
            None => {
                self.edge_index.insert(identity, self.edges.len());
                self.edges.push(edge);
            }
        }
    }

    pub fn add_unresolved(&mut self, reference: UnresolvedRef) {
        self.unresolved.push(reference);
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    /// Freezes the inputs.
    ///
    /// Order of work: validate sizes, collect the file table, sort nodes and edges into their
    /// canonical orders, intern strings in that order, then scatter into the forward and
    /// reverse CSR.
    pub fn build(self) -> Result<Graph, GraphBuildError> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(GraphBuildError::SchemaVersionMismatch {
                expected: SCHEMA_VERSION,
                found: self.schema_version,
            });
        }
        if self.nodes.len() >= u32::MAX as usize {
            return Err(GraphBuildError::TooManyNodes);
        }
        if self.edges.len() >= u32::MAX as usize {
            return Err(GraphBuildError::TooManyEdges);
        }

        // 1. File table: declared files plus every path a node or an edge points at.
        let mut paths: Vec<RepoPath> = self.files.keys().cloned().collect();
        for node in &self.nodes {
            if let Some(path) = &node.file {
                if !self.files.contains_key(path) {
                    paths.push(path.clone());
                }
            }
        }
        for edge in &self.edges {
            if let Some(path) = &edge.origin_file {
                if !self.files.contains_key(path) {
                    paths.push(path.clone());
                }
            }
        }
        paths.sort_unstable();
        paths.dedup();

        let mut files: Vec<FileEntry> = Vec::with_capacity(paths.len());
        let mut path_ix: HashMap<&RepoPath, FileIx> = HashMap::with_capacity(paths.len());
        let mut strings = Interner::new();
        for (raw, path) in paths.iter().enumerate() {
            let ix = FileIx::new(raw as u32);
            let path_id = intern(&mut strings, path.as_str())?;
            let (file_version_id, content_hash, language) = match self.files.get(path) {
                Some(declared) => (
                    declared.file_version_id,
                    declared.content_hash,
                    declared.language,
                ),
                None => {
                    let placeholder = FileInput::placeholder(path.clone());
                    (
                        placeholder.file_version_id,
                        placeholder.content_hash,
                        placeholder.language,
                    )
                }
            };
            files.push(FileEntry {
                path: path_id,
                file_version_id,
                content_hash,
                language,
                nodes: 0..0,
                edges_owned: 0..0,
            });
            path_ix.insert(path, ix);
        }
        let file_ix_of = |path: &RepoPath| path_ix.get(path).copied();

        // 2. Nodes: sort into `(has no file, file path, key)`.
        let mut nodes = self.nodes;
        nodes.sort_unstable_by(|a, b| {
            a.file
                .is_none()
                .cmp(&b.file.is_none())
                .then_with(|| a.file.cmp(&b.file))
                .then_with(|| a.id.key().cmp(&b.id.key()))
        });

        // 3. Intern strings in node order: id, name, qualified name, signature, extras.
        let mut node_data: Vec<NodeData> = Vec::with_capacity(nodes.len());
        let mut by_key: HashMap<NodeKey, NodeIx> = HashMap::with_capacity(nodes.len());
        for (raw, node) in nodes.iter().enumerate() {
            let ix = NodeIx::new(raw as u32);
            let id = intern(&mut strings, node.id.as_str())?;
            let name = intern(&mut strings, &node.name)?;
            let qualified_name = intern(&mut strings, &node.qualified_name)?;
            let signature = match &node.attrs.signature {
                Some(text) => Some(intern(&mut strings, text)?),
                None => None,
            };
            let mut extra_pairs = node.attrs.extra.clone();
            extra_pairs.sort_unstable();
            extra_pairs.dedup();
            let extra = if extra_pairs.is_empty() {
                None
            } else {
                let mut pairs = Vec::with_capacity(extra_pairs.len());
                for (key, value) in extra_pairs {
                    pairs.push((intern(&mut strings, &key)?, intern(&mut strings, &value)?));
                }
                Some(pairs.into_boxed_slice())
            };
            let file = node.file.as_ref().and_then(file_ix_of);
            by_key.insert(node.id.key(), ix);
            node_data.push(NodeData {
                key: node.id.key(),
                kind: node.kind,
                id,
                name,
                qualified_name,
                file,
                range: node.range,
                attrs: NodeAttrs {
                    visibility: node.attrs.visibility,
                    flags: node.attrs.flags,
                    signature,
                    body_hash: node.attrs.body_hash,
                    signature_hash: node.attrs.signature_hash,
                    parent: node.attrs.parent,
                    extra,
                },
            });
        }

        // 4. Per-file node ranges. Nodes are already grouped by file path, so a single pass
        //    over the sorted table yields contiguous ranges.
        let mut ranges: Vec<Option<Range<u32>>> = vec![None; files.len()];
        for (raw, node) in node_data.iter().enumerate() {
            if let Some(file) = node.file {
                let raw = raw as u32;
                match ranges[file.0 as usize] {
                    Some(ref mut range) => range.end = raw + 1,
                    None => ranges[file.0 as usize] = Some(raw..raw + 1),
                }
            }
        }
        for (ix, range) in ranges.into_iter().enumerate() {
            files[ix].nodes = range.unwrap_or(0..0);
        }

        // 5. Edges: resolve endpoints, sort by `(source key, kind, target key)`.
        let mut edges: Vec<(EdgeData, Edge)> = Vec::with_capacity(self.edges.len());
        for edge in self.edges {
            let identity = edge.identity();
            let source = by_key
                .get(&edge.source)
                .copied()
                .ok_or(GraphBuildError::DanglingEdge { identity })?;
            let target = by_key
                .get(&edge.target)
                .copied()
                .ok_or(GraphBuildError::DanglingEdge { identity })?;
            let origin_file = match &edge.origin_file {
                Some(path) => {
                    Some(file_ix_of(path).ok_or(GraphBuildError::DanglingEdge { identity })?)
                }
                None => None,
            };
            let (line, col) = match &edge.location {
                Some(location) => (location.line, location.col),
                None => (0, 0),
            };
            edges.push((
                EdgeData {
                    source,
                    target,
                    kind: edge.kind,
                    resolved_by: edge.resolved_by,
                    provenance: edge.provenance,
                    flags: edge.flags,
                    confidence: edge.confidence,
                    origin_file,
                    line,
                    col,
                    occurrences: edge.occurrences,
                },
                edge,
            ));
        }
        edges.sort_unstable_by(|(a, ae), (b, be)| {
            ae.source
                .cmp(&be.source)
                .then_with(|| a.kind.cmp(&b.kind))
                .then_with(|| ae.target.cmp(&be.target))
        });
        let edge_data: Vec<EdgeData> = edges.into_iter().map(|(data, _)| data).collect();

        // 6. Forward CSR: degrees by source, then scatter in canonical edge order so each
        //    slice keeps `(kind, target key)`.
        let mut out_degree = vec![0u32; node_data.len()];
        for data in &edge_data {
            out_degree[data.source.0 as usize] += 1;
        }
        let mut fwd = Csr::from_degrees(&out_degree, vec![EdgeIx::new(0); edge_data.len()]);
        let mut out_cursor = fwd.offsets.clone();
        for (raw, data) in edge_data.iter().enumerate() {
            let slot = &mut out_cursor[data.source.0 as usize];
            fwd.edges[*slot as usize] = EdgeIx::new(raw as u32);
            *slot += 1;
            fwd.mark(data.source.0 as usize, data.kind);
        }

        // 7. Reverse CSR: scatter by target, then counting-sort each slice by kind. The
        //    scatter leaves slices ordered by source key, and a stable counting sort by kind
        //    therefore yields `(kind, source key)`.
        let mut in_degree = vec![0u32; node_data.len()];
        for data in &edge_data {
            in_degree[data.target.0 as usize] += 1;
        }
        let mut rev = Csr::from_degrees(&in_degree, vec![EdgeIx::new(0); edge_data.len()]);
        let mut in_cursor = rev.offsets.clone();
        for (raw, data) in edge_data.iter().enumerate() {
            let slot = &mut in_cursor[data.target.0 as usize];
            rev.edges[*slot as usize] = EdgeIx::new(raw as u32);
            *slot += 1;
            rev.mark(data.target.0 as usize, data.kind);
        }
        counting_sort_by_kind(&mut rev, &edge_data);

        // 8. Per-file owned edges: counting sort by origin file, preserving edge order.
        let mut owned_offsets = vec![0u32; files.len() + 1];
        for data in &edge_data {
            if let Some(file) = data.origin_file {
                owned_offsets[file.0 as usize + 1] += 1;
            }
        }
        for raw in 0..files.len() {
            owned_offsets[raw + 1] += owned_offsets[raw];
        }
        let mut owned_edges = vec![EdgeIx::new(0); owned_offsets[files.len()] as usize];
        let mut owned_cursor = owned_offsets.clone();
        for (raw, data) in edge_data.iter().enumerate() {
            if let Some(file) = data.origin_file {
                let slot = &mut owned_cursor[file.0 as usize];
                owned_edges[*slot as usize] = EdgeIx::new(raw as u32);
                *slot += 1;
            }
        }
        for (ix, window) in owned_offsets.windows(2).enumerate() {
            files[ix].edges_owned = window[0]..window[1];
        }

        // 9. Unresolved references, ordered by `(file, ordinal, name)`.
        let mut unresolved = self.unresolved;
        unresolved.sort_unstable_by(|a, b| {
            a.file
                .cmp(&b.file)
                .then_with(|| a.ordinal.cmp(&b.ordinal))
                .then_with(|| a.name.cmp(&b.name))
        });
        let mut unresolved_by_name: HashMap<super::StrId, Vec<u32>> = HashMap::new();
        for (raw, reference) in unresolved.iter().enumerate() {
            let name = intern(&mut strings, &reference.name)?;
            unresolved_by_name.entry(name).or_default().push(raw as u32);
        }

        let mut by_path = HashMap::with_capacity(files.len());
        for (raw, entry) in files.iter().enumerate() {
            by_path.insert(entry.path, FileIx::new(raw as u32));
        }

        Ok(Graph {
            schema_version: SCHEMA_VERSION,
            nodes: node_data,
            by_key,
            edges: edge_data,
            fwd,
            rev,
            owned_edges,
            files,
            by_path,
            strings,
            unresolved,
            unresolved_by_name,
        })
    }
}

fn intern(strings: &mut Interner, value: &str) -> Result<super::StrId, GraphBuildError> {
    strings.intern(value).ok_or(GraphBuildError::TooManyStrings)
}

/// Stable counting sort of every reverse slice by `EdgeKind` (33 buckets, discriminants
/// `0..=32`).
fn counting_sort_by_kind(rev: &mut Csr, edges: &[EdgeData]) {
    let mut buffer: Vec<EdgeIx> = Vec::new();
    for node in 0..rev.node_count() {
        let start = rev.offsets[node] as usize;
        let end = rev.offsets[node + 1] as usize;
        if end <= start {
            continue;
        }
        let mut counts = [0usize; 33];
        for ix in &rev.edges[start..end] {
            let kind = edges[ix.0 as usize].kind.as_u8() as usize;
            counts[kind.min(32)] += 1;
        }
        let mut starts = [0usize; 33];
        let mut total = 0usize;
        for (slot, count) in counts.iter().enumerate() {
            starts[slot] = total;
            total += count;
        }
        buffer.clear();
        buffer.resize(end - start, EdgeIx::new(0));
        for ix in &rev.edges[start..end] {
            let kind = edges[ix.0 as usize].kind.as_u8() as usize;
            let slot = starts[kind.min(32)];
            buffer[slot] = *ix;
            starts[kind.min(32)] += 1;
        }
        rev.edges[start..end].copy_from_slice(&buffer);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::confidence::Confidence;
    use crate::edge::{Provenance, ResolvedBy};
    use crate::edge_kind::EdgeKind;
    use crate::node_id::NodeId;
    use review_core::location::RepoPath;

    fn path(s: &str) -> RepoPath {
        RepoPath::new(s).unwrap()
    }

    fn node(id: &str) -> NodeInput {
        NodeInput::new(
            NodeId::from_canonical(id),
            NodeKind::Function,
            id.rsplit('#').next().unwrap_or(id),
        )
    }

    fn edge(source: &str, target: &str) -> Edge {
        Edge::new(
            EdgeKind::Calls,
            NodeId::from_canonical(source).key(),
            NodeId::from_canonical(target).key(),
            Confidence::from_f32(0.95),
            ResolvedBy::Import,
            Provenance::Linker,
        )
    }

    #[test]
    fn key_collision_is_reported() {
        let mut builder = GraphBuilder::new(SCHEMA_VERSION);
        let first = node("ts:src/a.ts#A/function");
        let forced = first.id.key();
        builder.insert_node(first, Some(forced)).unwrap();

        // Same key, different id: a blake3-128 collision, never merged silently.
        let mut other = node("ts:src/b.ts#B/function");
        other.name = "other".to_owned();
        let err = builder.insert_node(other, Some(forced)).unwrap_err();
        assert!(matches!(
            err,
            GraphBuildError::KeyCollision { ref id_a, ref id_b, .. } if id_a != id_b
        ));
    }

    #[test]
    fn duplicate_node_id_is_idempotent() {
        let mut builder = GraphBuilder::new(SCHEMA_VERSION);
        builder.add_node(node("ts:src/a.ts#A/function")).unwrap();
        builder.add_node(node("ts:src/a.ts#A/function")).unwrap();
        assert_eq!(builder.node_count(), 1);
    }

    #[test]
    fn duplicate_edge_identity_merges_occurrences() {
        let mut builder = GraphBuilder::new(SCHEMA_VERSION);
        builder.add_edge(edge("ts:src/a.ts#A/f", "ts:src/b.ts#B/g"));
        builder.add_edge(edge("ts:src/a.ts#A/f", "ts:src/b.ts#B/g"));
        assert_eq!(builder.edge_count(), 1);
        assert_eq!(builder.edges[0].occurrences, 2);
    }

    #[test]
    fn dangling_edge_is_rejected() {
        let mut builder = GraphBuilder::new(SCHEMA_VERSION);
        builder.add_node(node("ts:src/a.ts#A/function")).unwrap();
        builder.add_edge(edge("ts:src/a.ts#A/function", "ts:src/ghost.ts#G/function"));
        let err = builder.build().unwrap_err();
        assert!(matches!(err, GraphBuildError::DanglingEdge { .. }));
    }

    #[test]
    fn schema_version_mismatch_is_rejected() {
        let err = GraphBuilder::new(SCHEMA_VERSION + 1).build().unwrap_err();
        assert_eq!(
            err,
            GraphBuildError::SchemaVersionMismatch {
                expected: SCHEMA_VERSION,
                found: SCHEMA_VERSION + 1
            }
        );
    }

    #[test]
    fn undeclared_files_are_registered_from_nodes() {
        let mut builder = GraphBuilder::new(SCHEMA_VERSION);
        builder
            .add_node(node("ts:src/a.ts#A/function").in_file(path("src/a.ts")))
            .unwrap();
        let graph = builder.build().unwrap();
        assert_eq!(graph.files().len(), 1);
        assert_eq!(graph.nodes_of_file(FileIx::new(0)).len(), 1);
    }
}
