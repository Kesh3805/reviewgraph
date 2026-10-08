//! `query_traversals` criterion benchmark (CG-007, later CG-009 additions).
//!
//! `query/neighbors_p50_p95` measures `GraphQueryExt::neighbors` over nodes whose out-degree
//! is at most 50 on the 1M-node synthetic graph: criterion records the distribution, and the
//! run also prints true p50/p95 samples for `benchmarks/perf/README.md`
//! (target: p95 < 5 µs).

use std::time::{Duration, Instant};

use codegraph::testkit::synthetic;
use codegraph::{
    Confidence, Direction, EdgeFilter, EdgeKindSet, GraphQuery, GraphQueryExt, NodeKey,
};
use criterion::{criterion_group, criterion_main, Criterion};

const AVG_DEGREE: u32 = 5;
const SEED: u64 = 42;
const SAMPLES: usize = 4096;

fn percentile(sorted: &[Duration], fraction: f64) -> Duration {
    let index = ((sorted.len() as f64 - 1.0) * fraction) as usize;
    sorted.get(index).copied().unwrap_or_default()
}

fn query_traversals(c: &mut Criterion) {
    let graph = synthetic(1_000_000, AVG_DEGREE, SEED);
    let filter = EdgeFilter::new(EdgeKindSet::ALL, Confidence::MIN);

    // Nodes whose out-degree is in 1..=50 — the population the target is stated over.
    let mut candidates: Vec<NodeKey> = Vec::new();
    graph.for_each_node(&mut |node| {
        if candidates.len() >= 2048 {
            return;
        }
        let degree = graph.degree(node.key, Direction::Out, &filter);
        if (1..=50).contains(&degree) {
            candidates.push(node.key);
        }
    });
    if candidates.is_empty() {
        println!("query/neighbors_p50_p95: no candidate nodes, benchmark skipped");
        return;
    }

    let mut samples: Vec<Duration> = Vec::with_capacity(SAMPLES);
    for index in 0..SAMPLES {
        let Some(&key) = candidates.get(index % candidates.len()) else {
            break;
        };
        let started = Instant::now();
        let found = graph.neighbors(key, Direction::Out, EdgeKindSet::ALL, Confidence::MIN);
        std::hint::black_box(found);
        samples.push(started.elapsed());
    }
    samples.sort_unstable();
    let p50 = percentile(&samples, 0.50);
    let p95 = percentile(&samples, 0.95);
    println!(
        "query/neighbors_p50_p95: n = {}, candidates = {}, p50 = {:.3} us, p95 = {:.3} us (target: p95 < 5 us)",
        SAMPLES,
        candidates.len(),
        p50.as_secs_f64() * 1e6,
        p95.as_secs_f64() * 1e6,
    );

    let mut cursor = 0usize;
    c.bench_function("query/neighbors_p50_p95", |b| {
        b.iter(|| {
            let key = candidates.get(cursor % candidates.len()).copied();
            cursor += 1;
            key.map(|key| graph.neighbors(key, Direction::Out, EdgeKindSet::ALL, Confidence::MIN))
        })
    });
}

criterion_group!(benches, query_traversals);
criterion_main!(benches);
