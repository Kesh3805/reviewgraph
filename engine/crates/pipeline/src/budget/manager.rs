use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use model_gateway::types::{CallBudget, TaskType};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

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
        Self::resume_with_events(limits, usage, Vec::new())
    }

    /// Like [`Self::resume`], also restoring the skipped units recorded so far.
    pub fn resume_with_events(
        limits: ReviewBudget,
        usage: &BudgetUsage,
        events: Vec<BudgetEvent>,
    ) -> Self {
        let used = std::array::from_fn(|i| AtomicU64::new(usage.used[i]));
        Self(Arc::new(Inner {
            limits,
            used,
            start: Instant::now(),
            events: Mutex::new(events),
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

    /// Cancels `token` (the run-level cancellation token) when the latency budget is spent.
    /// Abort the returned handle once the run finishes.
    pub fn watch_deadline(&self, token: CancellationToken) -> JoinHandle<()> {
        let deadline = self.deadline();
        tokio::spawn(async move {
            tokio::time::sleep_until(deadline).await;
            token.cancel();
        })
    }

    /// Derives the budget of one model call from what is left of the run budget. The requested
    /// `max_in` and `max_out` are clamped so a call can never be allowed to spend more than the
    /// remaining tokens, and the call deadline is the run deadline. Fails when no model call or
    /// no token is left (the orchestrator turns that into degradation, never a failure).
    /// This does not reserve anything: reserve with [`Self::try_reserve`] before the call.
    pub fn call_budget_for(
        &self,
        _task: TaskType,
        max_in: u32,
        max_out: u32,
    ) -> Result<CallBudget, BudgetExceeded> {
        let calls = self.remaining(BudgetKind::ModelCalls);
        let tokens = self.remaining(BudgetKind::ModelTokens);
        if calls == 0 || tokens == 0 {
            let kind = if calls == 0 {
                BudgetKind::ModelCalls
            } else {
                BudgetKind::ModelTokens
            };
            return Err(BudgetExceeded {
                kind,
                requested: 1,
                remaining: 0,
            });
        }
        if self.deadline_passed() {
            return Err(BudgetExceeded {
                kind: BudgetKind::Latency,
                requested: 1,
                remaining: 0,
            });
        }
        let max_in = u64::from(max_in).min(tokens);
        let max_out = u64::from(max_out).min(tokens.saturating_sub(max_in));
        let limit = self.0.limits.limit(BudgetKind::ModelTokens).max(1);
        Ok(CallBudget {
            max_input_tokens: u32::try_from(max_in).unwrap_or(u32::MAX),
            max_output_tokens: u32::try_from(max_out).unwrap_or(u32::MAX),
            max_cost_usd_micros: None,
            deadline: self.deadline(),
            remaining_fraction: (tokens as f64 / limit as f64) as f32,
            max_attempts: 3,
        })
    }

    /// Applies the candidate cap to candidates already ranked best first. Candidates beyond
    /// `max_candidate_findings` are returned (not discarded) so the caller can persist them as
    /// `SUPPRESSED_POLICY { rule: "candidate_cap" }`, and each is recorded as a skipped unit.
    pub fn cap_candidates<T>(
        &self,
        ranked: Vec<T>,
        unit: impl Fn(&T) -> String,
    ) -> (Vec<T>, Vec<T>) {
        let mut kept = Vec::new();
        let mut excess = Vec::new();
        for candidate in ranked {
            match self.try_reserve(BudgetKind::Candidates, 1) {
                Ok(r) => {
                    self.commit(r, 1);
                    kept.push(candidate);
                }
                Err(_) => {
                    self.record_skip(BudgetKind::Candidates, unit(&candidate));
                    excess.push(candidate);
                }
            }
        }
        (kept, excess)
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
    fn call_budget_never_exceeds_remaining() {
        let m = BudgetManager::new(small(1000, 10));
        let _r = m.try_reserve(BudgetKind::ModelTokens, 900).unwrap();
        let b = m
            .call_budget_for(TaskType::CorrectnessReview, 800, 400)
            .unwrap();
        assert!(u64::from(b.max_input_tokens) + u64::from(b.max_output_tokens) <= 100);
        assert!(b.remaining_fraction <= 0.11);
    }

    #[test]
    fn zero_budget_yields_summary_only_review() {
        let m = BudgetManager::new(small(0, 0));
        let e = m
            .call_budget_for(TaskType::CorrectnessReview, 10, 10)
            .unwrap_err();
        assert_eq!(e.remaining, 0);
        let m = BudgetManager::new(small(1000, 0));
        assert!(m
            .call_budget_for(TaskType::CorrectnessReview, 10, 10)
            .is_err());
    }

    #[test]
    fn candidate_cap_returns_excess_for_suppression() {
        let limits = ReviewBudget {
            max_candidate_findings: 2,
            ..ReviewBudget::DEFAULT
        };
        let m = BudgetManager::new(limits);
        let (kept, excess) = m.cap_candidates(vec!["a", "b", "c", "d"], |c| (*c).to_owned());
        assert_eq!(kept, ["a", "b"]);
        assert_eq!(excess, ["c", "d"]);
        assert_eq!(m.events().len(), 2);
        assert_eq!(m.events()[0].kind, BudgetKind::Candidates);
    }

    #[test]
    fn two_hundred_clusters_with_ten_call_budget_make_ten_calls() {
        let m = BudgetManager::new(small(1_000_000, 10));
        let mut calls = 0;
        for i in 0..200 {
            match m.try_reserve(BudgetKind::ModelCalls, 1) {
                Ok(r) => {
                    m.commit(r, 1);
                    calls += 1;
                }
                Err(_) => m.record_skip(BudgetKind::ModelCalls, format!("cluster-{i}")),
            }
        }
        assert_eq!(calls, 10);
        assert_eq!(m.events().len(), 190);
    }

    #[tokio::test(start_paused = true)]
    async fn deadline_cancels_run_token() {
        let limits = ReviewBudget {
            max_review_latency: std::time::Duration::from_secs(10),
            ..ReviewBudget::DEFAULT
        };
        let m = BudgetManager::new(limits);
        let token = CancellationToken::new();
        let _watch = m.watch_deadline(token.clone());
        assert!(!token.is_cancelled());
        tokio::time::sleep(std::time::Duration::from_secs(11)).await;
        assert!(token.is_cancelled());
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
