//! Process-local metrics for the diff pipeline ("Observability additions" of DIFF-002..006 and
//! CHG-001..009).

// Each stage wires its own recorders; a build without one stage must not warn.
#![allow(dead_code)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use opentelemetry::metrics::{Counter, Histogram};
use opentelemetry::KeyValue;

static FILES_DURATION: OnceLock<Histogram<f64>> = OnceLock::new();
static RENAME_LIMIT_HITS: OnceLock<Counter<u64>> = OnceLock::new();
static RENAME_LIMIT_HIT: AtomicBool = AtomicBool::new(false);
static HUNKS_DURATION: OnceLock<Histogram<f64>> = OnceLock::new();
static TOO_LARGE: OnceLock<Counter<u64>> = OnceLock::new();
static FILES_BY_DISPOSITION: OnceLock<Counter<u64>> = OnceLock::new();
static DISCREPANCIES: OnceLock<Counter<u64>> = OnceLock::new();
static UNANCHORABLE: OnceLock<Counter<u64>> = OnceLock::new();
static UNMAPPED: OnceLock<Counter<u64>> = OnceLock::new();
static SYMBOL_MAP_DURATION: OnceLock<Histogram<f64>> = OnceLock::new();
static CHANGED_SYMBOLS: OnceLock<Counter<u64>> = OnceLock::new();
static CHANGE_CLASSES: OnceLock<Counter<u64>> = OnceLock::new();
static CALLS_UNRESOLVED: OnceLock<Counter<u64>> = OnceLock::new();
static HEURISTIC: OnceLock<Counter<u64>> = OnceLock::new();
static AUTHZ_DIRECTION: OnceLock<Counter<u64>> = OnceLock::new();
static CHANGED_DEPENDENCIES: OnceLock<Counter<u64>> = OnceLock::new();
static CHANGED_SCHEMAS: OnceLock<Counter<u64>> = OnceLock::new();
static CHANGED_CONFIGS: OnceLock<Counter<u64>> = OnceLock::new();
static MODEL_DURATION: OnceLock<Histogram<f64>> = OnceLock::new();
static MODEL_TRUNCATED: OnceLock<Counter<u64>> = OnceLock::new();
static INTENT_ASSESSMENTS: OnceLock<Counter<u64>> = OnceLock::new();
static INTENT_REFINE_CALLS: OnceLock<Counter<u64>> = OnceLock::new();

fn counter(
    cell: &'static OnceLock<Counter<u64>>,
    name: &'static str,
    help: &'static str,
) -> &'static Counter<u64> {
    cell.get_or_init(|| {
        opentelemetry::global::meter("diff-engine")
            .u64_counter(name)
            .with_description(help)
            .build()
    })
}

fn histogram(
    cell: &'static OnceLock<Histogram<f64>>,
    name: &'static str,
    help: &'static str,
) -> &'static Histogram<f64> {
    cell.get_or_init(|| {
        opentelemetry::global::meter("diff-engine")
            .f64_histogram(name)
            .with_description(help)
            .build()
    })
}

pub(crate) fn record_files_duration(seconds: f64) {
    histogram(
        &FILES_DURATION,
        "diff_files_duration_seconds",
        "Duration of diff_commits file mapping",
    )
    .record(seconds, &[]);
}

pub(crate) fn record_rename_limit_hit() {
    if RENAME_LIMIT_HIT.swap(true, Ordering::Relaxed) {
        return;
    }
    counter(
        &RENAME_LIMIT_HITS,
        "diff_rename_limit_hits_total",
        "Diffs whose rename detection hit the configured limit",
    )
    .add(1, &[]);
}

pub(crate) fn record_hunks_duration(seconds: f64) {
    histogram(
        &HUNKS_DURATION,
        "diff_hunks_duration_seconds",
        "Duration of hunk computation for one diff",
    )
    .record(seconds, &[]);
}

pub(crate) fn record_too_large(n: u64) {
    if n > 0 {
        counter(
            &TOO_LARGE,
            "diff_files_too_large_total",
            "Files skipped as too large to diff",
        )
        .add(n, &[]);
    }
}

pub(crate) fn record_disposition(label: &'static str) {
    counter(
        &FILES_BY_DISPOSITION,
        "diff_files_total",
        "Changed files by disposition",
    )
    .add(1, &[KeyValue::new("disposition", label)]);
}

