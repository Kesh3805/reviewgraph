//! Finding lifecycle metrics (VER-001).

use std::sync::OnceLock;

use opentelemetry::metrics::Counter;
use opentelemetry::KeyValue;

fn counter(
    cell: &'static OnceLock<Counter<u64>>,
    name: &'static str,
    help: &'static str,
) -> &'static Counter<u64> {
    cell.get_or_init(|| {
        opentelemetry::global::meter("pipeline")
            .u64_counter(name)
            .with_description(help)
            .build()
    })
}

/// `finding_state_transitions_total{from,to}`: committed lifecycle transitions.
pub(crate) fn transition(from: &'static str, to: &'static str, n: u64) {
    static C: OnceLock<Counter<u64>> = OnceLock::new();
    if n > 0 {
        counter(
            &C,
            "finding_state_transitions_total",
            "Committed finding lifecycle transitions by from and to state",
        )
        .add(n, &[KeyValue::new("from", from), KeyValue::new("to", to)]);
    }
}

/// `finding_transition_conflicts_total`: CAS transitions lost to a concurrent writer.
pub(crate) fn conflict() {
    static C: OnceLock<Counter<u64>> = OnceLock::new();
    counter(
        &C,
        "finding_transition_conflicts_total",
        "Finding transitions that lost a compare-and-set race",
    )
    .add(1, &[]);
}
