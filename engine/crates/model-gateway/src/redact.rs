//! Secret patterns, text scrubbing and the mandatory pre-send redactor (GW-010).
//!
//! Used for provider error details (GW-002) and by the pre-send redactor. Each match becomes
//! `«redacted:<pattern>:<blake3[..8]>»`, so the same secret always gives the same placeholder (the
//! request hash stays stable) while 8 hex characters cannot be brute-forced back to the secret.
//!
//! The gateway runs a [`PreSendRedactor`] over every [`StructuredInput`] before hashing, caching
//! or sending it; there is no gateway without one (see `GatewayBuilder::build`).

use std::collections::BTreeMap;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use crate::types::StructuredInput;

/// One secret pattern.
#[derive(Debug)]
pub struct SecretPattern {
    pub name: &'static str,
    pub regex: Regex,
}

fn pattern(name: &'static str, re: &str) -> Option<SecretPattern> {
    Regex::new(re)
        .ok()
        .map(|regex| SecretPattern { name, regex })
}

static PATTERNS: LazyLock<Vec<SecretPattern>> = LazyLock::new(|| {
    [
        pattern(
            "pem_private_key",
            r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?(?:-----END [A-Z ]*PRIVATE KEY-----|\z)",
        ),
        pattern("aws_access_key_id", r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b"),
        pattern("github_token", r"\bgh[pousr]_[A-Za-z0-9]{20,}\b"),
        pattern("github_pat", r"\bgithub_pat_[A-Za-z0-9_]{20,}\b"),
        pattern("api_key", r"\bsk-[A-Za-z0-9_-]{20,}\b"),
        pattern(
            "jwt",
            r"\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\b",
        ),
        pattern(
            "authorization_header",
            r#"(?i)\bauthorization\s*[:=]\s*(?:bearer\s+|basic\s+|token\s+)?[^\s"',;]+"#,
        ),
        pattern(
            "env_assignment",
            r#"(?im)^[ \t]*(?:export[ \t]+)?[A-Z0-9_]*(?:KEY|SECRET|TOKEN|PASSWORD)[A-Z0-9_]*[ \t]*=[ \t]*[^\s#]+"#,
        ),
    ]
    .into_iter()
    .flatten()
    .collect()
});

/// The built-in secret patterns, in application order.
pub fn patterns() -> &'static [SecretPattern] {
    &PATTERNS
}

/// The placeholder for a matched secret.
pub fn placeholder(pattern: &str, secret: &str) -> String {
    let h = blake3::hash(secret.as_bytes()).to_hex();
    format!("«redacted:{pattern}:{}»", &h.as_str()[..8])
}

/// Replaces every secret in `text`. Returns the new text and per-pattern replacement counts.
pub fn redact_str(text: &str) -> (String, BTreeMap<&'static str, u32>) {
    let mut out = text.to_owned();
    let mut counts: BTreeMap<&'static str, u32> = BTreeMap::new();
    for p in patterns() {
        let mut n = 0u32;
        let replaced = p
            .regex
            .replace_all(&out, |caps: &regex::Captures<'_>| {
                n += 1;
                placeholder(p.name, &caps[0])
            })
            .into_owned();
        if n > 0 {
            *counts.entry(p.name).or_default() += n;
            out = replaced;
        }
    }
    (out, counts)
}

/// Replaces obvious secrets in free text (error details, logs).
pub fn scrub_text(text: &str) -> String {
    redact_str(text).0
}

/// Pattern name used for index-time secret fingerprints (SEC-003).
pub const KNOWN_SECRET: &str = "known_secret";
const PEM: &str = "pem_private_key";
/// Shorter literals are ignored: they would redact ordinary words.
const MIN_KNOWN_SECRET_CHARS: usize = 8;

/// What a redaction pass did. Never contains the secrets themselves.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RedactionReport {
    pub replacements: u32,
    pub by_pattern: BTreeMap<&'static str, u32>,
    /// The call must not be sent (a private key inside system context means a template bug).
    pub blocked: bool,
}

impl RedactionReport {
    fn merge(&mut self, counts: &BTreeMap<&'static str, u32>) {
        for (&k, &v) in counts {
            *self.by_pattern.entry(k).or_default() += v;
            self.replacements = self.replacements.saturating_add(v);
        }
    }
}

