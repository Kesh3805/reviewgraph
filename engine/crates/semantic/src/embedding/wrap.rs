//! Provider wrappers: normalization and checks, retry, redaction (SEM-001).

use std::borrow::Cow;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tracing::Instrument;

use super::{EmbedError, EmbedRequest, EmbedResponse, EmbeddingProvider, EmbeddingSpace};
use crate::metrics;
use crate::redact::redact;

/// Scales `v` to unit L2 length in place. A zero vector stays zero. Accumulates in `f64` in index
/// order, so the result does not depend on thread scheduling.
pub fn l2_normalize(v: &mut [f32]) {
    let norm = v
        .iter()
        .map(|x| f64::from(*x) * f64::from(*x))
        .sum::<f64>()
        .sqrt();
    if norm > 0.0 {
        for x in v.iter_mut() {
            *x = (f64::from(*x) / norm) as f32;
        }
    }
}

/// Checks the response shape and dimensionality, L2-normalizes, and records the
/// `embedding_request` span and metrics.
#[derive(Debug)]
pub struct Normalized<P> {
    inner: P,
}

impl<P> Normalized<P> {
    pub fn new(inner: P) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl<P: EmbeddingProvider> EmbeddingProvider for Normalized<P> {
    fn space(&self) -> &EmbeddingSpace {
        self.inner.space()
    }

    fn max_batch(&self) -> usize {
        self.inner.max_batch()
    }

    fn max_input_tokens(&self) -> usize {
        self.inner.max_input_tokens()
    }

    async fn embed(&self, req: EmbedRequest<'_>) -> Result<EmbedResponse, EmbedError> {
        let space = self.inner.space();
        let provider = space.provider.as_str();
        let span = tracing::info_span!(
            "embedding_request",
            provider,
            model = space.model.as_str(),
            batch_size = req.texts.len(),
            input_kind = req.kind.as_str(),
            organization_id = tracing::field::Empty,
            repository_id = tracing::field::Empty,
        );
        if let Some(org) = &req.trace.organization_id {
            span.record("organization_id", tracing::field::display(org));
        }
        if let Some(repo) = &req.trace.repository_id {
            span.record("repository_id", tracing::field::display(repo));
        }
        let expected_len = req.texts.len();
        let started = Instant::now();
        let result = self.inner.embed(req).instrument(span).await;
        let result = result.and_then(|mut resp| {
            if resp.vectors.len() != expected_len {
                return Err(EmbedError::Permanent(format!(
                    "provider returned {} vectors for {expected_len} inputs",
                    resp.vectors.len()
                )));
            }
            if let Some(bad) = resp
                .vectors
                .iter()
                .find(|v| v.len() != usize::from(space.dims))
            {
                return Err(EmbedError::DimensionMismatch {
                    expected: space.dims,
                    got: bad.len(),
                });
            }
            for v in &mut resp.vectors {
                l2_normalize(v);
            }
            if resp.latency_ms == 0 {
                resp.latency_ms = u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX);
            }
            Ok(resp)
        });
        match &result {
            Ok(resp) => metrics::embedding_call(provider, resp.latency_ms, resp.usage_tokens),
            Err(e) => metrics::embedding_error(provider, e.kind()),
        }
        result
    }
}

/// Source of jitter in `[0, upper]`, injectable so tests are deterministic.
pub trait Jitter: Send + Sync + std::fmt::Debug {
    fn uniform(&self, upper: Duration) -> Duration;
}

/// splitmix64 over an atomic counter, seeded from the clock. Not cryptographic.
#[derive(Debug)]
struct SystemJitter(AtomicU64);

impl SystemJitter {
    fn new() -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0x9E37_79B9_7F4A_7C15, |d| d.as_nanos() as u64);
        Self(AtomicU64::new(seed))
    }
}

impl Jitter for SystemJitter {
    fn uniform(&self, upper: Duration) -> Duration {
        let mut z = self
            .0
            .fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed)
            .wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        let frac = (z >> 11) as f64 / (1u64 << 53) as f64;
        upper.mul_f64(frac)
    }
}

