//! CG-004 acceptance: the in-memory graph builds deterministically and its adjacency is
//! internally consistent.
//!
//! The `key_collision_is_reported` case lives in `src/graph/builder.rs` instead of here:
//! forcing a blake3-128 collision needs the builder's test-only constructor, which is not
//! part of the crate's public API.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::{HashMap, HashSet};

use analysis_ir::reference::RefKind;
use codegraph::{
    confidence, Edge, EdgeKind, FileInput, Graph, GraphBuilder, NodeInput, NodeKind, Provenance,
    ResolvedBy, UnresolvedReason, UnresolvedRef, SCHEMA_VERSION,
};
use proptest::prelude::*;
use review_core::location::{Position, RepoPath, SourceRange};

fn path(raw: &str) -> RepoPath {
    RepoPath::new(raw).unwrap()
}

fn range(line: u32) -> SourceRange {
    SourceRange::new(
        Position::new(line, 0).unwrap(),
        Position::new(line + 4, 10).unwrap(),
    )
    .unwrap()
}

fn symbol(file: &str, n: usize) -> codegraph::NodeId {
    codegraph::NodeId::from_canonical(format!("ts:{file}#T{n}/fn{n}/function"))
}

/// One input in the order-independent multiset the builder is fed.
#[derive(Debug, Clone)]
enum Action {
    File(FileInput),
    Node(NodeInput),
    Edge(Edge),
    Unresolved(UnresolvedRef),
}

/// 20 files, 400 nodes, ~1000 edges plus duplicates, unresolved references and one synthetic
/// node with no file — enough shape to exercise every invariant CG-004 lists.
fn actions() -> Vec<Action> {
    let mut out = Vec::new();
    for f in 0..20 {
        out.push(Action::File(FileInput {
            path: path(&format!("src/mod{f}.ts")),
            file_version_id: Some(f as i64),
            content_hash: review_core::location::ContentHash::of(format!("mod{f}").as_bytes()),
            language: review_core::language::Language::Typescript,
        }));
    }
    out.push(Action::Node(
        NodeInput::new(
            codegraph::NodeId::repository(),
            NodeKind::Repository,
            "reviewgraph",
        )
        .qualified_name("reviewgraph"),
    ));
    for n in 0..400 {
        let file = format!("src/mod{}.ts", n % 20);
        out.push(Action::Node(
            NodeInput::new(symbol(&file, n), NodeKind::Function, format!("fn{n}"))
                .qualified_name(format!("T{n}.fn{n}"))
                .in_file(path(&file))
                .with_range(range(n as u32 % 500 + 1)),
        ));
    }
    for n in 0..1000 {
        let source = n % 400;
        let target = (n * 7 + 3) % 400;
        let kind = [
            EdgeKind::Calls,
            EdgeKind::UsesType,
            EdgeKind::Reads,
            EdgeKind::References,
        ][n % 4];
        let source_file = format!("src/mod{}.ts", source % 20);
        let target_file = format!("src/mod{}.ts", target % 20);
        let file = path(&source_file);
        let edge = Edge::new(
            kind,
            symbol(&source_file, source).key(),
            symbol(&target_file, target).key(),
            confidence::confidence_of(ResolvedBy::NameUnique),
            ResolvedBy::NameUnique,
            Provenance::Linker,
        )
        .with_origin_file(file.clone())
        .with_location(codegraph::Location::new(file, n as u32 % 400 + 1, 3));
        out.push(Action::Edge(edge.clone()));
        // Every third edge is added twice: the builder must merge the occurrence.
        if n % 3 == 0 {
            out.push(Action::Edge(edge));
        }
    }
    for n in 0..40 {
        let file = path(&format!("src/mod{}.ts", n % 20));
        out.push(Action::Unresolved(UnresolvedRef {
            file: file.clone(),
            ordinal: n as u32,
            from: None,
            name: format!("missing{n}"),
            kind: RefKind::Call,
            import_specifier: Some(format!("pkg{n}")),
            location: codegraph::Location::new(file, n as u32 + 1, 1),
            reason: UnresolvedReason::External,
            candidate_count: 0,
        }));
    }
    out
}

fn build(actions: Vec<Action>) -> Graph {
    let mut builder = GraphBuilder::new(SCHEMA_VERSION);
    for action in actions {
        match action {
            Action::File(file) => {
                builder.add_file(file).unwrap();
            }
            Action::Node(node) => {
                builder.add_node(node).unwrap();
            }
            Action::Edge(edge) => builder.add_edge(edge),
            Action::Unresolved(reference) => builder.add_unresolved(reference),
        }
    }
    builder.build().unwrap()
}

