//! Redaction for the OTLP export path: span attributes are scrubbed before export and log
//! record bodies are redacted before they reach the batch processor.
//!
//! Spans never carry source, diff or prompt content, so attribute keys named `prompt`,
//! `source`, `diff` or `body` are dropped outright as a second line of defense (OBS-004).

use std::time::Duration;

use opentelemetry::logs::{AnyValue, LogRecord};
use opentelemetry::trace::Status;
use opentelemetry::{InstrumentationScope, KeyValue, Value};
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::logs::{LogProcessor, SdkLogRecord};
use opentelemetry_sdk::trace::SpanData;

use super::layer::redact_field_counted;
use crate::spans::is_forbidden_attr;

/// Scrubs a span attribute list in place: forbidden keys are removed, sensitive keys and
/// secret-looking string values are replaced.
pub fn scrub_attributes(attributes: &mut Vec<KeyValue>) {
    attributes.retain(|kv| !is_forbidden_attr(kv.key.as_str()));
    for kv in attributes.iter_mut() {
        if let Value::String(s) = &kv.value {
            let cleaned = redact_field_counted(kv.key.as_str(), s.as_str());
            if cleaned != s.as_str() {
                kv.value = Value::String(cleaned.into());
            }
        } else if super::is_sensitive_key(kv.key.as_str()) {
            kv.value = Value::String(super::tag("sensitive_key").into());
        }
    }
}

/// Scrubs one finished span: attributes, event attributes and the error description.
pub fn scrub_span(span: &mut SpanData) {
    scrub_attributes(&mut span.attributes);
    for event in span.events.events.iter_mut() {
        scrub_attributes(&mut event.attributes);
    }
    if let Status::Error { description } = &span.status {
        let cleaned = redact_field_counted("status", description);
        if cleaned.as_str() != &**description {
            span.status = Status::error(cleaned);
        }
    }
}

/// A log processor that redacts the record body. It must be registered before the exporting
/// processor so the batch processor only ever sees redacted text.
#[derive(Debug, Default)]
pub struct RedactingLogProcessor;

impl LogProcessor for RedactingLogProcessor {
    fn emit(&self, data: &mut SdkLogRecord, _instrumentation: &InstrumentationScope) {
        let cleaned = match data.body() {
            Some(AnyValue::String(s)) => Some(redact_field_counted("message", s.as_str())),
            _ => None,
        };
        if let Some(text) = cleaned {
            data.set_body(AnyValue::String(text.into()));
        }
    }

    fn force_flush(&self) -> OTelSdkResult {
        Ok(())
    }

    fn shutdown_with_timeout(&self, _timeout: Duration) -> OTelSdkResult {
        Ok(())
    }
}
