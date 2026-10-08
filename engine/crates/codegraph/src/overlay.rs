//! The PR-head overlay: a base graph plus a delta, queried as if it were one graph (CG-010).
//!
//! A PR never copies the base (target-architecture §3.3). [`GraphOverlay`] holds
//! `Arc<Graph>` + `Arc<GraphDelta>` and implements [`GraphQuery`], so every consumer — impact,
//! context, verification, the API — works unchanged against a base snapshot or a PR head and
//! cannot tell which one it holds.
//!
//! # Merge
//!
//! [`GraphQuery::for_each_edge`] walks the base slice and the delta's added slice with two
//! cursors. Both are already sorted by `(kind, other key)`: the base CSR slices are built in that
//! order, and the delta's per-node lists are sorted the same way at construction. The merge is
//! therefore allocation-free and produces exactly the order [`GraphOverlay::flatten`] would,
//! which is what makes "overlay queries == flattened graph queries" a property rather than a
//! hope.
//!
//! Cost: construction `O(|Δ| log |Δ|)`, a query `O(base cost + added degree)`, memory `O(|Δ|)`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::ControlFlow;
use std::sync::Arc;

use review_core::location::RepoPath;

use crate::delta::{FileChangeKind, GraphDelta};
use crate::edge::{Edge, EdgeIdentity};
use crate::graph::{
    FileInput, Graph, GraphBuildError, GraphBuilder, Interner, NodeAttrs, NodeData, NodeInput,
    NodeInputAttrs, StrId, UnresolvedRef,
};
use crate::node_id::{NodeId, NodeKey};
use crate::node_kind::NodeKind;
use crate::query::{EdgeFilter, EdgeRef, FileView, GraphQuery, NodeRef};
use crate::schema::SCHEMA_VERSION;
use crate::Direction;

/// Why an overlay could not be built.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum OverlayError {
    /// The delta was computed against another schema version than this build speaks.
    #[error("delta targets schema version {found}, this build speaks {expected}")]
    SchemaVersionMismatch { expected: u32, found: u32 },
    /// The delta adds an edge identity the base already has, without tombstoning it first.
    ///
    /// Deltas stay explicit: an override is always tombstone + add (clarification C5), so a
    /// reviewer can see that an edge was replaced rather than only that it exists.
    #[error("delta adds {0}, which the base graph already has; add a tombstone first")]
    ImplicitOverride(EdgeIdentity),
    /// The delta adds a node or an unresolved reference inside a file it also deletes.
    #[error("delta adds content to {path}, which it also deletes")]
    ContentInDeletedFile { path: RepoPath },
}

/// One added node, with its strings already interned into the overlay's own interner.
#[derive(Debug, Clone, PartialEq)]
struct AddedNode {
    kind: NodeKind,
    id: StrId,
    name: StrId,
    qualified_name: StrId,
    file: Option<RepoPath>,
    range: Option<review_core::location::SourceRange>,
    attrs: NodeAttrs,
}

/// A base graph plus the delta that turns it into a head graph.
///
/// Immutable after [`GraphOverlay::new`] and freely shareable across tasks.
#[derive(Debug)]
pub struct GraphOverlay {
    base: Arc<Graph>,
    delta: Arc<GraphDelta>,
    added_nodes: BTreeMap<NodeKey, AddedNode>,
    removed_nodes: HashSet<NodeKey>,
    added_edges: Vec<Edge>,
    /// Index into `added_edges` per node, sorted by `(kind, other key)`.
    added_out: HashMap<NodeKey, Vec<u32>>,
    added_in: HashMap<NodeKey, Vec<u32>>,
    removed_edges: HashSet<EdgeIdentity>,
    file_overrides: BTreeMap<RepoPath, FileInput>,
    deleted_files: HashSet<RepoPath>,
    unresolved_replaced: BTreeMap<RepoPath, Vec<UnresolvedRef>>,
    strings: Interner,
    noop_tombstones: usize,
}

