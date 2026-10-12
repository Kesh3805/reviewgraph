//! `telemetry::redact` (OBS-006): the second line of defense after "never log it".
//!
//! One pattern catalog ([`patterns`]), a string redactor ([`redact_str`]), a JSON value
//! redactor and the helpers the log formatter uses ([`layer`]). Replacement text is
//! `«redacted:<pattern>»`; redacting redacted text is a no-op. The same patterns and corpus
//! (`packages/contracts/redaction-corpus.json`) drive `apps/api/src/telemetry/redact.ts`.

pub mod layer;
pub mod otel;
mod patterns;
mod secret;

use std::borrow::Cow;

use serde_json::Value;

pub use patterns::{
    is_sensitive_key, patterns, tag, Pattern, MAX_FIELD_BYTES, SENSITIVE_KEYS, TRUNCATED,
};
pub use secret::{Redacted, SecretString, PLACEHOLDER};

/// Objects nested deeper than this are replaced wholesale.
pub const MAX_DEPTH: usize = 6;

/// Redacts every pattern match in `input`, then truncates to [`MAX_FIELD_BYTES`].
pub fn redact_str(input: &str) -> String {
    redact_counted(input).0
}

/// [`redact_str`] plus how many matches each pattern replaced (for `log_redactions_total`).
pub fn redact_counted(input: &str) -> (String, Vec<(&'static str, u64)>) {
    let mut out = Cow::Borrowed(input);
    let mut counts = Vec::new();
    for p in patterns() {
        let mut n = 0u64;
        let replaced = p.regex.replace_all(&out, |caps: &regex::Captures<'_>| {
            n += 1;
            match caps.name("secret") {
                Some(secret) => {
                    let whole = &caps[0];
                    let start = secret.start() - caps.get(0).map_or(0, |m| m.start());
                    let end = secret.end() - caps.get(0).map_or(0, |m| m.start());
                    format!("{}{}{}", &whole[..start], tag(p.name), &whole[end..])
                }
                None => tag(p.name),
            }
        });
        if n > 0 {
            counts.push((p.name, n));
            out = Cow::Owned(replaced.into_owned());
        }
    }
    (truncate(&out), counts)
}

/// Cuts `text` to [`MAX_FIELD_BYTES`] (on a char boundary) and marks it. Idempotent.
fn truncate(text: &str) -> String {
    let base = text.strip_suffix(TRUNCATED).unwrap_or(text);
    if base.len() <= MAX_FIELD_BYTES && base.len() == text.len() {
        return text.to_owned();
    }
    let mut cut = base.len().min(MAX_FIELD_BYTES);
    while !base.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}{TRUNCATED}", &base[..cut])
}

/// Redacts a JSON value: strings through [`redact_str`], object members whose key is
/// sensitive replaced regardless of value, nesting limited to [`MAX_DEPTH`].
pub fn redact_value(value: &Value) -> Value {
    redact_value_at(value, 0)
}

fn redact_value_at(value: &Value, depth: usize) -> Value {
    match value {
        Value::String(s) => Value::String(redact_str(s)),
        Value::Array(items) => {
            if depth >= MAX_DEPTH {
                return Value::String(tag("depth"));
            }
            Value::Array(
                items
                    .iter()
                    .map(|v| redact_value_at(v, depth + 1))
                    .collect(),
            )
        }
        Value::Object(map) => {
            if depth >= MAX_DEPTH {
                return Value::String(tag("depth"));
            }
            Value::Object(
                map.iter()
                    .map(|(k, v)| {
                        let v = if is_sensitive_key(k) {
                            Value::String(tag("sensitive_key"))
                        } else {
                            redact_value_at(v, depth + 1)
                        };
                        (k.clone(), v)
                    })
                    .collect(),
            )
        }
        other => other.clone(),
    }
}

/// Redacts one named field value: a sensitive name wins over the content.
pub fn redact_field(name: &str, value: &str) -> String {
    if is_sensitive_key(name) {
        tag("sensitive_key")
    } else {
        redact_str(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn userinfo_keeps_scheme_and_host() {
        assert_eq!(
            redact_str("clone https://user:s3cret@example.com/repo.git"),
            "clone https://«redacted:url_userinfo»@example.com/repo.git"
        );
    }

    #[test]
    fn truncation_is_idempotent_on_multibyte_boundaries() {
        let text = format!("{}{}", "a".repeat(4093), "é".repeat(20));
        let once = redact_str(&text);
        assert!(once.ends_with(TRUNCATED));
        assert_eq!(redact_str(&once), once);
    }

    #[test]
    fn deep_values_are_replaced() {
        let mut v = serde_json::json!("x");
        for _ in 0..10 {
            v = serde_json::json!({ "a": v });
        }
        assert!(redact_value(&v).to_string().contains("redacted:depth"));
    }
}
