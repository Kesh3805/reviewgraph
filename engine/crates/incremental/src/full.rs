//! The reference full build: link every unit and assemble a [`Graph`].
//!
//! This is what an incremental update must be equivalent to (ADR-004): the oracle compares the
//! overlay of a delta with the graph this function builds from the head commit's units. It is also
//! what a local index run uses when there is no base snapshot to update.

use std::sync::Arc;

use analysis_ir::traits::ModuleResolver;
use analysis_ir::ParsedUnit;
use codegraph::{
    FileInput, Graph, GraphBuilder, LinkConfig, LinkInput, LinkOutput, Linker, SCHEMA_VERSION,
};

use crate::error::{Error, Result};

/// Links `units` (any order; they are sorted by path first) and builds the graph.
pub fn build_full_graph(
    units: &[Arc<ParsedUnit>],
    resolver: &dyn ModuleResolver,
    config: &LinkConfig,
    repository_name: &str,
) -> Result<Graph> {
    let (graph, _) = build_full_graph_with_output(units, resolver, config, repository_name)?;
    Ok(graph)
}

/// Like [`build_full_graph`], also returning the raw link output (global facts, stats).
pub fn build_full_graph_with_output(
    units: &[Arc<ParsedUnit>],
    resolver: &dyn ModuleResolver,
    config: &LinkConfig,
    repository_name: &str,
) -> Result<(Graph, LinkOutput)> {
    let mut sorted: Vec<Arc<ParsedUnit>> = units.to_vec();
    sorted.sort_by(|a, b| a.file.cmp(&b.file));
    let output = Linker::link_all(LinkInput {
        units: &sorted,
        resolver,
        config,
        repository_name,
    });

    let mut builder = GraphBuilder::new(SCHEMA_VERSION);
    for unit in &sorted {
        builder
            .add_file(FileInput {
                path: unit.file.clone(),
                file_version_id: None,
                content_hash: unit.content_hash,
                language: unit.language,
            })
            .map_err(|e| Error::Store(e.to_string()))?;
    }
    for node in &output.nodes {
        builder
            .add_node(node.clone())
            .map_err(|e| Error::KeyCollision(e.to_string()))?;
    }
    for edge in &output.edges {
        builder.add_edge(edge.clone());
    }
    for reference in &output.unresolved {
        builder.add_unresolved(reference.clone());
    }
    let graph = builder.build().map_err(|e| Error::Store(e.to_string()))?;
    Ok((graph, output))
}
