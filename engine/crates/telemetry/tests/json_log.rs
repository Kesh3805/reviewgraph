//! JSON log line shape, using a local (non-global) subscriber.

use std::io;
use std::sync::{Arc, Mutex};

use opentelemetry::trace::TracerProvider as _;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::trace::SdkTracerProvider;
use opentelemetry_sdk::trace::{SpanData, SpanExporter};
use review_core::ids::{OrganizationId, ReviewRunId};
use review_core::reviewer_type::ReviewerType;
use serde_json::Value;
use telemetry::attrs::Correlation;
use telemetry::init::json_layer;
use tracing_subscriber::layer::SubscriberExt;

#[derive(Clone, Default)]
struct Buf(Arc<Mutex<Vec<u8>>>);

impl io::Write for Buf {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if let Ok(mut b) = self.0.lock() {
            b.extend_from_slice(data);
        }
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Buf {
    fn lines(&self) -> Vec<Value> {
        let bytes = self.0.lock().map(|b| b.clone()).unwrap_or_default();
        String::from_utf8_lossy(&bytes)
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    }
}

fn capture(with_otel: bool, f: impl FnOnce()) -> Vec<Value> {
    let buf = Buf::default();
    let writer = buf.clone();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(NoopExporter)
        .build();
    let otel = tracing_opentelemetry::layer().with_tracer(provider.tracer("test"));
    let subscriber = tracing_subscriber::registry()
        .with(json_layer(move || writer.clone()))
        .with(with_otel.then_some(otel));
    tracing::subscriber::with_default(subscriber, f);
    buf.lines()
}

#[test]
fn json_log_contains_trace_and_span_ids() {
    let lines = capture(true, || {
        let span = tracing::info_span!("work");
        let _e = span.enter();
        tracing::info!(count = 3, "hello");
    });
    let line = &lines[0];
    assert_eq!(line["message"], "hello");
    assert_eq!(line["level"], "INFO");
    assert!(line["timestamp"].is_string());
    assert!(line["target"].is_string());
    assert_eq!(line["fields"]["count"], 3);
    assert_eq!(line["span"]["name"], "work");
    assert_eq!(line["spans"][0]["name"], "work");
    assert_eq!(line["trace_id"].as_str().map(str::len), Some(32));
    assert_eq!(line["span_id"].as_str().map(str::len), Some(16));
}

#[test]
fn no_trace_ids_without_otel_layer() {
    let lines = capture(false, || {
        let _e = tracing::info_span!("work").entered();
        tracing::info!("hello");
    });
    assert!(lines[0].get("trace_id").is_none());
}

#[test]
fn correlation_attrs_flattened_into_log_line() {
    let run = ReviewRunId::new();
    let org = OrganizationId::new();
    let lines = capture(true, || {
        let corr = Correlation::new()
            .review_run_id(run)
            .organization_id(org)
            .reviewer_type(ReviewerType::Security)
            .request_id("req-1");
        let outer = telemetry::correlation_span!("review", &corr);
        let _o = outer.enter();
        let inner = tracing::info_span!("inner", other = 1);
        let _i = inner.enter();
        tracing::warn!("deep");
    });
    let line = &lines[0];
    assert_eq!(line["review_run_id"], run.to_string());
    assert_eq!(line["organization_id"], org.to_string());
    assert_eq!(line["reviewer_type"], "security");
    assert_eq!(line["request_id"], "req-1");
    assert!(line.get("job_id").is_none(), "unset attrs are omitted");
    assert_eq!(line["spans"].as_array().map(Vec::len), Some(2));
    assert_eq!(line["span"]["name"], "inner");
}

/// Gives the provider a processor so spans get real (valid) contexts.
#[derive(Debug)]
struct NoopExporter;

impl SpanExporter for NoopExporter {
    async fn export(&self, _batch: Vec<SpanData>) -> OTelSdkResult {
        Ok(())
    }
}
