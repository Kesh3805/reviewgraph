//! Secret scrubbing for text that leaves the process (embedding inputs, SEM-001) or becomes an
//! embedding unit (SEM-006).
//!
//! Each match becomes `«redacted:<pattern>:<blake3[..8]>»`: the same secret always gives the same
//! placeholder, so content hashes stay stable, while 8 hex characters cannot be reversed. The
//! pattern set mirrors the model gateway's (GW-010); `semantic` may not depend on the gateway.

use std::borrow::Cow;
use std::sync::LazyLock;

use regex::Regex;

#[derive(Debug)]
struct SecretPattern {
    name: &'static str,
    regex: Regex,
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
        pattern(
            "secret_literal",
            r#"(?i)\b[a-z0-9_]*(?:password|passwd|secret|api_?key|access_?token|private_?key)[a-z0-9_]*\s*[:=]\s*["'`][^"'`\s]{6,}["'`]"#,
        ),
    ]
    .into_iter()
    .flatten()
    .collect()
});

fn placeholder(pattern: &str, secret: &str) -> String {
    let digest = blake3::hash(secret.as_bytes());
    format!("«redacted:{pattern}:{}»", &digest.to_hex()[..8])
}

/// Replaces every secret-looking substring. Borrowed when nothing matched.
pub fn redact(text: &str) -> Cow<'_, str> {
    let mut out: Cow<'_, str> = Cow::Borrowed(text);
    for p in PATTERNS.iter() {
        if p.regex.is_match(&out) {
            let replaced = p
                .regex
                .replace_all(&out, |caps: &regex::Captures<'_>| {
                    placeholder(p.name, caps.get(0).map_or("", |m| m.as_str()))
                })
                .into_owned();
            out = Cow::Owned(replaced);
        }
    }
    out
}

/// Whether `text` contains anything [`redact`] would replace.
pub fn contains_secret(text: &str) -> bool {
    PATTERNS.iter().any(|p| p.regex.is_match(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_text_is_borrowed() {
        assert!(matches!(redact("fn authorize(user)"), Cow::Borrowed(_)));
    }

    #[test]
    fn known_secrets_are_replaced_stably() {
        let text = "const key = \"AKIAABCDEFGHIJKLMNOP\";";
        let a = redact(text).into_owned();
        assert!(!a.contains("AKIAABCDEFGHIJKLMNOP"));
        assert!(a.contains("«redacted:"));
        assert_eq!(a, redact(text).into_owned());
    }

    #[test]
    fn password_literal_is_replaced() {
        let out = redact("const dbPassword = 'hunter2hunter2';").into_owned();
        assert!(!out.contains("hunter2hunter2"), "{out}");
        assert!(contains_secret("api_key: \"abcdef123456\""));
    }
}
