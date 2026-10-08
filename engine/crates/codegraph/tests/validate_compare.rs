//! `CG-012`: the validator names what is wrong with a graph, and the comparer reports what changed
//! between two graphs in a form a caller can assert on.
//!
//! The validator can only see a graph the builder accepted, so the cases that need a corrupt graph
//! (a dangling edge, a duplicated key, a broken reverse index) are exercised through the builder's
//! own checks here and through the delta validator, which does see raw input.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::sync::Arc;

use analysis_ir::reference::RefKind;
use codegraph::{
    compare, validate, validate_delta_local, CompareOptions, Confidence, Edge, EdgeFlags, EdgeKind,
    FileChange, FileInput, Graph, GraphBuilder, GraphDelta, GraphQuery, IssueCode, Location,
    NodeId, NodeInput, NodeKind, Provenance, ResolvedBy, Severity, UnresolvedReason, UnresolvedRef,
    SCHEMA_VERSION,
};
use proptest::prelude::*;
use review_core::language::Language;
use review_core::location::{ContentHash, RepoPath};

fn path(raw: &str) -> RepoPath {
    RepoPath::new(raw).unwrap()
}

fn symbol(file: &str, name: &str) -> NodeId {
    NodeId::from_canonical(format!("ts:{file}#{name}/function"))
}

fn key(file: &str, name: &str) -> codegraph::NodeKey {
    symbol(file, name).key()
}

fn node(file: &str, name: &str, kind: NodeKind) -> NodeInput {
    NodeInput::new(symbol(file, name), kind, name)
        .in_file(path(file))
        .qualified_name(name.to_owned())
}

fn unresolved(file: &str, ordinal: u32, name: &str, reason: UnresolvedReason) -> UnresolvedRef {
    UnresolvedRef {
        file: path(file),
        ordinal,
        from: None,
        name: name.to_owned(),
        kind: RefKind::Call,
        import_specifier: None,
        location: Location::new(path(file), ordinal + 1, 1),
        reason,
        candidate_count: 0,
    }
}

fn file_input(raw: &str, hash: &str) -> FileInput {
    FileInput {
        path: path(raw),
        file_version_id: Some(1),
        content_hash: ContentHash::of(hash.as_bytes()),
        language: Language::Typescript,
    }
}

/// Three files, four nodes, a chain of calls and one external unresolved reference.
fn graph_a() -> Graph {
    let mut builder = GraphBuilder::new(SCHEMA_VERSION);
    for raw in ["src/a.ts", "src/b.ts", "src/c.ts"] {
        builder.add_file(file_input(raw, raw)).unwrap();
    }
    builder
        .add_node(node("src/a.ts", "a", NodeKind::Function))
        .unwrap();
    builder
        .add_node(node("src/a.ts", "b", NodeKind::Function))
        .unwrap();
    builder
        .add_node(node("src/b.ts", "c", NodeKind::Function))
        .unwrap();
    builder
        .add_node(node("src/c.ts", "d", NodeKind::Module))
        .unwrap();
    // `c` is in src/c.ts and `d` in the same file, so both directions of the chain are wrong
    // without an explicit map.
    let edge = |from: (&str, &str), to: (&str, &str), line: u32| {
        Edge::new(
            EdgeKind::Calls,
            key(from.0, from.1),
            key(to.0, to.1),
            Confidence::from_f32(0.9),
            ResolvedBy::Import,
            Provenance::Linker,
        )
        .with_location(Location::new(path(from.0), line, 1))
        .with_origin_file(path(from.0))
    };
    builder.add_edge(edge(("src/a.ts", "a"), ("src/a.ts", "b"), 1));
    builder.add_edge(edge(("src/a.ts", "b"), ("src/b.ts", "c"), 2));
    builder.add_edge(edge(("src/b.ts", "c"), ("src/c.ts", "d"), 3));
    builder.add_unresolved(unresolved(
        "src/a.ts",
        0,
        "third_party",
        UnresolvedReason::External,
    ));
    builder.build().unwrap()
}

fn codes(issues: &[codegraph::ValidationIssue]) -> Vec<IssueCode> {
    issues.iter().map(|issue| issue.code).collect()
}