impl GraphOverlay {
    /// Builds the overlay index. `O(|Δ| log |Δ|)`.
    ///
    /// # Errors
    ///
    /// [`OverlayError::SchemaVersionMismatch`] when the delta was computed against another
    /// schema, and [`OverlayError::ImplicitOverride`] when an added edge identity already exists
    /// in the base without a tombstone.
    pub fn new(base: Arc<Graph>, delta: Arc<GraphDelta>) -> Result<Self, OverlayError> {
        if delta.base_schema_version != SCHEMA_VERSION {
            return Err(OverlayError::SchemaVersionMismatch {
                expected: SCHEMA_VERSION,
                found: delta.base_schema_version,
            });
        }

        let mut strings = Interner::new();
        let _ = strings.intern("");
        let mut added_nodes: BTreeMap<NodeKey, AddedNode> = BTreeMap::new();
        for node in &delta.nodes_added {
            added_nodes.insert(node.id.key(), interned_node(&mut strings, node));
        }

        let removed_edges: HashSet<EdgeIdentity> = delta.edges_removed.iter().copied().collect();
        let removed_nodes: HashSet<NodeKey> = delta.nodes_removed.iter().copied().collect();

        // Tombstones that name something neither the base nor the delta has are harmless; they
        // are counted so the caller can emit `graph_overlay_noop_tombstones_total`.
        let mut noop_tombstones = 0usize;
        for identity in &delta.edges_removed {
            let known = base.index_of_key(&identity.source).is_some()
                || added_nodes.contains_key(&identity.source);
            let known = known
                && (base.index_of_key(&identity.target).is_some()
                    || added_nodes.contains_key(&identity.target));
            if !known {
                noop_tombstones += 1;
            }
        }
        for key in &delta.nodes_removed {
            if base.index_of_key(key).is_none() && !added_nodes.contains_key(key) {
                noop_tombstones += 1;
            }
        }

        let mut added_edges: Vec<Edge> = Vec::with_capacity(delta.edges_added.len());
        for edge in &delta.edges_added {
            let identity = edge.identity();
            // An override is tombstone + add of the *same* identity (clarification C5), so an
            // identity being tombstoned does not disqualify it here: the tombstone hides the base
            // edge and this entry is what takes its place.
            if added_edges
                .iter()
                .any(|existing| existing.identity() == identity)
            {
                continue;
            }
            // An edge whose endpoint the delta removes cannot exist, so it is dropped rather than
            // carried into an overlay that would report a dangling endpoint.
            if removed_nodes.contains(&identity.source)
                && !added_nodes.contains_key(&identity.source)
            {
                continue;
            }
            if removed_nodes.contains(&identity.target)
                && !added_nodes.contains_key(&identity.target)
            {
                continue;
            }
            if base_identity_exists(&base, &removed_edges, &removed_nodes, identity) {
                return Err(OverlayError::ImplicitOverride(identity));
            }
            added_edges.push(edge.clone());
        }

        let mut added_out: HashMap<NodeKey, Vec<u32>> = HashMap::new();
        let mut added_in: HashMap<NodeKey, Vec<u32>> = HashMap::new();
        for (raw, edge) in added_edges.iter().enumerate() {
            let index = u32::try_from(raw).unwrap_or(u32::MAX);
            added_out.entry(edge.source).or_default().push(index);
            added_in.entry(edge.target).or_default().push(index);
        }
        // Both lists end up in `(kind, other key)` order, matching the base CSR slices. For an
        // out list `source` is constant so ordering by `(kind, source, target)` is `(kind,
        // target)`; for an in list `target` is constant so it is `(kind, source)`.
        for list in added_out.values_mut().chain(added_in.values_mut()) {
            list.sort_by_key(|a| edge_order(&added_edges, *a));
        }

        let mut file_overrides: BTreeMap<RepoPath, FileInput> = BTreeMap::new();
        let mut deleted_files: HashSet<RepoPath> = HashSet::new();
        for change in &delta.files {
            if matches!(change.change, FileChangeKind::Deleted) {
                deleted_files.insert(change.path.clone());
                continue;
            }
            let base_entry = base.file_by_path(&change.path);
            file_overrides.insert(
                change.path.clone(),
                FileInput {
                    path: change.path.clone(),
                    file_version_id: change.file_version_id.or_else(|| {
                        base_entry
                            .and_then(|ix| base.file(ix).and_then(|entry| entry.file_version_id))
                    }),
                    content_hash: change.content_hash.unwrap_or_else(|| {
                        base_entry
                            .and_then(|ix| base.file(ix).map(|entry| entry.content_hash))
                            .unwrap_or_else(|| {
                                review_core::location::ContentHash::from_bytes([0u8; 32])
                            })
                    }),
                    language: change.language.unwrap_or_else(|| {
                        base_entry
                            .and_then(|ix| base.file(ix).map(|entry| entry.language))
                            .unwrap_or(review_core::language::Language::Other)
                    }),
                },
            );
        }

        let unresolved_replaced: BTreeMap<RepoPath, Vec<UnresolvedRef>> = delta
            .unresolved_replaced
            .iter()
            .map(|(path, refs)| (path.clone(), refs.clone()))
            .collect();

        // A delta may not resurrect content inside a file it deletes. Rejecting it here keeps
        // `flatten` buildable and gives the caller one error to act on instead of a graph that
        // silently drops rows.
        for node in added_nodes.values() {
            if let Some(file) = node.file.as_ref() {
                if deleted_files.contains(file) {
                    return Err(OverlayError::ContentInDeletedFile { path: file.clone() });
                }
            }
        }
        for path in unresolved_replaced.keys() {
            if deleted_files.contains(path) {
                return Err(OverlayError::ContentInDeletedFile { path: path.clone() });
            }
        }

        Ok(Self {
            base,
            delta,
            added_nodes,
            removed_nodes,
            added_edges,
            added_out,
            added_in,
            removed_edges,
            file_overrides,
            deleted_files,
            unresolved_replaced,
            strings,
            noop_tombstones,
        })
    }

