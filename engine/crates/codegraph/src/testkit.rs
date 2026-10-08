//! Synthetic graphs for the criterion benches and other crates' tests (feature `testkit`).
//!
//! The generator is a pure function of `(nodes, avg_degree, seed)`: the same arguments always
//! produce the same graph, including byte-for-byte after [`crate::graph::GraphBuilder`] sorts
//! it into canonical order. That is what makes a benchmark number reproducible and what lets
//! `tests/graph_build.rs` assert order-independence against a graph it did not have to write
//! by hand.
//!
//! Enable it with `--features testkit`; nothing outside benchmarks and tests depends on it.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use review_core::location::RepoPath;

use crate::confidence::Confidence;
use crate::edge::{Edge, Provenance, ResolvedBy};
use crate::edge_kind::EdgeKind;
use crate::graph::{Graph, GraphBuilder, NodeInput};
use crate::node_id::NodeId;
use crate::node_kind::NodeKind;
use crate::schema::SCHEMA_VERSION;

/// `xorshift64*`: no dependency, deterministic, and good enough to spread targets across the
/// node table.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        })
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, bound: u64) -> u64 {
        if bound == 0 {
            return 0;
        }
        self.next() % bound
    }
}

/// Node kinds the synthetic graph cycles through, chosen to exercise every direction in the
/// CG-002 endpoint matrix rather than only `Function -> Function`.
const NODE_KINDS: [NodeKind; 8] = [
    NodeKind::Function,
    NodeKind::Class,
    NodeKind::Method,
    NodeKind::Interface,
    NodeKind::Variable,
    NodeKind::ApiEndpoint,
    NodeKind::DatabaseTable,
    NodeKind::TestCase,
];

/// Edge kinds the synthetic graph cycles through: one call-like, one type, one structural,
/// one data and one framework kind, so kind-mask filtering has something to filter.
const EDGE_KINDS: [EdgeKind; 5] = [
    EdgeKind::Calls,
    EdgeKind::UsesType,
    EdgeKind::References,
    EdgeKind::Reads,
    EdgeKind::RoutesTo,
];

/// Builds a synthetic graph with `nodes` nodes and roughly `nodes * avg_degree` edges.
///
/// Averages only: an edge whose target would be the node itself is skipped, and duplicate
/// identities are merged by [`crate::Edge::merge_occurrence`], so the exact edge count can be
/// a little under `nodes * avg_degree`.
pub fn synthetic(nodes: u32, avg_degree: u32, seed: u64) -> Graph {
    let mut rng = Rng::new(seed);
    let mut builder = GraphBuilder::new(SCHEMA_VERSION);
    let file_count = nodes.div_ceil(100).max(1);

    let files: Vec<RepoPath> = (0..file_count)
        .map(|raw| RepoPath::new(format!("src/mod{raw}.ts")).unwrap())
        .collect();

    for raw in 0..nodes {
        let file = &files[(raw % file_count) as usize];
        let kind = NODE_KINDS[(raw as usize) % NODE_KINDS.len()];
        let name = format!("fn{raw}");
        builder
            .add_node(
                NodeInput::new(node_id(file, &name), kind, name.clone())
                    .qualified_name(format!("{file}#{name}"))
                    .in_file(file.clone()),
            )
            .unwrap();
    }

    let target_count = u64::from(nodes) * u64::from(avg_degree);
    for raw in 0..target_count {
        let source = rng.below(u64::from(nodes)) as u32;
        let target = rng.below(u64::from(nodes)) as u32;
        if source == target {
            continue;
        }
        let source_file = &files[(source % file_count) as usize];
        let target_file = &files[(target % file_count) as usize];
        let kind = EDGE_KINDS[(raw as usize) % EDGE_KINDS.len()];
        builder.add_edge(
            Edge::new(
                kind,
                node_id(source_file, &format!("fn{source}")).key(),
                node_id(target_file, &format!("fn{target}")).key(),
                Confidence::from_f32(0.6),
                ResolvedBy::NameUnique,
                Provenance::Linker,
            )
            .with_origin_file(source_file.clone()),
        );
    }

    builder.build().unwrap()
}

/// The canonical id of a synthetic symbol: `ts:{file}#{name}/function`.
///
/// The generator keeps no table of ids: at a million nodes the table would cost more than the
/// graph's node rows, and recomputing is a couple of string operations.
fn node_id(file: &RepoPath, name: &str) -> NodeId {
    NodeId::from_canonical(format!("ts:{file}#{name}/function"))
}
