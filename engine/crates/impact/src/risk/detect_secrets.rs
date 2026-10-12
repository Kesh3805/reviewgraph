//! Secret-like literals in added lines and committed environment files (RISK-002).
//!
//! Only **added** lines of text files are scanned, with linear-time regexes. The matched value
//! never leaves [`scan_line`]: signals carry the file, the line, the rule name and a redacted
//! preview of at most four visible characters.

use std::sync::OnceLock;

use rayon::prelude::*;
use regex::Regex;
use review_core::location::RepoPath;

use super::new_signal;
use super::signal::{dedup_signals, RiskSignal, SignalEvidence, SignalSubject};
use super::RiskWarning;
use crate::input::{AddedLine, ChangeSet, FileDisposition, FileInput};
use crate::metrics;
use review_core::change::FileChangeStatus;

/// Entropy (bits per character) at or above which a long quoted string is flagged.
const ENTROPY_THRESHOLD: f64 = 4.0;
/// Shortest quoted string considered by the entropy detector.
const ENTROPY_MIN_LEN: usize = 32;

/// `(rule name, pattern)`. The entropy detector is separate.
const PATTERNS: &[(&str, &str)] = &[
    ("aws_access_key", r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b"),
    ("vcs_token", r"\bgh[pousr]_[A-Za-z0-9]{36,}\b"),
    ("private_key", r"-----BEGIN (?:[A-Z]+ )?PRIVATE KEY-----"),
    (
        "generic_assignment",
        r#"(?i)(?:secret|token|password|api[_-]?key)\s*[:=]\s*['"][^'"]{12,}['"]"#,
    ),
];
const ENTROPY_PATTERN: &str = r#"[:=]\s*['"]([A-Za-z0-9+/=_\-]{32,})['"]"#;

struct Compiled {
    rules: Vec<(&'static str, Regex)>,
    entropy: Option<Regex>,
}

fn compiled() -> &'static Compiled {
    static COMPILED: OnceLock<Compiled> = OnceLock::new();
    COMPILED.get_or_init(|| Compiled {
        rules: PATTERNS
            .iter()
            .filter_map(|&(name, pattern)| Regex::new(pattern).ok().map(|regex| (name, regex)))
            .collect(),
        entropy: Regex::new(ENTROPY_PATTERN).ok(),
    })
}

/// Shannon entropy of `text` in bits per byte.
fn entropy(text: &str) -> f64 {
    let mut counts = [0u32; 256];
    for byte in text.bytes() {
        counts[usize::from(byte)] += 1;
    }
    let len = text.len() as f64;
    if len == 0.0 {
        return 0.0;
    }
    counts
        .iter()
        .filter(|&&count| count > 0)
        .map(|&count| {
            let p = f64::from(count) / len;
            -p * p.log2()
        })
        .sum()
}

/// At most four visible characters followed by `****`.
fn redact(matched: &str) -> String {
    let head: String = matched.chars().take(4).collect();
    format!("{head}****")
}

fn is_placeholder(matched: &str) -> bool {
    matched.contains("${") || matched.contains("process.env")
}

/// A secret-like literal found in one line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretFinding {
    pub rule: &'static str,
    pub preview: String,
}

/// Scans one added line; the first matching rule wins.
pub fn scan_line(text: &str) -> Option<SecretFinding> {
    let compiled = compiled();
    for (name, regex) in &compiled.rules {
        if let Some(found) = regex.find(text) {
            if !is_placeholder(found.as_str()) {
                return Some(SecretFinding {
                    rule: name,
                    preview: redact(found.as_str()),
                });
            }
        }
    }
    let entropy_regex = compiled.entropy.as_ref()?;
    let captures = entropy_regex.captures(text)?;
    let value = captures.get(1)?.as_str();
    if value.len() >= ENTROPY_MIN_LEN && entropy(value) >= ENTROPY_THRESHOLD {
        return Some(SecretFinding {
            rule: "high_entropy",
            preview: redact(value),
        });
    }
    None
}

fn file_name(path: &RepoPath) -> &str {
    path.as_str().rsplit('/').next().unwrap_or("")
}

/// `.env`, `.env.local`, `.env.production`, but not `.env.example` and friends.
fn is_env_file(path: &RepoPath) -> bool {
    let name = file_name(path);
    name.starts_with(".env") && !is_example_env(name)
}

fn is_example_env(name: &str) -> bool {
    name.starts_with(".env")
        && [".example", ".sample", ".template", ".dist", ".defaults"]
            .iter()
            .any(|suffix| name.ends_with(suffix))
}

/// The outcome of the secrets scan.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SecretDetection {
    pub signals: Vec<RiskSignal>,
    pub warnings: Vec<RiskWarning>,
}

fn scan_file(file: &FileInput) -> SecretDetection {
    let mut out = SecretDetection::default();
    if file.status == FileChangeStatus::Deleted
        || matches!(
            file.disposition,
            FileDisposition::Generated
                | FileDisposition::Vendored
                | FileDisposition::Binary
                | FileDisposition::Minified
        )
    {
        return out;
    }
    let subject = SignalSubject::File(file.path.clone());
    if is_env_file(&file.path) {
        out.signals.extend(new_signal(
            "secrets.env_file",
            subject.clone(),
            1.0,
            vec![SignalEvidence {
                file: Some(file.path.clone()),
                detail: "environment file committed".to_owned(),
                ..SignalEvidence::default()
            }],
        ));
    }
    if is_example_env(file_name(&file.path)) {
        return out;
    }
    let Some(lines) = &file.added_lines else {
        out.warnings.push(RiskWarning {
            code: "secrets_scan_incomplete",
            path: Some(file.path.clone()),
        });
        return out;
    };
    for AddedLine { line, text } in lines {
        if let Some(finding) = scan_line(text) {
            metrics::record_secret_detection(finding.rule);
            out.signals.extend(new_signal(
                "secrets.literal_added",
                subject.clone(),
                1.0,
                vec![SignalEvidence {
                    file: Some(file.path.clone()),
                    line: Some(*line),
                    detail: format!("secret-like literal ({})", finding.rule),
                    redacted_preview: Some(finding.preview),
                    ..SignalEvidence::default()
                }],
            ));
        }
    }
    out
}

/// Scans the added lines of every changed file, in parallel; the output order is that of the
/// files.
pub fn detect_secret_signals(changes: &ChangeSet) -> SecretDetection {
    let per_file: Vec<SecretDetection> = changes.files.par_iter().map(scan_file).collect();
    let mut signals = Vec::new();
    let mut warnings = Vec::new();
    for detection in per_file {
        signals.extend(detection.signals);
        warnings.extend(detection.warnings);
    }
    SecretDetection {
        signals: dedup_signals(signals),
        warnings,
    }
}
