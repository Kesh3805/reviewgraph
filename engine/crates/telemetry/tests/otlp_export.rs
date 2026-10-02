//! OTLP export against a mocked OpenObserve.

use telemetry::{init, TelemetryConfig};
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn otlp_export_posts_protobuf_to_expected_paths() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let cfg = TelemetryConfig::new("otlp-test")
        .with_endpoint(format!("{}/api/default", server.uri()))
        .with_header("Authorization", "Basic dGVzdDp0ZXN0");
    let guard = init(cfg).expect("init");
    assert!(guard.otel_active());

    {
        let span = tracing::info_span!("export_me");
        let _e = span.enter();
        tracing::info!("a log line");
    }
    // Spans and logs are exported on shutdown; the metrics reader emits a final batch.
    guard.shutdown().await;

    let requests = server.received_requests().await.expect("requests recorded");
    let paths: Vec<&str> = requests.iter().map(|r| r.url.path()).collect();
    assert!(paths.contains(&"/api/default/v1/traces"), "{paths:?}");
    assert!(paths.contains(&"/api/default/v1/logs"), "{paths:?}");
    for r in &requests {
        let auth = r
            .headers
            .get("authorization")
            .map(|v| v.to_str().unwrap_or(""));
        assert_eq!(auth, Some("Basic dGVzdDp0ZXN0"), "{}", r.url.path());
        let ct = r
            .headers
            .get("content-type")
            .map(|v| v.to_str().unwrap_or(""));
        assert_eq!(ct, Some("application/x-protobuf"), "{}", r.url.path());
        assert!(!r.body.is_empty());
    }
}
