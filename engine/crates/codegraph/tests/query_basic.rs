//! CG-007 acceptance: the `GraphQuery` interface filters, orders and stops exactly as its
//! contract says.
//!
//! The fixture is a four-node graph with mixed kinds and confidences, built by hand so the
//! expectations in each test are literals rather than re-runs of the code under test.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashSet;
use std::ops::ControlFlow;

use codegraph::{
    Confidence, Direction, Edge, EdgeFilter, EdgeKind, EdgeKindSet, EdgeSelector, FileInput, Graph,
    GraphBuilder, GraphQuery, GraphQueryExt, NodeId, NodeInput, NodeKey, NodeKind, Provenance,
    ResolvedBy, ReverseView, UnresolvedReason, UnresolvedRef, SCHEMA_VERSION,
};
use proptest::prelude::*;
use review_core::language::Language;
use review_core::location::{ContentHash, RepoPath};

const REPO_ID: &str = "repo:/";
const A_ID: &str = "ts:src/a.ts#A/f/a";
const B_ID: &str = "ts:src/b.ts#B/f/b";
const C_ID: &str = "ts:src/b.ts#C/f/c";

fn path(raw: &str) -> RepoPath {
    RepoPath::new(raw).unwrap()
}

fn key(id: &str) -> NodeKey {
    NodeId::from_canonical(id).key()
}

