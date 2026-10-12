//! Retry backoff: `min(300 s, 5 s * 2^(attempts-1))` with full jitter.

use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Backoff {
    pub base: Duration,
    pub cap: Duration,
}

impl Default for Backoff {
    fn default() -> Self {
        Self {
            base: Duration::from_secs(5),
            cap: Duration::from_secs(300),
        }
    }
}

impl Backoff {
    /// Upper bound before jitter for the given attempt number (1-based).
    pub fn ceiling(&self, attempts: u32) -> Duration {
        let exp = attempts.saturating_sub(1).min(20);
        self.base.saturating_mul(1u32 << exp).min(self.cap)
    }

    /// Full jitter: `jitter` in `0.0..=1.0` (caller supplies randomness) scales the ceiling.
    pub fn delay(&self, attempts: u32, jitter: f64) -> Duration {
        self.ceiling(attempts).mul_f64(jitter.clamp(0.0, 1.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ceiling_doubles_then_caps() {
        let b = Backoff::default();
        assert_eq!(b.ceiling(1), Duration::from_secs(5));
        assert_eq!(b.ceiling(2), Duration::from_secs(10));
        assert_eq!(b.ceiling(10), Duration::from_secs(300));
    }

    #[test]
    fn jitter_within_bounds() {
        let b = Backoff::default();
        assert_eq!(b.delay(3, 0.0), Duration::ZERO);
        assert_eq!(b.delay(3, 1.0), Duration::from_secs(20));
        assert_eq!(b.delay(3, 7.0), Duration::from_secs(20));
    }
}