    pub fn delta(&self) -> &GraphDelta {
        &self.delta
    }

    pub fn base(&self) -> &Arc<Graph> {
        &self.base
    }

    /// Tombstones that named something neither the base nor the delta has. Harmless, and counted
    /// so the caller can emit `graph_overlay_noop_tombstones_total`.
    #[must_use]
    pub fn noop_tombstones(&self) -> usize {
        self.noop_tombstones
    }

    /// Does the node exist in this overlay?
    #[must_use]
    pub fn has_node(&self, key: &NodeKey) -> bool {
        !self.removed_nodes.contains(key)
            && (self.added_nodes.contains_key(key) || self.base.index_of_key(key).is_some())
    }

    /// Does the edge exist in this overlay?
    #[must_use]
    pub fn has_edge(&self, identity: &EdgeIdentity) -> bool {
        if self.removed_edges.contains(identity) {
            return false;
        }
        if !self.has_node(&identity.source) || !self.has_node(&identity.target) {
            return false;
        }
        self.added_edges
            .iter()
            .any(|edge| edge.identity() == *identity)
            || base_identity_exists(
                &self.base,
                &self.removed_edges,
                &self.removed_nodes,
                *identity,
            )
    }

    /// Materializes the overlay into a standalone [`Graph`]. `O(V + E)`.
    ///
    /// Used by compaction (GS-007) and by the incremental oracle (INC-012), which needs the head
    /// as an ordinary graph to compare against a from-scratch build.
    ///
    /// # Errors
    ///
    /// Whatever [`GraphBuilder::build`] reports — a dangling endpoint, a key collision, a size
    /// limit. Never a partially built graph.
    pub fn flatten(&self) -> Result<Graph, GraphBuildError> {
        let mut builder = GraphBuilder::new(SCHEMA_VERSION);

        for entry in self.base.files() {
            let raw = self.base.str(entry.path);
            let Ok(path) = RepoPath::new(raw) else {
                continue;
            };
            if self.deleted_files.contains(&path) || self.file_overrides.contains_key(&path) {
                continue;
            }
            builder.add_file(FileInput {
                path,
                file_version_id: entry.file_version_id,
                content_hash: entry.content_hash,
                language: entry.language,
            })?;
        }
        for file in self.file_overrides.values() {
            builder.add_file(file.clone())?;
        }

        for node in self.base.nodes() {
            if self.removed_nodes.contains(&node.key) || self.added_nodes.contains_key(&node.key) {
                continue;
            }
            builder.add_node(self.base_node_input(node))?;
        }
        for added in self.added_nodes.values() {
            builder.add_node(self.added_node_input(added))?;
        }

        for edge in self.base.edges() {
            let (Some(source), Some(target)) =
                (self.base.node(edge.source), self.base.node(edge.target))
            else {
                continue;
            };
            let identity = EdgeIdentity {
                source: source.key,
                kind: edge.kind,
                target: target.key,
            };
            if self.edge_shadowed(identity) {
                continue;
            }
            builder.add_edge(self.base_edge(edge, source.key, target.key));
        }
        for edge in &self.added_edges {
            // `new` already dropped edges whose endpoint the delta removes; this keeps `flatten`
            // correct if that set is ever extended.
            if self.removed_nodes.contains(&edge.source)
                || self.removed_nodes.contains(&edge.target)
            {
                continue;
            }
            builder.add_edge(edge.clone());
        }

        for reference in self.base.unresolved() {
            if self.unresolved_replaced.contains_key(&reference.file)
                || self.deleted_files.contains(&reference.file)
            {
                continue;
            }
            builder.add_unresolved(reference.clone());
        }
        for refs in self.unresolved_replaced.values() {
            for reference in refs {
                builder.add_unresolved(reference.clone());
            }
        }
        builder.build()
    }

