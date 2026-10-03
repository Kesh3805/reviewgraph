//! Cross-worker rate limiting (GW-007): token buckets per provider and model on requests,
//! input tokens and output tokens.
//!
//! The limiter logic ([`TokenBucketLimiter`]) is independent of where buckets live
//! ([`BucketStore`]): [`local::LocalBuckets`] is in process, the Redis store (feature `redis`)
//! shares buckets across workers, and [`FallbackStore`] degrades from Redis to local buckets
//! (scaled by `RG_WORKER_COUNT_HINT`) when Redis is unavailable: fail-local, not fail-closed.

pub mod local;
#[cfg(feature = "redis")]
pub mod redis_bucket;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::error::{GatewayError, RateScope};
use crate::types::ProviderId;

/// Per-model account limits from `routing.yaml` (`providers.<p>.limits`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ModelLimits {
    pub rpm: u32,
    pub input_tpm: u32,
    pub output_tpm: u32,
}

/// One bucket's shape.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BucketSpec {
    pub capacity: f64,
    pub refill_per_ms: f64,
}

impl BucketSpec {
    /// A bucket that refills `per_minute` tokens every minute and holds one minute of burst.
    pub fn per_minute(per_minute: u32) -> Self {
        let capacity = f64::from(per_minute.max(1));
        Self {
            capacity,
            refill_per_ms: capacity / 60_000.0,
        }
    }

    fn scaled(self, divisor: u32) -> Self {
        let d = f64::from(divisor.max(1));
        Self {
            capacity: (self.capacity / d).max(1.0),
            refill_per_ms: self.refill_per_ms / d,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Take {
    Granted,
    Denied { wait: Duration },
}

/// A bucket store failed (for example Redis is unreachable).
#[derive(Debug, Clone, thiserror::Error)]
#[error("bucket store error: {0}")]
pub struct StoreError(pub String);

#[async_trait]
pub trait BucketStore: Send + Sync {
    /// Atomically takes `cost` tokens if available.
    async fn take(&self, key: &str, spec: BucketSpec, cost: f64) -> Result<Take, StoreError>;
    /// Returns tokens, never above capacity.
    async fn refund(&self, key: &str, spec: BucketSpec, amount: f64) -> Result<(), StoreError>;
}

/// What to reserve for one attempt.
#[derive(Debug, Clone)]
pub struct LimitRequest {
    pub provider: ProviderId,
    pub model: String,
    pub limits: ModelLimits,
    pub est_input: u32,
    pub max_output: u32,
    pub deadline: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dim {
    Req,
    InTok,
    OutTok,
}

impl Dim {
    fn name(self) -> &'static str {
        match self {
            Dim::Req => "req",
            Dim::InTok => "in_tok",
            Dim::OutTok => "out_tok",
        }
    }

    fn scope(self) -> RateScope {
        match self {
            Dim::Req => RateScope::Requests,
            Dim::InTok => RateScope::InputTokens,
            Dim::OutTok => RateScope::OutputTokens,
        }
    }
}

#[derive(Debug, Clone)]
struct Held {
    dim: Dim,
    key: String,
    spec: BucketSpec,
    amount: f64,
}

/// What a successful acquire holds; pass it to `reconcile` with the actual usage.
#[derive(Debug, Clone, Default)]
pub struct Reservation {
    held: Vec<Held>,
    est_input: u32,
    max_output: u32,
}

#[async_trait]
pub trait RateLimiter: Send + Sync {
    /// Waits for capacity (bounded by the deadline) and reserves the worst case.
    async fn acquire(
        &self,
        req: &LimitRequest,
        cancel: &CancellationToken,
    ) -> Result<Reservation, GatewayError>;

    /// Refunds unused output and adjusts input after the response.
    async fn reconcile(&self, reservation: &Reservation, actual_input: u32, actual_output: u32);
}

/// No limits (default when a model has no configured limits).
#[derive(Debug, Default, Clone, Copy)]
pub struct NoLimit;

#[async_trait]
impl RateLimiter for NoLimit {
    async fn acquire(
        &self,
        _req: &LimitRequest,
        _cancel: &CancellationToken,
    ) -> Result<Reservation, GatewayError> {
        Ok(Reservation::default())
    }

    async fn reconcile(&self, _r: &Reservation, _actual_input: u32, _actual_output: u32) {}
}

fn bucket_key(provider: &ProviderId, model: &str, dim: Dim) -> String {
    format!("rg:rl:{provider}:{model}:{}", dim.name())
}

/// Token-bucket limiter over any [`BucketStore`].
pub struct TokenBucketLimiter {
    store: Arc<dyn BucketStore>,
    jitter: AtomicU64,
}

impl std::fmt::Debug for TokenBucketLimiter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenBucketLimiter").finish_non_exhaustive()
    }
}

impl TokenBucketLimiter {
    pub fn new(store: Arc<dyn BucketStore>) -> Self {
        Self {
            store,
            jitter: AtomicU64::new(0x2545_F491_4F6C_DD1D),
        }
    }

