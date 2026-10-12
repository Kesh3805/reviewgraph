//! Conversion from the in-memory `codegraph::Graph` to the storage payload (adapter boundary).
//!
//! `graph-storage` persists a plain, ordered picture of a snapshot ([`crate::model::Graph`]);
//! the linker and builder produce an interned, CSR-backed `codegraph::Graph`. The two share
//! discriminants for every kind enum (asserted by the tests below), so the conversion is a
//! straight field mapping that fails closed on anything it does not recognise.
//!
//! Symbol nodes (those owned by a file) become [`Node::Symbol`]; everything else (repository,
//! packages, endpoints, queues, ...) becomes [`Node::Synthetic`]. File version ids are carried
//! over from the codegraph file table, so the caller allocates them (via
//! `GraphStore::upsert_file_versions`) before building the graph, or patches them afterwards
//! with [`assign_file_versions`].

use std::collections::BTreeMap;

use analysis_ir::reference::RefKind;
use analysis_ir::symbol::Visibility;
use codegraph::{EdgeData, FileIx, Graph as CodeGraph, NodeData, UnresolvedRef as CodeUnresolved};
use review_core::location::SourceRange as CoreRange;

use crate::kinds::{Confidence, EdgeFlags, EdgeKind, NodeKind, Provenance, ResolvedBy};
use crate::model::{
    FileChange, Graph, GraphEdge, Node, SnapshotFile, SourceRange, SymbolNode, SyntheticNode,
    UnresolvedRef,
};

