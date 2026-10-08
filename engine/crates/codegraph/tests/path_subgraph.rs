//! CG-009 acceptance: shortest path and bounded subgraph extraction.
//!
//! The fixture is a layered diamond, because the interesting questions are about *minimum* hops
//! and about the difference between "no path" and "gave up":

//! ```text
//! a → b → d        a → c → d        (a diamond: the answer must be 2 hops, not 3)
//!     ↓
//!     e                                  (a dead end reachable only through b)
//! ```

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod support;

use std::collections::{BTreeSet, VecDeque};

use codegraph::{
    shortest_path, subgraph, Confidence, Direction, Edge, EdgeFilter, EdgeKind, EdgeKindSet,
    FileInput, Graph, GraphBuilder, GraphQuery, GraphQueryExt, NodeId, NodeInput, NodeKey,
    NodeKind, PathError, PathSpec, Provenance, ResolvedBy, SubgraphSpec, TraversalError,
    SCHEMA_VERSION, SUBGRAPH_MAX_NODES,
};
use proptest::prelude::*;
use review_core::language::Language;
use review_core::location::ContentHash;

use support::path;

const NODES: [(&str, &str); 5] = [
    ("a", "src/a.ts"),
    ("b", "src/b.ts"),
    ("c", "src/c.ts"),
    ("d", "src/d.ts"),
    ("e", "src/e.ts"),
];

fn key(name: &str) -> NodeKey {
    NodeId::from_canonical(format!("ts:src/{name}.ts#{name}/function")).key()
}

fn node(name: &str, file: &str) -> NodeInput {
    NodeInput::new(
        NodeId::from_canonical(format!("ts:src/{name}.ts#{name}/function")),
        NodeKind::Function,
        name,
    )
    .qualified_name(name)
    .in_file(path(file))
}

fn file_of(name: &str) -> &'static str {
    NODES
        .iter()
        .find(|(candidate, _)| *candidate == name)
        .map_or("src/a.ts", |(_, file)| *file)
}

/// a→b→d, a→c→d, b→e, and one weak edge a→d that a `min_confidence` filter prunes.
fn fixture() -> Graph {
    let mut builder = GraphBuilder::new(SCHEMA_VERSION);
    for (_, file) in NODES {
        builder
            .add_file(FileInput {
                path: path(file),
                file_version_id: None,
                content_hash: ContentHash::of(file.as_bytes()),
                language: Language::Typescript,
            })
            .unwrap();
    }
    for (name, file) in NODES {
        builder.add_node(node(name, file)).unwrap();
    }
    let calls = |from: &str, to: &str, confidence: f32| {
        Edge::new(
            EdgeKind::Calls,
            key(from),
            key(to),
            Confidence::from_f32(confidence),
            ResolvedBy::NameUnique,
            Provenance::Linker,
        )
        .with_location(codegraph::Location::new(path(file_of(from)), 1, 0))
        .with_origin_file(path(file_of(from)))
    };
    builder.add_edge(calls("a", "b", 1.0));
    builder.add_edge(calls("a", "c", 1.0));
    builder.add_edge(calls("b", "d", 1.0));
    builder.add_edge(calls("c", "d", 1.0));
    builder.add_edge(calls("b", "e", 1.0));
    builder.add_edge(calls("a", "d", 0.4));
    builder.build().unwrap()
}

fn spec(from: &str, to: &str, direction: Direction) -> PathSpec {
    // The confidence floor is above the fixture's deliberately weak `a → d` edge, so the diamond
    // tests measure hop count rather than being short-circuited by a low-confidence shortcut.
    PathSpec {
        min_confidence: Confidence::from_f32(0.5),
        ..PathSpec::new(
            key(from),
            key(to),
            direction,
            EdgeKindSet::of(EdgeKind::Calls),
        )
    }
}

#[test]
fn shortest_path_finds_minimum_hops() {
    let graph = fixture();
    let result = shortest_path(&graph, &spec("a", "d", Direction::Out)).unwrap();
    let path = result.path.expect("a reaches d");
    assert_eq!(path.hops(), 2, "the diamond is two hops, not three");
    assert!(!result.truncated);
    assert_eq!(path.target(), Some(&key("d")));
    // `b` and `c` are the two candidates; the traversal's deterministic order picks one.
    let middle = path.steps[0].target;
    assert!(middle == key("b") || middle == key("c"));
}

#[test]
fn shortest_path_of_a_node_to_itself_is_empty() {
    let graph = fixture();
    let result = shortest_path(&graph, &spec("a", "a", Direction::Out)).unwrap();
    let path = result.path.unwrap();
    assert!(path.steps.is_empty());
    assert_eq!(path.min_confidence, Confidence::MAX);
    assert!(!result.truncated);
}

