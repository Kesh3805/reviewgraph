//! JSON log line format: `{timestamp, level, target, message, fields, span, spans, trace_id,
//! span_id}` plus the correlation attributes flattened to the top level.

use std::fmt;
use std::sync::{Arc, Mutex};

use opentelemetry::trace::TraceContextExt;
use serde_json::{Map, Value};
use tracing::dispatcher::WeakDispatch;
use tracing::field::{Field, Visit};
use tracing::{Dispatch, Event, Subscriber};
use tracing_opentelemetry::get_otel_context;
use tracing_subscriber::fmt::format::{JsonFields, Writer};
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormattedFields};
use tracing_subscriber::registry::LookupSpan;

use crate::attrs::CORRELATION_KEYS;

/// Shared slot holding a weak handle to the subscriber this layer is installed in. Inside a
/// subscriber callback `tracing::Span::current()` is unavailable (re-entrancy guard), so the
/// OpenTelemetry context of the current span is looked up through this dispatch instead.
#[derive(Debug, Clone, Default)]
pub struct DispatchSlot(Arc<Mutex<Option<WeakDispatch>>>);

impl DispatchSlot {
    pub(crate) fn set(&self, dispatch: &Dispatch) {
        if let Ok(mut slot) = self.0.lock() {
            *slot = Some(dispatch.downgrade());
        }
    }

    fn get(&self) -> Option<Dispatch> {
        self.0.lock().ok()?.as_ref()?.upgrade()
    }
}

/// The event formatter installed for `RG_LOG_FORMAT=json`.
#[derive(Debug, Clone, Default)]
pub struct JsonLines {
    dispatch: DispatchSlot,
}

impl JsonLines {
    pub fn new(dispatch: DispatchSlot) -> Self {
        Self { dispatch }
    }
}

/// `fmt::Layer` wrapper that records the subscriber dispatch for [`JsonLines`].
#[derive(Debug)]
pub struct JsonLayer<S, W> {
    inner: tracing_subscriber::fmt::Layer<S, JsonFields, JsonLines, W>,
    slot: DispatchSlot,
}

impl<S, W> JsonLayer<S, W>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    W: for<'w> tracing_subscriber::fmt::MakeWriter<'w> + 'static,
{
    pub(crate) fn slot(&self) -> DispatchSlot {
        self.slot.clone()
    }

    pub fn new(writer: W) -> Self {
        let slot = DispatchSlot::default();
        let inner = tracing_subscriber::fmt::layer()
            .fmt_fields(JsonFields::new())
            .event_format(JsonLines::new(slot.clone()))
            .with_writer(writer);
        Self { inner, slot }
    }
}

impl<S, W> tracing_subscriber::Layer<S> for JsonLayer<S, W>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    W: for<'w> tracing_subscriber::fmt::MakeWriter<'w> + 'static,
{
    fn on_register_dispatch(&self, subscriber: &Dispatch) {
        self.slot.set(subscriber);
        self.inner.on_register_dispatch(subscriber);
    }

    fn on_layer(&mut self, subscriber: &mut S) {
        self.inner.on_layer(subscriber);
    }

    fn on_new_span(
        &self,
        attrs: &tracing::span::Attributes<'_>,
        id: &tracing::span::Id,
        ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        self.inner.on_new_span(attrs, id, ctx);
    }

    fn on_record(
        &self,
        id: &tracing::span::Id,
        values: &tracing::span::Record<'_>,
        ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        self.inner.on_record(id, values, ctx);
    }

    fn on_event(&self, event: &Event<'_>, ctx: tracing_subscriber::layer::Context<'_, S>) {
        self.inner.on_event(event, ctx);
    }

    fn on_close(&self, id: tracing::span::Id, ctx: tracing_subscriber::layer::Context<'_, S>) {
        self.inner.on_close(id, ctx);
    }
}

#[derive(Default)]
struct FieldCollector {
    message: Option<String>,
    fields: Map<String, Value>,
}

impl Visit for FieldCollector {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.message = Some(format!("{value:?}"));
        } else {
            self.fields
                .insert(field.name().to_owned(), Value::String(format!("{value:?}")));
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_owned());
        } else {
            self.fields
                .insert(field.name().to_owned(), Value::String(value.to_owned()));
        }
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.fields.insert(field.name().to_owned(), value.into());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.fields.insert(field.name().to_owned(), value.into());
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.fields.insert(field.name().to_owned(), value.into());
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        if let Some(n) = serde_json::Number::from_f64(value) {
            self.fields
                .insert(field.name().to_owned(), Value::Number(n));
        }
    }
}

impl<S> FormatEvent<S, JsonFields> for JsonLines
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, JsonFields>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let meta = event.metadata();
        let mut collector = FieldCollector::default();
        event.record(&mut collector);

        let mut line = Map::new();
        line.insert(
            "timestamp".into(),
            chrono::Utc::now()
                .to_rfc3339_opts(chrono::SecondsFormat::Micros, true)
                .into(),
        );
        line.insert("level".into(), meta.level().as_str().into());
        line.insert("target".into(), meta.target().into());
        line.insert(
            "message".into(),
            collector.message.take().unwrap_or_default().into(),
        );
        if !collector.fields.is_empty() {
            line.insert("fields".into(), Value::Object(collector.fields));
        }

        let mut spans = Vec::new();
        let mut flat = Map::new();
        if let Some(scope) = ctx.event_scope() {
            // Root first, so inner spans override outer correlation values.
            for span in scope.from_root() {
                let mut obj = Map::new();
                obj.insert("name".into(), span.name().into());
                if let Some(fields) = span.extensions().get::<FormattedFields<JsonFields>>() {
                    if let Ok(Value::Object(m)) = serde_json::from_str::<Value>(fields.as_str()) {
                        for (k, v) in m {
                            if CORRELATION_KEYS.contains(&k.as_str()) {
                                flat.insert(k.clone(), v.clone());
                            }
                            obj.insert(k, v);
                        }
                    }
                }
                spans.push(Value::Object(obj));
            }
        }
        if let Some(Value::Object(current)) = spans.last() {
            line.insert("span".into(), Value::Object(current.clone()));
        }
        line.insert("spans".into(), Value::Array(spans));
        for (k, v) in flat {
            line.insert(k, v);
        }

        let current = event
            .parent()
            .cloned()
            .or_else(|| ctx.lookup_current().map(|s| s.id()));
        if let (Some(id), Some(dispatch)) = (current, self.dispatch.get()) {
            if let Some(otel_cx) = get_otel_context(&id, &dispatch) {
                let span_ref = otel_cx.span();
                let sc = span_ref.span_context();
                if sc.is_valid() {
                    line.insert("trace_id".into(), sc.trace_id().to_string().into());
                    line.insert("span_id".into(), sc.span_id().to_string().into());
                }
            }
        }

        let text = serde_json::to_string(&Value::Object(line)).map_err(|_| fmt::Error)?;
        writeln!(writer, "{text}")
    }
}
