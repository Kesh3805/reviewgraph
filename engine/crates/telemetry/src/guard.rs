//! Shutdown handle returned by `init`.

use std::time::Duration;

use opentelemetry_sdk::logs::SdkLoggerProvider;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::trace::SdkTracerProvider;

/// Upper bound for flushing and shutting down each provider.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

/// Flushes and shuts down the OTel providers on drop. Hold it for the life of the process.
#[must_use = "dropping the guard immediately shuts telemetry down"]
#[derive(Debug)]
pub struct TelemetryGuard {
    pub(crate) tracer: Option<SdkTracerProvider>,
    pub(crate) meter: Option<SdkMeterProvider>,
    pub(crate) logger: Option<SdkLoggerProvider>,
}

impl TelemetryGuard {
    pub(crate) fn inactive() -> Self {
        Self {
            tracer: None,
            meter: None,
            logger: None,
        }
    }

    /// True when an OTLP pipeline was installed.
    pub fn otel_active(&self) -> bool {
        self.tracer.is_some()
    }

    /// Flushes pending telemetry and shuts the exporters down (bounded to 5 s per provider).
    /// Export failures are ignored. The work runs on the calling thread; exporters have their
    /// own threads, so it does not need the async runtime to make progress.
    pub async fn shutdown(mut self) {
        self.shutdown_inner();
    }

    fn shutdown_inner(&mut self) {
        if let Some(t) = self.tracer.take() {
            let _ = t.force_flush();
            let _ = t.shutdown_with_timeout(SHUTDOWN_TIMEOUT);
        }
        if let Some(l) = self.logger.take() {
            let _ = l.force_flush();
            let _ = l.shutdown_with_timeout(SHUTDOWN_TIMEOUT);
        }
        if let Some(m) = self.meter.take() {
            let _ = m.force_flush();
            let _ = m.shutdown_with_timeout(SHUTDOWN_TIMEOUT);
        }
    }
}

impl Drop for TelemetryGuard {
    fn drop(&mut self) {
        self.shutdown_inner();
    }
}
