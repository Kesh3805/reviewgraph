//! OBS-005: the metric registry contract.

#![allow(clippy::unwrap_used)]

use std::collections::BTreeSet;
use std::path::Path;

use telemetry::metrics::catalog::{render_markdown, ACCEPTANCE_FORMULA, FALSE_POSITIVE_FORMULA};
use telemetry::metrics::rates::FeedbackWindow;
use telemetry::metrics::{buckets, instruments, labels, Kind, Registry, CATALOG};

fn registry() -> Registry {
    Registry::new(&opentelemetry::global::meter("metrics-registry-test"))
}

#[test]
fn all_required_metrics_registered_with_expected_type_and_unit() {
    use Kind::{Counter as C, Gauge as G, Histogram as H};
    let required: [(&str, Kind, &str); 25] = [
        ("review_duration_seconds", H, "s"),
        ("incremental_index_duration_seconds", H, "s"),
        ("files_reparsed_total", C, "1"),
        ("symbols_changed_total", C, "1"),
        ("graph_nodes_total", G, "1"),
        ("graph_edges_total", G, "1"),
        ("graph_invalidations_total", C, "1"),
        ("context_tokens_total", C, "1"),
        ("context_symbols_total", C, "1"),
        ("qdrant_queries_total", C, "1"),
        ("qdrant_query_duration_seconds", H, "s"),
        ("llm_requests_total", C, "1"),
        ("llm_input_tokens_total", C, "1"),
        ("llm_output_tokens_total", C, "1"),
        ("llm_cached_tokens_total", C, "1"),
        ("llm_cost_estimate", C, "usd_micros"),
        ("candidate_findings_total", C, "1"),
        ("verified_findings_total", C, "1"),
        ("published_findings_total", C, "1"),
        ("suppressed_findings_total", C, "1"),
        ("finding_acceptance_rate", G, "ratio"),
        ("finding_false_positive_rate", G, "ratio"),
        ("queue_depth", G, "1"),
        ("queue_wait_seconds", H, "s"),
        ("worker_duration_seconds", H, "s"),
    ];
    let reg = registry();
    for (name, kind, unit) in required {
        assert_eq!(reg.kind_of(name), Some(kind), "{name}");
        assert_eq!(instruments::find(name).unwrap().unit, unit, "{name}");
    }
}

#[test]
fn catalog_names_are_unique_and_snake_case() {
    let mut seen = BTreeSet::new();
    for spec in CATALOG {
        assert!(seen.insert(spec.name), "duplicate {}", spec.name);
        assert!(spec
            .name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'));
    }
    assert_eq!(registry().names().len(), CATALOG.len());
}

#[test]
fn label_structs_reject_forbidden_label_names() {
    // Compile-time enforcement through typed label structs is a follow-up; the catalog is
    // checked here instead: no instrument may declare a label outside the allow-list.
    for spec in CATALOG {
        for label in spec.labels {
            assert!(labels::is_allowed(label), "{} uses {label}", spec.name);
            assert!(
                !labels::FORBIDDEN.contains(label),
                "{} uses {label}",
                spec.name
            );
        }
    }
    let reg = registry();
    reg.add(
        "files_reparsed_total",
        1,
        &[("language", "rust"), ("organization_id", "o-1")],
    );
    assert_eq!(
        reg.rejected(),
        0,
        "a forbidden label is dropped, not an error"
    );
    reg.add("not_a_metric", 1, &[]);
    assert_eq!(reg.rejected(), 1);
}

#[test]
fn histogram_buckets_cover_target_ranges() {
    let b = buckets::DURATION_SECONDS;
    assert_eq!(b.first().copied(), Some(0.05));
    assert_eq!(b.last().copied(), Some(900.0));
    assert!(b.windows(2).all(|w| w[0] < w[1]));
    for needed in [0.1, 1.0, 10.0, 60.0, 300.0] {
        assert!(b.contains(&needed));
    }
}

#[test]
fn unknown_label_value_maps_to_other() {
    assert_eq!(labels::bound("reviewer_type", "security"), "security");
    assert_eq!(labels::bound("reviewer_type", "organization-123"), "other");
    assert_eq!(labels::bound("severity", "catastrophic"), "other");
    assert_eq!(
        labels::bound("model", "claude-sonnet-5-5"),
        "claude-sonnet-5-5"
    );
    assert_eq!(labels::bound("model", "src/secret/path.rs"), "other");
    assert_eq!(labels::bound("language", &"x".repeat(200)), "other");
    assert_eq!(labels::bound("language", ""), "other");
}

#[test]
fn queue_depth_gauge_reports_per_queue_and_state() {
    let reg = registry();
    reg.set_gauge(
        "queue_depth",
        7.0,
        &[("queue", "pr-review"), ("state", "queued")],
    );
    reg.set_gauge(
        "queue_depth",
        2.0,
        &[("queue", "pr-review"), ("state", "running")],
    );
    reg.set_gauge(
        "queue_depth",
        1.0,
        &[("queue", "review-publish"), ("state", "queued")],
    );
    let read = |q: &str, s: &str| reg.gauge_value("queue_depth", &[("queue", q), ("state", s)]);
    assert_eq!(read("pr-review", "queued"), Some(7.0));
    assert_eq!(read("pr-review", "running"), Some(2.0));
    assert_eq!(read("review-publish", "queued"), Some(1.0));
    assert_eq!(read("review-publish", "running"), None);
    // Last write wins for one series.
    reg.set_gauge(
        "queue_depth",
        9.0,
        &[("state", "queued"), ("queue", "pr-review")],
    );
    assert_eq!(read("pr-review", "queued"), Some(9.0));
}

#[test]
fn acceptance_and_fp_rate_formulas_match_catalog_fixture() {
    let window = FeedbackWindow {
        useful: 6,
        already_handled: 2,
        false_positive: 1,
        published_with_feedback: 10,
    };
    assert_eq!(window.acceptance_rate(), Some(0.8));
    assert_eq!(window.false_positive_rate(), Some(0.1));
    assert_eq!(FeedbackWindow::default().acceptance_rate(), None);
    let doc = render_markdown();
    assert!(doc.contains(ACCEPTANCE_FORMULA));
    assert!(doc.contains(FALSE_POSITIVE_FORMULA));
}

#[test]
fn metrics_catalog_doc_is_up_to_date() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../docs/operations/metrics-catalog.md");
    let committed = std::fs::read_to_string(path).unwrap().replace("\r\n", "\n");
    assert_eq!(
        committed,
        render_markdown(),
        "docs/operations/metrics-catalog.md is stale; run `cargo run -p xtask -- metrics-catalog`"
    );
}

#[test]
fn shared_metrics_exist_in_the_catalog() {
    for name in instruments::SHARED_WITH_TYPESCRIPT {
        assert!(instruments::find(name).is_some(), "{name}");
    }
}