    /// Up to 10% extra wait, so workers do not wake in lockstep.
    fn with_jitter(&self, wait: Duration) -> Duration {
        let mut x = self.jitter.load(Ordering::Relaxed);
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.jitter.store(x, Ordering::Relaxed);
        wait + wait.mul_f64((x % 1000) as f64 / 10_000.0)
    }

    fn plan(req: &LimitRequest) -> Vec<(Dim, BucketSpec, f64)> {
        let l = req.limits;
        let mut dims = Vec::new();
        for (dim, per_min, cost) in [
            (Dim::Req, l.rpm, 1.0),
            (Dim::InTok, l.input_tpm, f64::from(req.est_input)),
            (Dim::OutTok, l.output_tpm, f64::from(req.max_output)),
        ] {
            if per_min == 0 {
                continue;
            }
            let spec = BucketSpec::per_minute(per_min);
            // A cost above capacity waits for a full bucket instead of deadlocking.
            dims.push((dim, spec, cost.min(spec.capacity)));
        }
        dims
    }
}

#[async_trait]
impl RateLimiter for TokenBucketLimiter {
    async fn acquire(
        &self,
        req: &LimitRequest,
        cancel: &CancellationToken,
    ) -> Result<Reservation, GatewayError> {
        let plan = Self::plan(req);
        loop {
            let mut held: Vec<Held> = Vec::new();
            let mut denied: Option<(Dim, Duration)> = None;
            for (dim, spec, cost) in &plan {
                let key = bucket_key(&req.provider, &req.model, *dim);
                match self.store.take(&key, *spec, *cost).await {
                    Ok(Take::Granted) => held.push(Held {
                        dim: *dim,
                        key,
                        spec: *spec,
                        amount: *cost,
                    }),
                    Ok(Take::Denied { wait }) => {
                        denied = Some((*dim, wait));
                        break;
                    }
                    // The store reports its own failure; a store that cannot decide must not
                    // block the call (fail-local lives in `FallbackStore`).
                    Err(e) => {
                        tracing::warn!(error = %e, "rate limiter store error; allowing call");
                        denied = None;
                        break;
                    }
                }
            }
            let Some((dim, wait)) = denied else {
                return Ok(Reservation {
                    held,
                    est_input: req.est_input,
                    max_output: req.max_output,
                });
            };
            // Partial acquire: give back what was taken.
            for h in &held {
                let _ = self.store.refund(&h.key, h.spec, h.amount).await;
            }
            let wait = self.with_jitter(wait.max(Duration::from_millis(1)));
            tracing::info!(
                provider = %req.provider,
                dim = dim.name(),
                wait_ms = u64::try_from(wait.as_millis()).unwrap_or(u64::MAX),
                "ratelimit_wait"
            );
            if Instant::now() + wait >= req.deadline {
                return Err(GatewayError::RateLimited {
                    retry_after: Some(wait),
                    provider: req.provider.clone(),
                    scope: dim.scope(),
                });
            }
            tokio::select! {
                () = cancel.cancelled() => return Err(GatewayError::Cancelled),
                () = tokio::time::sleep(wait) => {}
            }
        }
    }