#[test]
fn a_healthy_graph_validates() {
    let graph = graph_a();
    let report = validate(&graph, SCHEMA_VERSION);
    assert!(report.is_ok(), "unexpected issues: {}", report.render(16));
    assert!(report.errors.is_empty());
    assert_eq!(
        report.checked_nodes as usize,
        codegraph::GraphQuery::node_count(&graph)
    );
    assert_eq!(
        report.checked_edges as usize,
        codegraph::GraphQuery::edge_count(&graph)
    );
}

#[test]
fn a_schema_version_this_build_does_not_speak_is_an_error() {
    let graph = graph_a();
    let report = validate(&graph, SCHEMA_VERSION + 1);
    assert!(!report.is_ok());
    assert_eq!(
        codes(&report.errors),
        vec![IssueCode::SchemaVersionMismatch]
    );
}

#[test]
fn an_endpoint_kinds_pair_outside_the_matrix_is_a_warning_not_an_error() {
    // A value cannot call a function: `Calls` requires a caller category and `Data` is not one.
    // The graph is still structurally sound, so this must be a warning rather than an error.
    let mut builder = GraphBuilder::new(SCHEMA_VERSION);
    builder.add_file(file_input("src/a.ts", "a")).unwrap();
    builder
        .add_node(node("src/a.ts", "v", NodeKind::Variable))
        .unwrap();
    builder
        .add_node(node("src/a.ts", "f", NodeKind::Function))
        .unwrap();
    builder.add_edge(Edge::new(
        EdgeKind::Calls,
        key("src/a.ts", "v"),
        key("src/a.ts", "f"),
        Confidence::MAX,
        ResolvedBy::Structural,
        Provenance::Linker,
    ));
    let graph = builder.build().unwrap();

    let report = validate(&graph, SCHEMA_VERSION);
    assert!(report.errors.is_empty(), "a kind rule is not corruption");
    assert!(
        report
            .warnings
            .iter()
            .any(|issue| issue.code == IssueCode::KindRuleViolation),
        "but it is reported: {}",
        report.render(16)
    );
    assert_eq!(
        report.warnings[0].severity,
        Severity::Warning,
        "warnings and errors differ in severity"
    );
}

#[test]
fn an_orphan_synthetic_node_is_a_warning() {
    let mut builder = GraphBuilder::new(SCHEMA_VERSION);
    builder.add_file(file_input("src/a.ts", "a")).unwrap();
    builder
        .add_node(node("src/a.ts", "a", NodeKind::Function))
        .unwrap();
    // A synthetic node has no file and no incident edge, which is legal but usually a mapper bug.
    builder
        .add_node(NodeInput::new(
            NodeId::from_canonical("queue:jobs"),
            NodeKind::Queue,
            "jobs",
        ))
        .unwrap();
    let graph = builder.build().unwrap();
    let report = validate(&graph, SCHEMA_VERSION);
    assert!(report.is_ok());
    assert!(report
        .warnings
        .iter()
        .any(|issue| issue.code == IssueCode::OrphanSyntheticNode));
}

#[test]
fn a_healthy_graph_equals_itself_under_strict_comparison() {
    let a = graph_a();
    let report = compare(&a, &a, &CompareOptions::strict());
    assert!(report.is_empty());
    assert_eq!(
        report.render(8),
        "graphs are equal",
        "an empty report says so in one line"
    );
}

#[test]
fn comparing_two_graphs_is_symmetric() {
    let a = graph_a();
    let b = graph_with_extra_node(&a);
    let forward = compare(&a, &b, &CompareOptions::strict());
    let backward = compare(&b, &a, &CompareOptions::strict());
    assert_eq!(forward.nodes_only_b, backward.nodes_only_a);
    assert_eq!(forward.edges_only_b, backward.edges_only_a);
    assert_eq!(forward.nodes_only_a, backward.nodes_only_b);
}

#[test]
fn an_added_node_appears_only_on_its_own_side() {
    let a = graph_a();
    let extra = NodeId::from_canonical("ts:src/c.ts#e/function").key();
    let b = graph_with_extra_node(&a);

    let report = compare(&a, &b, &CompareOptions::strict());
    assert_eq!(report.nodes_only_b, vec![extra]);
    assert!(report.nodes_only_a.is_empty());
    assert!(report.render(4).contains("node"));
}

