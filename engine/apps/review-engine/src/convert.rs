//! The storage-to-query boundary: `graph_storage::model::Graph` (the persisted, ordered rows)
//! becomes a `codegraph::Graph` (interned tables plus CSR adjacency) that the query layer reads.
//!
//! The two crates keep bit-compatible taxonomies (`docs/graph-schema/storage.md`), so the kinds
//! convert through their PRD spellings. Anything the query layer does not understand (a node
//! kind written by a newer build, an edge whose endpoint is not a node) is dropped instead of
//! failing the whole snapshot: a read-only query surface prefers a slightly smaller graph to a
//! 503.

use std::collections::HashSet;

use analysis_ir::symbol::Visibility;
use codegraph::{
    Confidence, Edge, EdgeFlags, EdgeKind, FileInput, Graph, GraphBuilder, Location, NodeFlags,
    NodeId, NodeInput, NodeInputAttrs, NodeKey, NodeKind, Provenance, ResolvedBy, SCHEMA_VERSION,
};
use graph_storage::model as stored;
use review_core::location::{Position, RepoPath, SourceRange};

/// What was left out of the conversion, for a debug log.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ConvertStats {
    pub dropped_nodes: usize,
    pub dropped_edges: usize,
}

/// Builds the query graph of a materialized snapshot.
///
/// # Errors
///
/// The builder's message when it refuses the inputs (a 128-bit key collision, size limits).
pub fn to_query_graph(graph: &stored::Graph) -> Result<(Graph, ConvertStats), String> {
    let mut builder = GraphBuilder::new(SCHEMA_VERSION);
    let mut stats = ConvertStats::default();

    for file in &graph.files {
        let Ok(path) = RepoPath::new(file.path.clone()) else {
            continue;
        };
        let mut input = FileInput::placeholder(path);
        input.file_version_id = file.file_version_id;
        builder.add_file(input).map_err(|e| e.to_string())?;
    }

    let mut keys: HashSet<NodeKey> = HashSet::with_capacity(graph.nodes.len());
    for node in &graph.nodes {
        let Some(input) = node_input(node) else {
            stats.dropped_nodes += 1;
            continue;
        };
        keys.insert(input.id.key());
        builder.add_node(input).map_err(|e| e.to_string())?;
    }

    for edge in &graph.edges {
        let converted = edge_input(edge);
        match converted {
            Some(edge) if keys.contains(&edge.source) && keys.contains(&edge.target) => {
                builder.add_edge(edge);
            }
            _ => stats.dropped_edges += 1,
        }
    }

    let built = builder.build().map_err(|e| e.to_string())?;
    Ok((built, stats))
}

fn node_kind(kind: graph_storage::kinds::NodeKind) -> Option<NodeKind> {
    NodeKind::from_str_exact(kind.as_str())
}

fn node_input(node: &stored::Node) -> Option<NodeInput> {
    match node {
        stored::Node::Symbol(symbol) => {
            let kind = node_kind(symbol.kind)?;
            let id = NodeId::from_canonical(symbol.id.clone());
            let mut input = NodeInput::new(id, kind, symbol.name.clone())
                .qualified_name(symbol.qualified_name.clone());
            if let Ok(path) = RepoPath::new(symbol.file.clone()) {
                input = input.in_file(path);
            }
            if let Some(range) = source_range(&symbol.range) {
                input = input.with_range(range);
            }
            let mut flags = NodeFlags::EMPTY;
            if symbol.is_exported {
                flags |= NodeFlags::EXPORTED;
            }
            if symbol.is_generated {
                flags |= NodeFlags::GENERATED;
            }
            Some(input.with_attrs(NodeInputAttrs {
                visibility: visibility(symbol.visibility),
                flags,
                signature: symbol.signature.clone(),
                parent: symbol.parent,
                ..NodeInputAttrs::default()
            }))
        }
        stored::Node::Synthetic(synthetic) => {
            let kind = node_kind(synthetic.kind)?;
            let id = NodeId::from_canonical(synthetic.id.clone());
            Some(NodeInput::new(id, kind, synthetic.id.clone()))
        }
    }
}

fn visibility(raw: u8) -> Visibility {
    match raw {
        1 => Visibility::Protected,
        2 => Visibility::Private,
        3 => Visibility::EcmaPrivate,
        _ => Visibility::Public,
    }
}

fn source_range(range: &stored::SourceRange) -> Option<SourceRange> {
    let start = Position::new(range.start_line, range.start_col).ok()?;
    let end = Position::new(range.end_line, range.end_col).ok()?;
    SourceRange::new(start, end).ok()
}

fn edge_input(edge: &stored::GraphEdge) -> Option<Edge> {
    let kind = EdgeKind::from_str_exact(edge.kind.as_str())?;
    let resolved_by = ResolvedBy::from_str_exact(edge.resolved_by.as_str())?;
    let provenance = Provenance::from_str_exact(edge.provenance.as_str())?;
    let mut converted = Edge::new(
        kind,
        edge.source,
        edge.target,
        Confidence::from_f32(edge.confidence.as_f32()),
        resolved_by,
        provenance,
    )
    .with_flags(EdgeFlags::from_bits(edge.flags.bits()))
    .with_occurrences(edge.occurrences);
    if let Some(path) = edge
        .origin_path
        .as_ref()
        .and_then(|p| RepoPath::new(p.clone()).ok())
    {
        converted = match (edge.line, edge.col) {
            (Some(line), Some(col)) => converted.with_location(Location::new(path, line, col)),
            _ => converted.with_origin_file(path),
        };
    }
    Some(converted)
}