pub(crate) fn record_discrepancy(kind: &'static str) {
    counter(
        &DISCREPANCIES,
        "diff_discrepancies_total",
        "Local vs provider diff discrepancies",
    )
    .add(1, &[KeyValue::new("kind", kind)]);
}

pub(crate) fn record_unanchorable(n: u64) {
    if n > 0 {
        counter(
            &UNANCHORABLE,
            "diff_unanchorable_files_total",
            "Files without anchorable lines",
        )
        .add(n, &[]);
    }
}

pub(crate) fn record_unmapped(reason: &'static str) {
    counter(
        &UNMAPPED,
        "symbol_map_unmapped_total",
        "Changed ranges not mapped to a symbol",
    )
    .add(1, &[KeyValue::new("reason", reason)]);
}

pub(crate) fn record_symbol_map_duration(seconds: f64) {
    histogram(
        &SYMBOL_MAP_DURATION,
        "symbol_map_duration_seconds",
        "Duration of hunk to symbol mapping",
    )
    .record(seconds, &[]);
}

pub(crate) fn record_changed_symbol(change: &'static str) {
    counter(
        &CHANGED_SYMBOLS,
        "changed_symbols_total",
        "Changed symbols by change kind",
    )
    .add(1, &[KeyValue::new("change", change)]);
}

pub(crate) fn record_change_class(class: &'static str) {
    counter(
        &CHANGE_CLASSES,
        "change_classes_total",
        "Classified changes by class",
    )
    .add(1, &[KeyValue::new("class", class)]);
}

pub(crate) fn record_unresolved_call(side: &'static str) {
    counter(
        &CALLS_UNRESOLVED,
        "change_calls_unresolved_total",
        "Call facts without a resolved graph target",
    )
    .add(1, &[KeyValue::new("side", side)]);
}

pub(crate) fn record_heuristic(classifier: &'static str) {
    counter(
        &HEURISTIC,
        "change_heuristic_total",
        "Heuristic (not exact) classifications",
    )
    .add(1, &[KeyValue::new("classifier", classifier)]);
}

pub(crate) fn record_authz_direction(direction: &'static str) {
    counter(
        &AUTHZ_DIRECTION,
        "change_authorization_direction_total",
        "Authorization changes by direction",
    )
    .add(1, &[KeyValue::new("direction", direction)]);
}

pub(crate) fn record_changed_dependency(kind: &'static str) {
    counter(
        &CHANGED_DEPENDENCIES,
        "changed_dependencies_total",
        "Changed dependencies by change kind",
    )
    .add(1, &[KeyValue::new("kind", kind)]);
}

pub(crate) fn record_changed_schema(destructive: bool) {
    counter(
        &CHANGED_SCHEMAS,
        "changed_schemas_total",
        "Changed schema files",
    )
    .add(1, &[KeyValue::new("destructive", destructive)]);
}

pub(crate) fn record_changed_config(kind: &'static str) {
    counter(
        &CHANGED_CONFIGS,
        "changed_configs_total",
        "Changed configuration files by kind",
    )
    .add(1, &[KeyValue::new("kind", kind)]);
}

pub(crate) fn record_model_duration(seconds: f64) {
    histogram(
        &MODEL_DURATION,
        "change_model_build_duration_seconds",
        "Duration of change model assembly",
    )
    .record(seconds, &[]);
}

pub(crate) fn record_model_truncated(kind: &'static str) {
    counter(
        &MODEL_TRUNCATED,
        "change_model_truncated_total",
        "Change model sections truncated by a budget",
    )
    .add(1, &[KeyValue::new("kind", kind)]);
}

pub(crate) fn record_intent(intent: &'static str, source: &'static str) {
    counter(
        &INTENT_ASSESSMENTS,
        "intent_assessments_total",
        "Intent assessments by primary intent and source",
    )
    .add(
        1,
        &[
            KeyValue::new("intent", intent),
            KeyValue::new("source", source),
        ],
    );
}

pub(crate) fn record_intent_refine(result: &'static str) {
    counter(
        &INTENT_REFINE_CALLS,
        "intent_refine_calls_total",
        "Intent refiner invocations by result",
    )
    .add(1, &[KeyValue::new("result", result)]);
}
