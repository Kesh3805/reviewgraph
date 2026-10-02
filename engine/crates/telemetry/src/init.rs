//! `init`: installs the global subscriber and, when an endpoint is configured, the OTel
//! tracer, meter and logger providers.
//!
//! Layer order (outermost first): `EnvFilter` -> redaction slot (OBS-006) -> stdout format
//! layer -> OpenTelemetry span layer -> OpenTelemetry log appender.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use opentelemetry::trace::TracerProvider as _;
use opentelemetry::KeyValue;
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use opentelemetry_sdk::logs::SdkLoggerProvider;
use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::trace::{
    BatchConfigBuilder, BatchSpanProcessor, Sampler, SdkTracerProvider,
};
use opentelemetry_sdk::Resource;
use tracing_subscriber::filter::filter_fn;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer, Registry};

use crate::config::{LogFormat, TelemetryConfig};
use crate::error::{Error, Result};
use crate::guard::TelemetryGuard;
use crate::json_format::{DispatchSlot, JsonLayer};
use crate::otlp;

/// A boxed layer that sits directly on the registry (used for the redaction slot).
pub type BoxedLayer = Box<dyn Layer<Registry> + Send + Sync + 'static>;

const SPAN_QUEUE: usize = 2048;
const SPAN_SCHEDULE: Duration = Duration::from_secs(5);
const METRIC_INTERVAL: Duration = Duration::from_secs(15);

static INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Installs telemetry for this process. Callable once; a second call returns
/// [`Error::AlreadyInitialized`]. An unreachable OTLP endpoint never fails initialization.
pub fn init(config: TelemetryConfig) -> Result<TelemetryGuard> {
    init_with(config, None)
}

/// [`init`] with the redaction layer slot filled (OBS-006).
pub fn init_with(config: TelemetryConfig, redaction: Option<BoxedLayer>) -> Result<TelemetryGuard> {
    config.validate()?;
    if INITIALIZED
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err(Error::AlreadyInitialized);
    }
    let result = build(&config, redaction);
    if result.is_err() {
        INITIALIZED.store(false, Ordering::SeqCst);
    }
    result
}

fn build(config: &TelemetryConfig, redaction: Option<BoxedLayer>) -> Result<TelemetryGuard> {
    // The exporters report failures through `telemetry_export_failures_total` and a rate
    // limited stderr line, so their own tracing events stay off unless asked for explicitly.
    let directives = if config.filter.contains("opentelemetry") {
        config.filter.clone()
    } else {
        format!(
            "{},opentelemetry=off,opentelemetry_sdk=off,opentelemetry_otlp=off,opentelemetry_http=off",
            config.filter
        )
    };
    let filter = EnvFilter::try_new(directives).map_err(|e| Error::InvalidConfig {
        field: "RUST_LOG",
        reason: e.to_string(),
    })?;

    let mut layers: Vec<BoxedLayer> = vec![Box::new(filter)];
    if let Some(redaction) = redaction {
        layers.push(redaction);
    }
    let (stdout, slot) = stdout_layer(config.log_format);
    layers.push(stdout);

    let mut guard = TelemetryGuard::inactive();
    if let Some(base) = config.effective_endpoint() {
        let resource = resource(config);

        let span_exporter = otlp::span_exporter(config, base)?;
        let log_exporter = otlp::log_exporter(config, base)?;
        let metric_exporter = otlp::metric_exporter(config, base)?;

        let meter = SdkMeterProvider::builder()
            .with_resource(resource.clone())
            .with_reader(
                PeriodicReader::builder(metric_exporter)
                    .with_interval(METRIC_INTERVAL)
                    .build(),
            )
            .build();
        opentelemetry::global::set_meter_provider(meter.clone());
        otlp::register_self_metrics();

        let batch = BatchConfigBuilder::default()
            .with_max_queue_size(SPAN_QUEUE)
            .with_scheduled_delay(SPAN_SCHEDULE)
            .build();
        let tracer = SdkTracerProvider::builder()
            .with_resource(resource.clone())
            .with_sampler(Sampler::ParentBased(Box::new(Sampler::TraceIdRatioBased(
                config.sampler_ratio,
            ))))
            .with_span_processor(
                BatchSpanProcessor::builder(span_exporter)
                    .with_batch_config(batch)
                    .build(),
            )
            .build();
        opentelemetry::global::set_tracer_provider(tracer.clone());
        opentelemetry::global::set_text_map_propagator(TraceContextPropagator::new());

        let logger = SdkLoggerProvider::builder()
            .with_resource(resource)
            .with_batch_exporter(log_exporter)
            .build();

        layers.push(Box::new(
            tracing_opentelemetry::layer().with_tracer(tracer.tracer("telemetry")),
        ));
        // Keep the exporters' own HTTP stack out of the log pipeline (feedback loop).
        layers.push(Box::new(
            OpenTelemetryTracingBridge::new(&logger).with_filter(filter_fn(|meta| {
                let t = meta.target();
                !(t.starts_with("hyper")
                    || t.starts_with("h2")
                    || t.starts_with("reqwest")
                    || t.starts_with("opentelemetry"))
            })),
        ));

        guard.tracer = Some(tracer);
        guard.meter = Some(meter);
        guard.logger = Some(logger);
    }

    tracing_subscriber::registry()
        .with(layers)
        .try_init()
        .map_err(|_| Error::SubscriberAlreadySet)?;
    // Boxed/Vec layer wrappers do not forward `on_register_dispatch`, so hand the JSON layer
    // its dispatch explicitly.
    if let Some(slot) = slot {
        tracing::dispatcher::get_default(|d| slot.set(d));
    }
    Ok(guard)
}