fn fixture() -> Graph {
    let mut builder = GraphBuilder::new(SCHEMA_VERSION);
    builder
        .add_file(FileInput {
            path: path("src/a.ts"),
            file_version_id: Some(7),
            content_hash: ContentHash::of(b"a.ts"),
            language: Language::Typescript,
        })
        .unwrap();
    builder
        .add_file(FileInput {
            path: path("src/b.ts"),
            file_version_id: None,
            content_hash: ContentHash::of(b"b.ts"),
            language: Language::Typescript,
        })
        .unwrap();
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
    for (id, file) in [(A_ID, "src/a.ts"), (B_ID, "src/b.ts"), (C_ID, "src/b.ts")] {
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
    builder.add_edge(
        edge(EdgeKind::Calls, A_ID, B_ID, Confidence::MAX).with_location(codegraph::Location::new(
            path("src/a.ts"),
            10,
            4,
        )),
    );
    builder.add_edge(edge(EdgeKind::Calls, A_ID, C_ID, Confidence::from_f32(0.6)));
    builder.add_edge(edge(
        EdgeKind::UsesType,
        A_ID,
        B_ID,
        Confidence::from_f32(0.9),
    ));
    builder.add_edge(edge(EdgeKind::References, A_ID, REPO_ID, Confidence::MAX));
    builder.add_edge(
        edge(EdgeKind::Calls, B_ID, A_ID, Confidence::MAX).with_location(codegraph::Location::new(
            path("src/b.ts"),
            3,
            1,
        )),
    );
    builder.add_edge(edge(EdgeKind::Reads, C_ID, A_ID, Confidence::from_f32(0.7)));
    builder.add_unresolved(UnresolvedRef {
        file: path("src/a.ts"),
        ordinal: 0,
        from: Some(key(A_ID)),
        name: "missingHelper".to_owned(),
        kind: analysis_ir::reference::RefKind::Call,
        import_specifier: None,
        location: codegraph::Location::new(path("src/a.ts"), 12, 9),
        reason: UnresolvedReason::External,
        candidate_count: 0,
    });
    builder.build().unwrap()
}

/// Out- and in-edge queries each see their own direction, and the kind filter excludes the
/// rest.
#[test]
fn out_and_in_edges_respect_kind_filter() {
    let graph = fixture();
    let q: &dyn GraphQuery = &graph;
    let a = key(A_ID);

    let calls_out: Vec<NodeKey> = q
        .out_edges(a, EdgeKindSet::of(EdgeKind::Calls))
        .iter()
        .map(|edge| edge.target)
        .collect();
    assert_eq!(
        calls_out.iter().copied().collect::<HashSet<_>>(),
        HashSet::from([key(B_ID), key(C_ID)]),
        "A calls B and C"
    );
    assert_eq!(
        q.degree(
            a,
            Direction::Out,
            &EdgeFilter::kinds(EdgeKindSet::of(EdgeKind::Calls))
        ),
        2
    );

    let calls_in: Vec<NodeKey> = q
        .in_edges(a, EdgeKindSet::of(EdgeKind::Calls))
        .iter()
        .map(|edge| edge.source)
        .collect();
    assert_eq!(calls_in, vec![key(B_ID)], "only B calls A");
    assert_eq!(
        q.degree(
            a,
            Direction::In,
            &EdgeFilter::kinds(EdgeKindSet::of(EdgeKind::Calls))
        ),
        1
    );

    // The type edge is out of A, but no type edge points at A.
    assert_eq!(q.out_edges(a, EdgeKindSet::of(EdgeKind::UsesType)).len(), 1);
    assert_eq!(q.in_edges(a, EdgeKindSet::of(EdgeKind::UsesType)).len(), 0);
    // An empty kind set matches nothing in either direction.
    assert_eq!(q.degree(a, Direction::Both, &EdgeFilter::NONE), 0);
}

/// The confidence floor is inclusive and independent of the kind set.
#[test]
fn min_confidence_filters_edges() {
    let graph = fixture();
    let q: &dyn GraphQuery = &graph;
    let a = key(A_ID);
    let calls = EdgeKindSet::of(EdgeKind::Calls);

    assert_eq!(
        q.degree(a, Direction::Out, &EdgeFilter::new(calls, Confidence::MIN)),
        2,
        "no floor keeps both calls"
    );
    assert_eq!(
        q.degree(
            a,
            Direction::Out,
            &EdgeFilter::new(calls, Confidence::from_f32(0.9))
        ),
        1,
        "the 600 call drops below 900"
    );
    assert_eq!(
        q.degree(a, Direction::Out, &EdgeFilter::new(calls, Confidence::MAX)),
        1,
        "the floor is inclusive: 1000 still passes a 1000 floor"
    );
    assert_eq!(
        q.degree(
            a,
            Direction::Out,
            &EdgeFilter::new(
                EdgeKindSet::of(EdgeKind::UsesType),
                Confidence::from_f32(0.95)
            )
        ),
        0,
        "the kind passes but its 900 confidence is under the 950 floor"
    );
}

/// `CALLED_BY(k)` is exactly the in-edges of the stored `CALLS` kind — the view is an alias
/// for a reverse traversal, never a stored edge of its own.
#[test]
fn reverse_views_equal_in_edges_of_underlying_kind() {
    let graph = fixture();
    let q: &dyn GraphQuery = &graph;
    let a = key(A_ID);

    let via_view = q.view(a, ReverseView::CalledBy);
    let via_kinds = q.in_edges(a, EdgeKindSet::of(EdgeKind::Calls));
    let describe = |edges: &[codegraph::EdgeRef<'_>]| -> Vec<(EdgeKind, NodeKey, Confidence)> {
        edges
            .iter()
            .map(|edge| (edge.kind, edge.source, edge.confidence))
            .collect()
    };
    assert!(!via_view.is_empty());
    assert_eq!(describe(&via_view), describe(&via_kinds));
    assert!(via_view.iter().all(|edge| edge.kind == EdgeKind::Calls));

    // The same hold through `from_selectors`: the view lands on the in filter, the stored
    // kind on the out filter.
    let selectors = [
        EdgeSelector::from(EdgeKind::Calls),
        EdgeSelector::from(ReverseView::CalledBy),
    ];
    let (out, inn) = EdgeFilter::from_selectors(&selectors);
    let mut from_out = Vec::new();
    q.for_each_edge(a, Direction::Out, &out, &mut |edge| {
        from_out.push(edge);
        ControlFlow::Continue(())
    });
    let mut from_in = Vec::new();
    q.for_each_edge(a, Direction::In, &inn, &mut |edge| {
        from_in.push(edge);
        ControlFlow::Continue(())
    });
    assert_eq!(describe(&from_out), describe(&q.out_edges(a, out.kinds)));
    assert_eq!(describe(&from_in), describe(&via_view));
}

/// Edges come out sorted by kind discriminant, then by the other endpoint's key, in both
/// directions.
#[test]
fn iteration_order_is_kind_then_key() {
    let graph = fixture();
    let q: &dyn GraphQuery = &graph;
    let a = key(A_ID);
    for (dir, other_is_target) in [(Direction::Out, true), (Direction::In, false)] {
        let mut seen = Vec::new();
        q.for_each_edge(a, dir, &EdgeFilter::ALL, &mut |edge| {
            let other = if other_is_target {
                edge.target
            } else {
                edge.source
            };
            seen.push((edge.kind.as_u8(), other, edge.confidence));
            ControlFlow::Continue(())
        });
        let mut expected = seen.clone();
        expected.sort_by_key(|(kind, other, _)| (*kind, *other));
        assert_eq!(seen, expected, "{dir:?} edges are not in (kind, key) order");
        assert!(!seen.is_empty(), "the fixture must have {dir:?} edges");
    }
}

/// A node reachable through two kinds appears once, and the list is sorted by key.
#[test]
fn neighbors_dedupes_and_sorts() {
    let graph = fixture();
    let q: &dyn GraphQuery = &graph;
    let a = key(A_ID);
    let kinds = EdgeKindSet::EMPTY
        .union(EdgeKindSet::of(EdgeKind::Calls))
        .union(EdgeKindSet::of(EdgeKind::UsesType));

    let neighbors = q.neighbors(a, Direction::Out, kinds, Confidence::MIN);
    assert_eq!(
        neighbors,
        vec![key(B_ID), key(C_ID)]
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>(),
        "B is reachable by both CALLS and USES_TYPE but appears once, sorted"
    );

    let reverse = q.neighbors(a, Direction::In, EdgeKindSet::ALL, Confidence::MIN);
    let mut expected_in = vec![key(B_ID), key(C_ID)];
    expected_in.sort_unstable();
    assert_eq!(
        reverse, expected_in,
        "B and C point at A (CALLS and READS), sorted by key"
    );

    // A confidence floor that drops the C call leaves only B.
    let strict = q.neighbors(
        a,
        Direction::Out,
        EdgeKindSet::of(EdgeKind::Calls),
        Confidence::from_f32(0.9),
    );
    assert_eq!(strict, vec![key(B_ID)]);
}

/// A visitor that breaks stops the walk; the degree it could have counted is unchanged.
#[test]
fn visitor_break_stops_early() {
    let graph = fixture();
    let q: &dyn GraphQuery = &graph;
    let a = key(A_ID);
    let mut visited = 0usize;
    q.for_each_edge(a, Direction::Out, &EdgeFilter::ALL, &mut |_| {
        visited += 1;
        if visited == 1 {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    });
    assert_eq!(visited, 1, "the walk must stop at the first Break");
    assert_eq!(
        q.degree(a, Direction::Out, &EdgeFilter::ALL),
        4,
        "breaking does not change how many edges there are"
    );
    // A visitor that never breaks sees them all.
    let mut all = 0usize;
    q.for_each_edge(a, Direction::Out, &EdgeFilter::ALL, &mut |_| {
        all += 1;
        ControlFlow::Continue(())
    });
    assert_eq!(all, 4);
}

/// Unknown keys, paths and ids are empty answers, never errors.
#[test]
fn unknown_key_is_empty() {
    let graph = fixture();
    let q: &dyn GraphQuery = &graph;
    let missing = NodeId::from_canonical("ts:src/ghost.ts#G/f/g").key();

    assert!(q.node(missing).is_none());
    assert!(q.node_by_id("ts:src/ghost.ts#G/f/g").is_none());
    assert_eq!(q.degree(missing, Direction::Both, &EdgeFilter::ALL), 0);
    assert_eq!(
        q.neighbors(missing, Direction::Both, EdgeKindSet::ALL, Confidence::MIN),
        Vec::<NodeKey>::new()
    );

    let mut visited = 0;
    q.for_each_edge(missing, Direction::Both, &EdgeFilter::ALL, &mut |_| {
        visited += 1;
        ControlFlow::Continue(())
    });
    assert_eq!(visited, 0);

    let mut nodes = 0;
    q.nodes_in_file("src/nowhere.ts", &mut |_| nodes += 1);
    assert_eq!(nodes, 0);
    assert!(q.file("src/nowhere.ts").is_none());
    let mut owned = 0;
    q.edges_owned_by("src/nowhere.ts", &mut |_| owned += 1);
    assert_eq!(owned, 0);

    // The name index answers with nothing for a name nobody references.
    let mut refs = 0;
    q.unresolved_named("noSuchName", &mut |_| refs += 1);
    assert_eq!(refs, 0);
}

/// `node_by_id` and `node` are the same node reached two ways.
#[test]
fn node_by_id_matches_node_by_key() {
    let graph = fixture();
    let q: &dyn GraphQuery = &graph;
    let by_id = q.node_by_id(A_ID).expect("A is in the fixture");
    let by_key = q.node(key(A_ID)).expect("A is in the fixture");
    assert_eq!(by_id.key, by_key.key);
    assert_eq!(by_id.id, A_ID);
    assert_eq!(by_id.name, by_key.name);
    assert_eq!(by_id.kind, by_key.kind);
    assert_eq!(by_id.file, Some("src/a.ts"));
    assert_eq!(by_id.attrs, by_key.attrs);

    // The id is a plain string lookup: the interned id string round-trips.
    let mut ids = Vec::new();
    q.for_each_node(&mut |node| ids.push(node.id.to_owned()));
    assert!(
        ids.contains(&A_ID.to_owned()),
        "A is walked by for_each_node"
    );
}

/// The trait really is object safe: `&dyn GraphQuery` compiles and answers.
#[test]
fn graph_query_is_object_safe() {
    let graph = fixture();
    let query: &dyn GraphQuery = &graph;
    let q = query;
    assert_eq!(query.node_count(), graph.node_count());
    assert_eq!(query.edge_count(), 6);
    assert_eq!(query.schema_version(), SCHEMA_VERSION);
    assert!(q.node(key(A_ID)).is_some());

    // Extension helpers work through the trait object too.
    let ext: &dyn GraphQuery = &graph;
    let calls = ext.degree(
        key(A_ID),
        Direction::Out,
        &EdgeFilter::kinds(EdgeKindSet::of(EdgeKind::Calls)),
    );
    assert_eq!(calls, 2);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Proptest: the CSR answer is identical to a straight O(E) scan of the edge table, for
    /// random kind masks, confidence floors and directions — including empty filters.
    #[test]
    fn query_matches_naive_edge_scan(bits in any::<u64>(), floor in 0u16..=1000, dir in 0u8..3) {
        let graph = fixture();
        let q: &dyn GraphQuery = &graph;
        let a = key(A_ID);
        let kinds = EdgeKindSet::from_bits(bits);
        let filter = EdgeFilter::new(kinds, Confidence::from_f32(floor as f32 / 1000.0));
        let direction = match dir % 3 {
            0 => Direction::Out,
            1 => Direction::In,
            _ => Direction::Both,
        };

        let mut via_query = Vec::new();
        q.for_each_edge(a, direction, &filter, &mut |edge| {
            via_query.push((edge.kind, edge.source, edge.target, edge.confidence));
            ControlFlow::Continue(())
        });

        let mut naive: Vec<(EdgeKind, NodeKey, NodeKey, Confidence)> = graph
            .edges()
            .iter()
            .filter(|edge| filter.matches(edge))
            .filter(|edge| {
                let source = Graph::node(&graph, edge.source).map(|n| n.key);
                let target = Graph::node(&graph, edge.target).map(|n| n.key);
                match direction {
                    Direction::Out => source == Some(a),
                    Direction::In => target == Some(a),
                    Direction::Both => source == Some(a) || target == Some(a),
                }
            })
            .map(|edge| {
                (
                    edge.kind,
                    Graph::node(&graph, edge.source).map(|n| n.key).unwrap(),
                    Graph::node(&graph, edge.target).map(|n| n.key).unwrap(),
                    edge.confidence,
                )
            })
            .collect();

        // The scan is unordered; the contract is (kind, other key) inside each direction and
        // out-before-in for `Both`, so sort both sides the same way and compare.
        let key_of = |row: &(EdgeKind, NodeKey, NodeKey, Confidence),
                      outbound_first: bool| {
            let (kind, source, target, _) = row;
            let other = match direction {
                Direction::Out => *target,
                _ => *source,
            };
            (
                if outbound_first && *source == a { 0u8 } else { 1 },
                kind.as_u8(),
                other,
            )
        };
        let outbound_first = direction == Direction::Both;
        via_query.sort_by_key(|row| key_of(row, outbound_first));
        naive.sort_by_key(|row| key_of(row, outbound_first));
        prop_assert_eq!(via_query, naive);
    }
}
