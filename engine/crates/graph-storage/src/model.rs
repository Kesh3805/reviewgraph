//! The storage-side graph payload (GS-001).
//!
//! `graph-storage` persists a plain, ordered, serializable picture of a snapshot: the files it
//! lists, the nodes that exist in it, its resolved edges and the references that stayed
//! unresolved. [`flatten`] is the single normative definition of `base ⊕ delta chain` (ADR-003);
//! every adapter materializes a chain by loading the base `Graph`, loading each delta with
//! [`GraphStore::load_delta`](crate::GraphStore::load_delta) and calling `flatten`, so the
//! Postgres, file and in-memory adapters cannot drift.
//!
//! Mapping requirement: `Graph`/`GraphDelta` here are the wire shape the graph lane's
//! `codegraph::Graph` (CG-004) and `codegraph::GraphDelta` (CG-010) must be converted to at
//! the adapter boundary; see `docs/graph-schema/storage.md`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::kinds::{
    Confidence, Direction, EdgeFlags, EdgeKind, EdgeKindSet, NodeKey, NodeKind, Provenance,
    ResolvedBy,
};

/// 1-based source range of a symbol, in the file that declares it.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Default,
)]
pub struct SourceRange {
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

/// A node as it is persisted: either a source symbol (content-addressed in `symbols`) or a
/// synthetic node (per snapshot, in `synthetic_nodes`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Node {
    Symbol(SymbolNode),
    Synthetic(SyntheticNode),
}

/// A node produced from source, owned by exactly one file version.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SymbolNode {
    pub key: NodeKey,
    /// Canonical symbol id, e.g. `ts:src/auth/auth.service#AuthService.authorize/method`.
    pub id: String,
    /// Syntactic kind of the *declaration*; framework refinement never changes this key (C3).
    pub kind: NodeKind,
    pub name: String,
    pub qualified_name: String,
    /// Repository-relative path of the file version that declares this node.
    pub file: String,
    pub range: SourceRange,
    pub signature: Option<String>,
    pub body_hash: Option<[u8; 16]>,
    pub signature_hash: Option<[u8; 16]>,
    pub parent: Option<NodeKey>,
    pub visibility: u8,
    pub is_exported: bool,
    pub is_generated: bool,
    pub attrs: serde_json::Value,
}

/// A node that is not a source symbol: endpoint, queue, table, env var, package, test case…
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyntheticNode {
    pub key: NodeKey,
    /// Canonical node id, e.g. `http:GET /users/{id}` (CG-001 schemes).
    pub id: String,
    pub kind: NodeKind,
    pub attrs: serde_json::Value,
}

impl Node {
    pub fn key(&self) -> NodeKey {
        match self {
            Node::Symbol(s) => s.key,
            Node::Synthetic(s) => s.key,
        }
    }

    pub fn id(&self) -> &str {
        match self {
            Node::Symbol(s) => &s.id,
            Node::Synthetic(s) => &s.id,
        }
    }

    pub fn kind(&self) -> NodeKind {
        match self {
            Node::Symbol(s) => s.kind,
            Node::Synthetic(s) => s.kind,
        }
    }

    /// The file that owns this node, if any. Synthetic nodes belong to no file.
    pub fn file(&self) -> Option<&str> {
        match self {
            Node::Symbol(s) => Some(&s.file),
            Node::Synthetic(_) => None,
        }
    }

    pub fn is_synthetic(&self) -> bool {
        matches!(self, Node::Synthetic(_))
    }

    /// Estimated heap footprint, used by the byte-weighted graph cache (GS-008).
    pub fn heap_size_bytes(&self) -> usize {
        let attrs = self.attrs().as_str().map_or(0, str::len) + 64;
        match self {
            Node::Symbol(s) => {
                256 + s.id.len()
                    + s.name.len()
                    + s.qualified_name.len()
                    + s.file.len()
                    + s.signature.as_deref().map_or(0, str::len)
                    + attrs
            }
            Node::Synthetic(s) => 96 + s.id.len() + attrs,
        }
    }

    fn attrs(&self) -> &serde_json::Value {
        match self {
            Node::Symbol(s) => &s.attrs,
            Node::Synthetic(s) => &s.attrs,
        }
    }
}

