//! `graph_build` criterion benchmark (CG-004).
//!
//! `graph_build/{10k,100k}_nodes` are criterion means over repeated builds;
//! `graph_build/1m_nodes` times a single build of the target case (1M nodes / ~5M edges
//! under 4 s and a heap under 1.2 GB in the engine container) because repeating it a hundred
//! times would cost minutes for one number. The single-run figure is printed for the
//! baseline table in `benchmarks/perf/README.md`.

use std::time::Instant;

use codegraph::testkit::synthetic;
use codegraph::GraphQuery;
use criterion::{criterion_group, criterion_main, Criterion};

const AVG_DEGREE: u32 = 5;
const SEED: u64 = 42;

fn graph_build(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_build");
    // Builds are expensive per iteration; ten samples keep the run to seconds, not minutes.
    group.sample_size(10);
    for (label, nodes) in [("10k_nodes", 10_000u32), ("100k_nodes", 100_000u32)] {
        group.bench_function(label, |b| {
            b.iter(|| {
                let graph = synthetic(nodes, AVG_DEGREE, SEED);
                std::hint::black_box((graph.node_count(), graph.heap_size_bytes()))
            })
        });
    }
    group.finish();

    let started = Instant::now();
    let graph = synthetic(1_000_000, AVG_DEGREE, SEED);
    let elapsed = started.elapsed();
    let heap = graph.heap_size_bytes();
    println!(
        "graph_build/1m_nodes: build = {:.3} s, heap = {:.1} MiB, nodes = {}, edges = {} (target: build < 4 s, heap <= 1.2 GB)",
        elapsed.as_secs_f64(),
        heap as f64 / (1024.0 * 1024.0),
        graph.node_count(),
        graph.edge_count(),
    );
    assert!(
        heap < 1_200_000_000,
        "heap {} exceeds the 1.2 GB memory target",
        heap
    );
}

criterion_group!(benches, graph_build);
criterion_main!(benches);
