//! Process-local metrics for the diff pipeline (DIFF-002 "Observability additions").

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use opentelemetry::metrics::{Counter, Histogram};

static FILES_DURATION: OnceLock<Histogram<f64>> = OnceLock::new();
static RENAME_LIMIT_HITS: OnceLock<Counter<u64>> = OnceLock::new();
static RENAME_LIMIT_HIT: AtomicBool = AtomicBool::new(false);

pub(crate) fn record_files_duration(seconds: f64) {
    let histogram = FILES_DURATION.get_or_init(|| {
        opentelemetry::global::meter("diff-engine")
            .f64_histogram("diff_files_duration_seconds")
            .with_description("Duration of diff_commits file mapping")
            .build()
    });
    histogram.record(seconds, &[]);
}

pub(crate) fn record_rename_limit_hit() {
    if RENAME_LIMIT_HIT.swap(true, Ordering::Relaxed) {
        return;
    }
    let counter = RENAME_LIMIT_HITS.get_or_init(|| {
        opentelemetry::global::meter("diff-engine")
            .u64_counter("diff_rename_limit_hits_total")
            .with_description("Diffs whose rename detection hit the configured limit")
            .build()
    });
    counter.add(1, &[]);
}