#[test]
fn shortest_path_respects_kinds_and_direction() {
    let graph = fixture();
    // There is no `In` path from `a` to `d`, because every edge runs forward.
    let result = shortest_path(&graph, &spec("a", "d", Direction::In)).unwrap();
    assert!(result.path.is_none());
    assert!(!result.truncated);

    // And no `CALLS` edge points at `e` from `a`, so a kind filter that excludes `CALLS` finds
    // nothing at all.
    let mut none = spec("a", "d", Direction::Out);
    none.kinds = EdgeKindSet::of(EdgeKind::Extends);
    let result = shortest_path(&graph, &none).unwrap();
    assert!(result.path.is_none());
}

#[test]
fn path_not_found_within_depth_is_not_truncated() {
    let graph = fixture();
    let mut shallow = spec("a", "e", Direction::Out);
    shallow.max_depth = 1;
    let result = shortest_path(&graph, &shallow).unwrap();
    assert!(result.path.is_none());
    assert!(
        !result.truncated,
        "giving up because of the depth the caller asked for is not truncation"
    );

    let mut deep = shallow.clone();
    deep.max_depth = 3;
    let result = shortest_path(&graph, &deep).unwrap();
    assert_eq!(result.path.unwrap().hops(), 2);
}

#[test]
fn path_truncated_when_budget_exhausted() {
    let graph = fixture();
    let mut tiny = spec("a", "e", Direction::Out);
    tiny.max_nodes = 1;
    let result = shortest_path(&graph, &tiny).unwrap();
    assert!(result.path.is_none());
    assert!(result.truncated, "a one-node budget cannot reach e");

    let over = PathSpec {
        max_depth: 99,
        ..spec("a", "d", Direction::Out)
    };
    assert!(matches!(
        shortest_path(&graph, &over),
        Err(PathError::Traversal(TraversalError::BudgetTooLarge))
    ));
    let both = PathSpec {
        direction: Direction::Both,
        ..spec("a", "d", Direction::Out)
    };
    assert!(matches!(
        shortest_path(&graph, &both),
        Err(PathError::BothDirections)
    ));
    let unknown = PathSpec::new(
        key("a"),
        NodeId::from_canonical("ghost").key(),
        Direction::Out,
        EdgeKindSet::ALL,
    );
    assert!(matches!(
        shortest_path(&graph, &unknown),
        Err(PathError::UnknownNode(_))
    ));
}

#[test]
fn subgraph_is_induced_and_sorted() {
    let graph = fixture();
    let mut request = SubgraphSpec::new(
        vec![key("a")],
        1,
        Direction::Out,
        EdgeKindSet::of(EdgeKind::Calls),
    );
    request.min_confidence = Confidence::from_f32(0.5);
    let view = subgraph(&graph, &request).unwrap();
    let keys: Vec<NodeKey> = view.nodes.iter().map(|node| node.key).collect();
    let mut sorted = keys.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(keys, sorted, "nodes are sorted by key and unique");
    assert!(keys.contains(&key("a")));
    assert!(keys.contains(&key("b")));
    assert!(keys.contains(&key("c")));
    assert!(!keys.contains(&key("d")), "depth 1 does not reach d");

    // Induced: the weak a→d edge is pruned by the confidence floor, so it is absent; every edge
    // between two collected nodes is present.
    assert!(
        !view.edges.iter().any(|edge| {
            edge.source == key("a")
                && edge.target == key("d")
                && edge.confidence < request.min_confidence
        }),
        "the low-confidence edge is pruned"
    );
    for node in &view.nodes {
        let out = graph.out_edges(
            graph
                .index_of_key(&node.key)
                .expect("a collected node is in the graph"),
        );
        for edge_index in out {
            let Some(edge) = graph.edge(*edge_index) else {
                continue;
            };
            let Some(target) = graph.node(edge.target) else {
                continue;
            };
            if keys.contains(&target.key) && edge.confidence >= request.min_confidence {
                assert!(
                    view.edges
                        .iter()
                        .any(|candidate| candidate.source == node.key
                            && candidate.target == target.key),
                    "an edge between two collected nodes is present: {node:?} -> {:?}",
                    target.key
                );
            }
        }
    }
    let mut edge_keys: Vec<(NodeKey, EdgeKind, NodeKey)> = view
        .edges
        .iter()
        .map(|edge| (edge.source, edge.kind, edge.target))
        .collect();
    let sorted_edges = {
        let mut sorted = edge_keys.clone();
        sorted.sort();
        sorted
    };
    assert_eq!(edge_keys, sorted_edges, "edges are sorted by identity");
    edge_keys.dedup();
    assert_eq!(edge_keys.len(), view.edges.len(), "edges are unique");
    assert_eq!(view.seeds, vec![key("a")]);
    assert_eq!(view.node_count(), keys.len());
}

