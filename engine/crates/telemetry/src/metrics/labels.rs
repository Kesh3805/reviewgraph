//! Label allow-list and bounded label values (OBS-005).
//!
//! Only the names in [`ALLOWED`] may label a metric; identifiers (organization, repository,
//! review run, commit, path) never may. Values are bounded: a closed set where the domain is
//! small, otherwise a short `[a-z0-9._:-]` token. Anything else becomes [`OTHER`] so a bad
//! value can neither blow up cardinality nor leak source-derived text.

use std::borrow::Cow;

/// The value used for anything outside a label's domain.
pub const OTHER: &str = "other";

/// Every label name a metric may use.
pub const ALLOWED: [&str; 17] = [
    "reviewer_type",
    "model",
    "provider",
    "task",
    "tier",
    "queue",
    "outcome",
    "stage",
    "language",
    "severity",
    "category",
    "state",
    "kind",
    "reason",
    "pattern",
    "op",
    "policy",
];

/// Label names that must never appear on a metric (cardinality and tenant leakage).
pub const FORBIDDEN: [&str; 9] = [
    "organization_id",
    "repository_id",
    "review_run_id",
    "pull_request_id",
    "commit_sha",
    "job_id",
    "request_id",
    "path",
    "file_path",
];

const MAX_OPEN_LEN: usize = 64;

const REVIEWER_TYPES: [&str; 6] = [
    "correctness",
    "security",
    "test",
    "architecture",
    "performance",
    "maintainability",
];
const QUEUES: [&str; 6] = [
    "repository-index",
    "incremental-index",
    "pr-review",
    "review-publish",
    "history-ingest",
    "retention-sweep",
];
const STATES: [&str; 6] = [
    "queued",
    "running",
    "succeeded",
    "failed",
    "dead",
    "cancelled",
];
const SEVERITIES: [&str; 5] = ["info", "low", "medium", "high", "critical"];
const OUTCOMES: [&str; 12] = [
    "ok",
    "error",
    "failed",
    "skipped",
    "superseded",
    "timeout",
    "denied",
    "success",
    "retry",
    "cache_hit",
    "cache_miss",
    "unavailable",
];

/// Whether `name` may label a metric.
pub fn is_allowed(name: &str) -> bool {
    ALLOWED.iter().any(|a| *a == name)
}

/// The closed value set of `name`, if it has one.
fn closed_set(name: &str) -> Option<&'static [&'static str]> {
    match name {
        "reviewer_type" => Some(&REVIEWER_TYPES),
        "queue" => Some(&QUEUES),
        "state" => Some(&STATES),
        "severity" => Some(&SEVERITIES),
        "outcome" => Some(&OUTCOMES),
        _ => None,
    }
}

/// Maps `value` into the domain of label `name`: closed-set labels accept only their members,
/// open labels accept short lowercase tokens; everything else is [`OTHER`].
pub fn bound<'a>(name: &str, value: &'a str) -> Cow<'a, str> {
    if let Some(set) = closed_set(name) {
        return if set.iter().any(|s| *s == value) {
            Cow::Borrowed(value)
        } else {
            Cow::Borrowed(OTHER)
        };
    }
    let token = !value.is_empty()
        && value.len() <= MAX_OPEN_LEN
        && value.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b':' | b'-')
        });
    if token {
        Cow::Borrowed(value)
    } else {
        Cow::Borrowed(OTHER)
    }
}
