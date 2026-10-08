//! CG-008 acceptance: bounded BFS respects its budgets, reports truncation, reconstructs
//! paths and aggregates path confidence — and is deterministic.
//!
//! The fixture is a six-node graph with one low-confidence edge, so pruning and min-confidence
//! aggregation have something to bite on.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::{HashMap, HashSet, VecDeque};
use std::ops::ControlFlow;

use codegraph::{
    bounded_bfs, Confidence, Direction, Edge, EdgeFilter, EdgeKind, EdgeKindSet, FileInput, Graph,
    GraphBuilder, GraphQuery, GraphQueryExt, NodeId, NodeInput, NodeKey, NodeKind, Provenance,
    ResolvedBy, TraversalError, TraversalSpec, SCHEMA_VERSION,
};
use proptest::prelude::*;
use review_core::language::Language;
use review_core::location::{ContentHash, RepoPath};

const REPO_ID: &str = "repo:/";
const A_ID: &str = "ts:src/a.ts#A/f/a";
const B_ID: &str = "ts:src/b.ts#B/f/b";
const C_ID: &str = "ts:src/c.ts#C/f/c";
const D_ID: &str = "ts:src/d.ts#D/f/d";
const E_ID: &str = "ts:src/e.ts#E/f/e";

fn path(raw: &str) -> RepoPath {
    RepoPath::new(raw).unwrap()
}

fn key(id: &str) -> NodeKey {
    NodeId::from_canonical(id).key()
}

/// A → B (1000), A → C (400), B → D (700), E → A (900), B → A (1000).
///
/// A's out-neighbours are B and C; C is the only low-confidence discovery; D sits two hops
/// out; E reaches A only against the walk direction.
fn fixture() -> Graph {
    let mut builder = GraphBuilder::new(SCHEMA_VERSION);
    for (file, hash) in [
        ("src/a.ts", b"a.ts"),
        ("src/b.ts", b"b.ts"),
        ("src/c.ts", b"c.ts"),
        ("src/d.ts", b"d.ts"),
        ("src/e.ts", b"e.ts"),
    ] {
        builder
            .add_file(FileInput {
                path: path(file),
                file_version_id: None,
                content_hash: ContentHash::of(hash),
                language: Language::Typescript,
            })
            .unwrap();
    }
    builder
        .add_node(
            NodeInput::new(
                NodeId::from_canonical(REPO_ID),
                NodeKind::Repository,
                "repo",
            )
            .qualified_name("repo"),
        )
        .unwrap();
    for (id, file) in [
        (A_ID, "src/a.ts"),
        (B_ID, "src/b.ts"),
        (C_ID, "src/c.ts"),
        (D_ID, "src/d.ts"),
        (E_ID, "src/e.ts"),
    ] {
        builder
            .add_node(
                NodeInput::new(NodeId::from_canonical(id), NodeKind::Function, "f")
                    .qualified_name(id)
                    .in_file(path(file)),
            )
            .unwrap();
    }
    let edge = |kind, from, to, confidence| {
        Edge::new(
            kind,
            key(from),
            key(to),
            confidence,
            ResolvedBy::NameUnique,
            Provenance::Linker,
        )
    };
    builder.add_edge(edge(EdgeKind::Calls, A_ID, B_ID, Confidence::MAX));
    builder.add_edge(edge(EdgeKind::Calls, A_ID, C_ID, Confidence::from_f32(0.4)));
    builder.add_edge(edge(EdgeKind::Calls, B_ID, D_ID, Confidence::from_f32(0.7)));
    builder.add_edge(edge(EdgeKind::Reads, E_ID, A_ID, Confidence::from_f32(0.9)));
    builder.add_edge(edge(EdgeKind::Calls, B_ID, A_ID, Confidence::MAX));
    builder.build().unwrap()
}

fn spec(
    seeds: Vec<NodeKey>,
    direction: Direction,
    max_depth: u8,
    max_nodes: u32,
    min_confidence: Confidence,
) -> TraversalSpec {
    let mut spec = TraversalSpec::new(
        seeds,
        direction,
        EdgeKindSet::ALL,
        max_depth,
        max_nodes,
        min_confidence,
    );
    spec.max_edges_examined = 100_000;
    spec
}

/// Reaching `max_depth` is the caller's request, not truncation — and the frontier count is
/// what tells them work remained beyond it.
#[test]
fn max_depth_is_not_truncation() {
    let graph = fixture();
    let result = bounded_bfs(
        &graph,
        &spec(vec![key(A_ID)], Direction::Out, 1, 100, Confidence::MIN),
    )
    .unwrap();
    assert_eq!(result.visits.len(), 3, "A, B and C are at depth <= 1");
    assert!(!result.truncated, "depth 1 of a depth budget is not a cut");
    assert_eq!(result.truncation, None);
    assert_eq!(
        result.frontier_at_max_depth, 1,
        "B still has an out-edge; C does not"
    );
    assert_eq!(
        result
            .visits
            .iter()
            .map(|visit| visit.key)
            .collect::<HashSet<_>>(),
        HashSet::from([key(A_ID), key(B_ID), key(C_ID)])
    );
}