/// One resolved edge. Its logical identity is `(source, kind, target)` (C5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphEdge {
    pub source: NodeKey,
    pub kind: EdgeKind,
    pub target: NodeKey,
    pub confidence: Confidence,
    pub resolved_by: ResolvedBy,
    pub provenance: Provenance,
    pub flags: EdgeFlags,
    /// File whose content produced this edge; the re-link unit for incremental updates.
    pub origin_path: Option<String>,
    pub line: Option<u32>,
    pub col: Option<u32>,
    pub occurrences: u32,
}

/// `(source, kind, target)` — the primary key of an edge row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EdgeIdentity {
    pub source: NodeKey,
    pub kind: EdgeKind,
    pub target: NodeKey,
}

impl GraphEdge {
    pub fn identity(&self) -> EdgeIdentity {
        EdgeIdentity {
            source: self.source,
            kind: self.kind,
            target: self.target,
        }
    }

    /// Earliest location wins, then smallest identity, so ordering is total and deterministic.
    pub fn sort_key(&self) -> (NodeKey, EdgeKind, NodeKey, u32) {
        (
            self.source,
            self.kind,
            self.target,
            self.line.unwrap_or(u32::MAX),
        )
    }

    pub fn heap_size_bytes(&self) -> usize {
        64 + self.origin_path.as_deref().map_or(0, str::len)
    }
}

impl Eq for GraphEdge {}

impl Ord for GraphEdge {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.sort_key().cmp(&other.sort_key())
    }
}

impl PartialOrd for GraphEdge {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// A reference that did not resolve, scoped to one snapshot (C6).
///
/// `ref_kind` and `reason` are the raw smallint discriminants persisted in `unresolved_refs`;
/// mapping them to `analysis_ir`'s `IrRefKind` and the graph lane's `UnresolvedReason` is a
/// recorded cross-lane requirement (see `docs/graph-schema/storage.md`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnresolvedRef {
    /// Repository-relative path of the file whose references these are.
    pub file: String,
    /// Index in `ParsedUnit.references`.
    pub ordinal: u32,
    pub from: Option<NodeKey>,
    pub name: String,
    pub ref_kind: u16,
    pub import_specifier: Option<String>,
    pub reason: u16,
    pub candidate_count: u16,
    pub line: u32,
    pub col: u32,
}

/// How a path appears in a snapshot's file list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileChange {
    /// A full snapshot lists every path it contains with this change.
    Present,
    Added,
    Modified,
    Deleted,
    Renamed,
    /// Same content, references re-resolved (a linker version bump, INC-002).
    Relinked,
}

impl FileChange {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Present => "present",
            Self::Added => "added",
            Self::Modified => "modified",
            Self::Deleted => "deleted",
            Self::Renamed => "renamed",
            Self::Relinked => "relinked",
        }
    }

    pub fn from_db(raw: &str) -> Option<Self> {
        match raw {
            "present" => Some(Self::Present),
            "added" => Some(Self::Added),
            "modified" => Some(Self::Modified),
            "deleted" => Some(Self::Deleted),
            "renamed" => Some(Self::Renamed),
            "relinked" => Some(Self::Relinked),
            _ => None,
        }
    }

    /// True when this change supersedes whatever the base snapshot had for the path.
    pub const fn replaces_content(self) -> bool {
        !matches!(self, Self::Present)
    }
}

/// One row of `snapshot_files`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotFile {
    pub path: String,
    /// Adapter-allocated id in `file_versions`; `None` means "deleted in this snapshot".
    pub file_version_id: Option<i64>,
    pub change: FileChange,
    pub old_path: Option<String>,
}

/// A symbol transition recorded between two snapshots (SID-005 names).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineageTransition {
    Renamed,
    Moved,
    RenamedMoved,
    SignatureChangedMoved,
}

