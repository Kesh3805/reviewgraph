//! Jittered exponential retry for transient errors only (GW-002).
//!
//! For attempt `n` (1-based) the sleep is `uniform(0, min(cap, base * 2^(n-1)))` (full jitter).
//! A rate-limit sleep is at least the provider's `Retry-After`. If `now + sleep` would reach the
//! call deadline, the loop gives up with `BudgetExceeded { Deadline }`.

use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::error::{BudgetKind, GatewayError};
use crate::types::CallBudget;

/// Hard cap on attempts per call, whatever the budget says.
pub const MAX_ATTEMPTS: u8 = 3;

/// Source of jitter, injectable so tests are deterministic.
pub trait JitterRng: Send + Sync {
    /// A uniformly distributed duration in `[0, upper]`.
    fn uniform(&self, upper: Duration) -> Duration;
}

/// splitmix64 over an atomic counter, seeded from the clock. Not cryptographic.
#[derive(Debug)]
pub struct SystemJitter(AtomicU64);

impl Default for SystemJitter {
    fn default() -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0x9E37_79B9_7F4A_7C15, |d| d.as_nanos() as u64);
        Self(AtomicU64::new(seed))
    }
}

impl JitterRng for SystemJitter {
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

#[derive(Clone)]
pub struct RetryPolicy {
    pub base: Duration,
    pub cap: Duration,
    rng: Arc<dyn JitterRng>,
}

impl std::fmt::Debug for RetryPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RetryPolicy")
            .field("base", &self.base)
            .field("cap", &self.cap)
            .finish_non_exhaustive()
    }
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            base: Duration::from_millis(500),
            cap: Duration::from_secs(20),
            rng: Arc::new(SystemJitter::default()),
        }
    }
}

impl RetryPolicy {
    pub fn with_rng(mut self, rng: Arc<dyn JitterRng>) -> Self {
        self.rng = rng;
        self
    }

    pub fn with_delays(mut self, base: Duration, cap: Duration) -> Self {
        self.base = base;
        self.cap = cap;
        self
    }

    /// Full-jitter upper bound for 1-based `attempt`.
    pub fn jitter_bound(&self, attempt: u32) -> Duration {
        let exp = self.base.saturating_mul(
            1u32.checked_shl(attempt.saturating_sub(1))
                .unwrap_or(u32::MAX),
        );
        exp.min(self.cap)
    }

    /// Sleep before the next attempt after `err` on 1-based `attempt`.
    pub fn backoff(&self, attempt: u32, err: &GatewayError) -> Duration {
        let jittered = self.rng.uniform(self.jitter_bound(attempt));
        match err {
            GatewayError::RateLimited { retry_after, .. } => {
                jittered.max(retry_after.unwrap_or(Duration::ZERO))
            }
            _ => jittered,
        }
    }
}

/// Result of [`retry`]: the final result and how many attempts were made.
#[derive(Debug)]
pub struct RetryOutcome<T> {
    pub result: Result<T, GatewayError>,
    pub attempts: u8,
}

/// Runs `op(attempt)` until it succeeds, fails permanently, runs out of attempts or time, or is
/// cancelled. Only `Transient` and `RateLimited` errors are retried.
pub async fn retry<T, F, Fut>(
    policy: &RetryPolicy,
    budget: &CallBudget,
    cancel: &CancellationToken,
    mut op: F,
) -> RetryOutcome<T>
where
    F: FnMut(u8) -> Fut,
    Fut: Future<Output = Result<T, GatewayError>>,
{
    let max = budget.max_attempts.clamp(1, MAX_ATTEMPTS);
    let mut attempt: u8 = 1;
    loop {
        let res = tokio::select! {
            () = cancel.cancelled() => Err(GatewayError::Cancelled),
            r = op(attempt) => r,
        };
        let err = match res {
            Ok(v) => {
                return RetryOutcome {
                    result: Ok(v),
                    attempts: attempt,
                }
            }
            Err(e) => e,
        };
        if !err.is_retryable() || attempt >= max {
            return RetryOutcome {
                result: Err(err),
                attempts: attempt,
            };
        }
        let sleep = policy.backoff(u32::from(attempt), &err);
        if Instant::now() + sleep >= budget.deadline {
            return RetryOutcome {
                result: Err(GatewayError::BudgetExceeded {
                    kind: BudgetKind::Deadline,
                }),
                attempts: attempt,
            };
        }
        tracing::info!(
            attempt,
            sleep_ms = u64::try_from(sleep.as_millis()).unwrap_or(u64::MAX),
            reason = err.class(),
            "retry"
        );
        tokio::select! {
            () = cancel.cancelled() => {
                return RetryOutcome { result: Err(GatewayError::Cancelled), attempts: attempt };
            }
            () = tokio::time::sleep(sleep) => {}
        }
        attempt += 1;
    }
}