/// A canonical dump of everything the graph stores, including the interned string order. Two
/// graphs that produce the same string are identical.
fn canonical(graph: &Graph) -> String {
    let mut out = String::new();
    out.push_str(&format!("schema {}\n", graph.schema_version()));
    for entry in graph.files() {
        out.push_str(&format!(
            "file {} {:?} {:?} nodes={}..{} owned={}..{}\n",
            graph.str(entry.path),
            entry.content_hash,
            entry.language,
            entry.nodes.start,
            entry.nodes.end,
            entry.edges_owned.start,
            entry.edges_owned.end,
        ));
    }
    for node in graph.nodes() {
        out.push_str(&format!(
            "node {:?} {:?} {} {} file={:?} range={:?} flags={:?} vis={:?} parent={:?} sig={:?} extra={:?}\n",
            node.key,
            node.kind,
            graph.str(node.id),
            graph.str(node.name),
            node.file,
            node.range,
            node.attrs.flags,
            node.attrs.visibility,
            node.attrs.parent,
            node.attrs.signature.map(|id| graph.str(id).to_owned()),
            node.attrs.extra.as_ref().map(|pairs| {
                pairs
                    .iter()
                    .map(|(k, v)| (graph.str(*k).to_owned(), graph.str(*v).to_owned()))
                    .collect::<Vec<_>>()
            }),
        ));
    }
    for edge in graph.edges() {
        out.push_str(&format!(
            "edge {} -> {} {:?} {:?} {:?} {:?} {} file={:?} at={}:{} occ={}\n",
            edge.source,
            edge.target,
            edge.kind,
            edge.resolved_by,
            edge.provenance,
            edge.flags,
            edge.confidence,
            edge.origin_file,
            edge.line,
            edge.col,
            edge.occurrences,
        ));
    }
    for reference in graph.unresolved() {
        out.push_str(&format!(
            "unresolved {:?} {} {:?} {:?}\n",
            reference.file, reference.name, reference.kind, reference.reason
        ));
    }
    for (id, text) in graph.strings().iter() {
        out.push_str(&format!("str {id} {text:?}\n"));
    }
    out
}

/// Deterministic Fisher-Yates driven by xorshift, so the shuffle itself is reproducible.
fn shuffle<T>(items: &mut [T], seed: u64) {
    let mut state = if seed == 0 { 1 } else { seed };
    for i in (1..items.len()).rev() {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let j = (state % (i as u64 + 1)) as usize;
        items.swap(i, j);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]
    /// CG-004: building from the same multiset of inputs in any order yields the same graph,
    /// down to the interned string order.
    #[test]
    fn build_is_order_independent(seed in any::<u64>()) {
        let mut shuffled = actions();
        shuffle(&mut shuffled, seed);
        prop_assert_eq!(canonical(&build(actions())), canonical(&build(shuffled)));
    }
}

/// Every forward edge appears exactly once in reverse, and vice versa.
#[test]
fn csr_forward_reverse_symmetry() {
    let graph = build(actions());
    let mut seen_fwd = vec![0u32; graph.edges().len()];
    let mut seen_rev = vec![0u32; graph.edges().len()];
    for raw in 0..graph.nodes().len() {
        let v = codegraph::NodeIx::new(raw as u32);
        for ix in graph.out_edges(v) {
            seen_fwd[ix.get() as usize] += 1;
        }
        for ix in graph.in_edges(v) {
            seen_rev[ix.get() as usize] += 1;
        }
    }
    assert_eq!(seen_fwd, seen_rev);
    assert!(
        seen_fwd.iter().all(|count| *count == 1),
        "every edge must appear exactly once in each direction"
    );
    assert_eq!(
        graph.out_csr().edge_count(),
        graph.in_csr().edge_count(),
        "forward and reverse CSR hold the same number of edges"
    );
}