    /// True when the base edge must not be reported: tombstoned, endpoint removed, or replaced
    /// by an added edge with the same identity (which `new` rejects, so only the tombstone case
    /// can reach here together with an addition).
    fn edge_shadowed(&self, identity: EdgeIdentity) -> bool {
        self.removed_edges.contains(&identity)
            || self.removed_nodes.contains(&identity.source)
            || self.removed_nodes.contains(&identity.target)
    }

    /// The unresolved references the overlay exposes: the base ones, with each listed file's list
    /// replaced and each deleted file's dropped.
    fn visible_unresolved(&self) -> Vec<&UnresolvedRef> {
        self.base
            .unresolved()
            .iter()
            .filter(|reference| {
                !self.unresolved_replaced.contains_key(&reference.file)
                    && !self.deleted_files.contains(&reference.file)
            })
            .chain(
                self.unresolved_replaced
                    .values()
                    .flat_map(|refs| refs.iter()),
            )
            .collect()
    }

    /// Every key with at least one incident edge, used by `edge_count` and by tests.
    fn endpoints(&self) -> Vec<NodeKey> {
        let mut keys: Vec<NodeKey> = self.base.nodes().iter().map(|node| node.key).collect();
        keys.extend(self.added_nodes.keys().copied());
        keys.retain(|key| !self.removed_nodes.contains(key));
        keys
    }

    fn base_node_input(&self, node: &NodeData) -> NodeInput {
        let mut attrs = NodeInputAttrs {
            visibility: node.attrs.visibility,
            flags: node.attrs.flags,
            signature: node.attrs.signature.map(|id| self.base.str(id).to_owned()),
            body_hash: node.attrs.body_hash,
            signature_hash: node.attrs.signature_hash,
            parent: node.attrs.parent,
            extra: Vec::new(),
        };
        if let Some(pairs) = &node.attrs.extra {
            attrs.extra = pairs
                .iter()
                .map(|(key, value)| {
                    (
                        self.base.str(*key).to_owned(),
                        self.base.str(*value).to_owned(),
                    )
                })
                .collect();
        }
        let mut input = NodeInput::new(
            NodeId::from_canonical(self.base.str(node.id).to_owned()),
            node.kind,
            self.base.str(node.name),
        )
        .qualified_name(self.base.str(node.qualified_name))
        .with_attrs(attrs);
        if let Some(entry) = node.file.and_then(|ix| self.base.file(ix)) {
            let path = self.base.str(entry.path);
            if let Ok(path) = RepoPath::new(path) {
                input = input.in_file(path);
            }
        }
        if let Some(range) = node.range {
            input = input.with_range(range);
        }
        input
    }

