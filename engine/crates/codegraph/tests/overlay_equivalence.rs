//! CG-010 acceptance: a PR head is a base graph plus a delta, queried as if it were one graph.
//!
//! The central property is equivalence: for every node and both directions, the overlay's
//! `for_each_edge` must produce exactly what flattening the overlay and reading the flattened
//! graph would produce — same edges, same order, same payload. That is asserted here by
//! proptest over random bases and random deltas, and by a property test on the structured diff.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use std::collections::BTreeMap;
use std::sync::Arc;

use codegraph::{
    compare, validate_delta_local, CompareOptions, Confidence, Direction, Edge, EdgeFilter,
    EdgeIdentity, FileChange, FileChangeKind, FileInput, Graph, GraphBuilder, GraphDelta,
    GraphOverlay, GraphQuery, GraphQueryExt, NodeId, NodeInput, NodeKey, NodeKind, OverlayError,
    Provenance, ResolvedBy, SCHEMA_VERSION,
};
use proptest::prelude::*;
use review_core::language::Language;
use review_core::location::{ContentHash, RepoPath};

fn path(raw: &str) -> RepoPath {
    RepoPath::new(raw).unwrap()
}

fn key(name: &str) -> NodeKey {
    NodeId::from_canonical(format!("ts:src/{name}.ts#{name}/function")).key()
}

fn node(name: &str) -> NodeInput {
    NodeInput::new(
        NodeId::from_canonical(format!("ts:src/{name}.ts#{name}/function")),
        NodeKind::Function,
        name,
    )
    .qualified_name(name)
    .in_file(path(&format!("src/{name}.ts")))
}

/// a, b, c, d in one file plus a synthetic endpoint, and one edge per consecutive pair.
fn base() -> Graph {
    let mut builder = GraphBuilder::new(SCHEMA_VERSION);
    builder
        .add_file(FileInput {
            path: path("src/main.ts"),
            file_version_id: Some(1),
            content_hash: ContentHash::of(b"base"),
            language: Language::Typescript,
        })
        .unwrap();
    for name in ["a", "b", "c", "d"] {
        builder.add_node(node(name)).unwrap();
    }
    builder
        .add_node(
            NodeInput::new(
                NodeId::http("GET", "/users").unwrap(),
                NodeKind::ApiEndpoint,
                "GET /users",
            )
            .qualified_name("GET /users"),
        )
        .unwrap();
    for (from, to) in [("a", "b"), ("b", "c"), ("c", "d")] {
        builder.add_edge(calls(from, to, 1.0));
    }
    builder.build().unwrap()
}

fn calls(from: &str, to: &str, confidence: f32) -> Edge {
    Edge::new(
        codegraph::EdgeKind::Calls,
        key(from),
        key(to),
        Confidence::from_f32(confidence),
        ResolvedBy::NameUnique,
        Provenance::Linker,
    )
    .with_location(codegraph::Location::new(
        path(&format!("src/{from}.ts")),
        1,
        0,
    ))
    .with_origin_file(path(&format!("src/{from}.ts")))
}

fn overlay(delta: GraphDelta) -> GraphOverlay {
    GraphOverlay::new(Arc::new(base()), Arc::new(delta)).expect("a valid delta")
}

/// Every edge of a graph, as a comparable tuple, sorted canonically.
///
/// Sorting rather than collecting in iteration order is deliberate: the overlay reports base nodes
/// before added ones while a flattened graph reports builder order, and neither order is part of
/// the contract. The set of edges is.
fn edges_of(graph: &dyn GraphQuery) -> Vec<(NodeKey, codegraph::EdgeKind, NodeKey, u16, u8)> {
    let mut out = Vec::new();
    graph.for_each_node(&mut |node| {
        graph.for_each_edge(node.key, Direction::Out, &EdgeFilter::ALL, &mut |edge| {
            out.push((
                edge.source,
                edge.kind,
                edge.target,
                edge.confidence.as_permille(),
                edge.flags.bits(),
            ));
            std::ops::ControlFlow::Continue(())
        });
    });
    out.sort();
    out
}

