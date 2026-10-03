//! An unreachable endpoint must neither fail nor slow the process.

use std::time::{Duration, Instant};

use telemetry::{export_failures_total, init, TelemetryConfig};

#[test]
fn unreachable_endpoint_does_not_fail_or_block() {
    // Port 1 on loopback refuses connections immediately. The spec target is 100 ms; the bound
    // is looser so the test stays stable on a loaded CI machine.
    let cfg = TelemetryConfig::new("unreachable-test")
        .with_endpoint("http://127.0.0.1:1/api/default")
        .with_header("Authorization", "Basic c2VjcmV0");
    let started = Instant::now();
    let guard = init(cfg).expect("init must succeed");
    tracing::info_span!("work").in_scope(|| tracing::info!("hello"));
    assert!(
        started.elapsed() < Duration::from_millis(250),
        "{:?}",
        started.elapsed()
    );

    let dbg = format!("{guard:?}");
    assert!(!dbg.contains("c2VjcmV0"));
    drop(guard);
    assert!(export_failures_total() >= 1, "failed exports are counted");
}