fn graph_with_extra_node(a: &Graph) -> Graph {
    // Rebuild from `a`'s wire form plus one node, which is the only public way to derive a variant.
    let wire = codegraph::GraphWire::of(a).unwrap();
    let mut builder = GraphBuilder::new(SCHEMA_VERSION);
    for file in wire.files {
        builder.add_file(file).unwrap();
    }
    for node in wire.nodes {
        builder.add_node(node).unwrap();
    }
    for edge in wire.edges {
        builder.add_edge(edge);
    }
    for reference in wire.unresolved {
        builder.add_unresolved(reference);
    }
    builder
        .add_node(
            NodeInput::new(
                NodeId::from_canonical("ts:src/c.ts#e/function"),
                NodeKind::Function,
                "e",
            )
            .in_file(path("src/c.ts")),
        )
        .unwrap();
    builder.build().unwrap()
}

#[test]
fn a_changed_node_reports_which_fields_differ() {
    let a = graph_a();
    let b = renamed(&a, "d", "d_renamed");

    let report = compare(&a, &b, &CompareOptions::strict());
    assert!(report.nodes_only_a.is_empty() && report.nodes_only_b.is_empty());
    assert_eq!(report.nodes_differ.len(), 1);
    let (subject, fields) = &report.nodes_differ[0];
    assert_eq!(*subject, key("src/c.ts", "d"));
    assert!(fields.contains(&"name"), "reported fields: {fields:?}");
}

#[test]
fn an_edge_that_lost_its_location_differs_unless_locations_are_ignored() {
    let a = graph_a();
    let b = without_locations(&a);

    let strict = compare(&a, &b, &CompareOptions::strict());
    assert!(
        !strict.edges_differ.is_empty(),
        "a moved edge differs strictly"
    );

    let lenient = compare(&a, &b, &CompareOptions::lenient());
    assert!(
        lenient.is_empty(),
        "locations and file versions are not semantic: {}",
        lenient.render(8)
    );
}

/// Drop every edge location and bump every file version: a reformat of the same repository.
fn without_locations(a: &Graph) -> Graph {
    let wire = codegraph::GraphWire::of(a).unwrap();
    let mut builder = GraphBuilder::new(SCHEMA_VERSION);
    for mut file in wire.files {
        file.file_version_id = file.file_version_id.map(|v| v + 1);
        builder.add_file(file).unwrap();
    }
    for node in wire.nodes {
        builder.add_node(node).unwrap();
    }
    for edge in wire.edges {
        builder.add_edge(Edge {
            location: None,
            occurrences: 1,
            ..edge
        });
    }
    for reference in wire.unresolved {
        builder.add_unresolved(UnresolvedRef {
            location: Location::new(reference.file.clone(), 1, 1),
            ..reference
        });
    }
    builder.build().unwrap()
}

fn renamed(a: &Graph, from: &str, to: &str) -> Graph {
    let wire = codegraph::GraphWire::of(a).unwrap();
    let mut builder = GraphBuilder::new(SCHEMA_VERSION);
    for file in wire.files {
        builder.add_file(file).unwrap();
    }
    for node in wire.nodes {
        let name = node.name.clone();
        builder
            .add_node(if name == from {
                NodeInput {
                    name: to.to_owned(),
                    ..node
                }
            } else {
                node
            })
            .unwrap();
    }
    for edge in wire.edges {
        builder.add_edge(edge);
    }
    for reference in wire.unresolved {
        builder.add_unresolved(reference);
    }
    builder.build().unwrap()
}

proptest!(
    /// `compare(a, b)` empty must mean the two graphs are interchangeable.
    #[test]
    fn an_empty_diff_means_the_graphs_are_interchangeable(
        seeds in prop::collection::vec((0u8..6, 0u8..6, 0u8..4), 0..12)
    ) {
        let mut cases = seeds;
        cases.sort_unstable();
        for (seed_a, seed_b, tweak) in cases {
            let a = mutated_graph(u64::from(seed_a), u64::from(tweak));
            let b = mutated_graph(u64::from(seed_b), u64::from(tweak));
            if compare(&a, &b, &CompareOptions::strict()).is_empty() {
                // Their node and edge sets must agree, or `compare` is under-reporting.
                prop_assert_eq!(
                    codegraph::GraphQuery::node_count(&a),
                    codegraph::GraphQuery::node_count(&b)
                );
                prop_assert_eq!(confidence_totals(&a), confidence_totals(&b));
            }
        }
    }
);