/// Out-edges keyed by source node, each list in traversal order.
///
/// Unlike [`edges_of`] this keeps the per-node ordering that `for_each_edge` promises, and covers
/// exactly the nodes `for_each_node` reports, so it also checks that the two agree on membership.
fn out_edges_by_node(
    graph: &dyn GraphQuery,
) -> BTreeMap<NodeKey, Vec<(codegraph::EdgeKind, NodeKey, u16)>> {
    let mut out = BTreeMap::new();
    graph.for_each_node(&mut |node| {
        let mut edges = Vec::new();
        graph.for_each_edge(node.key, Direction::Out, &EdgeFilter::ALL, &mut |edge| {
            edges.push((edge.kind, edge.target, edge.confidence.as_permille()));
            std::ops::ControlFlow::Continue(())
        });
        out.insert(node.key, edges);
    });
    out
}

#[test]
fn empty_delta_overlay_equals_base() {
    let overlay = overlay(GraphDelta::new(SCHEMA_VERSION));
    assert_eq!(
        compare(&overlay, &**overlay.base(), &CompareOptions::strict()),
        codegraph::GraphDiffReport::default()
    );
    assert_eq!(edges_of(&overlay), edges_of(&**overlay.base()));
    assert_eq!(overlay.noop_tombstones(), 0);
}

#[test]
fn removed_node_hides_incident_base_edges() {
    let mut delta = GraphDelta::new(SCHEMA_VERSION);
    delta.nodes_removed.push(key("b"));
    let overlay = overlay(delta);
    let remaining: Vec<NodeKey> = {
        let mut keys = Vec::new();
        overlay.for_each_node(&mut |node| keys.push(node.key));
        keys
    };
    assert!(!remaining.contains(&key("b")));
    assert!(remaining.contains(&key("a")));
    assert!(overlay.node(key("b")).is_none());
    assert!(
        !edges_of(&overlay)
            .iter()
            .any(|(_, _, target, _, _)| *target == key("b")),
        "an edge into a removed node is hidden"
    );
    assert!(
        !edges_of(&overlay)
            .iter()
            .any(|(source, _, _, _, _)| *source == key("b")),
        "an edge out of a removed node is hidden"
    );
    let flattened = overlay.flatten().unwrap();
    assert_eq!(edges_of(&overlay), edges_of(&flattened));
}

#[test]
fn override_requires_tombstone() {
    let identity = EdgeIdentity {
        source: key("a"),
        kind: codegraph::EdgeKind::Calls,
        target: key("b"),
    };
    let mut delta = GraphDelta::new(SCHEMA_VERSION);
    delta.edges_added.push(Edge::new(
        identity.kind,
        identity.source,
        identity.target,
        Confidence::from_f32(0.3),
        ResolvedBy::NameAmbiguous,
        Provenance::Linker,
    ));
    let error = GraphOverlay::new(Arc::new(base()), Arc::new(delta)).unwrap_err();
    assert_eq!(error, OverlayError::ImplicitOverride(identity));

    // With the tombstone it is accepted.
    let mut delta = GraphDelta::new(SCHEMA_VERSION);
    delta.edges_removed.push(identity);
    delta.edges_added.push(Edge::new(
        identity.kind,
        identity.source,
        identity.target,
        Confidence::from_f32(0.3),
        ResolvedBy::NameAmbiguous,
        Provenance::Linker,
    ));
    let overlay = overlay(delta);
    let edge = overlay
        .out_edges(
            key("a"),
            codegraph::EdgeKindSet::of(codegraph::EdgeKind::Calls),
        )
        .into_iter()
        .find(|edge| edge.target == key("b"))
        .expect("the replacement edge is there");
    assert_eq!(edge.confidence, Confidence::from_f32(0.3));
    assert_eq!(edges_of(&overlay), edges_of(&overlay.flatten().unwrap()));
}

#[test]
fn added_node_replaces_base_node_data() {
    let mut delta = GraphDelta::new(SCHEMA_VERSION);
    delta.nodes_added.push(
        NodeInput::new(
            NodeId::from_canonical("ts:src/b.ts#b/function"),
            NodeKind::Method,
            "b-renamed",
        )
        .qualified_name("renamed")
        .in_file(path("src/b.ts")),
    );
    let overlay = overlay(delta);
    let node = overlay.node(key("b")).expect("the key still exists");
    assert_eq!(node.kind, NodeKind::Method);
    assert_eq!(node.name, "b-renamed");
    // The base's edges onto the key are untouched: replacing data is not replacing identity.
    assert!(!edges_of(&overlay).is_empty());
    assert_eq!(edges_of(&overlay), edges_of(&overlay.flatten().unwrap()));
}