impl LineageTransition {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Renamed => "renamed",
            Self::Moved => "moved",
            Self::RenamedMoved => "renamed_moved",
            Self::SignatureChangedMoved => "signature_changed_moved",
        }
    }

    pub fn from_db(raw: &str) -> Option<Self> {
        match raw {
            "renamed" => Some(Self::Renamed),
            "moved" => Some(Self::Moved),
            "renamed_moved" => Some(Self::RenamedMoved),
            "signature_changed_moved" => Some(Self::SignatureChangedMoved),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Lineage {
    pub from_key: NodeKey,
    pub to_key: NodeKey,
    pub transition: LineageTransition,
    pub similarity: f32,
}

/// Everything one snapshot persists: files, nodes, edges and unresolved references.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Graph {
    pub schema_version: u32,
    pub files: Vec<SnapshotFile>,
    pub nodes: Vec<Node>,
    pub edges: Vec<GraphEdge>,
    pub unresolved: Vec<UnresolvedRef>,
}

impl Graph {
    pub fn new(schema_version: u32) -> Self {
        Self {
            schema_version,
            ..Self::default()
        }
    }

    /// Normalizes to the deterministic order [`flatten`] produces, so a graph written and a
    /// graph loaded compare equal.
    pub fn normalized(mut self) -> Self {
        self.normalize();
        self
    }

    fn normalize(&mut self) {
        self.files.sort_by(|a, b| a.path.cmp(&b.path));
        self.files.dedup_by(|a, b| a.path == b.path);
        self.nodes.sort_by(node_order);
        self.nodes.dedup_by(|a, b| a.key() == b.key());
        self.edges.sort();
        self.edges.dedup_by(|a, b| a.identity() == b.identity());
        self.unresolved
            .sort_by(|a, b| (&a.file, a.ordinal).cmp(&(&b.file, b.ordinal)));
        self.unresolved
            .dedup_by(|a, b| a.file == b.file && a.ordinal == b.ordinal);
    }

    pub fn node(&self, key: NodeKey) -> Option<&Node> {
        self.nodes.iter().find(|n| n.key() == key)
    }

    pub fn edge(&self, identity: EdgeIdentity) -> Option<&GraphEdge> {
        self.edges.iter().find(|e| e.identity() == identity)
    }

    /// Out-edges of `key` matching the filter, in deterministic order.
    pub fn neighbors(
        &self,
        key: NodeKey,
        dir: Direction,
        kinds: EdgeKindSet,
        min_confidence: Confidence,
        limit: u32,
        cursor: Option<EdgeCursor>,
    ) -> Vec<&GraphEdge> {
        let limit = limit.clamp(1, 1000) as usize;
        let mut out: Vec<&GraphEdge> = self
            .edges
            .iter()
            .filter(|e| {
                let other = match dir {
                    Direction::Out => e.target,
                    Direction::In => e.source,
                };
                let endpoint = match dir {
                    Direction::Out => e.source,
                    Direction::In => e.target,
                };
                endpoint == key
                    && kinds.contains(e.kind)
                    && e.confidence >= min_confidence
                    && cursor.is_none_or(|c| (e.kind, other) > (c.kind, c.key))
            })
            .collect();
        out.sort_by(|a, b| {
            let ka = match dir {
                Direction::Out => a.target,
                Direction::In => a.source,
            };
            let kb = match dir {
                Direction::Out => b.target,
                Direction::In => b.source,
            };
            (a.kind, ka).cmp(&(b.kind, kb))
        });
        out.truncate(limit);
        out
    }

    /// Estimated heap footprint for the byte-weighted cache (GS-008), within ±15%.
    pub fn heap_size_bytes(&self) -> usize {
        let files: usize = self
            .files
            .iter()
            .map(|f| 96 + f.path.len() + f.old_path.as_deref().map_or(0, str::len))
            .sum();
        let nodes: usize = self.nodes.iter().map(Node::heap_size_bytes).sum();
        let edges: usize = self.edges.iter().map(GraphEdge::heap_size_bytes).sum();
        let unresolved: usize = self
            .unresolved
            .iter()
            .map(|u| {
                96 + u.name.len() + u.file.len() + u.import_specifier.as_deref().map_or(0, str::len)
            })
            .sum();
        std::mem::size_of::<Self>() + files + nodes + edges + unresolved
    }
}

/// Where a paginated neighbour query continues from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EdgeCursor {
    pub kind: EdgeKind,
    pub key: NodeKey,
}