/// `kind_mask[v]` is exactly the union of the kinds in `v`'s slice.
#[test]
fn kind_mask_matches_slices() {
    let graph = build(actions());
    for raw in 0..graph.nodes().len() {
        let v = codegraph::NodeIx::new(raw as u32);
        for (mask, slice) in [
            (graph.out_csr().kinds(raw), graph.out_edges(v)),
            (graph.in_csr().kinds(raw), graph.in_edges(v)),
        ] {
            let mut derived = 0u64;
            for ix in slice {
                derived |= 1u64 << graph.edge(*ix).unwrap().kind.as_u8();
            }
            assert_eq!(mask, derived, "mask disagrees with the slice of node {raw}");
            for kind in EdgeKind::ALL {
                assert_eq!(
                    mask & (1u64 << kind.as_u8()) != 0,
                    slice.iter().any(|ix| graph.edge(*ix).unwrap().kind == kind),
                    "node {raw} kind {kind} disagrees with its slice"
                );
            }
        }
    }
}

/// Each file owns one contiguous node range, ranges are disjoint, and together they cover
/// exactly the nodes that have a file.
#[test]
fn file_node_ranges_are_contiguous_and_complete() {
    let graph = build(actions());
    let mut covered = HashSet::new();
    for (raw, entry) in graph.files().iter().enumerate() {
        let start = entry.nodes.start;
        let end = entry.nodes.end;
        assert!(start <= end, "file {raw} has an inverted range");
        for index in start..end {
            let node = graph
                .node(codegraph::NodeIx::new(index))
                .unwrap_or_else(|| panic!("range of file {raw} points past the node table"));
            assert_eq!(
                node.file,
                Some(codegraph::FileIx::new(raw as u32)),
                "node {index} is inside file {raw}'s range but does not belong to it"
            );
            assert!(covered.insert(index), "node {index} is covered twice");
        }
    }
    for (raw, node) in graph.nodes().iter().enumerate() {
        if node.file.is_some() {
            assert!(
                covered.contains(&(raw as u32)),
                "node {raw} has a file but no file claims it"
            );
        }
    }
}

/// `edges_owned_by` partitions the edges that have an origin file: disjoint, complete, and in
/// canonical edge order.
#[test]
fn edges_owned_by_file_partitions_owned_edges() {
    let graph = build(actions());
    let mut seen = HashSet::new();
    for (raw, entry) in graph.files().iter().enumerate() {
        let mut last = None;
        let mut count = 0u32;
        for ix in graph.edges_owned_by(codegraph::FileIx::new(raw as u32)) {
            count += 1;
            assert!(
                seen.insert(ix.get()),
                "edge {} is owned by more than one file",
                ix.get()
            );
            let edge = graph.edge(*ix).unwrap();
            assert_eq!(
                edge.origin_file,
                Some(codegraph::FileIx::new(raw as u32)),
                "file {raw} owns an edge that originates elsewhere"
            );
            if let Some(previous) = last {
                assert!(
                    previous < ix.get(),
                    "file {raw} owns its edges out of canonical order"
                );
            }
            last = Some(ix.get());
        }
        assert_eq!(
            count,
            entry.owned_edge_count(),
            "file {raw} owns fewer edges than its table records"
        );
    }
    let with_origin = graph
        .edges()
        .iter()
        .filter(|edge| edge.origin_file.is_some())
        .count();
    assert_eq!(seen.len(), with_origin, "owned edges are not a partition");
}

/// Adding the same edge twice merges the occurrence instead of creating a second row.
#[test]
fn duplicate_edge_identity_merges_occurrences() {
    let source = codegraph::NodeId::from_canonical("ts:src/a.ts#A/f/function");
    let target = codegraph::NodeId::from_canonical("ts:src/b.ts#B/g/function");
    let source_key = source.key();
    let target_key = target.key();
    let make = move || {
        Edge::new(
            EdgeKind::Calls,
            source_key,
            target_key,
            confidence::confidence_of(ResolvedBy::NameUnique),
            ResolvedBy::NameUnique,
            Provenance::Linker,
        )
        .with_origin_file(path("src/a.ts"))
    };
    let mut builder = GraphBuilder::new(SCHEMA_VERSION);
    builder
        .add_node(NodeInput::new(source, NodeKind::Function, "f").in_file(path("src/a.ts")))
        .unwrap();
    builder
        .add_node(NodeInput::new(target, NodeKind::Function, "g").in_file(path("src/b.ts")))
        .unwrap();
    builder.add_edge(make());
    builder.add_edge(make());
    let graph = builder.build().unwrap();
    assert_eq!(graph.edges().len(), 1);
    assert_eq!(graph.edges()[0].occurrences, 2);
}

