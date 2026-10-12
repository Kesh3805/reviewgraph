//! Tunables of a full index run (IDX-001).

use std::time::Duration;

/// Knobs for full index runs. Defaults follow the task specification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexConfig {
    /// Worker threads of the dedicated parse pool (`available_parallelism - 1`, at least 1).
    pub parse_threads: usize,
    /// Files above this size are skipped, not parsed.
    pub max_file_bytes: u64,
    /// File versions persisted per batch.
    pub persist_batch: usize,
    /// Per-file analyzer budget.
    pub parse_timeout: Duration,
}

impl IndexConfig {
    /// Thread count for a host with `available` logical CPUs.
    pub fn threads_for(available: usize) -> usize {
        available.saturating_sub(1).max(1)
    }

    /// Capacity of the parse-to-persist channel (2 x `persist_batch`).
    pub fn channel_capacity(&self) -> usize {
        self.persist_batch.max(1).saturating_mul(2)
    }
}

impl Default for IndexConfig {
    fn default() -> Self {
        let available = std::thread::available_parallelism().map_or(1, |n| n.get());
        Self {
            parse_threads: Self::threads_for(available),
            max_file_bytes: 1024 * 1024,
            persist_batch: 500,
            parse_timeout: Duration::from_secs(5),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn threads_never_drop_below_one() {
        assert_eq!(IndexConfig::threads_for(0), 1);
        assert_eq!(IndexConfig::threads_for(1), 1);
        assert_eq!(IndexConfig::threads_for(8), 7);
    }

    #[test]
    fn defaults_match_the_spec() {
        let c = IndexConfig::default();
        assert_eq!(c.max_file_bytes, 1024 * 1024);
        assert_eq!(c.persist_batch, 500);
        assert_eq!(c.parse_timeout, Duration::from_secs(5));
        assert_eq!(c.channel_capacity(), 1000);
        assert!(c.parse_threads >= 1);
    }
}
