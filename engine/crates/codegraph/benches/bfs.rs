//! `bfs` criterion benchmark (CG-008).
//!
//! `bfs/depth2_hub_node` walks two hops from the highest-out-degree node of the 1M-node
//! synthetic graph (the hub case that made the legacy traversals unbounded), and
//! `bfs/depth3_max500` walks three hops with `max_nodes = 500`. Criterion records both; the
//! run also prints single-sample p50/p95 for `benchmarks/perf/README.md`
//! (target: p95 < 2 ms at `max_nodes = 500`).

use std::time::{Duration, Instant};

use codegraph::testkit::synthetic;
use codegraph::{
    bounded_bfs, Confidence, Direction, EdgeFilter, EdgeKindSet, GraphQuery, TraversalSpec,
};
use criterion::{criterion_group, criterion_main, Criterion};

const AVG_DEGREE: u32 = 5;
const SEED: u64 = 42;
const SAMPLES: usize = 256;

fn percentile(sorted: &[Duration], fraction: f64) -> Duration {
    let index = ((sorted.len() as f64 - 1.0) * fraction) as usize;
    sorted.get(index).copied().unwrap_or_default()
}

fn timed(spec: &TraversalSpec, graph: &codegraph::Graph) -> Duration {
    let started = Instant::now();
    if let Ok(result) = bounded_bfs(graph, spec) {
        std::hint::black_box(result);
    }
    started.elapsed()
}

fn bfs(c: &mut Criterion) {
    let graph = synthetic(1_000_000, AVG_DEGREE, SEED);
    let filter = EdgeFilter::new(EdgeKindSet::ALL, Confidence::MIN);

    let mut hub = None;
    let mut best = 0usize;
    graph.for_each_node(&mut |node| {
        let degree = graph.degree(node.key, Direction::Out, &filter);
        if degree > best {
            best = degree;
            hub = Some(node.key);
        }
    });
    let Some(hub) = hub else {
        println!("bfs: graph has no nodes, benchmark skipped");
        return;
    };
    println!("bfs: hub {hub:?} has out-degree {best}");

    let depth2 = TraversalSpec::new(
        vec![hub],
        Direction::Out,
        EdgeKindSet::ALL,
        2,
        500,
        Confidence::MIN,
    );
    let depth3 = TraversalSpec::new(
        vec![hub],
        Direction::Out,
        EdgeKindSet::ALL,
        3,
        500,
        Confidence::MIN,
    );
    for (label, spec) in [
        ("bfs/depth2_hub_node", &depth2),
        ("bfs/depth3_max500", &depth3),
    ] {
        let mut samples: Vec<Duration> = Vec::with_capacity(SAMPLES);
        for _ in 0..SAMPLES {
            samples.push(timed(spec, &graph));
        }
        samples.sort_unstable();
        println!(
            "{label}: p50 = {:.3} ms, p95 = {:.3} ms (target: p95 < 2 ms)",
            percentile(&samples, 0.50).as_secs_f64() * 1e3,
            percentile(&samples, 0.95).as_secs_f64() * 1e3,
        );
        c.bench_function(label, |b| {
            b.iter(|| {
                if let Ok(result) = bounded_bfs(&graph, spec) {
                    std::hint::black_box(result);
                }
            })
        });
    }
}

criterion_group!(benches, bfs);
criterion_main!(benches);
