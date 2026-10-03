//! Pure classification of provider failures into [`GatewayError`] (GW-002).
//!
//! Permanent rules are checked first, so a body such as "quota exceeded" on an HTTP 429 is
//! never retried. (The legacy agent runner learned this the hard way: `is_transient` must not
//! win over quota and auth words, and unknown errors are not retried.)
//!
//! | Signal | Class |
//! |---|---|
//! | body mentions `insufficient_quota`, `quota`, `billing`, `credit balance` | `Permanent(QuotaExhausted)`, even on 429 |
//! | 413, or `prompt is too long` / `context_length_exceeded` | `Permanent(ContextTooLarge)` |
//! | 404, or message matching `model.*not found` | `Permanent(ModelNotFound)` |
//! | 401 / 403 | `Permanent(Auth)` / `Permanent(Forbidden)` |
//! | 400 `invalid_request_error` | `Permanent(InvalidRequest)`; `UnsupportedParameter` if a parameter is named |
//! | 429 (not quota) | `RateLimited { retry_after = Retry-After (secs or HTTP-date) }` |
//! | 529 (`overloaded_error`) | `Transient(Overloaded)` |
//! | 408, 500, 502, 503, 504 | `Transient(ServerError)` |
//! | transport connect / timeout / body read | `Transient(Connect / Timeout / TruncatedBody)` |
//! | anything else | `Permanent(Unknown)`, not retried |

use std::time::{Duration, SystemTime};

use crate::error::{GatewayError, PermanentKind, RateScope, TransientKind};
use crate::types::ProviderId;

/// Maximum characters of provider error text kept in `detail`.
pub const MAX_DETAIL_CHARS: usize = 500;

/// A transport-level failure before any HTTP status was seen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportFailure {
    Connect,
    Timeout,
    Reset,
    BodyRead,
}

/// Truncates provider text to [`MAX_DETAIL_CHARS`] and scrubs secrets. Provider error bodies can
/// echo prompt fragments, so nothing is stored verbatim.
pub fn sanitize_detail(text: &str) -> String {
    let scrubbed = crate::redact::scrub_text(text);
    scrubbed.chars().take(MAX_DETAIL_CHARS).collect()
}

/// Parses a `Retry-After` header: delta seconds or an HTTP-date (relative to `now`).
pub fn parse_retry_after(value: &str, now: SystemTime) -> Option<Duration> {
    let v = value.trim();
    if let Ok(secs) = v.parse::<u64>() {
        return Some(Duration::from_secs(secs));
    }
    let when = httpdate::parse_http_date(v).ok()?;
    Some(when.duration_since(now).unwrap_or(Duration::ZERO))
}

/// Pulls `(type, code, message)` out of `{error:{type,code,message}}` bodies; falls back to the
/// raw text as the message.
fn error_fields(body: &str) -> (String, String, String) {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(body) {
        let e = v.get("error").unwrap_or(&v);
        let s = |k: &str| {
            e.get(k)
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let (t, c, m) = (s("type"), s("code"), s("message"));
        if !(t.is_empty() && c.is_empty() && m.is_empty()) {
            return (t, c, m);
        }
    }
    (String::new(), String::new(), body.to_owned())
}

fn permanent(
    kind: PermanentKind,
    provider: &ProviderId,
    status: u16,
    message: &str,
) -> GatewayError {
    GatewayError::Permanent {
        kind,
        provider: Some(provider.clone()),
        detail: sanitize_detail(&format!("http {status}: {message}")),
    }
}

fn mentions_model_not_found(lower: &str) -> bool {
    lower
        .find("model")
        .is_some_and(|i| lower[i..].contains("not found") || lower[i..].contains("does not exist"))
}

fn names_parameter(lower: &str) -> bool {
    [
        "unsupported parameter",
        "unknown parameter",
        "unrecognized request argument",
        "not supported with",
    ]
    .iter()
    .any(|p| lower.contains(p))
}

/// Classifies an HTTP error response.
pub fn classify_http(
    provider: &ProviderId,
    status: u16,
    body: &str,
    retry_after: Option<&str>,
    now: SystemTime,
) -> GatewayError {
    let (etype, ecode, emsg) = error_fields(body);
    let lower = format!("{etype} {ecode} {emsg}").to_lowercase();

    if ["insufficient_quota", "quota", "billing", "credit balance"]
        .iter()
        .any(|w| lower.contains(w))
    {
        return permanent(PermanentKind::QuotaExhausted, provider, status, &emsg);
    }
    if status == 413
        || lower.contains("prompt is too long")
        || lower.contains("context_length_exceeded")
    {
        return permanent(PermanentKind::ContextTooLarge, provider, status, &emsg);
    }
    if status == 404 || mentions_model_not_found(&lower) {
        return permanent(PermanentKind::ModelNotFound, provider, status, &emsg);
    }
    match status {
        401 => return permanent(PermanentKind::Auth, provider, status, &emsg),
        403 => return permanent(PermanentKind::Forbidden, provider, status, &emsg),
        400 if etype == "invalid_request_error" || lower.contains("invalid_request_error") => {
            let kind = if names_parameter(&lower) {
                PermanentKind::UnsupportedParameter
            } else {
                PermanentKind::InvalidRequest
            };
            return permanent(kind, provider, status, &emsg);
        }
        429 => {
            return GatewayError::RateLimited {
                retry_after: retry_after.and_then(|v| parse_retry_after(v, now)),
                provider: provider.clone(),
                scope: RateScope::Provider,
            }
        }
        529 => {
            return GatewayError::Transient {
                kind: TransientKind::Overloaded,
                provider: Some(provider.clone()),
            }
        }
        408 | 500 | 502 | 503 | 504 => {
            return GatewayError::Transient {
                kind: TransientKind::ServerError(status),
                provider: Some(provider.clone()),
            }
        }
        _ => {}
    }
    permanent(PermanentKind::Unknown, provider, status, &emsg)
}

/// Classifies a transport failure.
pub fn classify_transport(provider: &ProviderId, failure: TransportFailure) -> GatewayError {
    let kind = match failure {
        TransportFailure::Connect => TransientKind::Connect,
        TransportFailure::Timeout => TransientKind::Timeout,
        TransportFailure::Reset => TransientKind::Reset,
        TransportFailure::BodyRead => TransientKind::TruncatedBody,
    };
    GatewayError::Transient {
        kind,
        provider: Some(provider.clone()),
    }
}