/// An edge whose endpoint is not a node is refused rather than silently dropped.
#[test]
fn dangling_edge_is_rejected() {
    let mut builder = GraphBuilder::new(SCHEMA_VERSION);
    builder
        .add_node(
            NodeInput::new(
                codegraph::NodeId::from_canonical("ts:src/a.ts#A/f/function"),
                NodeKind::Function,
                "f",
            )
            .in_file(path("src/a.ts")),
        )
        .unwrap();
    builder.add_edge(Edge::new(
        EdgeKind::Calls,
        codegraph::NodeId::from_canonical("ts:src/a.ts#A/f/function").key(),
        codegraph::NodeId::from_canonical("ts:src/ghost.ts#G/f/function").key(),
        confidence::confidence_of(ResolvedBy::NameUnique),
        ResolvedBy::NameUnique,
        Provenance::Linker,
    ));
    assert!(matches!(
        builder.build(),
        Err(codegraph::GraphBuildError::DanglingEdge { .. })
    ));
}

/// The heap estimate tracks what the graph actually holds.
///
/// The CG-004 sketch measures this against a counting allocator; that needs a
/// `#[global_allocator]`, which needs `unsafe impl GlobalAlloc`, and this workspace forbids
/// `unsafe` outright (`unsafe_code = "forbid"`), so it cannot be written here. Two
/// independent stand-ins are used instead:
///
/// * a len-based floor — every table at `len`, every interned byte — which the estimator must
///   never fall below, and
/// * a capacity-aware oracle that rebuilds each table from the graph's public data, using the
///   same construction the builder uses (`with_capacity(len)` for the fixed tables, insertion
///   growth for the maps), so it reflects what the allocator is really asked for. The
///   estimator must land within 15% of that.
#[test]
fn heap_size_estimate_within_15_percent() {
    let graph = build(actions());
    let estimate = graph.heap_size_bytes();
    let floor = in_use_bytes(&graph);
    assert!(
        estimate >= floor,
        "estimate {estimate} under-reports the {floor} bytes in use"
    );
    let oracle = capacity_oracle(&graph);
    let ratio = estimate as f64 / oracle as f64;
    assert!(
        (0.85..=1.15).contains(&ratio),
        "estimate {estimate} is {ratio:.3}x the {oracle} byte capacity oracle, \
         outside the 15% budget"
    );
}

/// Rebuilds the graph's tables with the same construction the builder uses and reports what
/// the allocator is asked for: exact reservations for the fixed tables, insertion-order growth
/// for the maps, and every string byte — including the interner's duplicated map key.
#[allow(clippy::manual_slice_size_calculation)] // `len * size_of` is the point of an accounting helper
fn capacity_oracle(graph: &Graph) -> usize {
    let mut total = 0usize;
    // Fixed tables: `Vec::with_capacity(len)` / `vec![x; n]`, so capacity is exactly `len`.
    total += graph.nodes().len() * size_of::<codegraph::NodeData>();
    total += graph.edges().len() * size_of::<codegraph::EdgeData>();
    total += graph.files().len() * size_of::<codegraph::FileEntry>();
    total += graph.unresolved().len() * size_of::<codegraph::UnresolvedRef>();
    for csr in [graph.out_csr(), graph.in_csr()] {
        total += (csr.node_count() + 1) * size_of::<u32>();
        total += csr.edge_count() * size_of::<codegraph::EdgeIx>();
        total += csr.node_count() * size_of::<u64>();
    }
    for (raw, _) in graph.files().iter().enumerate() {
        total += graph
            .edges_owned_by(codegraph::FileIx::new(raw as u32))
            .len()
            * size_of::<codegraph::EdgeIx>();
    }
    // Reserved maps: `HashMap::with_capacity(len)`.
    total += graph.nodes().len()
        * (size_of::<codegraph::NodeKey>() + size_of::<codegraph::NodeIx>() + 8);
    total +=
        graph.files().len() * (size_of::<codegraph::StrId>() + size_of::<codegraph::FileIx>() + 8);
    // The string table grows by one push and one map insert per distinct string; replaying
    // that sequence reproduces the capacities the builder's table ended up with.
    let mut strings: Vec<String> = Vec::new();
    let mut index: HashMap<String, codegraph::StrId> = HashMap::new();
    let mut bytes = 0usize;
    for (_, text) in graph.strings().iter() {
        strings.push(text.to_owned());
        index.insert(text.to_owned(), codegraph::StrId::new(0));
        bytes += text.len();
    }
    total += strings.capacity() * size_of::<String>();
    total += bytes;
    total += index.capacity() * (16 + size_of::<String>() + size_of::<codegraph::StrId>());
    total += bytes; // the map owns a second copy of every key's bytes
                    // The name index grows by insert, one entry per distinct name.
    let mut names: Vec<codegraph::StrId> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for reference in graph.unresolved() {
        if seen.insert(reference.name.clone()) {
            names.push(codegraph::StrId::new(names.len() as u32));
        }
    }
    let mut by_name: HashMap<codegraph::StrId, Vec<u32>> = HashMap::new();
    for id in &names {
        by_name.entry(*id).or_default().push(0);
    }
    total += by_name.capacity() * (size_of::<codegraph::StrId>() + size_of::<Vec<u32>>() + 8);
    total += by_name.values().map(Vec::len).sum::<usize>() * size_of::<u32>();
    total += graph
        .unresolved()
        .iter()
        .map(|reference| {
            reference.name.len()
                + reference
                    .import_specifier
                    .as_deref()
                    .unwrap_or_default()
                    .len()
                + reference.file.as_str().len()
        })
        .sum::<usize>();
    total
}

