//! Borrowed views over the graph's tables (CG-007).
//!
//! A view is a handful of fields plus string references into the graph's interner: no
//! allocation, no ownership, and no way for a caller to observe a half-built node. Views are
//! `Copy`, so passing one to a visitor costs nothing.

use review_core::language::Language;
use review_core::location::{ContentHash, SourceRange};

use crate::graph::{FileEntry, Graph};
use crate::node_id::NodeKey;
use crate::node_kind::NodeKind;
use crate::{
    Confidence, EdgeData, EdgeFlags, EdgeKind, NodeAttrs, NodeData, Provenance, ResolvedBy,
};

/// The key an edge endpoint resolves to when the node is missing.
///
/// `GraphBuilder::build` refuses dangling edges, so this is unreachable; it exists so that
/// building an [`EdgeRef`] never panics.
const DANGLING_ENDPOINT: NodeKey = NodeKey::from_bytes([0u8; 16]);

/// A node as a query sees it.
#[derive(Debug, Clone, Copy)]
pub struct NodeRef<'g> {
    pub key: NodeKey,
    pub kind: NodeKind,
    pub id: &'g str,
    pub name: &'g str,
    pub qualified_name: &'g str,
    /// Repository-relative path of the owning file, `None` for synthetic nodes.
    pub file: Option<&'g str>,
    pub range: Option<SourceRange>,
    pub attrs: &'g NodeAttrs,
}

impl<'g> NodeRef<'g> {
    pub(crate) fn new(graph: &'g Graph, node: &'g NodeData) -> Self {
        Self {
            key: node.key,
            kind: node.kind,
            id: graph.str(node.id),
            name: graph.str(node.name),
            qualified_name: graph.str(node.qualified_name),
            file: node
                .file
                .and_then(|ix| graph.file(ix))
                .map(|entry| graph.str(entry.path)),
            range: node.range,
            attrs: &node.attrs,
        }
    }

    /// Builds a view from already-resolved parts. The overlay keeps added nodes outside the
    /// base graph's interner, so it cannot go through [`Self::new`].
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_parts(
        key: NodeKey,
        kind: NodeKind,
        id: &'g str,
        name: &'g str,
        qualified_name: &'g str,
        file: Option<&'g str>,
        range: Option<SourceRange>,
        attrs: &'g NodeAttrs,
    ) -> Self {
        Self {
            key,
            kind,
            id,
            name,
            qualified_name,
            file,
            range,
            attrs,
        }
    }
}

/// An edge as a query sees it.
#[derive(Debug, Clone, Copy)]
pub struct EdgeRef<'g> {
    pub kind: EdgeKind,
    pub source: NodeKey,
    pub target: NodeKey,
    pub confidence: Confidence,
    pub resolved_by: ResolvedBy,
    pub provenance: Provenance,
    pub flags: EdgeFlags,
    /// `(path, line, col)`, only when the edge carries a location and an origin file.
    pub location: Option<(&'g str, u32, u32)>,
    pub occurrences: u32,
    pub origin_file: Option<&'g str>,
}

impl<'g> EdgeRef<'g> {
    pub(crate) fn new(graph: &'g Graph, edge: &'g EdgeData) -> Self {
        let origin = edge.origin_file.and_then(|ix| graph.file(ix));
        let path = origin.map(|entry| graph.str(entry.path));
        Self {
            kind: edge.kind,
            source: endpoint_key(graph, edge.source),
            target: endpoint_key(graph, edge.target),
            confidence: edge.confidence,
            resolved_by: edge.resolved_by,
            provenance: edge.provenance,
            flags: edge.flags,
            location: if edge.has_location() {
                path.map(|path| (path, edge.line, edge.col))
            } else {
                None
            },
            occurrences: edge.occurrences,
            origin_file: path,
        }
    }

    /// Builds a view from already-resolved parts, for edges the overlay holds outside the base
    /// graph's tables.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_parts(
        kind: EdgeKind,
        source: NodeKey,
        target: NodeKey,
        confidence: Confidence,
        resolved_by: ResolvedBy,
        provenance: Provenance,
        flags: EdgeFlags,
        location: Option<(&'g str, u32, u32)>,
        occurrences: u32,
        origin_file: Option<&'g str>,
    ) -> Self {
        Self {
            kind,
            source,
            target,
            confidence,
            resolved_by,
            provenance,
            flags,
            location,
            occurrences,
            origin_file,
        }
    }
}

/// The other end of an edge, resolved through the node table.
fn endpoint_key(graph: &Graph, ix: crate::graph::NodeIx) -> NodeKey {
    graph
        .node(ix)
        .map(|node| node.key)
        .unwrap_or(DANGLING_ENDPOINT)
}

/// A file as a query sees it.
#[derive(Debug, Clone, Copy)]
pub struct FileView<'g> {
    pub path: &'g str,
    pub content_hash: ContentHash,
    pub language: Language,
    pub file_version_id: Option<i64>,
}

impl<'g> FileView<'g> {
    pub(crate) fn new(graph: &'g Graph, entry: &'g FileEntry) -> Self {
        Self {
            path: graph.str(entry.path),
            content_hash: entry.content_hash,
            language: entry.language,
            file_version_id: entry.file_version_id,
        }
    }

    /// Builds a file view from already-resolved parts, for overlay file overrides.
    pub(crate) fn from_parts(
        path: &'g str,
        content_hash: ContentHash,
        language: Language,
        file_version_id: Option<i64>,
    ) -> Self {
        Self {
            path,
            content_hash,
            language,
            file_version_id,
        }
    }
}
