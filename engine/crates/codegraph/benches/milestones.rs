//! `CG-009`/`CG-010`/`CG-011`/`CG-012` criterion benchmarks.
//!
//! Four groups, one per task that carries a runtime budget:
//!
//! * `path` — shortest path and bounded subgraph extraction over the synthetic graph, at the two
//!   sizes that matter: a small graph where the whole answer fits under the node cap, and a large
//!   one where the cap is what stops the walk.
//! * `overlay` — a delta applied to a base, as `GraphQuery` reads merge the two edge lists, plus
//!   the same delta flattened (what compaction pays).
//! * `codec` — encode and decode at a size a pull request actually moves and at a size that makes
//!   the byte counts meaningful.
//! * `validate` — the consistency check, the delta check and the diff.
//!
//! Every measurement prints p50/p95 for `benchmarks/perf/README.md` alongside the criterion
//! numbers, so one run produces both the regression signal and the table entry. Timings are for the
//! optimized `bench` profile; the input size is printed with each number so a reader can tell
//! whether two runs are comparable.

use std::sync::Arc;
use std::time::{Duration, Instant};

use codegraph::testkit::synthetic;
use codegraph::{
    clamp_max_nodes, compare, decode_graph, encode_graph, shortest_path, subgraph, validate,
    CompareOptions, Confidence, DecodeLimits, Direction, Edge, EdgeKind, EdgeKindSet, FileChange,
    Graph, GraphDelta, GraphOverlay, GraphQuery, NodeId, NodeInput, NodeKey, NodeKind, PathSpec,
    Provenance, ResolvedBy, SubgraphSpec, SCHEMA_VERSION, SUBGRAPH_MAX_NODES,
};
use criterion::{criterion_group, criterion_main, Criterion};
use review_core::location::{ContentHash, RepoPath};

const SEED: u64 = 42;
const SAMPLES: usize = 128;

fn percentile(sorted: &[Duration], fraction: f64) -> Duration {
    let index = ((sorted.len() as f64 - 1.0) * fraction) as usize;
    sorted.get(index).copied().unwrap_or_default()
}

/// Times `body` `SAMPLES` times and reports p50/p95, then hands the same closure to criterion.
///
/// `size` describes the input so a printed number is self-describing; there is deliberately no
/// invented pass/fail budget here, because the only budget this crate owns is `bfs`'s.
fn report(c: &mut Criterion, label: &str, size: &str, mut body: impl FnMut()) {
    let mut samples: Vec<Duration> = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let started = Instant::now();
        body();
        samples.push(started.elapsed());
    }
    samples.sort_unstable();
    println!(
        "{label} [{size}]: p50 = {:.3} ms, p95 = {:.3} ms",
        percentile(&samples, 0.50).as_secs_f64() * 1e3,
        percentile(&samples, 0.95).as_secs_f64() * 1e3,
    );
    c.bench_function(label, |b| b.iter(&mut body));
}

/// The first `count` node keys, which are enough to seed a traversal.
fn seeds(graph: &dyn GraphQuery, count: usize) -> Vec<NodeKey> {
    let mut out: Vec<NodeKey> = Vec::with_capacity(count);
    graph.for_each_node(&mut |node| {
        if out.len() < count {
            out.push(node.key);
        }
    });
    out
}

fn benches_path(c: &mut Criterion) {
    for count in [1_000u32, 100_000] {
        let graph = synthetic(count, 5, SEED);
        let ends = seeds(&graph, 2);
        let Some(spec) = ends.first().zip(ends.get(1)).map(|(from, to)| {
            let mut spec = PathSpec::new(*from, *to, Direction::Out, EdgeKindSet::ALL);
            spec.max_nodes = clamp_max_nodes(SUBGRAPH_MAX_NODES);
            spec
        }) else {
            println!("path/{count}_nodes: graph has too few nodes, benchmark skipped");
            continue;
        };
        report(
            c,
            &format!("path/{count}_nodes"),
            &format!("{count} nodes"),
            || {
                let _ = std::hint::black_box(shortest_path(&graph, &spec));
            },
        );

        let sub = SubgraphSpec::new(seeds(&graph, 4), 2, Direction::Out, EdgeKindSet::ALL);
        report(
            c,
            &format!("subgraph/{count}_nodes"),
            &format!("{count} nodes, depth 2"),
            || {
                let _ = std::hint::black_box(subgraph(&graph, &sub));
            },
        );
    }
}