/// Rewrites a structured input in place so that no secret leaves the process. Must be pure and
/// deterministic: the request hash is computed from its output.
pub trait PreSendRedactor: Send + Sync {
    fn redact(&self, input: &mut StructuredInput) -> RedactionReport;
}

/// The default redactor: the built-in [`patterns`] plus optional known secret literals from the
/// index (SEC-003 fingerprints).
#[derive(Debug, Clone, Default)]
pub struct DefaultRedactor {
    known_secrets: Vec<String>,
}

impl DefaultRedactor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds exact secret literals detected at index time. Literals shorter than 8 characters are
    /// ignored.
    pub fn with_known_secrets(mut self, secrets: impl IntoIterator<Item = String>) -> Self {
        self.known_secrets.extend(
            secrets
                .into_iter()
                .filter(|s| s.chars().count() >= MIN_KNOWN_SECRET_CHARS),
        );
        // Longest first, so a secret that contains another is replaced whole.
        self.known_secrets
            .sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
        self.known_secrets.dedup();
        self
    }

    /// Redacts one string with the built-in patterns and the known secrets.
    pub fn redact_text(&self, text: &str) -> (String, BTreeMap<&'static str, u32>) {
        let (mut out, mut counts) = redact_str(text);
        for secret in &self.known_secrets {
            let n = out.matches(secret.as_str()).count();
            if n > 0 {
                out = out.replace(secret.as_str(), &placeholder(KNOWN_SECRET, secret));
                *counts.entry(KNOWN_SECRET).or_default() += u32::try_from(n).unwrap_or(u32::MAX);
            }
        }
        (out, counts)
    }

    /// Redacts every string inside `value`. Returns whether a private key was found.
    fn redact_value(&self, value: &mut Value, report: &mut RedactionReport) -> bool {
        match value {
            Value::String(s) => {
                let (out, counts) = self.redact_text(s);
                if counts.is_empty() {
                    return false;
                }
                *s = out;
                report.merge(&counts);
                counts.contains_key(PEM)
            }
            Value::Array(items) => {
                let mut pem = false;
                for item in items {
                    pem |= self.redact_value(item, report);
                }
                pem
            }
            Value::Object(map) => {
                let mut pem = false;
                for item in map.values_mut() {
                    pem |= self.redact_value(item, report);
                }
                pem
            }
            Value::Null | Value::Bool(_) | Value::Number(_) => false,
        }
    }
}

impl PreSendRedactor for DefaultRedactor {
    fn redact(&self, input: &mut StructuredInput) -> RedactionReport {
        let mut report = RedactionReport::default();
        // The system prompt is a static template: any secret in it is a template bug.
        let (system, counts) = self.redact_text(&input.system.text);
        if !counts.is_empty() {
            report.merge(&counts);
            report.blocked = true;
            input.system.text = system.into();
        }
        for section in &mut input.sections {
            let pem = self.redact_value(&mut section.content, &mut report);
            if pem && section.cache_breakpoint {
                report.blocked = true;
            }
        }
        if let Some(repair) = &mut input.repair {
            self.redact_value(&mut repair.previous_output, &mut report);
        }
        report
    }
}

/// Leaves the input untouched. Test builds only (the `testing` feature exposes it to the
/// integration tests of this and dependent crates).
#[cfg(any(test, feature = "testing"))]
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopRedactor;

#[cfg(any(test, feature = "testing"))]
impl PreSendRedactor for NoopRedactor {
    fn redact(&self, _input: &mut StructuredInput) -> RedactionReport {
        RedactionReport::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrubs_common_secrets() {
        let t = "token ghp_abcdefghijklmnopqrstuvwxyz0123 and AKIAABCDEFGHIJKLMNOP\nDB_PASSWORD=hunter2\n";
        let s = scrub_text(t);
        assert!(!s.contains("ghp_abc"), "{s}");
        assert!(!s.contains("AKIAABCD"), "{s}");
        assert!(!s.contains("hunter2"), "{s}");
        assert!(s.contains("«redacted:github_token:"));
    }

    #[test]
    fn same_secret_same_placeholder() {
        assert_eq!(placeholder("x", "abc"), placeholder("x", "abc"));
        assert_ne!(placeholder("x", "abc"), placeholder("x", "abd"));
    }

    #[test]
    fn all_patterns_compile() {
        assert_eq!(patterns().len(), 8);
    }
}
