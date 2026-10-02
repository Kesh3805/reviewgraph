//! `shutdown().await` flushes spans that are still queued (the batch delay is 5 s).

use std::time::{Duration, Instant};

use telemetry::{init, TelemetryConfig};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_flushes_pending_spans() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/default/v1/traces"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let cfg = TelemetryConfig::new("flush-test")
        .with_endpoint(format!("{}/api/default", server.uri()))
        .with_filter("info");
    let guard = init(cfg).expect("init");
    tracing::info_span!("pending").in_scope(|| {});

    let started = Instant::now();
    assert!(server
        .received_requests()
        .await
        .unwrap_or_default()
        .is_empty());
    guard.shutdown().await;
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "flush must beat the batch delay"
    );

    let n = server
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .filter(|r| r.url.path().ends_with("/v1/traces"))
        .count();
    assert!(n >= 1, "span batch was not flushed");
}
