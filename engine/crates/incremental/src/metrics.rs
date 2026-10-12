//! First-class incremental counters (INC-010, ADR-004).
//!
//! One [`IncrementalCounters`] exists per update attempt and is shared by the reparse, symbol,
//! re-link, delta and invalidation stages. It is deliberately not global: concurrent updates must
//! not contaminate each other's assertions. [`IncrementalCounters::snapshot`] freezes it into the
//! plain [`CountersSnapshot`] that is persisted in `snapshots.stats.incremental` and returned in
//! the job result.
//!
//! Semantics:
//!
//! * `files_reparsed_total` counts analyzer invocations or cache-served units for changed-hash
//!   paths only.
//! * `files_skipped_unchanged_total` counts changed-path entries whose content hash equals the
//!   base file's hash.
//! * `symbols_*` count identity-level transitions: a renamed pair is not also added and removed,
//!   and a cosmetic range move counts nowhere.
//! * `edges_added/removed` count identity-level changes; location-only changes are
//!   `edges_relocated`.
//! * `graph_invalidations_total` counts the rows of the invalidation set.

use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

/// Atomic counters shared by one update's stages.
#[derive(Debug, Default)]
pub struct IncrementalCounters {
    files_reparsed: AtomicU64,
    files_reparsed_analyzer: AtomicU64,
    files_reparsed_cache: AtomicU64,
    files_skipped_unchanged: AtomicU64,
    symbols_added: AtomicU64,
    symbols_removed: AtomicU64,
    symbols_modified: AtomicU64,
    symbols_renamed: AtomicU64,
    edges_added: AtomicU64,
    edges_removed: AtomicU64,
    edges_relocated: AtomicU64,
    graph_invalidations: AtomicU64,
    files_reparsed_for_relink: AtomicU64,
    inbound_files_relinked: AtomicU64,
}

/// A frozen copy of [`IncrementalCounters`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CountersSnapshot {
    pub files_reparsed_total: u64,
    pub files_reparsed_analyzer_total: u64,
    pub files_reparsed_cache_total: u64,
    pub files_skipped_unchanged_total: u64,
    pub symbols_added_total: u64,
    pub symbols_removed_total: u64,
    pub symbols_modified_total: u64,
    pub symbols_renamed_total: u64,
    pub edges_added_total: u64,
    pub edges_removed_total: u64,
    pub edges_relocated_total: u64,
    pub graph_invalidations_total: u64,
    pub files_reparsed_for_relink_total: u64,
    pub inbound_files_relinked_total: u64,
}

macro_rules! adders {
    ($($(#[$doc:meta])* $name:ident => $field:ident),* $(,)?) => {
        impl IncrementalCounters {
            $(
                $(#[$doc])*
                pub fn $name(&self, n: u64) {
                    self.$field.fetch_add(n, Ordering::Relaxed);
                }
            )*
        }
    };
}

adders! {
    /// Adds changed-hash files that were parsed or served from the unit cache.
    add_files_reparsed => files_reparsed,
    /// Adds files the analyzer actually parsed.
    add_files_reparsed_analyzer => files_reparsed_analyzer,
    /// Adds files served from the unit cache.
    add_files_reparsed_cache => files_reparsed_cache,
    /// Adds changed paths whose hash equalled the base hash.
    add_files_skipped_unchanged => files_skipped_unchanged,
    /// Adds added symbols.
    add_symbols_added => symbols_added,
    /// Adds removed symbols.
    add_symbols_removed => symbols_removed,
    /// Adds modified symbols.
    add_symbols_modified => symbols_modified,
    /// Adds renamed or moved symbols.
    add_symbols_renamed => symbols_renamed,
    /// Adds added edge identities.
    add_edges_added => edges_added,
    /// Adds removed edge identities.
    add_edges_removed => edges_removed,
    /// Adds edges whose identity stayed but whose payload changed.
    add_edges_relocated => edges_relocated,
    /// Adds invalidation rows.
    add_graph_invalidations => graph_invalidations,
    /// Adds unchanged files re-parsed because their IR blob was missing.
    add_files_reparsed_for_relink => files_reparsed_for_relink,
    /// Adds unchanged files re-linked because of inbound changes.
    add_inbound_files_relinked => inbound_files_relinked,
}

impl IncrementalCounters {
    /// A fresh set of zeroed counters.
    pub fn new() -> Self {
        Self::default()
    }

    /// Freezes the current values.
    pub fn snapshot(&self) -> CountersSnapshot {
        let get = |a: &AtomicU64| a.load(Ordering::Relaxed);
        CountersSnapshot {
            files_reparsed_total: get(&self.files_reparsed),
            files_reparsed_analyzer_total: get(&self.files_reparsed_analyzer),
            files_reparsed_cache_total: get(&self.files_reparsed_cache),
            files_skipped_unchanged_total: get(&self.files_skipped_unchanged),
            symbols_added_total: get(&self.symbols_added),
            symbols_removed_total: get(&self.symbols_removed),
            symbols_modified_total: get(&self.symbols_modified),
            symbols_renamed_total: get(&self.symbols_renamed),
            edges_added_total: get(&self.edges_added),
            edges_removed_total: get(&self.edges_removed),
            edges_relocated_total: get(&self.edges_relocated),
            graph_invalidations_total: get(&self.graph_invalidations),
            files_reparsed_for_relink_total: get(&self.files_reparsed_for_relink),
            inbound_files_relinked_total: get(&self.inbound_files_relinked),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_reflects_additions_and_roundtrips_json() {
        let counters = IncrementalCounters::new();
        counters.add_files_reparsed(2);
        counters.add_files_skipped_unchanged(1);
        counters.add_edges_relocated(3);
        let snapshot = counters.snapshot();
        assert_eq!(snapshot.files_reparsed_total, 2);
        assert_eq!(snapshot.files_skipped_unchanged_total, 1);
        assert_eq!(snapshot.edges_relocated_total, 3);
        let json = serde_json::to_string(&snapshot).unwrap_or_default();
        let back: Option<CountersSnapshot> = serde_json::from_str(&json).ok();
        assert_eq!(back, Some(snapshot));
    }

    #[test]
    fn counters_are_not_shared_between_instances() {
        let a = IncrementalCounters::new();
        let b = IncrementalCounters::new();
        a.add_symbols_added(5);
        assert_eq!(b.snapshot().symbols_added_total, 0);
    }
}