/// Retry policy: up to `max_retries` retries of transient and rate-limited failures, with full
/// jitter over `min(cap, base * 2^(n-1))`, never shorter than a provider's `retry_after`.
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    pub max_retries: u32,
    pub base: Duration,
    pub cap: Duration,
    jitter: Arc<dyn Jitter>,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 3,
            base: Duration::from_millis(250),
            cap: Duration::from_secs(8),
            jitter: Arc::new(SystemJitter::new()),
        }
    }
}

impl RetryPolicy {
    /// No sleeping between attempts (tests).
    pub fn immediate() -> Self {
        Self {
            base: Duration::ZERO,
            cap: Duration::ZERO,
            ..Self::default()
        }
    }

    pub fn with_jitter(mut self, jitter: Arc<dyn Jitter>) -> Self {
        self.jitter = jitter;
        self
    }

    /// Sleep before retry `attempt` (1-based) after `err`.
    pub fn backoff(&self, attempt: u32, err: &EmbedError) -> Duration {
        let exp = self
            .base
            .saturating_mul(
                1u32.checked_shl(attempt.saturating_sub(1))
                    .unwrap_or(u32::MAX),
            )
            .min(self.cap);
        let jittered = self.jitter.uniform(exp);
        match err {
            EmbedError::RateLimited { retry_after_ms } => {
                jittered.max(Duration::from_millis(*retry_after_ms))
            }
            _ => jittered,
        }
    }
}

/// Retries transient and rate-limited failures; permanent ones are returned at once.
#[derive(Debug)]
pub struct RetryingProvider<P> {
    inner: P,
    policy: RetryPolicy,
}

impl<P> RetryingProvider<P> {
    pub fn new(inner: P, policy: RetryPolicy) -> Self {
        Self { inner, policy }
    }
}

#[async_trait]
impl<P: EmbeddingProvider> EmbeddingProvider for RetryingProvider<P> {
    fn space(&self) -> &EmbeddingSpace {
        self.inner.space()
    }

    fn max_batch(&self) -> usize {
        self.inner.max_batch()
    }

    fn max_input_tokens(&self) -> usize {
        self.inner.max_input_tokens()
    }

    async fn embed(&self, req: EmbedRequest<'_>) -> Result<EmbedResponse, EmbedError> {
        let mut attempt: u32 = 0;
        loop {
            match self.inner.embed(req.clone()).await {
                Ok(resp) => return Ok(resp),
                Err(e) if e.is_retryable() && attempt < self.policy.max_retries => {
                    attempt += 1;
                    let wait = self.policy.backoff(attempt, &e);
                    tracing::debug!(attempt, kind = e.kind(), "retrying embedding request");
                    if !wait.is_zero() {
                        tokio::time::sleep(wait).await;
                    }
                }
                Err(e) => return Err(e),
            }
        }
    }
}

/// Scrubs secrets from every input before the wrapped provider sees it.
#[derive(Debug)]
pub struct Redacting<P> {
    inner: P,
}

impl<P> Redacting<P> {
    pub fn new(inner: P) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl<P: EmbeddingProvider> EmbeddingProvider for Redacting<P> {
    fn space(&self) -> &EmbeddingSpace {
        self.inner.space()
    }

    fn max_batch(&self) -> usize {
        self.inner.max_batch()
    }

    fn max_input_tokens(&self) -> usize {
        self.inner.max_input_tokens()
    }

    async fn embed(&self, req: EmbedRequest<'_>) -> Result<EmbedResponse, EmbedError> {
        let mut changed = false;
        let texts: Vec<String> = req
            .texts
            .iter()
            .map(|t| match redact(t) {
                Cow::Borrowed(b) => b.to_owned(),
                Cow::Owned(o) => {
                    changed = true;
                    o
                }
            })
            .collect();
        if !changed {
            return self.inner.embed(req).await;
        }
        let redacted = EmbedRequest {
            kind: req.kind,
            texts: &texts,
            trace: req.trace,
        };
        self.inner.embed(redacted).await
    }
}