/// Hitting `max_nodes` stops the walk and says so.
#[test]
fn truncated_set_when_max_nodes_hit() {
    let graph = fixture();
    let result = bounded_bfs(
        &graph,
        &spec(vec![key(A_ID)], Direction::Out, 5, 2, Confidence::MIN),
    )
    .unwrap();
    assert_eq!(result.visits.len(), 2, "seed plus one discovery");
    assert!(result.truncated);
    assert_eq!(
        result.truncation,
        Some(codegraph::Truncation::MaxNodes),
        "the stop reason must be reported, never silent"
    );
}

/// `path_to` walks parent pointers back to the seed and returns the steps in walk order.
#[test]
fn path_to_reconstructs_edges_in_order() {
    let graph = fixture();
    let result = bounded_bfs(
        &graph,
        &spec(vec![key(A_ID)], Direction::Out, 5, 100, Confidence::MIN),
    )
    .unwrap();
    let path = result.path_to(key(D_ID)).expect("D is two hops out");
    assert_eq!(path.len(), 2, "A -> B -> D");
    assert_eq!(path[0].source, key(A_ID));
    assert_eq!(path[0].target, key(B_ID));
    assert_eq!(path[0].kind, EdgeKind::Calls);
    assert_eq!(path[1].source, key(B_ID));
    assert_eq!(path[1].target, key(D_ID));
    assert_eq!(path[1].kind, EdgeKind::Calls);
    assert_eq!(
        result.path_to(key(A_ID)),
        Some(Vec::new()),
        "a seed is an empty path, not None"
    );
    assert_eq!(result.path_to(key(REPO_ID)), None, "unvisited is None");
}

/// `path_confidence` is the minimum edge confidence along the path, seeds included.
#[test]
fn path_confidence_is_min_along_path() {
    let graph = fixture();
    let result = bounded_bfs(
        &graph,
        &spec(vec![key(A_ID)], Direction::Out, 5, 100, Confidence::MIN),
    )
    .unwrap();
    let steps = result.path_to(key(D_ID)).unwrap();
    let min_along_path = steps
        .iter()
        .map(|step| step.confidence)
        .min()
        .expect("two steps");
    assert_eq!(
        result.visit(key(D_ID)).unwrap().path_confidence,
        min_along_path
    );
    assert_eq!(
        result.visit(key(D_ID)).unwrap().path_confidence,
        Confidence::from_f32(0.7),
        "min(1000, 700) = 700"
    );
    assert_eq!(
        result.visit(key(C_ID)).unwrap().path_confidence,
        Confidence::from_f32(0.4),
        "the weak single hop carries its own confidence"
    );
    assert_eq!(
        result.visit(key(A_ID)).unwrap().path_confidence,
        Confidence::MAX,
        "seeds start at 1.0"
    );
}

/// Edges under `min_confidence` are never followed, so their targets never appear.
#[test]
fn min_confidence_prunes_low_edges() {
    let graph = fixture();
    let result = bounded_bfs(
        &graph,
        &spec(
            vec![key(A_ID)],
            Direction::Out,
            5,
            100,
            Confidence::from_f32(0.5),
        ),
    )
    .unwrap();
    let visited: HashSet<NodeKey> = result.visits.iter().map(|visit| visit.key).collect();
    assert_eq!(
        visited,
        HashSet::from([key(A_ID), key(B_ID), key(D_ID)]),
        "the 400-permille A -> C edge is pruned, C is unreachable"
    );
    assert_eq!(result.truncation, None, "pruning is not truncation");
}

/// Same graph, same spec, byte-identical result — every time.
#[test]
fn deterministic_across_runs() {
    let graph = fixture();
    let spec = spec(vec![key(A_ID)], Direction::Both, 4, 100, Confidence::MIN);
    let first = serde_json::to_string(&bounded_bfs(&graph, &spec).unwrap()).unwrap();
    for run in 0..5 {
        let again = serde_json::to_string(&bounded_bfs(&graph, &spec).unwrap()).unwrap();
        assert_eq!(again, first, "run {run} diverged");
    }
}