    fn added_node_input(&self, node: &AddedNode) -> NodeInput {
        let mut attrs = NodeInputAttrs {
            visibility: node.attrs.visibility,
            flags: node.attrs.flags,
            signature: node
                .attrs
                .signature
                .map(|id| self.strings.resolve(id).to_owned()),
            body_hash: node.attrs.body_hash,
            signature_hash: node.attrs.signature_hash,
            parent: node.attrs.parent,
            extra: Vec::new(),
        };
        if let Some(pairs) = &node.attrs.extra {
            attrs.extra = pairs
                .iter()
                .map(|(key, value)| {
                    (
                        self.strings.resolve(*key).to_owned(),
                        self.strings.resolve(*value).to_owned(),
                    )
                })
                .collect();
        }
        let mut input = NodeInput::new(
            NodeId::from_canonical(self.strings.resolve(node.id).to_owned()),
            node.kind,
            self.strings.resolve(node.name).to_owned(),
        )
        .qualified_name(self.strings.resolve(node.qualified_name).to_owned())
        .with_attrs(attrs);
        if let Some(path) = &node.file {
            input = input.in_file(path.clone());
        }
        if let Some(range) = node.range {
            input = input.with_range(range);
        }
        input
    }

    fn base_edge(&self, edge: &crate::graph::EdgeData, source: NodeKey, target: NodeKey) -> Edge {
        let mut owned = Edge::new(
            edge.kind,
            source,
            target,
            edge.confidence,
            edge.resolved_by,
            edge.provenance,
        )
        .with_flags(edge.flags)
        .with_occurrences(edge.occurrences);
        if let Some(entry) = edge.origin_file.and_then(|ix| self.base.file(ix)) {
            let raw = self.base.str(entry.path);
            if let Ok(path) = RepoPath::new(raw) {
                owned = if edge.has_location() {
                    owned.with_location(crate::edge::Location::new(path, edge.line, edge.col))
                } else {
                    owned.with_origin_file(path)
                };
            }
        }
        owned
    }

    fn base_edge_view<'g>(&'g self, edge: &'g crate::graph::EdgeData) -> Option<EdgeRef<'g>> {
        let source = self.base.node(edge.source)?.key;
        let target = self.base.node(edge.target)?.key;
        let path = edge
            .origin_file
            .and_then(|ix| self.base.file(ix))
            .map(|entry| self.base.str(entry.path));
        Some(EdgeRef::from_parts(
            edge.kind,
            source,
            target,
            edge.confidence,
            edge.resolved_by,
            edge.provenance,
            edge.flags,
            if edge.has_location() {
                path.map(|path| (path, edge.line, edge.col))
            } else {
                None
            },
            edge.occurrences,
            path,
        ))
    }