/// A delta shaped like a real PR head: one node deleted, `touched` added with their edges.
fn delta_for(base: &Graph, touched: usize) -> GraphDelta {
    let mut delta = GraphDelta::new(SCHEMA_VERSION);
    let keys = seeds(base, touched);

    if let Some(removed) = keys.first().copied() {
        delta.nodes_removed.push(removed);
    }
    for n in 0..touched {
        let Ok(file) = RepoPath::new(format!("src/new{n}.rs")) else {
            continue;
        };
        delta.nodes_added.push(
            NodeInput::new(
                NodeId::from_canonical(format!("rs:src/new{n}.rs#n/function")),
                NodeKind::Function,
                format!("new{n}"),
            )
            .in_file(file.clone()),
        );
        delta.files.push(FileChange::modified(
            file,
            ContentHash::of(format!("new{n}").as_bytes()),
        ));
    }
    for (n, key) in keys.iter().enumerate().skip(1) {
        delta.edges_added.push(Edge::new(
            EdgeKind::Calls,
            *key,
            NodeId::from_canonical(format!("rs:src/new{n}.rs#n/function")).key(),
            Confidence::from_f32(0.9),
            ResolvedBy::NameUnique,
            Provenance::Linker,
        ));
    }
    delta.normalize();
    delta
}

fn benches_overlay(c: &mut Criterion) {
    let base = synthetic(50_000, 5, SEED);
    let delta = delta_for(&base, 200);
    let overlay = match GraphOverlay::new(Arc::new(base), Arc::new(delta)) {
        Ok(overlay) => overlay,
        Err(error) => {
            println!("overlay: fixture delta rejected ({error}), benchmark skipped");
            return;
        }
    };
    let Some(from) = seeds(&overlay, 1).first().copied() else {
        println!("overlay: graph has no nodes, benchmark skipped");
        return;
    };

    report(
        c,
        "overlay/degree_merged",
        "50k nodes + 200-node delta",
        || {
            std::hint::black_box(overlay.degree(from, Direction::Out, &codegraph::EdgeFilter::ALL));
        },
    );
    report(c, "overlay/flatten", "50k nodes + 200-node delta", || {
        if let Ok(flat) = overlay.flatten() {
            std::hint::black_box(flat);
        }
    });
    report(
        c,
        "overlay/compare_with_flatten",
        "50k nodes + 200-node delta",
        || {
            if let Ok(flat) = overlay.flatten() {
                std::hint::black_box(compare(&overlay, &flat, &CompareOptions::lenient()));
            }
        },
    );
}

fn benches_codec(c: &mut Criterion) {
    for count in [10_000u32, 200_000] {
        let graph = synthetic(count, 5, SEED);
        let mut bytes = Vec::new();
        let stats = encode_graph(&graph, &mut bytes).unwrap_or_default();
        println!(
            "codec/{count}_nodes: {} KiB compressed from {} KiB raw",
            bytes.len() / 1024,
            stats.bytes_uncompressed / 1024
        );

        report(
            c,
            &format!("codec/encode_{count}"),
            &format!("{count} nodes"),
            || {
                let mut out = Vec::with_capacity(bytes.len());
                let _ = std::hint::black_box(encode_graph(&graph, &mut out));
            },
        );
        report(
            c,
            &format!("codec/decode_{count}"),
            &format!("{count} nodes"),
            || {
                let _ = std::hint::black_box(decode_graph(
                    &mut bytes.as_slice(),
                    DecodeLimits::default(),
                ));
            },
        );
    }
}

fn benches_validate(c: &mut Criterion) {
    let a = synthetic(50_000, 5, SEED);
    let b = synthetic(50_000, 5, SEED + 1);
    let delta = delta_for(&a, 200);

    report(c, "validate/50k_nodes", "50k nodes", || {
        std::hint::black_box(validate(&a, SCHEMA_VERSION));
    });
    report(
        c,
        "validate/delta_local",
        "50k nodes + 200-node delta",
        || {
            std::hint::black_box(codegraph::validate_delta_local(&a, &delta));
        },
    );
    report(c, "compare/50k_nodes", "two 50k-node graphs", || {
        std::hint::black_box(compare(&a, &b, &CompareOptions::lenient()));
    });
}

criterion_group!(
    benches,
    benches_path,
    benches_overlay,
    benches_codec,
    benches_validate
);
criterion_main!(benches);
