//! Hand-built graphs and views for unit tests. Only compiled for tests.
#![allow(dead_code)]

use std::collections::BTreeMap;

use analysis_ir::facts::{FactKind, SyntaxFact};
use analysis_ir::symbol::AttrValue;
use codegraph::graph::{Graph, GraphBuilder, NodeInput, NodeInputAttrs};
use codegraph::node_id::NodeId;
use codegraph::query::GraphQuery;
use codegraph::{
    Confidence, Edge, EdgeFlags, EdgeKind, NodeKind, Provenance, ResolvedBy, SCHEMA_VERSION,
};
use review_core::location::{Position, RepoPath, SourceRange};

use crate::conventions::miner::GraphView;

/// Builds a small graph: nodes by canonical id, edges by endpoint id.
#[derive(Debug)]
pub struct TestGraph {
    builder: GraphBuilder,
}

impl TestGraph {
    pub fn new() -> Self {
        Self {
            builder: GraphBuilder::new(SCHEMA_VERSION),
        }
    }

    /// A node in `file` (or synthetic when `file` is `None`).
    pub fn node(&mut self, kind: NodeKind, id: &str, name: &str, file: Option<&str>) -> &mut Self {
        self.node_with(kind, id, name, file, NodeInputAttrs::default())
    }

    pub fn node_with(
        &mut self,
        kind: NodeKind,
        id: &str,
        name: &str,
        file: Option<&str>,
        attrs: NodeInputAttrs,
    ) -> &mut Self {
        let mut input = NodeInput::new(NodeId::from_canonical(id), kind, name).with_attrs(attrs);
        if let Some(file) = file {
            input = input.in_file(RepoPath::new(file).unwrap());
        }
        self.builder.add_node(input).unwrap();
        self
    }

    pub fn edge(&mut self, kind: EdgeKind, from: &str, to: &str) -> &mut Self {
        self.edge_flagged(kind, from, to, EdgeFlags::EMPTY)
    }

    pub fn edge_flagged(
        &mut self,
        kind: EdgeKind,
        from: &str,
        to: &str,
        flags: EdgeFlags,
    ) -> &mut Self {
        self.builder.add_edge(
            Edge::new(
                kind,
                NodeId::from_canonical(from).key(),
                NodeId::from_canonical(to).key(),
                Confidence::from_f32(0.9),
                ResolvedBy::NameUnique,
                Provenance::Linker,
            )
            .with_flags(flags),
        );
        self
    }

    pub fn build(self) -> Graph {
        self.builder.build().unwrap()
    }
}

/// An owned view over a graph with facts, for tests that need a `&dyn GraphView`.
#[derive(Debug)]
pub struct OwnedView {
    graph: Graph,
    facts: BTreeMap<String, Vec<SyntaxFact>>,
}

impl OwnedView {
    pub fn new(graph: Graph) -> Self {
        Self {
            graph,
            facts: BTreeMap::new(),
        }
    }

    pub fn empty() -> Self {
        Self::new(TestGraph::new().build())
    }

    pub fn with_facts(mut self, node_id: &str, facts: Vec<SyntaxFact>) -> Self {
        self.facts.insert(node_id.to_owned(), facts);
        self
    }
}

impl GraphView for OwnedView {
    fn graph(&self) -> &dyn GraphQuery {
        &self.graph
    }

    fn facts(&self, node_id: &str) -> &[SyntaxFact] {
        self.facts.get(node_id).map(Vec::as_slice).unwrap_or(&[])
    }
}

/// A syntax fact with an optional string/bool detail.
pub fn fact(kind: FactKind, key: &str, detail: &[(&str, AttrValue)]) -> SyntaxFact {
    let at = Position { line: 1, column: 1 };
    SyntaxFact {
        kind,
        key: key.to_owned(),
        range: SourceRange { start: at, end: at },
        detail: detail
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.clone()))
            .collect(),
    }
}

/// An empty profile, standing in for the partial profile a miner receives.
pub fn partial() -> crate::model::RepositoryProfile {
    use chrono::{TimeZone, Utc};
    use review_core::ids::{RepositoryId, SnapshotId};
    use review_core::version::ConfigHash;
    crate::model::RepositoryProfile::empty(
        RepositoryId::new(),
        SnapshotId::new(),
        ConfigHash::of(b"c"),
        Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
    )
}
