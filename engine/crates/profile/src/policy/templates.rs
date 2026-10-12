//! Comment templates per rule key (POL-005).
//!
//! A template answers PRD §59 questions 1–3 deterministically (what changed, which rule, why
//! it matters); the corrective direction comes from the rule's `fix` field when present.
//! Placeholders are `{name}`; a missing parameter renders empty.

use std::collections::BTreeMap;

fn template(rule_key: &str) -> &'static str {
    match rule_key {
        "forbidden_dependency" => {
            "`{from_symbol}` ({from}) now depends on `{to_symbol}` ({to}) via {import}, which \
             `{rule_id}` forbids: {reason}"
        }
        "migrations_only" => {
            "{what} outside the migration paths; `{rule_id}` requires every schema change to \
             go through a migration ({migration_paths})"
        }
        "public_api_changes_require_tests" => {
            "The API contract of `{endpoint}` changed ({what}) but no test covering it was added \
             or changed in this pull request; `{rule_id}` requires tests for public API changes"
        }
        "queue_deterministic_job_id" => {
            "`{symbol}` enqueues a job {problem}; `{rule_id}` requires a deterministic jobId so \
             a retried enqueue does not run the job twice"
        }
        _ => "`{rule_id}` is violated: {reason}",
    }
}

/// Renders the message of a violation.
pub fn render(rule_key: &str, params: &BTreeMap<String, String>) -> String {
    let mut out = substitute(template(rule_key), params);
    if let Some(fix) = params.get("fix").filter(|f| !f.trim().is_empty()) {
        out.push_str(". Fix: ");
        out.push_str(fix.trim());
    }
    if !out.ends_with('.') {
        out.push('.');
    }
    out
}

fn substitute(template: &str, params: &BTreeMap<String, String>) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find('}') {
            Some(close) => {
                let name = &after[..close];
                if let Some(value) = params.get(name) {
                    out.push_str(value.trim_end_matches('.'));
                }
                rest = &after[close + 1..];
            }
            None => {
                out.push_str(&rest[open..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}