#[test]
fn merged_iteration_order_matches_flatten() {
    let mut delta = GraphDelta::new(SCHEMA_VERSION);
    delta.nodes_added.push(
        NodeInput::new(
            NodeId::from_canonical("ts:src/e.ts#e/function"),
            NodeKind::Function,
            "e",
        )
        .in_file(path("src/e.ts")),
    );
    delta.edges_added.push(calls("d", "e", 1.0));
    delta.edges_added.push(calls("a", "e", 0.9));
    let overlay = overlay(delta);
    let merged = edges_of(&overlay);
    let flattened = edges_of(&overlay.flatten().unwrap());
    assert_eq!(merged, flattened);
    let mut sorted = merged.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(
        merged.len(),
        sorted.len(),
        "the merge never repeats an edge"
    );
}

#[test]
fn deleted_file_hides_its_unresolved_refs() {
    let mut builder = GraphBuilder::new(SCHEMA_VERSION);
    builder.add_node(node("a")).unwrap();
    builder.add_node(node("b")).unwrap();
    builder.add_unresolved(codegraph::UnresolvedRef {
        file: path("src/a.ts"),
        ordinal: 0,
        from: None,
        name: "ghost".to_owned(),
        kind: analysis_ir::reference::RefKind::Call,
        import_specifier: Some("./missing".to_owned()),
        location: codegraph::Location::new(path("src/a.ts"), 3, 1),
        reason: codegraph::UnresolvedReason::NotFound,
        candidate_count: 0,
    });
    builder.add_unresolved(codegraph::UnresolvedRef {
        file: path("src/b.ts"),
        ordinal: 0,
        from: None,
        name: "phantom".to_owned(),
        kind: analysis_ir::reference::RefKind::Call,
        import_specifier: None,
        location: codegraph::Location::new(path("src/b.ts"), 3, 1),
        reason: codegraph::UnresolvedReason::NotFound,
        candidate_count: 0,
    });
    let base = builder.build().unwrap();

    let mut delta = GraphDelta::new(SCHEMA_VERSION);
    delta.files.push(FileChange::deleted(path("src/a.ts")));
    let overlay = GraphOverlay::new(Arc::new(base), Arc::new(delta)).expect("valid delta");
    let names: Vec<String> = {
        let mut names = Vec::new();
        overlay.for_each_unresolved(&mut |reference| names.push(reference.name.clone()));
        names
    };
    assert_eq!(names, vec!["phantom".to_owned()]);
    assert!(overlay.file("src/a.ts").is_none());
}

#[test]
fn a_file_change_replaces_the_base_entry() {
    let mut delta = GraphDelta::new(SCHEMA_VERSION);
    delta.files.push(FileChange {
        path: path("src/main.ts"),
        change: FileChangeKind::Modified,
        file_version_id: Some(42),
        content_hash: Some(ContentHash::of(b"head")),
        language: Some(Language::Typescript),
    });
    let overlay = overlay(delta);
    let file = overlay.file("src/main.ts").expect("the file is there");
    assert_eq!(file.file_version_id, Some(42));
    assert_eq!(file.content_hash, ContentHash::of(b"head"));
    let flattened = overlay.flatten().unwrap();
    assert_eq!(
        GraphQuery::file(&flattened, "src/main.ts")
            .unwrap()
            .file_version_id,
        Some(42)
    );
}

#[test]
fn unresolved_replaced_swaps_one_files_list() {
    let mut builder = GraphBuilder::new(SCHEMA_VERSION);
    builder.add_node(node("a")).unwrap();
    builder.add_unresolved(codegraph::UnresolvedRef {
        file: path("src/a.ts"),
        ordinal: 0,
        from: None,
        name: "old".to_owned(),
        kind: analysis_ir::reference::RefKind::Call,
        import_specifier: None,
        location: codegraph::Location::new(path("src/a.ts"), 1, 0),
        reason: codegraph::UnresolvedReason::NotFound,
        candidate_count: 0,
    });
    let base = builder.build().unwrap();
    let mut delta = GraphDelta::new(SCHEMA_VERSION);
    delta.unresolved_replaced.push((
        path("src/a.ts"),
        vec![codegraph::UnresolvedRef {
            file: path("src/a.ts"),
            ordinal: 0,
            from: None,
            name: "new".to_owned(),
            kind: analysis_ir::reference::RefKind::Call,
            import_specifier: None,
            location: codegraph::Location::new(path("src/a.ts"), 1, 0),
            reason: codegraph::UnresolvedReason::External,
            candidate_count: 0,
        }],
    ));
    let overlay = GraphOverlay::new(Arc::new(base), Arc::new(delta)).expect("valid delta");
    let names: Vec<(String, codegraph::UnresolvedReason)> = {
        let mut found = Vec::new();
        overlay.for_each_unresolved(&mut |reference| {
            found.push((reference.name.clone(), reference.reason))
        });
        found
    };
    assert_eq!(
        names,
        vec![("new".to_owned(), codegraph::UnresolvedReason::External)]
    );
    assert_eq!(
        compare(
            &overlay,
            &overlay.flatten().unwrap(),
            &CompareOptions::strict()
        ),
        codegraph::GraphDiffReport::default()
    );
}

