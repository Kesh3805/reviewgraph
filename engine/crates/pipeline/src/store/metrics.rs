//! Process-wide stage cache counters (PIPE-005): `stage_cache_total{stage,result}` and
//! `stage_output_conflicts_total{stage}`. Tests (PIPE-009) read them to assert that a resumed run
//! hit the cache and that no stage produced two different outputs for one input.

use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StageCounts {
    pub hits: u64,
    pub misses: u64,
    pub conflicts: u64,
}

fn table() -> &'static Mutex<BTreeMap<String, StageCounts>> {
    static TABLE: OnceLock<Mutex<BTreeMap<String, StageCounts>>> = OnceLock::new();
    TABLE.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn bump(stage: &str, f: impl FnOnce(&mut StageCounts)) {
    if let Ok(mut t) = table().lock() {
        f(t.entry(stage.to_owned()).or_default());
    }
}

pub fn record_hit(stage: &str) {
    bump(stage, |c| c.hits += 1);
}

pub fn record_miss(stage: &str) {
    bump(stage, |c| c.misses += 1);
}

/// A second `put` for the same key carried a different output hash (`nondeterministic_stage`).
pub fn record_conflict(stage: &str) {
    bump(stage, |c| c.conflicts += 1);
}

/// Counts for one stage (zeros when never seen).
pub fn counts(stage: &str) -> StageCounts {
    table()
        .lock()
        .ok()
        .and_then(|t| t.get(stage).copied())
        .unwrap_or_default()
}

/// Total conflicts over all stages.
pub fn total_conflicts() -> u64 {
    table()
        .lock()
        .map(|t| t.values().map(|c| c.conflicts).sum())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_accumulate_per_stage() {
        record_hit("metrics_test_stage");
        record_hit("metrics_test_stage");
        record_miss("metrics_test_stage");
        let c = counts("metrics_test_stage");
        assert_eq!((c.hits, c.misses, c.conflicts), (2, 1, 0));
    }
}