    fn added_edge_view(&self, index: u32) -> Option<EdgeRef<'_>> {
        let edge = self.added_edges.get(index as usize)?;
        let path = edge
            .location
            .as_ref()
            .map(|location| location.file.as_str())
            .or_else(|| edge.origin_file.as_ref().map(|path| path.as_str()));
        Some(EdgeRef::from_parts(
            edge.kind,
            edge.source,
            edge.target,
            edge.confidence,
            edge.resolved_by,
            edge.provenance,
            edge.flags,
            edge.location
                .as_ref()
                .and_then(|location| path.map(|path| (path, location.line, location.col))),
            edge.occurrences,
            path,
        ))
    }

    /// One merged walk over the base slice and the added slice.
    ///
    /// Returns `false` when the visitor asked to stop, so `Both` can skip the reverse half.
    fn for_each_direction<'a>(
        &'a self,
        key: NodeKey,
        out: bool,
        filter: &EdgeFilter,
        visit: &mut dyn FnMut(EdgeRef<'a>) -> ControlFlow<()>,
    ) -> bool {
        let added: &[u32] = if out {
            self.added_out.get(&key).map_or(&[], Vec::as_slice)
        } else {
            self.added_in.get(&key).map_or(&[], Vec::as_slice)
        };
        let base_slice: Vec<&crate::graph::EdgeData> = self
            .base
            .index_of_key(&key)
            .map(|index| {
                let slice = if out {
                    self.base.out_edges(index)
                } else {
                    self.base.in_edges(index)
                };
                slice.iter().filter_map(|ix| self.base.edge(*ix)).collect()
            })
            .unwrap_or_default();

        let mut left = 0usize;
        let mut right = 0usize;
        loop {
            // Advance each cursor to its next passing edge. Skipped edges are consumed without
            // being visited, which keeps both streams aligned with the base CSR order.
            let base_view = loop {
                let Some(edge) = base_slice.get(left) else {
                    break None;
                };
                let Some(view) = self.base_edge_view(edge) else {
                    left += 1;
                    continue;
                };
                if self.edge_shadowed(EdgeIdentity {
                    source: view.source,
                    kind: view.kind,
                    target: view.target,
                }) {
                    left += 1;
                    continue;
                }
                if !filter.kinds.contains(view.kind) || view.confidence < filter.min_confidence {
                    left += 1;
                    continue;
                }
                break Some(view);
            };
            let added_view = loop {
                let Some(index) = added.get(right) else {
                    break None;
                };
                let Some(view) = self.added_edge_view(*index) else {
                    right += 1;
                    continue;
                };
                if !filter.kinds.contains(view.kind) || view.confidence < filter.min_confidence {
                    right += 1;
                    continue;
                }
                break Some(view);
            };

            let take_added = match (base_view, added_view) {
                (None, Some(_)) => true,
                (Some(_), None) => false,
                (Some(base), Some(extra)) => order(extra, key) < order(base, key),
                (None, None) => break,
            };
            let view = if take_added {
                right += 1;
                added_view
            } else {
                left += 1;
                base_view
            };
            if let Some(view) = view {
                if matches!(visit(view), ControlFlow::Break(())) {
                    return false;
                }
            }
        }
        true
    }
}

impl GraphQuery for GraphOverlay {
    fn schema_version(&self) -> u32 {
        SCHEMA_VERSION
    }