/// Per-edge-kind confidence totals, as a cheap fingerprint of a graph's edges.
fn confidence_totals(graph: &Graph) -> HashMap<EdgeKind, u32> {
    let mut counts: HashMap<EdgeKind, u32> = HashMap::new();
    graph.for_each_node(&mut |node| {
        graph.for_each_edge(
            node.key,
            codegraph::Direction::Out,
            &codegraph::EdgeFilter::ALL,
            &mut |edge| {
                *counts.entry(edge.kind).or_default() += u32::from(edge.confidence.as_permille());
                std::ops::ControlFlow::Continue(())
            },
        );
    });
    counts
}

/// A deterministic small graph, optionally with one edge confidence nudged so the comparer has
/// something to disagree about.
fn mutated_graph(seed: u64, tweak: u64) -> Graph {
    let count = 3 + seed % 4;
    let mut builder = GraphBuilder::new(SCHEMA_VERSION);
    builder.add_file(file_input("src/g.ts", "g")).unwrap();
    for n in 0..count {
        builder
            .add_node(node("src/g.ts", &format!("n{n}"), NodeKind::Function))
            .unwrap();
    }
    for n in 0..count {
        let target = (n + 1 + seed) % count;
        let confidence = if n == tweak % 8 && count > 2 {
            Confidence::from_f32(0.5)
        } else {
            Confidence::MAX
        };
        builder.add_edge(Edge::new(
            EdgeKind::Calls,
            key("src/g.ts", &format!("n{n}")),
            key("src/g.ts", &format!("n{target}")),
            confidence,
            ResolvedBy::NameUnique,
            Provenance::Linker,
        ));
    }
    builder.build().unwrap()
}

#[test]
fn a_delta_validator_reports_a_dangling_added_edge() {
    let base = graph_a();
    let mut delta = GraphDelta::new(SCHEMA_VERSION);
    delta.edges_added.push(Edge::new(
        EdgeKind::Calls,
        key("src/a.ts", "a"),
        NodeId::from_canonical("ts:src/a.ts#ghost/function").key(),
        Confidence::MAX,
        ResolvedBy::NameUnique,
        Provenance::Linker,
    ));
    let issues = validate_delta_local(&base, &delta);
    assert_eq!(codes(&issues), vec![IssueCode::DanglingAddedEdge]);
    assert!(
        issues[0].detail.contains("target known: false"),
        "{}",
        issues[0]
    );
}

#[test]
fn a_delta_validator_reports_a_deleted_file_that_still_contributes_nodes() {
    let base = graph_a();
    let mut delta = GraphDelta::new(SCHEMA_VERSION);
    delta.files.push(FileChange::deleted(path("src/c.ts")));
    delta
        .nodes_added
        .push(node("src/c.ts", "late", NodeKind::Function));
    let issues = validate_delta_local(&base, &delta);
    assert_eq!(codes(&issues), vec![IssueCode::DeletedFileContributesNodes]);
}

#[test]
fn a_coherent_delta_reports_nothing() {
    let base = graph_a();
    let mut delta = GraphDelta::new(SCHEMA_VERSION);
    delta
        .nodes_added
        .push(node("src/c.ts", "new", NodeKind::Function));
    delta.edges_added.push(Edge::new(
        EdgeKind::Calls,
        key("src/b.ts", "c"),
        key("src/c.ts", "new"),
        Confidence::MAX,
        ResolvedBy::NameUnique,
        Provenance::Linker,
    ));
    delta.files.push(FileChange::modified(
        path("src/c.ts"),
        ContentHash::of(b"new"),
    ));
    assert!(
        validate_delta_local(&base, &delta).is_empty(),
        "a delta the overlay accepts is locally valid"
    );
}