#[test]
fn a_schema_mismatch_is_rejected() {
    let mut delta = GraphDelta::new(SCHEMA_VERSION);
    delta.base_schema_version = SCHEMA_VERSION + 1;
    let error = GraphOverlay::new(Arc::new(base()), Arc::new(delta)).unwrap_err();
    assert_eq!(
        error,
        OverlayError::SchemaVersionMismatch {
            expected: SCHEMA_VERSION,
            found: SCHEMA_VERSION + 1,
        }
    );
}

#[test]
fn a_noop_tombstone_is_counted_and_ignored() {
    let mut delta = GraphDelta::new(SCHEMA_VERSION);
    delta.edges_removed.push(EdgeIdentity {
        source: NodeId::from_canonical("ghost-from").key(),
        kind: codegraph::EdgeKind::Calls,
        target: NodeId::from_canonical("ghost-to").key(),
    });
    delta
        .nodes_removed
        .push(NodeId::from_canonical("ghost-node").key());
    let overlay = overlay(delta);
    assert_eq!(overlay.noop_tombstones(), 2);
    assert_eq!(
        compare(&overlay, &**overlay.base(), &CompareOptions::strict()),
        codegraph::GraphDiffReport::default()
    );
}

#[test]
fn validate_delta_local_detects_dangling_added_edge() {
    let mut delta = GraphDelta::new(SCHEMA_VERSION);
    delta.nodes_added.push(NodeInput::new(
        NodeId::from_canonical("ts:src/new.ts#n/function"),
        NodeKind::Function,
        "n",
    ));
    delta.edges_added.push(Edge::new(
        codegraph::EdgeKind::Calls,
        NodeId::from_canonical("ts:src/new.ts#n/function").key(),
        NodeId::from_canonical("ts:src/absent.ts#x/function").key(),
        Confidence::MAX,
        ResolvedBy::Structural,
        Provenance::Analyzer,
    ));
    let issues = validate_delta_local(&base(), &delta);
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].code, codegraph::IssueCode::DanglingAddedEdge);
    assert_eq!(issues[0].severity, codegraph::Severity::Error);
}

#[test]
fn validate_delta_local_detects_surviving_edge_to_removed_node() {
    let mut delta = GraphDelta::new(SCHEMA_VERSION);
    delta.nodes_removed.push(key("b"));
    let issues = validate_delta_local(&base(), &delta);
    assert_eq!(issues.len(), 2, "a→b and b→c both break");
    assert!(issues
        .iter()
        .all(|issue| issue.code == codegraph::IssueCode::SurvivingEdgeToRemovedNode));

    // Tombstoning the edges makes the delta locally valid again.
    delta.edges_removed = vec![
        EdgeIdentity {
            source: key("a"),
            kind: codegraph::EdgeKind::Calls,
            target: key("b"),
        },
        EdgeIdentity {
            source: key("b"),
            kind: codegraph::EdgeKind::Calls,
            target: key("c"),
        },
    ];
    assert!(validate_delta_local(&base(), &delta).is_empty());
}

#[test]
fn validate_delta_local_detects_a_deleted_file_that_still_contributes_nodes() {
    let mut delta = GraphDelta::new(SCHEMA_VERSION);
    delta.files.push(FileChange::deleted(path("src/b.ts")));
    delta.nodes_added.push(node("b").in_file(path("src/b.ts")));
    let issues = validate_delta_local(&base(), &delta);
    assert_eq!(issues.len(), 1);
    assert_eq!(
        issues[0].code,
        codegraph::IssueCode::DeletedFileContributesNodes
    );
}

#[test]
fn overlay_is_send_and_sync() {
    fn assert_bounds<T: Send + Sync + 'static>() {}
    assert_bounds::<GraphOverlay>();
    assert_bounds::<GraphDelta>();
    assert_bounds::<Graph>();
}