    fn node(&self, key: NodeKey) -> Option<NodeRef<'_>> {
        if self.removed_nodes.contains(&key) {
            return None;
        }
        if let Some(added) = self.added_nodes.get(&key) {
            return Some(NodeRef::from_parts(
                key,
                added.kind,
                self.strings.resolve(added.id),
                self.strings.resolve(added.name),
                self.strings.resolve(added.qualified_name),
                added.file.as_ref().map(|path| path.as_str()),
                added.range,
                &added.attrs,
            ));
        }
        let node = self.base.node(self.base.index_of_key(&key)?)?;
        Some(NodeRef::from_parts(
            key,
            node.kind,
            self.base.str(node.id),
            self.base.str(node.name),
            self.base.str(node.qualified_name),
            node.file
                .and_then(|ix| self.base.file(ix))
                .map(|entry| self.base.str(entry.path)),
            node.range,
            &node.attrs,
        ))
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
        if self.removed_nodes.contains(&key) {
            return;
        }
        match dir {
            Direction::Out => {
                self.for_each_direction(key, true, filter, visit);
            }
            Direction::In => {
                self.for_each_direction(key, false, filter, visit);
            }
            Direction::Both => {
                if self.for_each_direction(key, true, filter, visit) {
                    self.for_each_direction(key, false, filter, visit);
                }
            }
        }
    }

    fn degree(&self, key: NodeKey, dir: Direction, filter: &EdgeFilter) -> usize {
        let mut count = 0usize;
        self.for_each_edge(key, dir, filter, &mut |_| {
            count += 1;
            ControlFlow::Continue(())
        });
        count
    }

    fn node_count(&self) -> usize {
        self.endpoints().len()
    }

    fn edge_count(&self) -> usize {
        let mut count = 0usize;
        for key in self.endpoints() {
            self.for_each_edge(key, Direction::Out, &EdgeFilter::ALL, &mut |_| {
                count += 1;
                ControlFlow::Continue(())
            });
        }
        count
    }

    fn for_each_node<'a>(&'a self, visit: &mut dyn FnMut(NodeRef<'a>)) {
        for node in self.base.nodes() {
            if self.removed_nodes.contains(&node.key) || self.added_nodes.contains_key(&node.key) {
                continue;
            }
            let path = node
                .file
                .and_then(|ix| self.base.file(ix))
                .map(|entry| self.base.str(entry.path));
            visit(NodeRef::from_parts(
                node.key,
                node.kind,
                self.base.str(node.id),
                self.base.str(node.name),
                self.base.str(node.qualified_name),
                path,
                node.range,
                &node.attrs,
            ));
        }
        for (key, added) in &self.added_nodes {
            if self.removed_nodes.contains(key) {
                continue;
            }
            visit(NodeRef::from_parts(
                *key,
                added.kind,
                self.strings.resolve(added.id),
                self.strings.resolve(added.name),
                self.strings.resolve(added.qualified_name),
                added.file.as_ref().map(|path| path.as_str()),
                added.range,
                &added.attrs,
            ));
        }
    }

    fn nodes_in_file<'a>(&'a self, path: &str, visit: &mut dyn FnMut(NodeRef<'a>)) {
        // An unparseable path visits nothing: the graph simply has no such file.
        let raw = path.to_owned();
        let Ok(parsed) = RepoPath::new(raw.as_str()) else {
            return;
        };
        if self.deleted_files.contains(&parsed) {
            return;
        }
        let base_file = self.base.file_by_path(&parsed);
        if let Some(file) = base_file {
            for node in self.base.nodes_of_file(file) {
                if self.removed_nodes.contains(&node.key)
                    || self.added_nodes.contains_key(&node.key)
                {
                    continue;
                }
                visit(NodeRef::from_parts(
                    node.key,
                    node.kind,
                    self.base.str(node.id),
                    self.base.str(node.name),
                    self.base.str(node.qualified_name),
                    self.base.file(file).map(|entry| self.base.str(entry.path)),
                    node.range,
                    &node.attrs,
                ));
            }
        }
        for (key, added) in &self.added_nodes {
            if added.file.as_ref() != Some(&parsed) || self.removed_nodes.contains(key) {
                continue;
            }
            visit(NodeRef::from_parts(
                *key,
                added.kind,
                self.strings.resolve(added.id),
                self.strings.resolve(added.name),
                self.strings.resolve(added.qualified_name),
                added.file.as_ref().map(|path| path.as_str()),
                added.range,
                &added.attrs,
            ));
        }
    }

    fn edges_owned_by<'a>(&'a self, path: &str, visit: &mut dyn FnMut(EdgeRef<'a>)) {
        let Ok(parsed) = RepoPath::new(path) else {
            return;
        };
        if self.deleted_files.contains(&parsed) {
            return;
        }
        if let Some(file) = self.base.file_by_path(&parsed) {
            for index in self.base.edges_owned_by(file) {
                let Some(edge) = self.base.edge(*index) else {
                    continue;
                };
                let Some(view) = self.base_edge_view(edge) else {
                    continue;
                };
                if self.edge_shadowed(EdgeIdentity {
                    source: view.source,
                    kind: view.kind,
                    target: view.target,
                }) {
                    continue;
                }
                visit(view);
            }
        }
        for (raw, edge) in self.added_edges.iter().enumerate() {
            let owned = edge
                .origin_file
                .as_ref()
                .or_else(|| edge.location.as_ref().map(|location| &location.file));
            if owned != Some(&parsed) {
                continue;
            }
            if self.removed_nodes.contains(&edge.source)
                || self.removed_nodes.contains(&edge.target)
            {
                continue;
            }
            let index = u32::try_from(raw).unwrap_or(u32::MAX);
            if let Some(view) = self.added_edge_view(index) {
                visit(view);
            }
        }
    }

    fn unresolved_named<'a>(&'a self, name: &str, visit: &mut dyn FnMut(&'a UnresolvedRef)) {
        for reference in self.visible_unresolved() {
            if reference.name == name {
                visit(reference);
            }
        }
    }

    fn for_each_unresolved<'a>(&'a self, visit: &mut dyn FnMut(&'a UnresolvedRef)) {
        let mut visible = self.visible_unresolved();
        visible.sort_by(|a, b| {
            a.file
                .cmp(&b.file)
                .then_with(|| a.ordinal.cmp(&b.ordinal))
                .then_with(|| a.name.cmp(&b.name))
        });
        for reference in visible {
            visit(reference);
        }
    }

    fn file(&self, path: &str) -> Option<FileView<'_>> {
        let parsed = RepoPath::new(path).ok()?;
        if self.deleted_files.contains(&parsed) {
            return None;
        }
        if let Some(input) = self.file_overrides.get(&parsed) {
            return Some(FileView::from_parts(
                input.path.as_str(),
                input.content_hash,
                input.language,
                input.file_version_id,
            ));
        }
        let file = self.base.file_by_path(&parsed)?;
        let entry = self.base.file(file)?;
        Some(FileView::from_parts(
            self.base.str(entry.path),
            entry.content_hash,
            entry.language,
            entry.file_version_id,
        ))
    }
}

