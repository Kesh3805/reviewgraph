//! OBS-006: the JSON log layer never emits a raw secret (event message, fields, span fields,
//! error source chains).

#![allow(clippy::unwrap_used)]

use std::io;
use std::sync::{Arc, Mutex};

use serde_json::Value;
use telemetry::init::json_layer;
use tracing_subscriber::layer::SubscriberExt;

const TOKEN: &str = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";

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

fn capture(f: impl FnOnce()) -> String {
    let buf = Buf::default();
    let writer = buf.clone();
    let subscriber = tracing_subscriber::registry().with(json_layer(move || writer.clone()));
    tracing::subscriber::with_default(subscriber, f);
    let bytes = buf.0.lock().unwrap().clone();
    String::from_utf8(bytes).unwrap()
}

#[derive(Debug)]
struct Failure;
impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "failed with {TOKEN}")
    }
}
impl std::error::Error for Failure {}

#[test]
fn message_and_fields_never_contain_the_raw_token() {
    let out = capture(|| {
        tracing::info!(detail = %format!("saw {TOKEN}"), password = "hunter2", count = 3, "leaked {TOKEN}");
    });
    assert!(!out.contains(TOKEN), "raw token in {out}");
    assert!(!out.contains("hunter2"));
    let line: Value = serde_json::from_str(out.lines().next().unwrap()).unwrap();
    assert_eq!(line["message"], "leaked «redacted:github_token»");
    assert_eq!(line["fields"]["detail"], "saw «redacted:github_token»");
    assert_eq!(line["fields"]["password"], "«redacted:sensitive_key»");
    assert_eq!(line["fields"]["count"], 3);
}

#[test]
fn span_fields_are_redacted() {
    let out = capture(|| {
        let span =
            tracing::info_span!("work", url = %format!("https://x/?t={TOKEN}"), token = "abc");
        let _guard = span.enter();
        tracing::info!("inside");
    });
    assert!(!out.contains(TOKEN), "raw token in {out}");
    assert!(!out.contains("\"abc\""));
}

#[test]
fn error_fields_redact_the_source_chain() {
    let out = capture(|| {
        let err = Failure;
        tracing::error!(error = &err as &(dyn std::error::Error + 'static), "boom");
    });
    assert!(!out.contains(TOKEN), "raw token in {out}");
    assert!(out.contains("«redacted:github_token»"));
}

#[test]
fn long_fields_are_truncated() {
    let out = capture(|| {
        tracing::info!(blob = %"x".repeat(10_000), "big");
    });
    assert!(out.len() < 6_000, "line was {} bytes", out.len());
    assert!(out.contains("…[truncated]"));
}
