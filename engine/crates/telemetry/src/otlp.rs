//! OTLP/HTTP protobuf exporters (traces, metrics, logs) with failure accounting.
//!
//! Export failures never propagate: they are counted into `telemetry_export_failures_total`
//! and reported to stderr at most once every 60 seconds.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use opentelemetry::metrics::Counter;
use opentelemetry::KeyValue;
use opentelemetry_otlp::{Protocol, WithExportConfig, WithHttpConfig};
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::logs::{LogBatch, LogExporter};
use opentelemetry_sdk::metrics::data::ResourceMetrics;
use opentelemetry_sdk::metrics::exporter::PushMetricExporter;
use opentelemetry_sdk::metrics::Temporality;
use opentelemetry_sdk::trace::{SpanData, SpanExporter};
use opentelemetry_sdk::Resource;

use crate::config::TelemetryConfig;
use crate::error::{Error, Result};

/// Exporter request timeout.
pub(crate) const EXPORT_TIMEOUT: Duration = Duration::from_secs(10);
const LOG_INTERVAL: Duration = Duration::from_secs(60);

static FAILURES: AtomicU64 = AtomicU64::new(0);
static LAST_REPORT: Mutex<Option<Instant>> = Mutex::new(None);
static COUNTER: OnceLock<Counter<u64>> = OnceLock::new();

/// Number of failed export attempts in this process (`telemetry_export_failures_total`).
pub fn export_failures_total() -> u64 {
    FAILURES.load(Ordering::Relaxed)
}

/// Registers the self-metric on the global meter provider. Safe to call repeatedly.
pub(crate) fn register_self_metrics() {
    let _ = COUNTER.get_or_init(|| {
        opentelemetry::global::meter("telemetry")
            .u64_counter("telemetry_export_failures_total")
            .with_description("Failed OTLP export attempts")
            .build()
    });
}

fn observe(signal: &'static str, result: &OTelSdkResult) {
    let Err(err) = result else { return };
    FAILURES.fetch_add(1, Ordering::Relaxed);
    if let Some(counter) = COUNTER.get() {
        counter.add(1, &[KeyValue::new("signal", signal)]);
    }
    let due = match LAST_REPORT.lock() {
        Ok(mut last) => {
            let due = last.is_none_or(|t| t.elapsed() >= LOG_INTERVAL);
            if due {
                *last = Some(Instant::now());
            }
            due
        }
        Err(_) => false,
    };
    if due {
        // The error text never contains request headers (it is an SDK error string).
        eprintln!("telemetry: {signal} export failed: {err}");
    }
}

#[derive(Debug)]
pub(crate) struct CountedSpans<E>(E);

impl<E: SpanExporter> SpanExporter for CountedSpans<E> {
    async fn export(&self, batch: Vec<SpanData>) -> OTelSdkResult {
        let r = self.0.export(batch).await;
        observe("traces", &r);
        r
    }

    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        self.0.shutdown_with_timeout(timeout)
    }

    fn force_flush(&self) -> OTelSdkResult {
        self.0.force_flush()
    }

    fn set_resource(&mut self, resource: &Resource) {
        self.0.set_resource(resource);
    }
}

#[derive(Debug)]
pub(crate) struct CountedLogs<E>(E);

impl<E: LogExporter> LogExporter for CountedLogs<E> {
    async fn export(&self, batch: LogBatch<'_>) -> OTelSdkResult {
        let r = self.0.export(batch).await;
        observe("logs", &r);
        r
    }

    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        self.0.shutdown_with_timeout(timeout)
    }

    fn set_resource(&mut self, resource: &Resource) {
        self.0.set_resource(resource);
    }
}

pub(crate) struct CountedMetrics<E>(E);

impl<E: PushMetricExporter> PushMetricExporter for CountedMetrics<E> {
    async fn export(&self, metrics: &ResourceMetrics) -> OTelSdkResult {
        let r = self.0.export(metrics).await;
        observe("metrics", &r);
        r
    }

    fn force_flush(&self) -> OTelSdkResult {
        self.0.force_flush()
    }

    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        self.0.shutdown_with_timeout(timeout)
    }

    fn temporality(&self) -> Temporality {
        self.0.temporality()
    }
}

fn headers(cfg: &TelemetryConfig) -> HashMap<String, String> {
    cfg.headers
        .iter()
        .map(|(k, v)| (k.clone(), v.expose().to_owned()))
        .collect()
}

fn url(base: &str, signal: &str) -> String {
    format!("{}/v1/{signal}", base.trim_end_matches('/'))
}

fn setup_err(signal: &str, e: impl std::fmt::Display) -> Error {
    Error::Exporter(format!("{signal} exporter: {e}"))
}

pub(crate) fn span_exporter(
    cfg: &TelemetryConfig,
    base: &str,
) -> Result<CountedSpans<opentelemetry_otlp::SpanExporter>> {
    opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .with_protocol(Protocol::HttpBinary)
        .with_endpoint(url(base, "traces"))
        .with_timeout(EXPORT_TIMEOUT)
        .with_headers(headers(cfg))
        .build()
        .map(CountedSpans)
        .map_err(|e| setup_err("trace", e))
}

pub(crate) fn log_exporter(
    cfg: &TelemetryConfig,
    base: &str,
) -> Result<CountedLogs<opentelemetry_otlp::LogExporter>> {
    opentelemetry_otlp::LogExporter::builder()
        .with_http()
        .with_protocol(Protocol::HttpBinary)
        .with_endpoint(url(base, "logs"))
        .with_timeout(EXPORT_TIMEOUT)
        .with_headers(headers(cfg))
        .build()
        .map(CountedLogs)
        .map_err(|e| setup_err("log", e))
}

pub(crate) fn metric_exporter(
    cfg: &TelemetryConfig,
    base: &str,
) -> Result<CountedMetrics<opentelemetry_otlp::MetricExporter>> {
    opentelemetry_otlp::MetricExporter::builder()
        .with_http()
        .with_protocol(Protocol::HttpBinary)
        .with_endpoint(url(base, "metrics"))
        .with_timeout(EXPORT_TIMEOUT)
        .with_headers(headers(cfg))
        .build()
        .map(CountedMetrics)
        .map_err(|e| setup_err("metric", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_trims_trailing_slash() {
        assert_eq!(
            url("http://h/api/default/", "traces"),
            "http://h/api/default/v1/traces"
        );
        assert_eq!(
            url("http://h/api/default", "logs"),
            "http://h/api/default/v1/logs"
        );
    }
}
