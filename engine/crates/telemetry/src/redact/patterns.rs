//! The shared secret pattern catalog. Mirrors `apps/api/src/telemetry/redact.ts`; the two
//! implementations are kept in agreement by `packages/contracts/redaction-corpus.json`.
//!
//! All regexes are compiled once. The `regex` crate matches in linear time, so no pattern can
//! be turned into a ReDoS. A pattern with a named `secret` group redacts only that group (the
//! surrounding text, for example the URL scheme and host, is kept).

use std::sync::LazyLock;

use regex::Regex;

/// One named secret pattern.
#[derive(Debug)]
pub struct Pattern {
    pub name: &'static str,
    pub regex: Regex,
}

/// `(name, regex source)` in application order. Order matters: private key blocks go first so
/// later patterns cannot split them.
const SOURCES: &[(&str, &str)] = &[
    (
        "private_key",
        r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----",
    ),
    (
        "aws_access_key",
        r"(?-u:\b)(?:AKIA|ASIA)[0-9A-Z]{16}(?-u:\b)",
    ),
    (
        "github_token",
        r"(?-u:\b)(?:gh[pousr]_[A-Za-z0-9]{36,}|github_pat_[A-Za-z0-9_]{22,})(?-u:\b)",
    ),
    ("api_key", r"(?-u:\b)sk-(?:ant-)?[A-Za-z0-9_-]{20,}"),
    (
        "jwt",
        r"(?-u:\b)eyJ[A-Za-z0-9_-]{5,}\.eyJ[A-Za-z0-9_-]{5,}\.[A-Za-z0-9_-]{5,}",
    ),
    (
        "auth_header",
        r"(?i)(?:authorization\s*[:=]\s*)?(?-u:\b)(?:Bearer|Basic)\s+[A-Za-z0-9._~+/=-]{6,}",
    ),
    ("hub_signature", r"(?i)sha256=[0-9a-f]{64}(?-u:\b)"),
    (
        "url_userinfo",
        r"(?i)(?-u:\b)[a-z][a-z0-9+.-]*://(?P<secret>[^\s/@:]+:[^\s/@]+)@",
    ),
    (
        "env_assignment",
        r"(?-u:\b)[A-Z0-9_]*(?:KEY|SECRET|TOKEN|PASSWORD|PASSWD|PRIVATE)[A-Z0-9_]*\s*[=:]\s*\S+",
    ),
];

static PATTERNS: LazyLock<Vec<Pattern>> = LazyLock::new(|| {
    SOURCES
        .iter()
        .filter_map(|(name, src)| Regex::new(src).ok().map(|regex| Pattern { name, regex }))
        .collect()
});

/// The built-in patterns, in application order. Shared with the model gateway (GW-010).
pub fn patterns() -> &'static [Pattern] {
    &PATTERNS
}

/// Field keys (case-insensitive substrings) whose values are redacted regardless of content.
pub const SENSITIVE_KEYS: [&str; 6] = [
    "authorization",
    "cookie",
    "password",
    "token",
    "secret",
    "private_key",
];

/// Maximum bytes of one field before it is truncated.
pub const MAX_FIELD_BYTES: usize = 4096;

/// Appended to a truncated field.
pub const TRUNCATED: &str = "…[truncated]";

/// The log replacement for a match of `pattern`.
pub fn tag(pattern: &str) -> String {
    format!("«redacted:{pattern}»")
}

/// Whether `key` names a sensitive field.
pub fn is_sensitive_key(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    SENSITIVE_KEYS.iter().any(|s| k.contains(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pattern_compiles() {
        assert_eq!(patterns().len(), SOURCES.len());
    }
}