    async fn reconcile(&self, r: &Reservation, actual_input: u32, actual_output: u32) {
        for h in &r.held {
            match h.dim {
                Dim::Req => {}
                Dim::OutTok => {
                    let unused = r.max_output.saturating_sub(actual_output);
                    if unused > 0 {
                        let _ = self.store.refund(&h.key, h.spec, f64::from(unused)).await;
                    }
                }
                Dim::InTok => {
                    let delta = i64::from(actual_input) - i64::from(r.est_input);
                    if delta < 0 {
                        let _ = self
                            .store
                            .refund(&h.key, h.spec, delta.unsigned_abs() as f64)
                            .await;
                    } else if delta > 0 {
                        // Best effort: record the extra usage, but never block on it.
                        let _ = self.store.take(&h.key, h.spec, delta as f64).await;
                    }
                }
            }
        }
    }
}

/// Redis (or any shared store) with automatic degradation to in-process buckets.
///
/// A primary call that errors or exceeds `timeout` opens a circuit for `open_for`; while open,
/// local buckets scaled by `worker_hint` are used so a Redis outage never halts reviews.
pub struct FallbackStore {
    primary: Arc<dyn BucketStore>,
    local: local::LocalBuckets,
    worker_hint: u32,
    timeout: Duration,
    open_for: Duration,
    open_until: std::sync::Mutex<Option<Instant>>,
    degraded: AtomicU64,
}

impl std::fmt::Debug for FallbackStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FallbackStore")
            .field("worker_hint", &self.worker_hint)
            .finish_non_exhaustive()
    }
}

impl FallbackStore {
    pub fn new(primary: Arc<dyn BucketStore>, worker_hint: u32) -> Self {
        Self {
            primary,
            local: local::LocalBuckets::new(),
            worker_hint: worker_hint.max(1),
            timeout: Duration::from_millis(50),
            open_for: Duration::from_secs(30),
            open_until: std::sync::Mutex::new(None),
            degraded: AtomicU64::new(0),
        }
    }

    pub fn with_timing(mut self, timeout: Duration, open_for: Duration) -> Self {
        self.timeout = timeout;
        self.open_for = open_for;
        self
    }

    /// Number of calls served by the local fallback (`llm_ratelimit_degraded_total`).
    pub fn degraded_total(&self) -> u64 {
        self.degraded.load(Ordering::Relaxed)
    }

    fn circuit_open(&self) -> bool {
        self.open_until
            .lock()
            .ok()
            .and_then(|g| *g)
            .is_some_and(|t| Instant::now() < t)
    }

    fn trip(&self) {
        if let Ok(mut g) = self.open_until.lock() {
            let was_open = g.is_some_and(|t| Instant::now() < t);
            *g = Some(Instant::now() + self.open_for);
            if !was_open {
                tracing::warn!("rate limit store unavailable; using local buckets");
            }
        }
    }
}

#[async_trait]
impl BucketStore for FallbackStore {
    async fn take(&self, key: &str, spec: BucketSpec, cost: f64) -> Result<Take, StoreError> {
        if !self.circuit_open() {
            match tokio::time::timeout(self.timeout, self.primary.take(key, spec, cost)).await {
                Ok(Ok(t)) => return Ok(t),
                _ => self.trip(),
            }
        }
        self.degraded.fetch_add(1, Ordering::Relaxed);
        let scaled = spec.scaled(self.worker_hint);
        self.local
            .take(key, scaled, cost.min(scaled.capacity))
            .await
    }

    async fn refund(&self, key: &str, spec: BucketSpec, amount: f64) -> Result<(), StoreError> {
        if !self.circuit_open() {
            match tokio::time::timeout(self.timeout, self.primary.refund(key, spec, amount)).await {
                Ok(Ok(())) => return Ok(()),
                _ => self.trip(),
            }
        }
        self.local
            .refund(key, spec.scaled(self.worker_hint), amount)
            .await
    }
}

/// Builds the production limiter from the environment: `REDIS_URL` selects shared Redis buckets
/// (with local fallback sized by `RG_WORKER_COUNT_HINT`, default 4); without it, or when Redis
/// cannot be reached at startup, buckets are process-local.
pub async fn limiter_from_lookup(lookup: impl Fn(&str) -> Option<String>) -> TokenBucketLimiter {
    let hint = lookup("RG_WORKER_COUNT_HINT")
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(4)
        .max(1);
    #[cfg(feature = "redis")]
    if let Some(url) = lookup("REDIS_URL").filter(|u| !u.trim().is_empty()) {
        match redis_bucket::RedisBuckets::connect(&url).await {
            Ok(redis) => {
                return TokenBucketLimiter::new(Arc::new(FallbackStore::new(
                    Arc::new(redis),
                    hint,
                )));
            }
            Err(e) => {
                tracing::warn!(error = %e, "redis unavailable at startup; using local rate limit buckets");
            }
        }
    }
    let _ = hint;
    TokenBucketLimiter::new(Arc::new(local::LocalBuckets::new()))
}
