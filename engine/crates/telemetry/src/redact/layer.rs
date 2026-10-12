//! The logging-path half of redaction: the field formatter helpers the JSON log layer applies
//! to every event and span field, and the fail-closed guard around one formatted line.

use std::error::Error;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::time::{Duration, Instant};

use serde_json::json;

use super::redact_counted;
use crate::redact::{is_sensitive_key, tag};

/// Redacting one event may take at most this long; past it the event is dropped (fail closed).
pub const REDACTION_BUDGET: Duration = Duration::from_millis(5);

/// The message of a line whose redaction failed.
pub const FAILED: &str = "«redaction-failed»";

/// Redacts one named field value and counts the matches in `log_redactions_total{pattern}`.
pub fn redact_field_counted(name: &str, value: &str) -> String {
    if is_sensitive_key(name) {
        count(&[("sensitive_key", 1)]);
        return tag("sensitive_key");
    }
    let (out, counts) = redact_counted(value);
    count(&counts);
    out
}

/// The text of `err` and every `source()` below it, redacted.
pub fn redact_error_chain(err: &(dyn Error + 'static)) -> String {
    let mut text = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    let (out, counts) = redact_counted(&text);
    count(&counts);
    out
}

fn count(counts: &[(&'static str, u64)]) {
    if counts.is_empty() {
        return;
    }
    let registry = crate::metrics::global();
    for &(pattern, n) in counts {
        registry.add("log_redactions_total", n, &[("pattern", pattern)]);
    }
}

/// Runs `build` (which formats and redacts one log line) and returns its output. If it panics,
/// returns `None`, or takes longer than `budget`, the original is discarded and a line holding
/// only the level and target and [`FAILED`] is returned instead: redaction never fails open.
pub fn fail_closed<F>(level: &str, target: &str, budget: Duration, build: F) -> String
where
    F: FnOnce() -> Option<String>,
{
    let start = Instant::now();
    match catch_unwind(AssertUnwindSafe(build)) {
        Ok(Some(line)) if start.elapsed() <= budget => line,
        _ => json!({ "level": level, "target": target, "message": FAILED }).to_string(),
    }
}
