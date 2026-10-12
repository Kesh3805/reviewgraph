//! Process-local metrics named by the POL and PROF specs.
#![allow(dead_code)]

use std::sync::OnceLock;

use opentelemetry::metrics::{Counter, Histogram};
use opentelemetry::KeyValue;

const METER: &str = "profile";

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

pub(crate) fn config_validation_error(kind: &'static str) {
    static C: OnceLock<Counter<u64>> = OnceLock::new();
    counter(
        &C,
        "config_validation_errors_total",
        "Config validation errors by kind",
    )
    .add(1, &[KeyValue::new("kind", kind)]);
}

pub(crate) fn review_policy_changed() {
    static C: OnceLock<Counter<u64>> = OnceLock::new();
    counter(
        &C,
        "review_policy_changed_total",
        "Pull requests that edit review policy",
    )
    .add(1, &[]);
}

pub(crate) fn rule_evaluation(rule_key: &'static str, status: &'static str) {
    static C: OnceLock<Counter<u64>> = OnceLock::new();
    counter(
        &C,
        "rule_evaluations_total",
        "Rule evaluations by rule and status",
    )
    .add(
        1,
        &[
            KeyValue::new("rule_key", rule_key),
            KeyValue::new("status", status),
        ],
    );
}

pub(crate) fn rule_violations(rule_key: &'static str, n: usize) {
    static C: OnceLock<Counter<u64>> = OnceLock::new();
    if n > 0 {
        counter(&C, "rule_violations_total", "Rule violations by rule")
            .add(n as u64, &[KeyValue::new("rule_key", rule_key)]);
    }
}

pub(crate) fn policy_conflict() {
    static C: OnceLock<Counter<u64>> = OnceLock::new();
    counter(&C, "policy_conflicts_total", "Same-rank policy conflicts").add(1, &[]);
}

pub(crate) fn candidate_findings(reviewer: &'static str, n: usize) {
    static C: OnceLock<Counter<u64>> = OnceLock::new();
    if n > 0 {
        counter(
            &C,
            "candidate_findings_total",
            "Candidate findings by reviewer",
        )
        .add(n as u64, &[KeyValue::new("reviewer", reviewer)]);
    }
}

pub(crate) fn suppressed_finding(kind: &'static str) {
    static C: OnceLock<Counter<u64>> = OnceLock::new();
    counter(
        &C,
        "suppressed_findings_total",
        "Findings suppressed by policy",
    )
    .add(
        1,
        &[
            KeyValue::new("reason", "policy"),
            KeyValue::new("kind", kind),
        ],
    );
}

pub(crate) fn convention_mined(miner: &'static str, enforceable: bool) {
    static C: OnceLock<Counter<u64>> = OnceLock::new();
    counter(&C, "conventions_mined_total", "Conventions mined").add(
        1,
        &[
            KeyValue::new("miner", miner),
            KeyValue::new("enforceable", enforceable),
        ],
    );
}

pub(crate) fn miner_duration(miner: &'static str, seconds: f64) {
    static H: OnceLock<Histogram<f64>> = OnceLock::new();
    histogram(
        &H,
        "convention_miner_duration_seconds",
        "Convention miner duration",
    )
    .record(seconds, &[KeyValue::new("miner", miner)]);
}

pub(crate) fn profile_compute_duration(seconds: f64) {
    static H: OnceLock<Histogram<f64>> = OnceLock::new();
    histogram(
        &H,
        "profile_compute_duration_seconds",
        "Profile computation duration",
    )
    .record(seconds, &[]);
}

pub(crate) fn profile_cache(hit: bool) {
    static HITS: OnceLock<Counter<u64>> = OnceLock::new();
    static MISSES: OnceLock<Counter<u64>> = OnceLock::new();
    if hit {
        counter(&HITS, "profile_cache_hits_total", "Profile cache hits").add(1, &[]);
    } else {
        counter(
            &MISSES,
            "profile_cache_misses_total",
            "Profile cache misses",
        )
        .add(1, &[]);
    }
}

pub(crate) fn documentation_rules(source: &str, n: usize) {
    static C: OnceLock<Counter<u64>> = OnceLock::new();
    if n > 0 {
        counter(
            &C,
            "documentation_rules_total",
            "Documentation rules ingested",
        )
        .add(n as u64, &[KeyValue::new("source", source.to_owned())]);
    }
}

pub(crate) fn deterministic_findings(rule: &'static str, n: usize) {
    static C: OnceLock<Counter<u64>> = OnceLock::new();
    if n > 0 {
        counter(
            &C,
            "deterministic_findings_total",
            "Deterministic pre-check findings",
        )
        .add(n as u64, &[KeyValue::new("rule", rule)]);
    }
}

pub(crate) fn cycle_detection_duration(seconds: f64) {
    static H: OnceLock<Histogram<f64>> = OnceLock::new();
    histogram(
        &H,
        "cycle_detection_duration_seconds",
        "Cycle detection duration",
    )
    .record(seconds, &[]);
}