/// An independent accounting of what the graph logically holds: every table at `len` (never
/// capacity), both adjacency structures derived from their public dimensions, the owned-edge
/// list walked file by file, every interned byte, and one hash-map entry per stored pair at
/// `key + value + 8` (the bucket's hash word).
#[allow(clippy::manual_slice_size_calculation)] // `len * size_of` is the point of an accounting helper
fn in_use_bytes(graph: &Graph) -> usize {
    let mut total = 0usize;
    total += graph.nodes().len() * size_of::<codegraph::NodeData>();
    total += graph.edges().len() * size_of::<codegraph::EdgeData>();
    total += graph.files().len() * size_of::<codegraph::FileEntry>();
    total += graph.unresolved().len() * size_of::<codegraph::UnresolvedRef>();
    // Adjacency: offsets, edge ids and the kind mask, forward and reverse.
    for csr in [graph.out_csr(), graph.in_csr()] {
        total += (csr.node_count() + 1) * size_of::<u32>();
        total += csr.edge_count() * size_of::<codegraph::EdgeIx>();
        total += csr.node_count() * size_of::<u64>();
    }
    for (raw, _) in graph.files().iter().enumerate() {
        total += graph
            .edges_owned_by(codegraph::FileIx::new(raw as u32))
            .len()
            * size_of::<codegraph::EdgeIx>();
    }
    total += graph.strings().len() * (size_of::<String>() + size_of::<codegraph::StrId>());
    total += graph.strings().iter().map(|(_, s)| s.len()).sum::<usize>();
    total += graph.nodes().len()
        * (size_of::<codegraph::NodeKey>() + size_of::<codegraph::NodeIx>() + 8);
    total +=
        graph.files().len() * (size_of::<codegraph::StrId>() + size_of::<codegraph::FileIx>() + 8);
    total += graph
        .unresolved()
        .iter()
        .map(|reference| {
            reference.name.len()
                + reference
                    .import_specifier
                    .as_deref()
                    .unwrap_or_default()
                    .len()
                + reference.file.as_str().len()
        })
        .sum::<usize>();
    total += graph.unresolved().len() * (size_of::<codegraph::StrId>() + 40);
    total
}

/// A directional answer really is directional: an edge in `out_edges(source)` starts at that
/// node, and one in `in_edges(target)` ends at it.
#[test]
fn neighbour_slices_respect_direction() {
    let graph = build(actions());
    let mut outgoing = 0usize;
    for raw in 0..graph.nodes().len() {
        let v = codegraph::NodeIx::new(raw as u32);
        for ix in graph.out_edges(v) {
            let edge = graph.edge(*ix).unwrap();
            assert_eq!(
                edge.source, v,
                "out slice of {raw} holds an edge it does not start"
            );
            outgoing += 1;
        }
        for ix in graph.in_edges(v) {
            let edge = graph.edge(*ix).unwrap();
            assert_eq!(
                edge.target, v,
                "in slice of {raw} holds an edge it does not end"
            );
        }
    }
    assert_eq!(
        outgoing,
        graph.edges().len(),
        "every edge is in exactly one out slice"
    );
}