/// The stdout (json) or stderr (pretty) layer for `format`, plus the dispatch slot the JSON
/// layer needs to find trace ids (empty for the pretty layer).
fn stdout_layer(format: LogFormat) -> (BoxedLayer, Option<DispatchSlot>) {
    match format {
        LogFormat::Json => {
            let layer = JsonLayer::new(std::io::stdout);
            let slot = layer.slot();
            (Box::new(layer), Some(slot))
        }
        LogFormat::Pretty => (
            Box::new(
                tracing_subscriber::fmt::layer()
                    .pretty()
                    .with_writer(std::io::stderr),
            ),
            None,
        ),
    }
}

/// The JSON line layer writing to `writer`. Public so tests and tools can target a buffer.
pub fn json_layer<S, W>(writer: W) -> impl Layer<S>
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
    W: for<'w> tracing_subscriber::fmt::MakeWriter<'w> + 'static,
{
    crate::json_format::JsonLayer::new(writer)
}

fn resource(config: &TelemetryConfig) -> Resource {
    let mut attrs = vec![
        KeyValue::new("service.version", config.service_version.clone()),
        KeyValue::new("service.instance.id", instance_id()),
        KeyValue::new("deployment.environment", config.environment.clone()),
    ];
    if let Some(sha) = &config.git_sha {
        attrs.push(KeyValue::new("git.sha", sha.clone()));
    }
    Resource::builder()
        .with_service_name(config.service_name.clone())
        .with_attributes(attrs)
        .build()
}

fn instance_id() -> String {
    let host = std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .ok()
        .or_else(|| {
            std::fs::read_to_string("/etc/hostname")
                .ok()
                .map(|s| s.trim().to_owned())
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_owned());
    format!("{host}-{}", std::process::id())
}

/// Helpers for tests in other crates.
pub mod testing {
    use std::sync::OnceLock;

    use super::*;

    static GUARD: OnceLock<Option<TelemetryGuard>> = OnceLock::new();

    /// Installs stdout-only telemetry once per process; safe to call from many tests.
    pub fn init_for_test() {
        let _ = GUARD.get_or_init(|| {
            let mut cfg = TelemetryConfig::new("test");
            cfg.otel_enabled = false;
            cfg.log_format = LogFormat::Pretty;
            init(cfg).ok()
        });
    }
}