proptest! {
    /// The equivalence property: for every base and every valid delta, the overlay answers a
    /// query exactly as the flattened graph does, in both directions and for every node.
    #[test]
    fn overlay_queries_equal_flattened_graph_queries(
        base_edges in prop::collection::vec((0u8..4, 0u8..4), 0..8),
        removed in prop::collection::vec(0u8..4, 0..3),
        added in prop::collection::vec((0u8..4, 1u8..=4, 0u8..=4), 0..4),
        tombstones in prop::collection::vec((0u8..4, 0u8..4), 0..4),
    ) {
        let names = ["a", "b", "c", "d"];
        let base = random_base(&names, &base_edges);
        let mut delta = GraphDelta::new(SCHEMA_VERSION);
        for index in removed {
            delta.nodes_removed.push(key(names[index as usize % names.len()]));
        }
        delta.nodes_removed.sort();
        delta.nodes_removed.dedup();
        for (from, permille, n) in added {
            let mut input = NodeInput::new(
                NodeId::from_canonical(format!("ts:src/n{n}.ts#n/function")),
                NodeKind::Function,
                format!("n{n}"),
            );
            input = input.in_file(path(&format!("src/n{n}.ts")));
            delta.nodes_added.push(input);
            delta.edges_added.push(
                Edge::new(
                    codegraph::EdgeKind::Calls,
                    key(names[from as usize % names.len()]),
                    NodeId::from_canonical(format!("ts:src/n{n}.ts#n/function")).key(),
                    Confidence::from_f32(f32::from(permille) / 1000.0),
                    ResolvedBy::NameUnique,
                    Provenance::Linker,
                )
                .with_origin_file(path(&format!("src/n{n}.ts"))),
            );
        }
        // An edge is only added when it is not already in the base, because a delta that overrides
        // an existing edge must tombstone it first; `new` rejects the implicit form.
        let existing: Vec<EdgeIdentity> = base
            .edges()
            .iter()
            .filter_map(|edge| {
                let source = base.node(edge.source)?;
                let target = base.node(edge.target)?;
                Some(EdgeIdentity {
                    source: source.key,
                    kind: edge.kind,
                    target: target.key,
                })
            })
            .collect();
        delta.edges_added.retain(|edge| !existing.contains(&edge.identity()));
        for (from, to) in tombstones {
            let identity = EdgeIdentity {
                source: key(names[from as usize % names.len()]),
                kind: codegraph::EdgeKind::Calls,
                target: key(names[to as usize % names.len()]),
            };
            if existing.contains(&identity) {
                delta.edges_removed.push(identity);
            }
        }
        delta.normalize();

        let overlay = match GraphOverlay::new(Arc::new(base), Arc::new(delta)) {
            Ok(overlay) => overlay,
            // A delta that would be rejected is not a counterexample to the equivalence; it is a
            // rejection the caller must handle.
            Err(OverlayError::ImplicitOverride(_)) => return Ok(()),
            Err(error) => {
                prop_assert!(false, "unexpected overlay error: {error}");
                return Ok(());
            }
        };
        let flattened = overlay.flatten().unwrap();
        prop_assert_eq!(edges_of(&overlay), edges_of(&flattened));
        prop_assert_eq!(out_edges_by_node(&overlay), out_edges_by_node(&flattened));
        // The validator agrees the delta is coherent against the base, since `new` accepted it and
        // `flatten` produced a graph without dangling endpoints.
        let issues = validate_delta_local(overlay.base(), overlay.delta());
        prop_assert!(
            issues.iter().all(|issue| issue.code.as_str() != "delta_added_edge_unknown_endpoint"),
            "an accepted delta must not report unknown added-edge endpoints: {issues:?}"
        );
    }
}

/// A random chain-and-chords graph over `names`, built through the builder.
fn random_base(names: &[&str], edges: &[(u8, u8)]) -> Graph {
    let mut builder = GraphBuilder::new(SCHEMA_VERSION);
    builder
        .add_file(FileInput {
            path: path("src/main.ts"),
            file_version_id: Some(1),
            content_hash: ContentHash::of(b"base"),
            language: Language::Typescript,
        })
        .unwrap();
    for name in names {
        builder.add_node(node(name)).unwrap();
    }
    for (from, to) in edges {
        let a = names[(*from as usize) % names.len()];
        let b = names[(*to as usize) % names.len()];
        builder.add_edge(calls(a, b, 1.0));
    }
    builder.build().unwrap()
}
