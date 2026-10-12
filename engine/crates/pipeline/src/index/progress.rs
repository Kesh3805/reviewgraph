//! Index progress reporting (IDX-002).

use async_trait::async_trait;

/// Phases of an index run, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum IndexPhase {
    Walking,
    Parsing,
    Linking,
    Building,
    Persisting,
    Done,
    Failed,
}

impl IndexPhase {
    /// Spelling stored in `index_runs.phase`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Walking => "walking",
            Self::Parsing => "parsing",
            Self::Linking => "linking",
            Self::Building => "building",
            Self::Persisting => "persisting",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }
}

/// Receives progress. Advisory: implementations must never fail the index.
#[async_trait]
pub trait ProgressSink: Send + Sync {
    async fn phase(&self, phase: IndexPhase);
    async fn files(&self, done: u64, total: u64);
}

/// Sink that discards everything (CLI and tests).
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopProgress;

#[async_trait]
impl ProgressSink for NoopProgress {
    async fn phase(&self, _phase: IndexPhase) {}
    async fn files(&self, _done: u64, _total: u64) {}
}

/// Decides when a file-count update is worth writing: at most once per `min_interval_ms` and
/// per `min_files` files (both must have elapsed), and always for the first and final counts.
#[derive(Debug, Clone)]
pub struct ProgressThrottle {
    min_interval_ms: u64,
    min_files: u64,
    last_ms: Option<u64>,
    last_files: u64,
}

impl ProgressThrottle {
    pub fn new(min_interval_ms: u64, min_files: u64) -> Self {
        Self {
            min_interval_ms,
            min_files,
            last_ms: None,
            last_files: 0,
        }
    }

    /// Defaults of the spec: one second and 1,000 files.
    pub fn standard() -> Self {
        Self::new(1_000, 1_000)
    }

    /// True when the caller should write `done` out of `total` at time `now_ms`.
    pub fn should_write(&mut self, now_ms: u64, done: u64, total: u64) -> bool {
        let due = match self.last_ms {
            None => true,
            Some(last) => {
                done >= total
                    || (now_ms.saturating_sub(last) >= self.min_interval_ms
                        && done.saturating_sub(self.last_files) >= self.min_files)
            }
        };
        if due {
            self.last_ms = Some(now_ms);
            self.last_files = done;
        }
        due
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn throttle_limits_writes() {
        let mut t = ProgressThrottle::standard();
        assert!(t.should_write(0, 1, 10_000));
        assert!(!t.should_write(10, 2, 10_000));
        assert!(!t.should_write(2_000, 500, 10_000));
        assert!(t.should_write(2_000, 1_200, 10_000));
        assert!(!t.should_write(2_100, 1_300, 10_000));
    }

    #[test]
    fn throttle_always_writes_the_final_count() {
        let mut t = ProgressThrottle::standard();
        assert!(t.should_write(0, 1, 5));
        assert!(t.should_write(1, 5, 5));
    }

    #[test]
    fn phases_order_and_spell() {
        assert!(IndexPhase::Walking < IndexPhase::Parsing);
        assert_eq!(IndexPhase::Persisting.as_str(), "persisting");
    }
}
