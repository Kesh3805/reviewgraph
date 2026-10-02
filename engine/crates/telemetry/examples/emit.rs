//! Emits a small trace and a few log lines through `telemetry::init` using the environment
//! (`OTEL_EXPORTER_OTLP_ENDPOINT`, `OTEL_EXPORTER_OTLP_HEADERS`, ...). Used to verify export
//! against a running OpenObserve:
//!
//!   engine/scripts/cargo.sh run -p telemetry --example emit

use review_core::ids::ReviewRunId;
use telemetry::attrs::Correlation;
use telemetry::{correlation_span, init, TelemetryConfig};

#[tokio::main]
async fn main() -> anyhow_free::Result {
    let service = std::env::var("OTEL_SERVICE_NAME").unwrap_or_else(|_| "telemetry-example".into());
    let config = TelemetryConfig::from_env(&service)?;
    println!("config: {config:?}");
    let guard = init(config)?;

    let corr = Correlation::new()
        .review_run_id(ReviewRunId::new())
        .request_id("example-request");
    {
        let span = correlation_span!("example_review", &corr);
        let _e = span.enter();
        tracing::info!("example started");
        tracing::info_span!("example_child").in_scope(|| {
            tracing::warn!(items = 3, "example child work");
        });
    }
    println!("otel_active={}", guard.otel_active());
    guard.shutdown().await;
    println!("export_failures={}", telemetry::export_failures_total());
    Ok(())
}

mod anyhow_free {
    pub type Result = std::result::Result<(), Box<dyn std::error::Error>>;
}
