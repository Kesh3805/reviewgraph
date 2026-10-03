//! HTTP helpers shared by the live adapters.

use std::time::SystemTime;

use serde_json::Value;

use crate::classify::{classify_http, classify_transport, TransportFailure};
use crate::error::{GatewayError, PermanentKind, TransientKind};
use crate::types::ProviderId;

/// Builds the shared client: pooled connections, rustls, no header logging.
pub(crate) fn build_client() -> Result<reqwest::Client, GatewayError> {
    reqwest::Client::builder()
        .user_agent(concat!(
            "reviewgraph-model-gateway/",
            env!("CARGO_PKG_VERSION")
        ))
        .build()
        .map_err(|e| GatewayError::Permanent {
            kind: PermanentKind::Unknown,
            provider: None,
            detail: crate::classify::sanitize_detail(&format!("http client: {e}")),
        })
}

/// Rejects a plain-http base URL in production.
pub(crate) fn check_base_url(base_url: &str, env_name: Option<&str>) -> Result<(), String> {
    if env_name == Some("production") && !base_url.starts_with("https://") {
        return Err("base URL override must use https in production".into());
    }
    Ok(())
}

pub(crate) fn transport_error(provider: &ProviderId, e: &reqwest::Error) -> GatewayError {
    let failure = if e.is_timeout() {
        TransportFailure::Timeout
    } else if e.is_connect() {
        TransportFailure::Connect
    } else if e.is_body() || e.is_decode() {
        TransportFailure::BodyRead
    } else {
        TransportFailure::Reset
    };
    classify_transport(provider, failure)
}

/// A successful HTTP response body with the headers adapters care about.
pub(crate) struct HttpOk {
    pub body: Value,
    pub request_id: Option<String>,
}

/// Sends `builder` and returns the parsed JSON body of a 2xx response, or a classified error.
pub(crate) async fn execute(
    provider: &ProviderId,
    builder: reqwest::RequestBuilder,
    request_id_header: &str,
    span: &tracing::Span,
) -> Result<HttpOk, GatewayError> {
    let resp = builder
        .send()
        .await
        .map_err(|e| transport_error(provider, &e))?;
    let status = resp.status();
    span.record("http.response.status_code", status.as_u16());
    let request_id = resp
        .headers()
        .get(request_id_header)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    if let Some(id) = &request_id {
        span.record("provider.request_id", id.as_str());
    }
    let retry_after = resp
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let text = resp
        .text()
        .await
        .map_err(|e| transport_error(provider, &e))?;
    if !status.is_success() {
        return Err(classify_http(
            provider,
            status.as_u16(),
            &text,
            retry_after.as_deref(),
            SystemTime::now(),
        ));
    }
    let body = serde_json::from_str::<Value>(&text).map_err(|_| GatewayError::Transient {
        kind: TransientKind::TruncatedBody,
        provider: Some(provider.clone()),
    })?;
    Ok(HttpOk { body, request_id })
}

pub(crate) fn unexpected(provider: &ProviderId, what: &str) -> GatewayError {
    GatewayError::Permanent {
        kind: PermanentKind::Unknown,
        provider: Some(provider.clone()),
        detail: crate::classify::sanitize_detail(what),
    }
}