/// `Both` expands the out slice first, then the in slice; a node reached both ways keeps its
/// first discovery.
#[test]
fn both_direction_expansion_order() {
    let graph = fixture();
    let q: &dyn GraphQuery = &graph;
    let a = key(A_ID);
    let result = bounded_bfs(
        &graph,
        &spec(vec![a], Direction::Both, 1, 100, Confidence::MIN),
    )
    .unwrap();

    let mut expected: Vec<NodeKey> = Vec::new();
    for edge in q.out_edges(a, EdgeKindSet::ALL) {
        if !expected.contains(&edge.target) {
            expected.push(edge.target);
        }
    }
    for edge in q.in_edges(a, EdgeKindSet::ALL) {
        if !expected.contains(&edge.source) {
            expected.push(edge.source);
        }
    }
    let depth_one: Vec<NodeKey> = result
        .visits
        .iter()
        .filter(|visit| visit.depth == 1)
        .map(|visit| visit.key)
        .collect();
    assert_eq!(depth_one, expected, "out neighbours first, then in");
    assert!(
        depth_one.contains(&key(E_ID)),
        "E reaches A only against the walk direction"
    );
    assert_eq!(
        depth_one.iter().filter(|seen| **seen == key(B_ID)).count(),
        1,
        "B is discovered once even though it is both an out- and an in-neighbour"
    );
}

/// Budgets are validated before any work happens.
#[test]
fn budget_too_large_is_rejected() {
    let graph = fixture();
    let mut deep = spec(vec![key(A_ID)], Direction::Out, 17, 10, Confidence::MIN);
    deep.max_edges_examined = 10;
    assert_eq!(
        bounded_bfs(&graph, &deep),
        Err(TraversalError::BudgetTooLarge)
    );
    let mut wide = spec(
        vec![key(A_ID)],
        Direction::Out,
        4,
        1_000_001,
        Confidence::MIN,
    );
    wide.max_edges_examined = 10;
    assert_eq!(
        bounded_bfs(&graph, &wide),
        Err(TraversalError::BudgetTooLarge)
    );
}

/// Unknown seeds are dropped; only "all unknown" is an error.
#[test]
fn unknown_seeds_are_dropped() {
    let graph = fixture();
    let result = bounded_bfs(
        &graph,
        &spec(
            vec![
                NodeId::from_canonical("ts:src/nope.ts#X/f/x").key(),
                key(A_ID),
            ],
            Direction::Out,
            5,
            100,
            Confidence::MIN,
        ),
    )
    .unwrap();
    assert_eq!(
        result.visits.first().unwrap().key,
        key(A_ID),
        "the unknown seed is gone"
    );
    assert_eq!(
        bounded_bfs(
            &graph,
            &spec(
                vec![NodeId::from_canonical("ts:src/nope.ts#X/f/x").key()],
                Direction::Out,
                5,
                100,
                Confidence::MIN
            ),
        ),
        Err(TraversalError::NoValidSeeds)
    );
}

/// Deterministic depth-first lookup used as the reference for `bfs_depths_are_shortest`.
fn reference_depths(
    graph: &dyn GraphQuery,
    seeds: &[NodeKey],
    direction: Direction,
    filter: &EdgeFilter,
) -> HashMap<NodeKey, u8> {
    let mut sorted = seeds.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let mut depth: HashMap<NodeKey, u8> = HashMap::new();
    let mut queue: VecDeque<NodeKey> = VecDeque::new();
    for seed in sorted {
        if graph.node(seed).is_some() && depth.insert(seed, 0).is_none() {
            queue.push_back(seed);
        }
    }
    while let Some(key) = queue.pop_front() {
        let here = depth.get(&key).copied().unwrap_or(0);
        if here >= 16 {
            continue;
        }
        graph.for_each_edge(key, direction, filter, &mut |edge| {
            let other = if edge.source == key {
                edge.target
            } else {
                edge.source
            };
            if let std::collections::hash_map::Entry::Vacant(slot) = depth.entry(other) {
                slot.insert(here + 1);
                queue.push_back(other);
            }
            ControlFlow::Continue(())
        });
    }
    depth
}

