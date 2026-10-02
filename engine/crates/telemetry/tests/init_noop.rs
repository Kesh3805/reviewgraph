//! Without an endpoint `init` installs stdout layers only and never touches the network.

use telemetry::{init, Error, TelemetryConfig};

#[test]
fn init_without_endpoint_is_stdout_only() {
    let cfg = TelemetryConfig::new("noop-test");
    assert!(cfg.effective_endpoint().is_none());
    match init(cfg) {
        Ok(guard) => assert!(!guard.otel_active()),
        // Another test in this binary won the race for the global subscriber.
        Err(e) => assert!(matches!(e, Error::AlreadyInitialized)),
    }
}

#[test]
fn disabled_flag_wins_over_endpoint() {
    let mut cfg = TelemetryConfig::new("noop-test").with_endpoint("http://127.0.0.1:1");
    cfg.otel_enabled = false;
    assert!(cfg.effective_endpoint().is_none());
    if let Ok(guard) = init(cfg) {
        assert!(!guard.otel_active());
    }
}
