//! Cost of an instrumented empty span with the OTLP exporter disabled (OBS-001).

use criterion::{criterion_group, criterion_main, Criterion};
use telemetry::{init, TelemetryConfig};

fn span_overhead(c: &mut Criterion) {
    let mut cfg = TelemetryConfig::new("bench");
    cfg.otel_enabled = false;
    let _guard = init(cfg);
    c.bench_function("empty_info_span", |b| {
        b.iter(|| {
            let span = tracing::info_span!("bench_span");
            let _entered = span.enter();
        });
    });
}

criterion_group!(benches, span_overhead);
criterion_main!(benches);