/// Why a codegraph could not be mapped onto the storage payload.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ConvertError {
    /// A discriminant of the graph lane has no storage counterpart.
    #[error("unknown {what} discriminant {value}")]
    UnknownDiscriminant { what: &'static str, value: u32 },
    /// An index in the codegraph tables points outside its table.
    #[error("dangling {what} index {index}")]
    DanglingIndex { what: &'static str, index: u32 },
}

fn visibility_code(v: Visibility) -> u8 {
    match v {
        Visibility::Public => 0,
        Visibility::Protected => 1,
        Visibility::Private => 2,
        Visibility::EcmaPrivate => 3,
    }
}

fn range_of(r: &CoreRange) -> SourceRange {
    SourceRange {
        start_line: r.start.line,
        start_col: r.start.column,
        end_line: r.end.line,
        end_col: r.end.column,
    }
}

fn node_kind(raw: u8) -> Result<NodeKind, ConvertError> {
    NodeKind::from_i16(i16::from(raw)).ok_or(ConvertError::UnknownDiscriminant {
        what: "node kind",
        value: u32::from(raw),
    })
}

fn ref_kind_code(kind: RefKind) -> u16 {
    RefKind::ALL
        .iter()
        .position(|k| *k == kind)
        .and_then(|i| u16::try_from(i).ok())
        .unwrap_or(0)
}

fn file_path(g: &CodeGraph, ix: FileIx) -> Result<&str, ConvertError> {
    let entry = g.file(ix).ok_or(ConvertError::DanglingIndex {
        what: "file",
        index: ix.get(),
    })?;
    Ok(g.str(entry.path))
}

fn convert_node(g: &CodeGraph, n: &NodeData) -> Result<Node, ConvertError> {
    let kind = node_kind(n.kind.as_u8())?;
    let id = g.str(n.id).to_owned();
    let mut attrs = serde_json::Map::new();
    if let Some(extra) = &n.attrs.extra {
        for (k, v) in extra.iter() {
            attrs.insert(g.str(*k).to_owned(), serde_json::Value::from(g.str(*v)));
        }
    }
    let attrs = serde_json::Value::Object(attrs);
    match n.file {
        Some(file) => Ok(Node::Symbol(SymbolNode {
            key: n.key,
            id,
            kind,
            name: g.str(n.name).to_owned(),
            qualified_name: g.str(n.qualified_name).to_owned(),
            file: file_path(g, file)?.to_owned(),
            range: n.range.as_ref().map(range_of).unwrap_or_default(),
            signature: n.attrs.signature.map(|s| g.str(s).to_owned()),
            body_hash: n.attrs.body_hash.map(|h| *h.as_bytes()),
            signature_hash: n.attrs.signature_hash.map(|h| *h.as_bytes()),
            parent: n.attrs.parent,
            visibility: visibility_code(n.attrs.visibility),
            is_exported: n.attrs.flags.contains(codegraph::NodeFlags::EXPORTED),
            is_generated: n.attrs.flags.contains(codegraph::NodeFlags::GENERATED),
            attrs,
        })),
        None => Ok(Node::Synthetic(SyntheticNode {
            key: n.key,
            id,
            kind,
            attrs,
        })),
    }
}

fn convert_edge(g: &CodeGraph, e: &EdgeData) -> Result<GraphEdge, ConvertError> {
    let key_of = |ix: codegraph::NodeIx| {
        g.node(ix)
            .map(|n| n.key)
            .ok_or(ConvertError::DanglingIndex {
                what: "node",
                index: ix.get(),
            })
    };
    let kind =
        EdgeKind::from_i16(i16::from(e.kind.as_u8())).ok_or(ConvertError::UnknownDiscriminant {
            what: "edge kind",
            value: u32::from(e.kind.as_u8()),
        })?;
    let resolved_by = ResolvedBy::from_i16(i16::from(e.resolved_by.as_u8())).ok_or(
        ConvertError::UnknownDiscriminant {
            what: "resolved_by",
            value: u32::from(e.resolved_by.as_u8()),
        },
    )?;
    let provenance = Provenance::from_i16(i16::from(e.provenance.as_u8())).ok_or(
        ConvertError::UnknownDiscriminant {
            what: "provenance",
            value: u32::from(e.provenance.as_u8()),
        },
    )?;
    let origin_path = match e.origin_file {
        Some(f) => Some(file_path(g, f)?.to_owned()),
        None => None,
    };
    Ok(GraphEdge {
        source: key_of(e.source)?,
        kind,
        target: key_of(e.target)?,
        confidence: Confidence::from_permille(e.confidence.as_permille()),
        resolved_by,
        provenance,
        flags: EdgeFlags::from_bits(e.flags.bits()),
        origin_path,
        line: e.has_location().then_some(e.line),
        col: e.has_location().then_some(e.col),
        occurrences: e.occurrences,
    })
}

fn convert_unresolved(u: &CodeUnresolved) -> UnresolvedRef {
    UnresolvedRef {
        file: u.file.as_str().to_owned(),
        ordinal: u.ordinal,
        from: u.from,
        name: u.name.clone(),
        ref_kind: ref_kind_code(u.kind),
        import_specifier: u.import_specifier.clone(),
        reason: u16::from(u.reason.as_u8()),
        candidate_count: u.candidate_count,
        line: u.location.line,
        col: u.location.col,
    }
}

/// Maps a built `codegraph::Graph` onto the storage payload. The result is normalized, so it
/// compares equal to what any adapter loads back after `write_full`.
pub fn from_codegraph(g: &CodeGraph) -> Result<Graph, ConvertError> {
    let mut out = Graph::new(g.schema_version());
    for entry in g.files() {
        out.files.push(SnapshotFile {
            path: g.str(entry.path).to_owned(),
            file_version_id: entry.file_version_id,
            change: FileChange::Present,
            old_path: None,
        });
    }
    for n in g.nodes() {
        out.nodes.push(convert_node(g, n)?);
    }
    for e in g.edges() {
        out.edges.push(convert_edge(g, e)?);
    }
    for u in g.unresolved() {
        out.unresolved.push(convert_unresolved(u));
    }
    Ok(out.normalized())
}

/// Groups the symbol nodes of a storage graph by owning file path, in path order.
pub fn symbols_by_file(g: &Graph) -> BTreeMap<&str, Vec<Node>> {
    let mut out: BTreeMap<&str, Vec<Node>> = BTreeMap::new();
    for node in &g.nodes {
        if let Node::Symbol(s) = node {
            out.entry(s.file.as_str()).or_default().push(node.clone());
        }
    }
    out
}

/// Sets `file_version_id` on every listed file of `g`. Paths without an entry keep theirs.
pub fn assign_file_versions(g: &mut Graph, ids: &BTreeMap<String, i64>) {
    for file in &mut g.files {
        if let Some(id) = ids.get(&file.path) {
            file.file_version_id = Some(*id);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use codegraph::{
        confidence_of, Edge, EdgeKind as CgEdgeKind, FileInput, GraphBuilder, NodeId,
        NodeKind as CgNodeKind, Provenance as CgProvenance, ResolvedBy as CgResolvedBy,
        SCHEMA_VERSION,
    };
    use review_core::language::Language;
    use review_core::location::{ContentHash, RepoPath};

    #[test]
    fn discriminants_match_the_graph_lane() {
        for kind in codegraph::ALL_NODE_KINDS {
            let mapped = NodeKind::from_i16(i16::from(kind.as_u8())).unwrap();
            assert_eq!(mapped.as_str(), kind.as_str());
        }
        for kind in codegraph::ALL_EDGE_KINDS {
            let mapped = EdgeKind::from_i16(i16::from(kind.as_u8())).unwrap();
            assert_eq!(mapped.as_str(), kind.as_str());
        }
        assert_eq!(SCHEMA_VERSION, crate::types::GRAPH_SCHEMA_VERSION);
    }

    fn sample() -> CodeGraph {
        let path = RepoPath::new("src/a.ts").unwrap();
        let mut b = GraphBuilder::new(SCHEMA_VERSION);
        b.add_file(FileInput {
            path: path.clone(),
            file_version_id: Some(7),
            content_hash: ContentHash::of(b"a"),
            language: Language::Typescript,
        })
        .unwrap();
        let file = codegraph::NodeInput::new(NodeId::file(&path), CgNodeKind::File, "a.ts")
            .in_file(path.clone());
        let repo = codegraph::NodeInput::new(NodeId::repository(), CgNodeKind::Repository, "repo");
        let (fk, rk) = (file.id.key(), repo.id.key());
        b.add_node(file).unwrap();
        b.add_node(repo).unwrap();
        b.add_edge(
            Edge::new(
                CgEdgeKind::Contains,
                rk,
                fk,
                confidence_of(CgResolvedBy::Structural),
                CgResolvedBy::Structural,
                CgProvenance::Analyzer,
            )
            .with_origin_file(path),
        );
        b.build().unwrap()
    }

    #[test]
    fn converts_nodes_edges_and_files() {
        let g = from_codegraph(&sample()).unwrap();
        assert_eq!(g.schema_version, crate::types::GRAPH_SCHEMA_VERSION);
        assert_eq!(g.files.len(), 1);
        assert_eq!(g.files[0].path, "src/a.ts");
        assert_eq!(g.files[0].file_version_id, Some(7));
        assert_eq!(g.nodes.len(), 2);
        assert_eq!(g.nodes.iter().filter(|n| n.is_synthetic()).count(), 1);
        assert_eq!(g.edges.len(), 1);
        assert_eq!(g.edges[0].kind, EdgeKind::Contains);
        assert_eq!(g.edges[0].origin_path.as_deref(), Some("src/a.ts"));
        assert_eq!(g.edges[0].line, None);
    }

    #[test]
    fn conversion_is_deterministic_and_normalized() {
        let a = from_codegraph(&sample()).unwrap();
        let b = from_codegraph(&sample()).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.clone().normalized(), a);
    }

    #[test]
    fn symbols_group_by_file_and_versions_can_be_assigned() {
        let mut g = from_codegraph(&sample()).unwrap();
        let grouped = symbols_by_file(&g);
        assert_eq!(grouped.len(), 1);
        assert_eq!(grouped["src/a.ts"].len(), 1);
        let mut ids = BTreeMap::new();
        ids.insert("src/a.ts".to_owned(), 42);
        assign_file_versions(&mut g, &ids);
        assert_eq!(g.files[0].file_version_id, Some(42));
    }
}
