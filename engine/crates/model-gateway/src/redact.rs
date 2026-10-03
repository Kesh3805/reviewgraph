//! Secret patterns and text scrubbing.
//!
//! Used for provider error details (GW-002) and by the pre-send redactor (GW-010). Each match
//! becomes `«redacted:<pattern>:<blake3[..8]>»`, so the same secret always gives the same
//! placeholder (the request hash stays stable) while 8 hex characters cannot be brute-forced
//! back to the secret.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use regex::Regex;

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