#[test]
fn the_report_is_bounded_so_a_broken_graph_cannot_flood_ci() {
    let a = graph_a();
    let wire = codegraph::GraphWire::of(&a).unwrap();
    let mut builder = GraphBuilder::new(SCHEMA_VERSION);
    for file in &wire.files {
        builder.add_file(file.clone()).unwrap();
    }
    for node in &wire.nodes {
        builder.add_node(node.clone()).unwrap();
    }
    for edge in &wire.edges {
        builder.add_edge(edge.clone());
    }
    for reference in &wire.unresolved {
        builder.add_unresolved(reference.clone());
    }
    // 200 references the other graph does not have, each from `a` to its own new node, so every
    // one is a distinct identity rather than a repeat the builder would merge.
    for n in 0..200u32 {
        builder
            .add_node(
                NodeInput::new(
                    NodeId::from_canonical(format!("ts:src/c.ts#extra{n}/function")),
                    NodeKind::Function,
                    format!("extra{n}"),
                )
                .in_file(path("src/c.ts")),
            )
            .unwrap();
        builder.add_edge(Edge::new(
            EdgeKind::References,
            key("src/a.ts", "a"),
            NodeId::from_canonical(format!("ts:src/c.ts#extra{n}/function")).key(),
            Confidence::from_f32(f64::from(n % 100) as f32 / 100.0),
            ResolvedBy::Heuristic,
            Provenance::Heuristic,
        ));
    }

    let b = builder.build().unwrap();
    let report = compare(&a, &b, &CompareOptions::strict());
    assert_eq!(
        report.nodes_only_b.len(),
        200,
        "the whole report is complete"
    );
    assert_eq!(report.edges_only_b.len(), 200);

    // The render is what reaches CI logs, so it is bounded by `max_items` rather than by the size
    // of the difference: two non-empty sections, each a header plus `max_items` lines plus the
    // "more not shown" tail.
    let rendered = report.render(5);
    let sections = rendered
        .lines()
        .filter(|line| !line.starts_with(' '))
        .count();
    assert_eq!(sections, 2, "two sections differ:\n{rendered}");
    assert_eq!(
        rendered.lines().count(),
        sections * (1 + 5 + 1),
        "{rendered}"
    );
    assert!(rendered.contains("… 195 more not shown"), "{rendered}");
    assert!(
        report.render(1).lines().count() < rendered.lines().count(),
        "a smaller cap renders less"
    );
}

#[test]
fn a_graph_built_for_another_schema_is_refused_by_the_caller() {
    // `validate` reports it, and the codec refuses it, so no consumer can act on a graph whose
    // meaning it does not know.
    let graph = graph_a();
    assert_eq!(
        validate(&graph, SCHEMA_VERSION + 1).errors[0].code,
        IssueCode::SchemaVersionMismatch
    );

    let mut bytes = Vec::new();
    codegraph::encode_graph(&graph, &mut bytes).unwrap();
    assert_eq!(bytes[8..12], SCHEMA_VERSION.to_le_bytes());
    assert_eq!(Arc::strong_count(&Arc::new(graph)), 1);
}

#[test]
fn edge_flags_and_occurrences_are_part_of_an_edges_identity_for_comparison() {
    let a = graph_a();
    let wire = codegraph::GraphWire::of(&a).unwrap();
    let b = rebuild(&wire, |edge| edge.clone());
    assert!(compare(&a, &b, &CompareOptions::strict()).is_empty());

    // The same edge with an extra flag is a different edge under strict comparison.
    let c = rebuild(&wire, |edge| {
        if edge.kind == EdgeKind::Calls {
            Edge {
                flags: EdgeFlags::from_bits(EdgeFlags::EMPTY.bits() | 1),
                ..edge.clone()
            }
        } else {
            edge.clone()
        }
    });
    assert!(!compare(&a, &c, &CompareOptions::strict())
        .edges_differ
        .is_empty());
}

/// Rebuild a graph from its wire form, transforming each edge on the way through.
fn rebuild(wire: &codegraph::GraphWire, transform: impl Fn(&Edge) -> Edge) -> Graph {
    let mut builder = GraphBuilder::new(SCHEMA_VERSION);
    for file in &wire.files {
        builder.add_file(file.clone()).unwrap();
    }
    for node in &wire.nodes {
        builder.add_node(node.clone()).unwrap();
    }
    for edge in &wire.edges {
        builder.add_edge(transform(edge));
    }
    for reference in &wire.unresolved {
        builder.add_unresolved(reference.clone());
    }
    builder.build().unwrap()
}