fn node_order(a: &Node, b: &Node) -> std::cmp::Ordering {
    // Nodes with a file sort first by (path, key); file-less synthetic nodes go last by key.
    match (a.file(), b.file()) {
        (Some(x), Some(y)) => x.cmp(y).then_with(|| a.key().cmp(&b.key())),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.key().cmp(&b.key()),
    }
}

/// The incremental half of a chain (ADR-003): what one delta adds and what it removes.
///
/// Symbol-node changes are *derived* from `files` by the adapter's `load_delta` (the symbol rows
/// themselves live content-addressed in `symbols`); `nodes_added`/`nodes_removed` as written by
/// an indexer therefore carry synthetic nodes only. `flatten` applies both, so materialization
/// never depends on which side supplied them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct GraphDelta {
    pub files: Vec<SnapshotFile>,
    pub nodes_added: Vec<Node>,
    pub nodes_removed: Vec<NodeKey>,
    pub edges_added: Vec<GraphEdge>,
    /// Tombstones: edges of `base` this delta removes, or a re-add of the same identity.
    pub edges_removed: Vec<GraphEdge>,
    /// Complete unresolved set for every file this delta re-linked (C6). An empty vector
    /// clears the base rows of that path.
    pub unresolved_replaced: Vec<(String, Vec<UnresolvedRef>)>,
    pub lineage: Vec<Lineage>,
}

impl GraphDelta {
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
            && self.nodes_added.is_empty()
            && self.nodes_removed.is_empty()
            && self.edges_added.is_empty()
            && self.edges_removed.is_empty()
            && self.unresolved_replaced.is_empty()
            && self.lineage.is_empty()
    }
}

/// Materializes `base ⊕ deltas`, the normative reading of a chain (ADR-003, C6).
///
/// Deterministic: the result only depends on the multiset of inputs, never on hash order.
pub fn flatten(base: &Graph, deltas: &[GraphDelta]) -> Graph {
    let mut files: BTreeMap<String, SnapshotFile> = base
        .files
        .iter()
        .map(|f| (f.path.clone(), f.clone()))
        .collect();
    let mut nodes: BTreeMap<NodeKey, Node> =
        base.nodes.iter().map(|n| (n.key(), n.clone())).collect();
    let mut edges: BTreeMap<EdgeIdentity, GraphEdge> = base
        .edges
        .iter()
        .map(|e| (e.identity(), e.clone()))
        .collect();
    let mut unresolved: BTreeMap<String, Vec<UnresolvedRef>> = group_unresolved(&base.unresolved);

    for delta in deltas {
        let mut superseded: Vec<String> = Vec::new();

        for file in &delta.files {
            match file.change {
                FileChange::Deleted => {
                    files.remove(&file.path);
                    superseded.push(file.path.clone());
                    drop_file_nodes(&mut nodes, &file.path);
                }
                FileChange::Renamed => {
                    if let Some(old) = file.old_path.as_deref() {
                        files.remove(old);
                        superseded.push(old.to_owned());
                        drop_file_nodes(&mut nodes, old);
                    }
                    files.insert(file.path.clone(), file.clone());
                    drop_file_nodes(&mut nodes, &file.path);
                    superseded.push(file.path.clone());
                }
                FileChange::Present => {}
                FileChange::Added | FileChange::Modified | FileChange::Relinked => {
                    files.insert(file.path.clone(), file.clone());
                    drop_file_nodes(&mut nodes, &file.path);
                    superseded.push(file.path.clone());
                }
            }
        }
        for path in &superseded {
            unresolved.remove(path);
        }

        // Removals first: a tombstone plus a re-add of the same identity is how a delta
        // overrides an edge (C5), so the add must win.
        for key in &delta.nodes_removed {
            nodes.remove(key);
        }
        for node in &delta.nodes_added {
            nodes.insert(node.key(), node.clone());
        }
        for edge in &delta.edges_removed {
            edges.remove(&edge.identity());
        }
        for edge in &delta.edges_added {
            edges.insert(edge.identity(), edge.clone());
        }
        for (path, rows) in &delta.unresolved_replaced {
            if rows.is_empty() {
                unresolved.remove(path);
            } else {
                let mut rows = rows.clone();
                rows.sort_by_key(|r| r.ordinal);
                rows.dedup_by(|a, b| a.ordinal == b.ordinal);
                unresolved.insert(path.clone(), rows);
            }
        }
    }

    let files: Vec<SnapshotFile> = files.into_values().collect();
    let nodes: Vec<Node> = nodes.into_values().collect();
    let edges: Vec<GraphEdge> = edges.into_values().collect();
    let unresolved: Vec<UnresolvedRef> = unresolved.into_values().flatten().collect();

    Graph {
        schema_version: base.schema_version,
        files,
        nodes,
        edges,
        unresolved,
    }
    // Normalizing keeps `flatten` and `Graph::normalized` in agreement: the two orders only
    // differed for the node list (key order vs (file, key) order), which would make a
    // single-file and a multi-file chain disagree.
    .normalized()
}

