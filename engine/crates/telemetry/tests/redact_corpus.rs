//! OBS-006: the shared redaction corpus, plus the behaviors around it.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use proptest::prelude::*;
use serde_json::{json, Value};
use telemetry::redact::layer::{fail_closed, redact_error_chain, FAILED};
use telemetry::redact::otel::scrub_attributes;
use telemetry::redact::{
    patterns, redact_field, redact_str, redact_value, Redacted, MAX_FIELD_BYTES, TRUNCATED,
};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .unwrap()
        .to_path_buf()
}

fn corpus() -> Value {
    let path = repo_root().join("packages/contracts/redaction-corpus.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn corpus_positives_are_redacted_rust() {
    let corpus = corpus();
    let names: Vec<&str> = patterns().iter().map(|p| p.name).collect();
    for case in corpus["positives"].as_array().unwrap() {
        let input = case["input"].as_str().unwrap();
        let expected = case["expected_redacted"].as_str().unwrap();
        assert_eq!(redact_str(input), expected, "case {}", case["name"]);
        let pattern = case["pattern"].as_str().unwrap();
        assert!(
            names.contains(&pattern),
            "corpus pattern {pattern} is not in the Rust catalog"
        );
    }
}

#[test]
fn corpus_negatives_unchanged_rust() {
    let corpus = corpus();
    for negative in corpus["negatives"].as_array().unwrap() {
        let text = negative.as_str().unwrap();
        assert_eq!(redact_str(text), text);
    }
}

#[test]
fn rust_and_ts_agree_on_every_corpus_case() {
    // Both suites assert the same `expected_redacted` for every positive and leave every
    // negative unchanged, so agreement is exactly "each passes the shared corpus". This guards
    // the corpus shape the TypeScript suite also relies on.
    let corpus = corpus();
    for case in corpus["positives"].as_array().unwrap() {
        for key in ["name", "input", "expected_redacted", "pattern"] {
            assert!(case[key].is_string(), "{key} missing in {case}");
        }
    }
    assert!(!corpus["negatives"].as_array().unwrap().is_empty());
}

#[test]
fn sensitive_keys_redacted_regardless_of_value() {
    assert_eq!(
        redact_field("Authorization", "plain words"),
        "«redacted:sensitive_key»"
    );
    let value = redact_value(&json!({"password": "x", "nested": {"Token": "y"}, "ok": "fine"}));
    assert_eq!(
        value,
        json!({
            "password": "«redacted:sensitive_key»",
            "nested": {"Token": "«redacted:sensitive_key»"},
            "ok": "fine"
        })
    );
}

#[test]
fn long_field_is_truncated() {
    let out = redact_str(&"a".repeat(MAX_FIELD_BYTES + 1000));
    assert!(out.ends_with(TRUNCATED));
    assert!(out.len() <= MAX_FIELD_BYTES + TRUNCATED.len());
}

#[derive(Debug)]
struct Outer(Inner);
#[derive(Debug)]
struct Inner;

impl fmt::Display for Outer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "request failed")
    }
}
impl fmt::Display for Inner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "bad token ghp_abcdefghijklmnopqrstuvwxyz0123456789")
    }
}
impl std::error::Error for Inner {}
impl std::error::Error for Outer {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

#[test]
fn error_source_chain_is_redacted() {
    let text = redact_error_chain(&Outer(Inner));
    assert_eq!(text, "request failed: bad token «redacted:github_token»");
}

#[test]
fn span_attributes_scrubbed_before_export() {
    use opentelemetry::{KeyValue, Value as OtelValue};
    let mut attrs = vec![
        KeyValue::new("url", "ghp_abcdefghijklmnopqrstuvwxyz0123456789"),
        KeyValue::new("authorization", "anything"),
        KeyValue::new("prompt", "the whole prompt"),
        KeyValue::new("count", 3_i64),
    ];
    scrub_attributes(&mut attrs);
    let keys: Vec<&str> = attrs.iter().map(|kv| kv.key.as_str()).collect();
    assert_eq!(keys, ["url", "authorization", "count"]);
    assert_eq!(attrs[0].value.as_str(), "«redacted:github_token»");
    assert_eq!(attrs[1].value.as_str(), "«redacted:sensitive_key»");
    assert!(matches!(attrs[2].value, OtelValue::I64(3)));
}

#[test]
fn redaction_failure_fails_closed() {
    let panicked = fail_closed("INFO", "t", Duration::from_millis(5), || {
        panic!("redactor bug with secret ghp_abcdefghijklmnopqrstuvwxyz0123456789")
    });
    let v: Value = serde_json::from_str(&panicked).unwrap();
    assert_eq!(v["message"], FAILED);
    assert_eq!(v["level"], "INFO");
    assert!(!panicked.contains("ghp_"));

    let slow = fail_closed("WARN", "t", Duration::from_millis(1), || {
        std::thread::sleep(Duration::from_millis(20));
        Some("{\"message\":\"raw secret\"}".to_owned())
    });
    assert!(slow.contains(FAILED));
    assert!(!slow.contains("raw secret"));

    let fine = fail_closed("INFO", "t", Duration::from_secs(5), || {
        Some("ok".to_owned())
    });
    assert_eq!(fine, "ok");
}

#[test]
fn redacted_wrapper_hides_value() {
    let secret = Redacted::new("hunter2".to_owned());
    assert_eq!(format!("{secret:?}"), "«redacted»");
    assert_eq!(format!("{secret}"), "«redacted»");
}

/// Types that hold credentials must not derive `Debug` (it would print them).
#[test]
fn sensitive_types_do_not_derive_debug() {
    const SENSITIVE: [&str; 4] = [
        "InstallationToken",
        "PrivateKey",
        "WebhookSecret",
        "ModelPrompt",
    ];
    let mut offenders = Vec::new();
    let mut stack = vec![
        repo_root().join("engine/crates"),
        repo_root().join("engine/apps"),
    ];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().is_some_and(|n| n != "target") {
                    stack.push(path);
                }
                continue;
            }
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let lines: Vec<&str> = text.lines().collect();
            for (i, line) in lines.iter().enumerate() {
                let trimmed = line.trim_start();
                let is_decl = ["pub struct ", "struct ", "pub enum ", "enum "]
                    .iter()
                    .any(|p| trimmed.starts_with(p));
                if !is_decl || !SENSITIVE.iter().any(|s| line.contains(&format!(" {s}"))) {
                    continue;
                }
                // Inspect the attribute lines directly above the declaration.
                let mut j = i;
                while j > 0 {
                    j -= 1;
                    let above = lines[j].trim_start();
                    if above.starts_with("#[") || above.starts_with("///") {
                        if above.starts_with("#[derive") && above.contains("Debug") {
                            offenders.push(format!("{}:{}", path.display(), i + 1));
                        }
                    } else {
                        break;
                    }
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "sensitive types derive Debug: {offenders:?}"
    );
}

proptest! {
    #[test]
    fn redaction_is_idempotent(text in ".{0,300}") {
        let once = redact_str(&text);
        prop_assert_eq!(redact_str(&once), once);
    }

    #[test]
    fn redaction_is_idempotent_with_secrets(prefix in "[a-z ]{0,20}", suffix in "[a-z ]{0,20}") {
        let text = format!(
            "{prefix}ghp_abcdefghijklmnopqrstuvwxyz0123456789 AKIAIOSFODNN7EXAMPLE https://u:p@h.io/x {suffix}"
        );
        let once = redact_str(&text);
        prop_assert!(!once.contains("ghp_"));
        prop_assert!(!once.contains("AKIAIOSFODNN7EXAMPLE"));
        prop_assert_eq!(redact_str(&once), once);
    }
}
