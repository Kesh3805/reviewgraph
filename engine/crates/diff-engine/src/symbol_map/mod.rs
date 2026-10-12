//! Hunk-to-symbol mapping (DIFF-006).
//!
//! Every zero-context changed range of an analysable file is mapped to the innermost enclosing
//! symbol: new-side ranges against the head unit, old-side ranges against the base unit (of
//! `old_path` for renames). A base symbol that still exists in head (directly or through a
//! rename) merges into the head hit; one that does not is a `Base`-side hit, i.e. a deleted
//! symbol. Lines outside every symbol map to the module symbol. Files that cannot be mapped are
//! listed in [`SymbolMap::unmapped`] with a reason, never dropped.
//!
//! The mapping reads symbols from [`analysis_ir::ParsedUnit`]s through [`UnitSource`] (the
//! same units the graph is built from), so it does not depend on a stored graph.

pub mod intervals;
pub mod mapping;

use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::Arc;

use analysis_ir::symbol::LocalId;
use analysis_ir::unit::ParsedUnit;
use review_core::ids::{SymbolId, SymbolKey};
use review_core::location::{DiffSide, RepoPath};
use review_core::symbol::SymbolKind;
use serde::Serialize;

use crate::disposition::FileDisposition;

pub use intervals::{HitScope, LineIndex};
pub use mapping::{map_file, map_hunks};

/// Access to the parsed units of both sides.
pub trait UnitSource: Send + Sync {
    /// The unit of `path` on `side`, if the file exists there and was analysed.
    fn unit(&self, side: DiffSide, path: &RepoPath) -> Option<Arc<ParsedUnit>>;
}

/// An in-memory [`UnitSource`].
#[derive(Debug, Clone, Default)]
pub struct UnitMap {
    /// Base-side units by path.
    pub base: BTreeMap<RepoPath, Arc<ParsedUnit>>,
    /// Head-side units by path.
    pub head: BTreeMap<RepoPath, Arc<ParsedUnit>>,
}

impl UnitMap {
    /// Add a unit on `side` under its own path.
    pub fn insert(&mut self, side: DiffSide, unit: ParsedUnit) {
        let path = unit.file.clone();
        let unit = Arc::new(unit);
        match side {
            DiffSide::Base => self.base.insert(path, unit),
            DiffSide::Head => self.head.insert(path, unit),
        };
    }
}

impl UnitSource for UnitMap {
    fn unit(&self, side: DiffSide, path: &RepoPath) -> Option<Arc<ParsedUnit>> {
        match side {
            DiffSide::Base => self.base.get(path).cloned(),
            DiffSide::Head => self.head.get(path).cloned(),
        }
    }
}

/// Mapping configuration.
#[derive(Debug, Clone, Default)]
pub struct MapConfig {
    /// Symbol lineage (SID-005): base symbol id to its head id for renamed/moved symbols, so a
    /// pure rename yields one hit on the new key.
    pub renames: BTreeMap<SymbolId, SymbolId>,
}

/// One symbol touched by the diff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SymbolHit {
    /// Storage key of `symbol_id`.
    pub key: SymbolKey,
    /// Canonical symbol id on `side`.
    pub symbol_id: SymbolId,
    /// Symbol kind.
    pub kind: SymbolKind,
    /// `Head` for symbols present in head, `Base` for deleted symbols.
    pub side: DiffSide,
    /// File on `side`.
    pub path: RepoPath,
    /// Changed line ranges inside the symbol on `side` (1-based, `[start, end)`).
    pub ranges: Vec<Range<u32>>,
    /// Old-side changed ranges merged into a `Head` hit (deletions inside a surviving symbol).
    pub ranges_old: Vec<Range<u32>>,
    /// Indices into the file's `hunks` that touch the symbol.
    pub hunk_ids: Vec<u32>,
    /// Most significant part of the symbol touched.
    pub scope: HitScope,
    /// Every line of the symbol is changed (added or removed as a whole).
    pub whole_symbol: bool,
    /// At least one changed line is code (not blank or comment-only).
    pub touches_code: bool,
    /// Index of the symbol in its side's unit.
    pub local: u32,
    /// Index of the symbol in the base unit, for `Head` hits that also exist in base.
    pub base_local: Option<u32>,
    /// Base-side id when it differs from `symbol_id` (renamed symbols).
    pub base_symbol_id: Option<SymbolId>,
}

impl SymbolHit {
    /// The symbol's local id on its side.
    pub fn local_id(&self) -> LocalId {
        LocalId(self.local)
    }
}

/// Why a changed range could not be mapped to a symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UnmappedReason {
    /// The file is not analysed (binary, generated, vendored, ...).
    Disposition {
        /// The file's disposition.
        disposition: FileDisposition,
    },
    /// No unit for the side (unsupported language or the analyzer did not run).
    NoUnit,
    /// The unit failed to parse.
    ParseFailed,
    /// The range lies past the end of the analysed file.
    BeyondEof,
}

impl UnmappedReason {
    /// Stable metric label.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Disposition { .. } => "disposition",
            Self::NoUnit => "no_unit",
            Self::ParseFailed => "parse_failed",
            Self::BeyondEof => "beyond_eof",
        }
    }
}

/// A changed range that maps to no symbol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnmappedRange {
    /// The file.
    pub path: RepoPath,
    /// Which side the lines are numbered on.
    pub side: DiffSide,
    /// The lines (1-based, `[start, end)`).
    pub range: Range<u32>,
    /// Why.
    pub reason: UnmappedReason,
}

/// Result of [`map_hunks`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SymbolMap {
    /// Hits sorted by `(path, side, key)`.
    pub hits: Vec<SymbolHit>,
    /// Ranges that map to no symbol, sorted by `(path, side, range)`.
    pub unmapped: Vec<UnmappedRange>,
}

impl SymbolMap {
    /// Hits of one file.
    pub fn hits_in<'a>(&'a self, path: &'a str) -> impl Iterator<Item = &'a SymbolHit> + 'a {
        self.hits.iter().filter(move |h| h.path.as_str() == path)
    }

    /// The hit with this symbol id, if any.
    pub fn hit(&self, symbol_id: &str) -> Option<&SymbolHit> {
        self.hits.iter().find(|h| h.symbol_id.as_str() == symbol_id)
    }
}