fn group_unresolved(rows: &[UnresolvedRef]) -> BTreeMap<String, Vec<UnresolvedRef>> {
    let mut grouped: BTreeMap<String, Vec<UnresolvedRef>> = BTreeMap::new();
    for row in rows {
        grouped
            .entry(row.file.clone())
            .or_default()
            .push(row.clone());
    }
    for rows in grouped.values_mut() {
        rows.sort_by_key(|r| r.ordinal);
        rows.dedup_by_key(|r| r.ordinal);
    }
    grouped
}

fn drop_file_nodes(nodes: &mut BTreeMap<NodeKey, Node>, path: &str) {
    nodes.retain(|_, node| node.file() != Some(path));
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::kinds::{EdgeKind, NodeKind, Provenance, ResolvedBy};

    fn key(n: u8) -> NodeKey {
        NodeKey::from_bytes([n; 16])
    }

    fn symbol(n: u8, file: &str) -> Node {
        Node::Symbol(SymbolNode {
            key: key(n),
            id: format!("ts:{file}#S{n}"),
            kind: NodeKind::Function,
            name: format!("s{n}"),
            qualified_name: format!("s{n}"),
            file: file.to_owned(),
            range: SourceRange::default(),
            signature: None,
            body_hash: None,
            signature_hash: None,
            parent: None,
            visibility: 0,
            is_exported: false,
            is_generated: false,
            attrs: serde_json::json!({}),
        })
    }

    fn edge(source: u8, target: u8, confidence: u16) -> GraphEdge {
        GraphEdge {
            source: key(source),
            kind: EdgeKind::Calls,
            target: key(target),
            confidence: Confidence::from_permille(confidence),
            resolved_by: ResolvedBy::NameUnique,
            provenance: Provenance::Linker,
            flags: EdgeFlags::EMPTY,
            origin_path: Some("src/a.ts".to_owned()),
            line: Some(1),
            col: Some(2),
            occurrences: 1,
        }
    }

    fn full() -> Graph {
        Graph {
            schema_version: 1,
            files: vec![SnapshotFile {
                path: "src/a.ts".to_owned(),
                file_version_id: Some(1),
                change: FileChange::Present,
                old_path: None,
            }],
            nodes: vec![symbol(1, "src/a.ts"), symbol(2, "src/a.ts")],
            edges: vec![edge(1, 2, 900)],
            unresolved: vec![UnresolvedRef {
                file: "src/a.ts".to_owned(),
                ordinal: 0,
                from: Some(key(1)),
                name: "missing".to_owned(),
                ref_kind: 1,
                import_specifier: None,
                reason: 1,
                candidate_count: 0,
                line: 3,
                col: 5,
            }],
        }
    }

    #[test]
    fn flatten_without_deltas_is_normalization() {
        let g = full();
        let flat = flatten(&g, &[]);
        assert_eq!(flat, g.normalized());
        assert_eq!(flat.files.len(), 1);
        assert_eq!(flat.nodes.len(), 2);
        assert_eq!(flat.edges.len(), 1);
        assert_eq!(flat.unresolved.len(), 1);
    }

    #[test]
    fn tombstone_removes_edge_and_re_add_wins() {
        let g = full();
        let mut removed = edge(1, 2, 900);
        removed.confidence = Confidence::from_permille(0);
        let mut readded = edge(1, 2, 400);
        readded.confidence = Confidence::from_permille(400);
        let delta = GraphDelta {
            edges_removed: vec![removed.clone()],
            edges_added: vec![readded.clone()],
            ..GraphDelta::default()
        };
        let flat = flatten(&g, std::slice::from_ref(&delta));
        assert_eq!(flat.edges.len(), 1);
        assert_eq!(flat.edges[0].confidence, Confidence::from_permille(400));

        let only_tombstone = GraphDelta {
            edges_removed: vec![removed],
            ..GraphDelta::default()
        };
        let flat = flatten(&g, std::slice::from_ref(&only_tombstone));
        assert!(flat.edges.is_empty());
    }

    #[test]
    fn deleted_file_drops_its_nodes_and_unresolved() {
        let g = full();
        let delta = GraphDelta {
            files: vec![SnapshotFile {
                path: "src/a.ts".to_owned(),
                file_version_id: None,
                change: FileChange::Deleted,
                old_path: None,
            }],
            ..GraphDelta::default()
        };
        let flat = flatten(&g, std::slice::from_ref(&delta));
        assert!(flat.files.is_empty());
        assert!(flat.nodes.is_empty());
        assert!(flat.unresolved.is_empty());
    }

    #[test]
    fn relinked_file_replaces_unresolved_rows() {
        let g = full();
        let replacement = UnresolvedRef {
            ordinal: 0,
            name: "other".to_owned(),
            ..g.unresolved[0].clone()
        };
        let delta = GraphDelta {
            files: vec![SnapshotFile {
                path: "src/a.ts".to_owned(),
                file_version_id: Some(2),
                change: FileChange::Relinked,
                old_path: None,
            }],
            unresolved_replaced: vec![("src/a.ts".to_owned(), vec![replacement.clone()])],
            ..GraphDelta::default()
        };
        let flat = flatten(&g, std::slice::from_ref(&delta));
        assert_eq!(flat.unresolved, vec![replacement]);
        assert_eq!(flat.files[0].file_version_id, Some(2));
    }

    #[test]
    fn neighbors_filter_sort_and_paginate() {
        let mut g = Graph::new(1);
        g.edges = vec![edge(1, 2, 900), edge(1, 3, 500), edge(2, 1, 1000)];
        g.nodes = vec![
            symbol(1, "src/a.ts"),
            symbol(2, "src/a.ts"),
            symbol(3, "src/a.ts"),
        ];

        let page = g.neighbors(
            key(1),
            Direction::Out,
            EdgeKindSet::ALL,
            Confidence::MIN,
            10,
            None,
        );
        assert_eq!(page.len(), 2);
        assert_eq!(page[0].target, key(2));

        let filtered = g.neighbors(
            key(1),
            Direction::Out,
            EdgeKindSet::of(EdgeKind::Calls),
            Confidence::from_permille(800),
            10,
            None,
        );
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].target, key(2));

        let incoming = g.neighbors(
            key(1),
            Direction::In,
            EdgeKindSet::ALL,
            Confidence::MIN,
            10,
            None,
        );
        assert_eq!(incoming.len(), 1);
        assert_eq!(incoming[0].source, key(2));

        let first = g.neighbors(
            key(1),
            Direction::Out,
            EdgeKindSet::ALL,
            Confidence::MIN,
            1,
            None,
        );
        let cursor = EdgeCursor {
            kind: first[0].kind,
            key: first[0].target,
        };
        let second = g.neighbors(
            key(1),
            Direction::Out,
            EdgeKindSet::ALL,
            Confidence::MIN,
            10,
            Some(cursor),
        );
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].target, key(3));
    }

    #[test]
    fn heap_size_is_positive_and_grows_with_content() {
        let small = Graph::new(1);
        let big = full();
        assert!(small.heap_size_bytes() > 0);
        assert!(big.heap_size_bytes() > small.heap_size_bytes());
    }
}
