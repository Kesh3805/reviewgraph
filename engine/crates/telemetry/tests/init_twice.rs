use telemetry::{init, Error, TelemetryConfig};

#[test]
fn init_twice_returns_already_initialized() {
    let _first = init(TelemetryConfig::new("twice-test"));
    let second = init(TelemetryConfig::new("twice-test"));
    assert!(matches!(second, Err(Error::AlreadyInitialized)));
}

#[test]
fn invalid_config_fails_init_with_typed_error() {
    let cfg = TelemetryConfig::new("twice-test").with_header("bad name", "v");
    assert!(matches!(init(cfg), Err(Error::InvalidConfig { .. })));
}
