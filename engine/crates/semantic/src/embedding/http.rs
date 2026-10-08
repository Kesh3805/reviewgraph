//! HTTP plumbing shared by the remote embedding adapters (SEM-002).

use std::time::Duration;

use serde_json::Value;

use super::EmbedError;
use crate::redact::redact;

pub(crate) const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_ERROR_DETAIL: usize = 300;

/// Pooled rustls client with the adapter timeouts. Headers are never logged.
pub(crate) fn build_client() -> Result<reqwest::Client, EmbedError> {
    reqwest::Client::builder()
        .user_agent(concat!("reviewgraph-semantic/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| EmbedError::Permanent(format!("http client: {e}")))
}

fn detail(body: &str) -> String {
    let scrubbed = redact(body);
    let mut s: String = scrubbed.chars().take(MAX_ERROR_DETAIL).collect();
    if scrubbed.chars().count() > MAX_ERROR_DETAIL {
        s.push('…');
    }
    s
}

/// `Retry-After` (seconds or HTTP date is not supported: seconds only) or `retry-after-ms`.
fn retry_after_ms(headers: &reqwest::header::HeaderMap) -> u64 {
    let get = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
            .map(str::to_owned)
    };
    if let Some(ms) = get("retry-after-ms").and_then(|v| v.parse::<f64>().ok()) {
        return ms.max(0.0) as u64;
    }
    get("retry-after")
        .and_then(|v| v.parse::<f64>().ok())
        .map_or(0, |s| (s.max(0.0) * 1000.0) as u64)
}

/// Maps a status code to the error taxonomy: 429 rate limited, 408 and 5xx transient, other 4xx
/// permanent.
pub(crate) fn classify_status(
    status: reqwest::StatusCode,
    headers: &reqwest::header::HeaderMap,
    body: &str,
) -> EmbedError {
    let code = status.as_u16();
    if code == 429 {
        EmbedError::RateLimited {
            retry_after_ms: retry_after_ms(headers),
        }
    } else if code == 408 || status.is_server_error() {
        EmbedError::Transient(format!("http {code}: {}", detail(body)))
    } else {
        EmbedError::Permanent(format!("http {code}: {}", detail(body)))
    }
}

fn transport(e: &reqwest::Error) -> EmbedError {
    if e.is_timeout() {
        EmbedError::Transient("request timed out".into())
    } else if e.is_connect() {
        EmbedError::Transient("connection failed".into())
    } else if e.is_decode() || e.is_body() {
        EmbedError::Transient("response body could not be read".into())
    } else {
        EmbedError::Transient("transport error".into())
    }
}

/// POSTs `body` with a bearer token and returns the parsed 2xx JSON body.
pub(crate) async fn post_json(
    client: &reqwest::Client,
    url: &str,
    api_key: &str,
    body: &Value,
) -> Result<Value, EmbedError> {
    let resp = client
        .post(url)
        .bearer_auth(api_key)
        .json(body)
        .send()
        .await
        .map_err(|e| transport(&e))?;
    let status = resp.status();
    let headers = resp.headers().clone();
    let text = resp.text().await.map_err(|e| transport(&e))?;
    if !status.is_success() {
        return Err(classify_status(status, &headers, &text));
    }
    serde_json::from_str(&text)
        .map_err(|e| EmbedError::Permanent(format!("unexpected response body: {e}")))
}

/// Parses `{"data": [{"index": i, "embedding": [...]}, ...]}` and orders vectors by `index`.
pub(crate) fn vectors_by_index(body: &Value, expected: usize) -> Result<Vec<Vec<f32>>, EmbedError> {
    let data = body
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| EmbedError::Permanent("response has no data array".into()))?;
    let mut slots: Vec<Option<Vec<f32>>> = vec![None; expected];
    for (pos, item) in data.iter().enumerate() {
        let index = item
            .get("index")
            .and_then(Value::as_u64)
            .map_or(pos, |i| i as usize);
        let embedding = item
            .get("embedding")
            .and_then(Value::as_array)
            .ok_or_else(|| EmbedError::Permanent("data item has no embedding".into()))?;
        let vector = embedding
            .iter()
            .map(|x| x.as_f64().map(|f| f as f32))
            .collect::<Option<Vec<f32>>>()
            .ok_or_else(|| EmbedError::Permanent("embedding is not numeric".into()))?;
        let slot = slots
            .get_mut(index)
            .ok_or_else(|| EmbedError::Permanent(format!("index {index} out of range")))?;
        *slot = Some(vector);
    }
    slots
        .into_iter()
        .enumerate()
        .map(|(i, v)| v.ok_or_else(|| EmbedError::Permanent(format!("missing vector {i}"))))
        .collect()
}

/// `usage.total_tokens` (falls back to `usage.prompt_tokens`, then 0).
pub(crate) fn usage_tokens(body: &Value) -> u32 {
    let usage = body.get("usage");
    usage
        .and_then(|u| u.get("total_tokens"))
        .or_else(|| usage.and_then(|u| u.get("prompt_tokens")))
        .and_then(Value::as_u64)
        .map_or(0, |n| u32::try_from(n).unwrap_or(u32::MAX))
}

/// Splits `texts` into provider batches, runs them concurrently under `permits`, and returns the
/// vectors in input order with summed usage.
pub(crate) async fn embed_batched<'a, F, Fut>(
    texts: &'a [String],
    max_batch: usize,
    permits: &tokio::sync::Semaphore,
    provider: &'static str,
    call: F,
) -> Result<super::EmbedResponse, EmbedError>
where
    F: Fn(&'a [String]) -> Fut,
    Fut: std::future::Future<Output = Result<(Vec<Vec<f32>>, u32), EmbedError>>,
{
    let started = std::time::Instant::now();
    let batches = texts.chunks(max_batch.max(1)).map(|batch| {
        let fut = call(batch);
        async move {
            let _permit = permits
                .acquire()
                .await
                .map_err(|_| EmbedError::Permanent("provider is shutting down".into()))?;
            crate::metrics::embedding_batch(provider);
            fut.await
        }
    });
    let results = futures::future::try_join_all(batches).await?;
    let mut vectors = Vec::with_capacity(texts.len());
    let mut usage_tokens: u32 = 0;
    for (v, tokens) in results {
        vectors.extend(v);
        usage_tokens = usage_tokens.saturating_add(tokens);
    }
    Ok(super::EmbedResponse {
        vectors,
        usage_tokens,
        latency_ms: u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX),
    })
}
