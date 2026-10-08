//! `incremental` crate: per-file symbol diff (SID-004), rename/move matching and lineage (SID-005),
//! and the counters both report (ADR-004). See docs/architecture/target-architecture.md §2.
//!
//! Nothing here does I/O: the differ and the matcher are pure functions of parsed units, so the
//! indexer can run them inside rayon workers and the tests can compare two snapshots directly.

pub mod counters;
pub mod error;
pub mod lineage;
pub mod matcher;
pub mod symbol_diff;

pub use error::{Error, Result};
pub use review_core::lineage::{
    AmbiguityNote, LineageIndex, LineageRecord, MatchRule, SymbolLineageRow, SymbolTransition,
};
pub use review_core::matcher::{MatchResult, MatcherConfig, SymbolRef};
pub use symbol_diff::{diff_units, DiffCounts, FileSymbolDiff, SymbolChange, SymbolChangeKind};
