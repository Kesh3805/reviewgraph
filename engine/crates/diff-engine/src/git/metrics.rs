//! Process-local metrics for git reads (DIFF-001 "Observability additions").

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

use opentelemetry::metrics::{Counter, Histogram};

static OBJECT_READS: AtomicU64 = AtomicU64::new(0);
static OBJECT_COUNTER: OnceLock<Counter<u64>> = OnceLock::new();
static BLOB_DURATION: OnceLock<Histogram<f64>> = OnceLock::new();

pub(super) fn record_object_read(kind: &'static str) {
    OBJECT_READS.fetch_add(1, Ordering::Relaxed);
    let counter = OBJECT_COUNTER.get_or_init(|| {
        opentelemetry::global::meter("diff-engine")
            .u64_counter("git_object_reads_total")
            .with_description("Git objects read through GitRepo")
            .build()
    });
    counter.add(1, &[opentelemetry::KeyValue::new("kind", kind)]);
}

pub(super) fn record_blob_duration(seconds: f64) {
    let histogram = BLOB_DURATION.get_or_init(|| {
        opentelemetry::global::meter("diff-engine")
            .f64_histogram("git_read_blob_duration_seconds")
            .with_description("Duration of GitRepo::read_blob calls")
            .build()
    });
    histogram.record(seconds, &[]);
}

pub(crate) fn object_reads_total() -> u64 {
    OBJECT_READS.load(Ordering::Relaxed)
}
