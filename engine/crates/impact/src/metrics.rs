//! Process-local metrics of the impact, clustering, planning and risk stages.
//!
//! Instruments are created lazily on the global meter; without an installed meter provider they
//! are no-ops, so unit tests pay nothing.

#![allow(dead_code)]

use std::sync::OnceLock;
use std::time::Duration;

use opentelemetry::metrics::{Counter, Histogram};
use opentelemetry::KeyValue;

use crate::graph::model::ImpactGraph;

const METER: &str = "impact";

fn counter(
    cell: &'static OnceLock<Counter<u64>>,
    name: &'static str,
    help: &'static str,
) -> &'static Counter<u64> {
    cell.get_or_init(|| {
        opentelemetry::global::meter(METER)
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
        opentelemetry::global::meter(METER)
            .f64_histogram(name)
            .with_description(help)
            .build()
    })
}

static IMPACT_MS: OnceLock<Histogram<f64>> = OnceLock::new();
static TRUNCATIONS: OnceLock<Counter<u64>> = OnceLock::new();
static ELEMENTS: OnceLock<Counter<u64>> = OnceLock::new();
static ENDPOINT_VISITS: OnceLock<Counter<u64>> = OnceLock::new();
static UNTESTED: OnceLock<Counter<u64>> = OnceLock::new();
static TEST_SCORE: OnceLock<Histogram<f64>> = OnceLock::new();
static BUDGET_EXHAUSTED: OnceLock<Counter<u64>> = OnceLock::new();
static CLUSTER_SIZE: OnceLock<Histogram<f64>> = OnceLock::new();
static UNITS_PLANNED: OnceLock<Counter<u64>> = OnceLock::new();
static UNREVIEWED: OnceLock<Counter<u64>> = OnceLock::new();
static RISK_SIGNALS: OnceLock<Counter<u64>> = OnceLock::new();
static SECRET_DETECTIONS: OnceLock<Counter<u64>> = OnceLock::new();
static RISK_SCORE: OnceLock<Histogram<f64>> = OnceLock::new();
static RISK_LEVEL: OnceLock<Counter<u64>> = OnceLock::new();
static LOW_RISK: OnceLock<Counter<u64>> = OnceLock::new();
static LOW_RISK_OVERRIDES: OnceLock<Counter<u64>> = OnceLock::new();

/// `impact_analysis_ms`, `impact_elements_total{relation}`, `impact_truncations_total{relation}`
/// and `impact_endpoint_search_visits_total` for one finished impact graph.
pub(crate) fn record_impact(graph: &ImpactGraph, elapsed: Duration) {
    histogram(
        &IMPACT_MS,
        "impact_analysis_ms",
        "Duration of one impact graph build",
    )
    .record(elapsed.as_secs_f64() * 1000.0, &[]);
    let elements = counter(
        &ELEMENTS,
        "impact_elements_total",
        "Impact elements emitted, by relation",
    );
    for (relation, count) in &graph.stats.elements_by_relation {
        elements.add(
            u64::from(*count),
            &[KeyValue::new("relation", relation.as_str())],
        );
    }
    let truncations = counter(
        &TRUNCATIONS,
        "impact_truncations_total",
        "Impact expansions stopped by a budget, by relation",
    );
    for entry in &graph.stats.truncations {
        let relation = entry
            .truncation
            .relation
            .map_or("seed", |relation| relation.as_str());
        truncations.add(1, &[KeyValue::new("relation", relation)]);
    }
    counter(
        &ENDPOINT_VISITS,
        "impact_endpoint_search_visits_total",
        "Nodes visited by the endpoint reachability search",
    )
    .add(u64::from(graph.stats.endpoint_search_visits), &[]);
    let untested = graph
        .symbols
        .iter()
        .filter(|symbol| symbol.untested && symbol.skipped.is_none())
        .count() as u64;
    counter(
        &UNTESTED,
        "impact_untested_changed_symbols_total",
        "Changed symbols no test covers",
    )
    .add(untested, &[]);
    let score = histogram(
        &TEST_SCORE,
        "test_mapping_score",
        "Score of each test mapped to a changed symbol",
    );
    for symbol in &graph.symbols {
        for element in &symbol.elements {
            if let Some(test) = &element.test {
                score.record(f64::from(test.score), &[]);
            }
        }
    }
}

/// `impact_budget_exhausted_total`.
pub(crate) fn record_budget_exhausted() {
    counter(
        &BUDGET_EXHAUSTED,
        "impact_budget_exhausted_total",
        "Impact builds that hit the PR element cap",
    )
    .add(1, &[]);
}

/// `cluster_size` for every cluster of a clustering.
pub(crate) fn record_cluster_sizes(sizes: impl IntoIterator<Item = usize>) {
    let histogram = histogram(&CLUSTER_SIZE, "cluster_size", "Members per change cluster");
    for size in sizes {
        histogram.record(size as f64, &[]);
    }
}

/// `review_units_planned_total{risk_level}`.
pub(crate) fn record_unit_planned(level: &'static str) {
    counter(
        &UNITS_PLANNED,
        "review_units_planned_total",
        "Review units planned, by risk level",
    )
    .add(1, &[KeyValue::new("risk_level", level)]);
}

/// `unreviewed_regions_total{reason}`.
pub(crate) fn record_unreviewed(reason: &'static str) {
    counter(
        &UNREVIEWED,
        "unreviewed_regions_total",
        "Changed regions left unreviewed, by reason",
    )
    .add(1, &[KeyValue::new("reason", reason)]);
}

/// `risk_signals_total{category,source}`.
pub(crate) fn record_signal(category: &'static str, source: &'static str) {
    counter(
        &RISK_SIGNALS,
        "risk_signals_total",
        "Risk signals emitted, by category and source",
    )
    .add(
        1,
        &[
            KeyValue::new("category", category),
            KeyValue::new("source", source),
        ],
    );
}

/// `risk_secret_detections_total{rule}`.
pub(crate) fn record_secret_detection(rule: &'static str) {
    counter(
        &SECRET_DETECTIONS,
        "risk_secret_detections_total",
        "Secret-like literals detected in added lines, by rule",
    )
    .add(1, &[KeyValue::new("rule", rule)]);
}

/// `risk_score` and `risk_level_total{level}`.
pub(crate) fn record_risk(score: f32, level: &'static str) {
    histogram(&RISK_SCORE, "risk_score", "Pull request risk score").record(f64::from(score), &[]);
    counter(
        &RISK_LEVEL,
        "risk_level_total",
        "Risk assessments, by level",
    )
    .add(1, &[KeyValue::new("level", level)]);
}

/// `low_risk_symbols_total{kind}`.
pub(crate) fn record_low_risk(kind: &'static str) {
    counter(
        &LOW_RISK,
        "low_risk_symbols_total",
        "Symbols classified low-risk, by kind",
    )
    .add(1, &[KeyValue::new("kind", kind)]);
}

/// `low_risk_overrides_total{reason}`.
pub(crate) fn record_low_risk_override(reason: &'static str) {
    counter(
        &LOW_RISK_OVERRIDES,
        "low_risk_overrides_total",
        "Low-risk classifications overridden, by reason",
    )
    .add(1, &[KeyValue::new("reason", reason)]);
}
