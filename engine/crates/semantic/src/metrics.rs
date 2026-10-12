//! Metric instruments of the semantic layer (OBS-002 conventions).
//!
//! Instruments live on the global meter provider (a no-op until `telemetry::init` installs one).
//! The few counters that tests and health checks need to read in-process are mirrored in atomics.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

use opentelemetry::metrics::{Counter, Gauge, Histogram};
use opentelemetry::KeyValue;

struct Instruments {
    embedding_latency_ms: Histogram<f64>,
    embedding_tokens_total: Counter<u64>,
    embedding_errors_total: Counter<u64>,
    embedding_batches_total: Counter<u64>,
    qdrant_latency_ms: Histogram<f64>,
    qdrant_errors_total: Counter<u64>,
    qdrant_scope_violation_total: Counter<u64>,
    semantic_available: Gauge<u64>,
    semantic_collection_transitions_total: Counter<u64>,
    embedding_units_built_total: Counter<u64>,
    embedding_unit_chars: Histogram<f64>,
    embedding_points_upserted_total: Counter<u64>,
    embedding_skipped_unchanged_total: Counter<u64>,
    embedding_rekeyed_total: Counter<u64>,
    embedding_points_deleted_total: Counter<u64>,
    embedding_deferred_total: Counter<u64>,
    semantic_invalidated_units_total: Counter<u64>,
}

static INSTRUMENTS: OnceLock<Instruments> = OnceLock::new();
static SCOPE_VIOLATIONS: AtomicU64 = AtomicU64::new(0);
static AVAILABLE: AtomicU64 = AtomicU64::new(0);

fn instruments() -> &'static Instruments {
    INSTRUMENTS.get_or_init(|| {
        let m = opentelemetry::global::meter("semantic");
        Instruments {
            embedding_latency_ms: m.f64_histogram("embedding_latency_ms").build(),
            embedding_tokens_total: m.u64_counter("embedding_tokens_total").build(),
            embedding_errors_total: m.u64_counter("embedding_errors_total").build(),
            embedding_batches_total: m.u64_counter("embedding_batches_total").build(),
            qdrant_latency_ms: m.f64_histogram("qdrant_latency_ms").build(),
            qdrant_errors_total: m.u64_counter("qdrant_errors_total").build(),
            qdrant_scope_violation_total: m
                .u64_counter("qdrant_scope_violation_total")
                .with_description("Search hits or writes that fell outside the tenant scope")
                .build(),
            semantic_available: m.u64_gauge("semantic_available").build(),
            semantic_collection_transitions_total: m
                .u64_counter("semantic_collection_transitions_total")
                .build(),
            embedding_units_built_total: m.u64_counter("embedding_units_built_total").build(),
            embedding_unit_chars: m.f64_histogram("embedding_unit_chars").build(),
            embedding_points_upserted_total: m
                .u64_counter("embedding_points_upserted_total")
                .build(),
            embedding_skipped_unchanged_total: m
                .u64_counter("embedding_skipped_unchanged_total")
                .build(),
            embedding_rekeyed_total: m.u64_counter("embedding_rekeyed_total").build(),
            embedding_points_deleted_total: m.u64_counter("embedding_points_deleted_total").build(),
            embedding_deferred_total: m.u64_counter("embedding_deferred_total").build(),
            semantic_invalidated_units_total: m
                .u64_counter("semantic_invalidated_units_total")
                .build(),
        }
    })
}

pub(crate) fn embedding_call(provider: &'static str, latency_ms: u32, tokens: u32) {
    let i = instruments();
    let attrs = [KeyValue::new("provider", provider)];
    i.embedding_latency_ms.record(f64::from(latency_ms), &attrs);
    i.embedding_tokens_total.add(u64::from(tokens), &attrs);
}

pub(crate) fn embedding_error(provider: &'static str, kind: &'static str) {
    instruments().embedding_errors_total.add(
        1,
        &[
            KeyValue::new("provider", provider),
            KeyValue::new("kind", kind),
        ],
    );
}

pub(crate) fn embedding_batch(provider: &'static str) {
    instruments()
        .embedding_batches_total
        .add(1, &[KeyValue::new("provider", provider)]);
}

pub(crate) fn qdrant_call(op: &'static str, latency_ms: f64) {
    instruments()
        .qdrant_latency_ms
        .record(latency_ms, &[KeyValue::new("op", op)]);
}

pub(crate) fn qdrant_error(op: &'static str, kind: &'static str) {
    instruments()
        .qdrant_errors_total
        .add(1, &[KeyValue::new("op", op), KeyValue::new("kind", kind)]);
}

pub(crate) fn scope_violation() {
    SCOPE_VIOLATIONS.fetch_add(1, Ordering::Relaxed);
    instruments().qdrant_scope_violation_total.add(1, &[]);
}

/// `qdrant_scope_violation_total` observed in this process. Must stay 0 (alert on > 0).
pub fn scope_violations_total() -> u64 {
    SCOPE_VIOLATIONS.load(Ordering::Relaxed)
}

pub(crate) fn set_available(up: bool) {
    let v = u64::from(up);
    AVAILABLE.store(v, Ordering::Relaxed);
    instruments().semantic_available.record(v, &[]);
}

/// Last value of the `semantic_available` gauge.
pub fn semantic_available() -> bool {
    AVAILABLE.load(Ordering::Relaxed) == 1
}

pub(crate) fn collection_transition(to: &'static str) {
    instruments()
        .semantic_collection_transitions_total
        .add(1, &[KeyValue::new("to", to)]);
}

pub(crate) fn unit_built(kind: &'static str, chars: usize) {
    let i = instruments();
    let attrs = [KeyValue::new("kind", kind)];
    i.embedding_units_built_total.add(1, &attrs);
    i.embedding_unit_chars.record(chars as f64, &attrs);
}

pub(crate) fn sync_counts(upserted: u64, skipped: u64, rekeyed: u64, deleted: u64, deferred: u64) {
    let i = instruments();
    i.embedding_points_upserted_total.add(upserted, &[]);
    i.embedding_skipped_unchanged_total.add(skipped, &[]);
    i.embedding_rekeyed_total.add(rekeyed, &[]);
    i.embedding_points_deleted_total.add(deleted, &[]);
    i.embedding_deferred_total.add(deferred, &[]);
}

pub(crate) fn invalidated(kind: &'static str, n: u64) {
    instruments()
        .semantic_invalidated_units_total
        .add(n, &[KeyValue::new("kind", kind)]);
}