/// Random graph: `nodes` function nodes plus random edges with random kinds and confidences.
fn arb_graph() -> impl Strategy<Value = (Graph, Vec<NodeKey>)> {
    (
        2usize..9,
        proptest::collection::vec((any::<u8>(), any::<u8>(), 0usize..33, any::<u16>()), 0..24),
    )
        .prop_map(|(nodes, raw_edges)| {
            let mut builder = GraphBuilder::new(SCHEMA_VERSION);
            builder
                .add_file(FileInput {
                    path: path("src/x.ts"),
                    file_version_id: None,
                    content_hash: ContentHash::of(b"x.ts"),
                    language: Language::Typescript,
                })
                .unwrap();
            let ids: Vec<String> = (0..nodes)
                .map(|index| format!("ts:src/x.ts#n{index}/f/x"))
                .collect();
            for id in &ids {
                builder
                    .add_node(
                        NodeInput::new(
                            NodeId::from_canonical(id.as_str()),
                            NodeKind::Function,
                            "f",
                        )
                        .qualified_name(id.as_str())
                        .in_file(path("src/x.ts")),
                    )
                    .unwrap();
            }
            for (from, to, kind, confidence) in raw_edges {
                let from = from as usize % nodes;
                let to = to as usize % nodes;
                builder.add_edge(Edge::new(
                    EdgeKind::ALL
                        .get(kind % EdgeKind::ALL.len())
                        .copied()
                        .unwrap(),
                    NodeId::from_canonical(ids[from].as_str()).key(),
                    NodeId::from_canonical(ids[to].as_str()).key(),
                    Confidence::from_f32(f32::from(confidence) / 1000.0),
                    ResolvedBy::NameUnique,
                    Provenance::Linker,
                ));
            }
            let keys: Vec<NodeKey> = ids
                .iter()
                .map(|id| NodeId::from_canonical(id.as_str()).key())
                .collect();
            (builder.build().unwrap(), keys)
        })
}

fn direction(value: u8) -> Direction {
    match value % 3 {
        0 => Direction::Out,
        1 => Direction::In,
        _ => Direction::Both,
    }
}

fn seeds_from(keys: &[NodeKey], bits: u64) -> Vec<NodeKey> {
    keys.iter()
        .enumerate()
        .filter(|(index, _)| bits & (1u64 << (index % 64)) != 0)
        .map(|(_, key)| *key)
        .collect()
}

proptest! {
    /// The two hard budgets are hard: whatever the graph and the spec, `visits` never grows
    /// past `max_nodes`, `edges_examined` never past `max_edges_examined`, and depth never
    /// past `max_depth` — with `truncated` flagging every (and only) budget stop.
    #[test]
    fn bfs_never_exceeds_max_nodes_or_edges_examined(
        (graph, keys) in arb_graph(),
        dir in any::<u8>(),
        max_depth in 0u8..17,
        max_nodes in 1u32..40,
        max_edges in 1u32..600,
        floor in 0u16..1000,
        bits in any::<u64>(),
    ) {
        let seeds = seeds_from(&keys, bits);
        let direction = direction(dir);
        let confidence = Confidence::from_f32(f32::from(floor) / 1000.0);
        let mut spec = TraversalSpec::new(
            seeds,
            direction,
            EdgeKindSet::ALL,
            max_depth,
            max_nodes,
            confidence,
        );
        spec.max_edges_examined = max_edges;
        let result = match bounded_bfs(&graph, &spec) {
            Ok(result) => result,
            Err(TraversalError::NoValidSeeds) => return Ok(()),
            Err(other) => return Err(TestCaseError::fail(other.to_string())),
        };
        prop_assert!(result.visits.len() <= max_nodes as usize);
        prop_assert!(result.edges_examined <= max_edges);
        prop_assert!(result.visits.iter().all(|visit| visit.depth <= max_depth));
        prop_assert_eq!(result.truncated, result.truncation.is_some());
        let unique: HashSet<NodeKey> = result.visits.iter().map(|visit| visit.key).collect();
        prop_assert_eq!(unique.len(), result.visits.len(), "a node is visited once");
    }

    /// With budgets out of the way, BFS depths equal an independent reference BFS on the
    /// same filter — the first discovery really is a shortest path.
    #[test]
    fn bfs_depths_are_shortest(
        (graph, keys) in arb_graph(),
        dir in any::<u8>(),
        floor in 0u16..1000,
        bits in any::<u64>(),
    ) {
        let seeds = seeds_from(&keys, bits);
        let direction = direction(dir);
        let confidence = Confidence::from_f32(f32::from(floor) / 1000.0);
        let mut spec = TraversalSpec::new(
            seeds.clone(),
            direction,
            EdgeKindSet::ALL,
            16,
            1_000,
            confidence,
        );
        spec.max_edges_examined = u32::MAX;
        let result = match bounded_bfs(&graph, &spec) {
            Ok(result) => result,
            Err(TraversalError::NoValidSeeds) => return Ok(()),
            Err(other) => return Err(TestCaseError::fail(other.to_string())),
        };
        prop_assert_eq!(result.truncation, None, "nothing should be cut");
        let filter = EdgeFilter::new(EdgeKindSet::ALL, confidence);
        let reference = reference_depths(&graph, &seeds, direction, &filter);
        prop_assert_eq!(result.visits.len(), reference.len());
        for (key, depth) in &reference {
            let visit = result.visit(*key)
                .expect("reference node missing from the traversal");
            prop_assert_eq!(visit.depth, *depth, "depth of {:?}", key);
            prop_assert!(visit.depth <= 16);
        }
    }
}