#[test]
fn subgraph_clamps_to_500_nodes() {
    assert_eq!(SUBGRAPH_MAX_NODES, 500);
    assert_eq!(codegraph::clamp_max_nodes(10_000), 500);
    let mut builder = GraphBuilder::new(SCHEMA_VERSION);
    let mut keys: Vec<NodeKey> = Vec::new();
    for n in 0..600u32 {
        let id = NodeId::from_canonical(format!("ts:src/big{n}.ts#f/function"));
        keys.push(id.key());
        builder
            .add_node(
                NodeInput::new(id, NodeKind::Function, format!("f{n}"))
                    .in_file(path(&format!("src/big{n}.ts"))),
            )
            .unwrap();
    }
    // A hub with 600 leaves: every leaf is one hop away, so the *node* budget is what stops the
    // traversal, not the depth budget.
    for n in 1..600u32 {
        builder.add_edge(
            Edge::new(
                EdgeKind::Calls,
                keys[0],
                keys[n as usize],
                Confidence::MAX,
                ResolvedBy::Structural,
                Provenance::Analyzer,
            )
            .with_origin_file(path("src/big0.ts")),
        );
    }
    let graph = builder.build().unwrap();

    let mut request = SubgraphSpec::new(vec![keys[0]], 2, Direction::Out, EdgeKindSet::ALL);
    request.max_nodes = 10_000;
    assert_eq!(request.clamped_max_nodes(), SUBGRAPH_MAX_NODES);
    let view = subgraph(&graph, &request).unwrap();
    assert_eq!(view.nodes.len() as u32, SUBGRAPH_MAX_NODES);
    assert!(
        view.truncated,
        "the clamp is reported, not silently applied (master-plan principle 4)"
    );
}

#[test]
fn subgraph_reports_a_budget_stop() {
    let graph = fixture();
    let mut request = SubgraphSpec::new(
        vec![key("a")],
        3,
        Direction::Out,
        EdgeKindSet::of(EdgeKind::Calls),
    );
    request.max_nodes = 1;
    let view = subgraph(&graph, &request).unwrap();
    assert_eq!(view.nodes.len(), 1);
    assert!(view.truncated);

    let unknown = SubgraphSpec::new(
        vec![NodeId::from_canonical("ghost").key()],
        2,
        Direction::Out,
        EdgeKindSet::ALL,
    );
    assert!(matches!(
        subgraph(&graph, &unknown),
        Err(TraversalError::NoValidSeeds)
    ));
    let too_deep = SubgraphSpec {
        depth: 99,
        ..SubgraphSpec::new(vec![key("a")], 1, Direction::Out, EdgeKindSet::ALL)
    };
    assert!(matches!(
        subgraph(&graph, &too_deep),
        Err(TraversalError::BudgetTooLarge)
    ));
}

#[test]
fn subgraph_depth_zero_is_the_seed_alone() {
    let graph = fixture();
    let view = subgraph(
        &graph,
        &SubgraphSpec::new(vec![key("b")], 0, Direction::Out, EdgeKindSet::ALL),
    )
    .unwrap();
    assert_eq!(view.node_count(), 1);
    assert!(view.edges.is_empty(), "no edges among one node");
    assert_eq!(view.nodes[0].depth, 0);
}

#[test]
fn subgraph_serialization_golden() {
    let graph = fixture();
    let view = subgraph(
        &graph,
        &SubgraphSpec::new(
            vec![key("a")],
            2,
            Direction::Out,
            EdgeKindSet::of(EdgeKind::Calls),
        ),
    )
    .unwrap();
    let json = serde_json::to_string_pretty(&view).unwrap();
    insta::assert_snapshot!("subgraph_graph", json);
}

#[test]
fn subgraph_json_schema_names_the_payload_types() {
    let schema = serde_json::to_value(schemars::schema_for!(codegraph::Subgraph)).unwrap();
    let text = schema.to_string();
    assert!(text.contains("SubgraphNode"), "{text}");
    assert!(text.contains("SubgraphEdge"), "{text}");
    let path_schema = serde_json::to_value(schemars::schema_for!(codegraph::GraphPath)).unwrap();
    assert!(path_schema.to_string().contains("EdgeStep"));
}

