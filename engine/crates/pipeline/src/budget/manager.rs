use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use super::limits::{BudgetKind, ReviewBudget};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("budget exhausted: {kind:?} (requested {requested}, remaining {remaining})")]
pub struct BudgetExceeded {
    pub kind: BudgetKind,
    pub requested: u64,
    pub remaining: u64,
}

/// Recorded for every skipped unit; PIPE-008 turns these into degraded coverage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetEvent {
    pub kind: BudgetKind,
    pub skipped_unit: String,
    pub at_ms: u64,
}

/// A held reservation. Must be passed to [`BudgetManager::commit`].
#[derive(Debug)]
#[must_use = "commit the reservation with the actual usage"]
pub struct Reservation {
    kind: BudgetKind,
    amount: u64,
}

/// Snapshot persisted in `review_runs.budget_usage` at stage boundaries.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BudgetUsage {
    pub used: [u64; 6],
    pub limits: [u64; 6],
}

impl BudgetUsage {
    pub fn used(&self, kind: BudgetKind) -> u64 {
        self.used[kind.index()]
    }
}

#[derive(Debug)]
struct Inner {
    limits: ReviewBudget,
    used: [AtomicU64; 6],
    start: Instant,
    events: Mutex<Vec<BudgetEvent>>,
}

/// Lock-free accounting; reservations are linearizable (CAS loop, no overshoot).
#[derive(Debug, Clone)]
pub struct BudgetManager(Arc<Inner>);

impl BudgetManager {
    pub fn new(limits: ReviewBudget) -> Self {
        Self::resume(limits, &BudgetUsage::default())
    }

    /// Restore usage persisted at a previous stage boundary.
    pub fn resume(limits: ReviewBudget, usage: &BudgetUsage) -> Self {
        let used = std::array::from_fn(|i| AtomicU64::new(usage.used[i]));
        Self(Arc::new(Inner {
            limits,
            used,
            start: Instant::now(),
            events: Mutex::new(Vec::new()),
        }))
    }

    pub fn try_reserve(
        &self,
        kind: BudgetKind,
        amount: u64,
    ) -> Result<Reservation, BudgetExceeded> {
        let limit = self.0.limits.limit(kind);
        let slot = &self.0.used[kind.index()];
        let mut cur = slot.load(Ordering::Acquire);
        loop {
            let Some(next) = cur.checked_add(amount).filter(|n| *n <= limit) else {
                return Err(BudgetExceeded {
                    kind,
                    requested: amount,
                    remaining: limit.saturating_sub(cur),
                });
            };
            match slot.compare_exchange_weak(cur, next, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return Ok(Reservation { kind, amount }),
                Err(seen) => cur = seen,
            }
        }
    }

    /// Replace the reservation with the actual usage; the unused part is refunded.
    pub fn commit(&self, r: Reservation, actual: u64) {
        let slot = &self.0.used[r.kind.index()];
        if actual <= r.amount {
            slot.fetch_sub(r.amount - actual, Ordering::AcqRel);
        } else {
            slot.fetch_add(actual - r.amount, Ordering::AcqRel);
        }
    }

    pub fn remaining(&self, kind: BudgetKind) -> u64 {
        let limit = self.0.limits.limit(kind);
        match kind {
            BudgetKind::Latency => limit.saturating_sub(self.0.start.elapsed().as_millis() as u64),
            _ => limit.saturating_sub(self.0.used[kind.index()].load(Ordering::Acquire)),
        }
    }

    pub fn deadline(&self) -> Instant {
        self.0.start + self.0.limits.max_review_latency
    }

    pub fn deadline_passed(&self) -> bool {
        Instant::now() >= self.deadline()
    }

    /// Record a skipped unit (exhaustion is degradation, never failure).
    pub fn record_skip(&self, kind: BudgetKind, unit: impl Into<String>) {
        let at_ms = self.0.start.elapsed().as_millis() as u64;
        if let Ok(mut ev) = self.0.events.lock() {
            ev.push(BudgetEvent {
                kind,
                skipped_unit: unit.into(),
                at_ms,
            });
        }
    }

    pub fn events(&self) -> Vec<BudgetEvent> {
        self.0.events.lock().map(|e| e.clone()).unwrap_or_default()
    }

    pub fn usage(&self) -> BudgetUsage {
        BudgetUsage {
            used: std::array::from_fn(|i| self.0.used[i].load(Ordering::Acquire)),
            limits: std::array::from_fn(|i| self.0.limits.limit(BudgetKind::ALL[i])),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn small(tokens: u64, calls: u32) -> ReviewBudget {
        ReviewBudget {
            max_model_tokens: tokens,
            max_model_calls: calls,
            ..ReviewBudget::DEFAULT
        }
    }

    #[test]
    fn reserve_never_overshoots_under_contention() {
        let m = BudgetManager::new(small(1000, 10));
        let handles: Vec<_> = (0..64)
            .map(|_| {
                let m = m.clone();
                std::thread::spawn(move || m.try_reserve(BudgetKind::ModelCalls, 1).is_ok())
            })
            .collect();
        let ok = handles
            .into_iter()
            .filter(|_| true)
            .map(|h| h.join().unwrap())
            .filter(|ok| *ok)
            .count();
        assert_eq!(ok, 10);
        assert_eq!(m.usage().used(BudgetKind::ModelCalls), 10);
    }

    #[test]
    fn commit_refunds_unused_tokens() {
        let m = BudgetManager::new(small(1000, 10));
        let r = m.try_reserve(BudgetKind::ModelTokens, 600).unwrap();
        assert!(m.try_reserve(BudgetKind::ModelTokens, 500).is_err());
        m.commit(r, 100);
        assert_eq!(m.remaining(BudgetKind::ModelTokens), 900);
    }

    #[test]
    fn clamp_reports_fields() {
        let over = ReviewBudget {
            max_model_calls: 999,
            max_review_latency: Duration::from_secs(9999),
            ..ReviewBudget::DEFAULT
        };
        let (c, fields) = over.clamp_to(&ReviewBudget::HARD_CAPS);
        assert_eq!(c, ReviewBudget::HARD_CAPS);
        assert_eq!(fields, ["max_model_calls", "max_review_latency"]);
    }

    #[test]
    fn exhaustion_records_events() {
        let m = BudgetManager::new(small(0, 0));
        let e = m.try_reserve(BudgetKind::ModelCalls, 1).unwrap_err();
        assert_eq!(e.remaining, 0);
        m.record_skip(BudgetKind::ModelCalls, "cluster-7");
        assert_eq!(m.events()[0].skipped_unit, "cluster-7");
    }

    #[test]
    fn usage_restored_on_resume() {
        let m = BudgetManager::new(small(1000, 10));
        let _r = m.try_reserve(BudgetKind::ModelTokens, 400).unwrap();
        let u = m.usage();
        let m2 = BudgetManager::resume(small(1000, 10), &u);
        assert_eq!(m2.remaining(BudgetKind::ModelTokens), 600);
    }
}