/// The `(kind, source, target)` order of an added edge, which is `(kind, other key)` inside any
/// one node's list.
fn edge_order(edges: &[Edge], index: u32) -> (crate::EdgeKind, NodeKey, NodeKey) {
    edges.get(index as usize).map_or_else(
        || {
            (
                crate::EdgeKind::Contains,
                NodeKey::from_bytes([0u8; 16]),
                NodeKey::from_bytes([0u8; 16]),
            )
        },
        |edge| (edge.kind, edge.source, edge.target),
    )
}

/// The `(kind, other key)` order position of an edge as seen from `key`.
fn order(edge: EdgeRef<'_>, key: NodeKey) -> (crate::EdgeKind, NodeKey) {
    let other = if edge.source == key {
        edge.target
    } else {
        edge.source
    };
    (edge.kind, other)
}

/// Does the base graph still hold this edge identity, with both endpoints surviving?
///
/// A tombstoned identity counts as gone, which is what makes an override — tombstone plus an add
/// of the same identity — legal instead of an implicit one.
fn base_identity_exists(
    base: &Graph,
    removed_edges: &HashSet<EdgeIdentity>,
    removed: &HashSet<NodeKey>,
    identity: EdgeIdentity,
) -> bool {
    if removed_edges.contains(&identity) {
        return false;
    }
    if removed.contains(&identity.source) || removed.contains(&identity.target) {
        return false;
    }
    let Some(source) = base.index_of_key(&identity.source) else {
        return false;
    };
    if base.index_of_key(&identity.target).is_none() {
        return false;
    }
    base.out_edges(source)
        .iter()
        .filter_map(|ix| base.edge(*ix))
        .any(|edge| {
            edge.kind == identity.kind
                && base.node(edge.source).map(|node| node.key) == Some(identity.source)
                && base.node(edge.target).map(|node| node.key) == Some(identity.target)
        })
}

/// Interns one [`NodeInput`] into the overlay's own string table.
fn interned_node(strings: &mut Interner, node: &NodeInput) -> AddedNode {
    let mut intern = |value: &str| strings.intern(value).unwrap_or(StrId::EMPTY);
    let signature = node.attrs.signature.as_ref().map(|text| intern(text));
    let pairs: &[(String, String)] = node.attrs.extra.as_slice();
    let extra: Option<Box<[(StrId, StrId)]>> = if pairs.is_empty() {
        None
    } else {
        let mut interned: Vec<(StrId, StrId)> = pairs
            .iter()
            .map(|(key, value)| (intern(key), intern(value)))
            .collect();
        interned.sort();
        Some(interned.into_boxed_slice())
    };
    AddedNode {
        kind: node.kind,
        id: intern(node.id.as_str()),
        name: intern(&node.name),
        qualified_name: intern(&node.qualified_name),
        file: node.file.clone(),
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
    }
}

/// A local delta-validity check, limited to the nodes the delta touches: `O(|Δ| · deg)`.
///
/// Never fails; it reports. The caller decides whether an invalid delta is a rejected snapshot
/// (INC-011) or a logged incident.
#[must_use]
pub fn validate_delta_local(
    base: &Graph,
    delta: &GraphDelta,
) -> Vec<crate::validate::ValidationIssue> {
    crate::validate::validate_delta_local(base, delta)
}

/// Total edges the overlay reports, for span attributes.
#[must_use]
pub fn edge_total(overlay: &GraphOverlay) -> usize {
    GraphQuery::edge_count(overlay)
}