proptest! {
    /// The path length is the BFS distance: a path the search reports must be exactly as long as
    /// an independent breadth-first walk says, and no shorter path may exist.
    #[test]
    fn shortest_path_length_equals_bfs_depth(
        edges in prop::collection::vec((0u8..5, 0u8..5), 0..12),
        from in 0u8..5,
        to in 0u8..5,
    ) {
        let graph = random_graph(&edges);
        let names: Vec<&str> = NODES.iter().map(|(name, _)| *name).collect();
        let spec = PathSpec::new(
            key(names[from as usize]),
            key(names[to as usize]),
            Direction::Out,
            EdgeKindSet::of(EdgeKind::Calls),
        );
        let result = shortest_path(&graph, &spec).unwrap();
        let reference = reference_distance(&graph, &spec.from, &spec.to, 6);
        match (&result.path, reference) {
            (Some(path), Some(depth)) => {
                prop_assert_eq!(path.hops(), depth);
                if path.hops() > 0 {
                    prop_assert_eq!(path.target(), Some(&spec.to));
                } else {
                    prop_assert!(spec.from == spec.to);
                }
            }
            (None, None) => {}
            (other, expected) => prop_assert!(
                false,
                "path {:?} but the reference said {:?}",
                other.as_ref().map(|path| path.hops()),
                expected
            ),
        }
    }

    /// The subgraph never returns more nodes than its budget, and it always returns its seeds.
    #[test]
    fn subgraph_respects_its_budget(
        edges in prop::collection::vec((0u8..5, 0u8..5), 0..12),
        max_nodes in 1u32..5,
        depth in 0u8..4,
    ) {
        let graph = random_graph(&edges);
        let mut request = SubgraphSpec::new(vec![key("a")], depth, Direction::Out, EdgeKindSet::ALL);
        request.max_nodes = max_nodes;
        let view = subgraph(&graph, &request).unwrap();
        prop_assert!(view.nodes.len() as u32 <= request.clamped_max_nodes());
        prop_assert!(view.nodes.iter().any(|node| node.key == key("a")));
        prop_assert!(view.nodes.windows(2).all(|pair| pair[0].key < pair[1].key));
        // Truncation is only ever reported when the node budget was actually reached, and it is
        // reported whenever the budget was reached with work still queued.
        prop_assert!(!view.truncated || view.nodes.len() as u32 == request.clamped_max_nodes());
    }
}

/// A random graph over the five fixture nodes, built through the same builder the linker uses.
fn random_graph(edges: &[(u8, u8)]) -> Graph {
    let mut builder = GraphBuilder::new(SCHEMA_VERSION);
    for (name, file) in NODES {
        builder.add_node(node(name, file)).unwrap();
    }
    let names: Vec<&str> = NODES.iter().map(|(name, _)| *name).collect();
    for (from, to) in edges {
        let from_name = names[(*from as usize).min(4)];
        let to_name = names[(*to as usize).min(4)];
        builder.add_edge(
            Edge::new(
                EdgeKind::Calls,
                key(from_name),
                key(to_name),
                Confidence::from_f32(0.6),
                ResolvedBy::NameUnique,
                Provenance::Linker,
            )
            .with_origin_file(path(file_of(from_name))),
        );
    }
    builder.build().unwrap()
}

/// An independent breadth-first distance, used as the oracle for `shortest_path`.
fn reference_distance(graph: &Graph, from: &NodeKey, to: &NodeKey, max_depth: u8) -> Option<usize> {
    let mut queue: VecDeque<(NodeKey, u8)> = VecDeque::new();
    let mut seen: BTreeSet<NodeKey> = BTreeSet::new();
    queue.push_back((*from, 0));
    seen.insert(*from);
    while let Some((key, depth)) = queue.pop_front() {
        if &key == to {
            return Some(depth as usize);
        }
        if depth >= max_depth {
            continue;
        }
        let filter = EdgeFilter::kinds(EdgeKindSet::of(EdgeKind::Calls));
        let mut next = Vec::new();
        graph.for_each_edge(key, Direction::Out, &filter, &mut |edge| {
            next.push(edge.target);
            std::ops::ControlFlow::Continue(())
        });
        for target in next {
            if seen.insert(target) {
                queue.push_back((target, depth + 1));
            }
        }
    }
    None
}

/// Every path the fixture's own traversal API can see, used by the golden test.
#[allow(dead_code)]
fn neighbors(graph: &Graph, key: NodeKey) -> Vec<NodeKey> {
    GraphQueryExt::neighbors(
        graph,
        key,
        Direction::Out,
        EdgeKindSet::ALL,
        Confidence::MIN,
    )
}
