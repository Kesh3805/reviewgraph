//! Rename/move matching (SID-005, ADR-005).
//!
//! The driver: pool the symbols one snapshot removed and the other added, generate candidate pairs
//! with [`rules`], assign them one-to-one with [`assign`], and turn the accepted pairs into
//! `symbol_lineage` records. Everything is a pure function of immutable slices, so the result does
//! not depend on input order, thread count or how the caller batched the work.

pub mod assign;
pub mod rules;

use std::collections::{BTreeMap, BTreeSet};

use analysis_ir::ParsedUnit;
use review_core::ids::SymbolKey;
use review_core::matcher::CandidateEdge;

use crate::symbol_diff::{diff_units, symbol_refs};

pub use review_core::matcher::{
    ambiguity_notes, records_from_edges, tied_symbol_indices, MatchResult, MatcherConfig,
    SymbolRef, TiedSymbols,
};

/// A file rename known from the git diff. It only raises a candidate's priority: two files with
/// identical content still pair by hash, and a renamed file never creates a match by itself.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RenameHints {
    /// `old module path -> new module path`, from DIFF.
    pub paths: Vec<(String, String)>,
}

impl RenameHints {
    /// Builds hints from `(old path, new path)` pairs.
    pub fn from_pairs<'a>(pairs: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
        Self {
            paths: pairs
                .into_iter()
                .map(|(from, to)| (from.to_owned(), to.to_owned()))
                .collect(),
        }
    }

    /// Whether `to` is the new path of a renamed file that contains `from`.
    pub fn is_renamed(&self, from: &str, to: &str) -> bool {
        self.paths.iter().any(|(old, new)| old == from && new == to)
    }
}

/// Pairs removed and added symbols of the same kind.
///
/// The pools are the snapshot-wide sets SID-004 produced, so a symbol can move between files.
/// Candidates only exist for equal [`review_core::symbol::SymbolKind`]s, and members are only paired
/// inside matched containers once their parents are decided (see [`rules`]).
pub fn match_symbols(
    removed: &[SymbolRef],
    added: &[SymbolRef],
    cfg: &MatcherConfig,
    hints: &RenameHints,
) -> MatchResult {
    if removed.is_empty() || added.is_empty() {
        let mut unmatched_removed: Vec<SymbolKey> =
            removed.iter().map(|symbol| symbol.key).collect();
        let mut unmatched_added: Vec<SymbolKey> = added.iter().map(|symbol| symbol.key).collect();
        unmatched_removed.sort();
        unmatched_added.sort();
        return MatchResult {
            unmatched_removed,
            unmatched_added,
            ..MatchResult::default()
        };
    }
    let degraded = removed.len() > cfg.degrade_threshold || added.len() > cfg.degrade_threshold;
    let edges = rules::candidate_edges(removed, added, cfg, hints, degraded);
    // Symbols whose best candidate ties with another are dropped before assignment: guessing would
    // attach the wrong history to an unrelated symbol. The notes come from the full candidate set,
    // because the candidates that made a symbol ambiguous are exactly the ones dropped here.
    let ambiguous = ambiguity_notes(removed, added, &edges);
    let usable = undisputed(&edges, tied_symbol_indices(removed, added, &edges));
    let assigned = assign::assign(removed, added, usable, cfg);
    let mut result = records_from_edges(removed, added, &assigned, cfg);
    result.ambiguous = ambiguous;
    result.degraded = degraded;
    result
}

/// The candidates whose endpoints none of the tie rules forbids, in input order.
fn undisputed(edges: &[CandidateEdge], tied: TiedSymbols) -> Vec<CandidateEdge> {
    edges
        .iter()
        .filter(|edge| !tied.removed.contains(&edge.from) && !tied.added.contains(&edge.to))
        .cloned()
        .collect()
}

/// Convenience wrapper: diff every file, pool the removed and added symbols and match them.
pub fn match_units<'a>(
    base: impl IntoIterator<Item = &'a ParsedUnit>,
    head: impl IntoIterator<Item = &'a ParsedUnit>,
    cfg: &MatcherConfig,
    hints: &RenameHints,
) -> MatchResult {
    let (removed, added) = pools(base, head);
    match_symbols(&removed, &added, cfg, hints)
}

/// The snapshot-wide removed and added pools, sorted by id.
pub fn pools<'a>(
    base: impl IntoIterator<Item = &'a ParsedUnit>,
    head: impl IntoIterator<Item = &'a ParsedUnit>,
) -> (Vec<SymbolRef>, Vec<SymbolRef>) {
    let base_units: Vec<&ParsedUnit> = base.into_iter().collect();
    let head_units: Vec<&ParsedUnit> = head.into_iter().collect();
    let mut removed: Vec<SymbolRef> = Vec::new();
    let mut added: Vec<SymbolRef> = Vec::new();
    for (index, unit) in base_units.iter().enumerate() {
        let counterpart = head_units
            .iter()
            .find(|candidate| candidate.file == unit.file);
        let diff = diff_units(Some(unit), counterpart.copied());
        if !diff.is_known() {
            // An unparsable file contributes every symbol as removed and added, so the matcher
            // still sees the file's symbols instead of silently dropping them.
            removed.extend(symbol_refs(unit));
            continue;
        }
        for id in diff.removed() {
            if let Some(symbol) = unit.symbols.iter().find(|symbol| {
                analysis_ir::identity::symbol_id_of(unit, symbol.local_id.0).as_ref() == Some(id)
            }) {
                if let Some(reference) = crate::symbol_diff::symbol_ref(unit, symbol) {
                    removed.push(reference);
                }
            }
        }
        let _ = index;
    }
    for unit in &head_units {
        let counterpart = base_units
            .iter()
            .find(|candidate| candidate.file == unit.file);
        let diff = diff_units(counterpart.copied(), Some(unit));
        if !diff.is_known() {
            added.extend(symbol_refs(unit));
            continue;
        }
        for id in diff.added() {
            if let Some(symbol) = unit.symbols.iter().find(|symbol| {
                analysis_ir::identity::symbol_id_of(unit, symbol.local_id.0).as_ref() == Some(id)
            }) {
                if let Some(reference) = crate::symbol_diff::symbol_ref(unit, symbol) {
                    added.push(reference);
                }
            }
        }
    }
    sort_refs(&mut removed);
    sort_refs(&mut added);
    (removed, added)
}

fn sort_refs(refs: &mut Vec<SymbolRef>) {
    refs.sort_by(|left, right| left.id.as_str().cmp(right.id.as_str()));
    refs.dedup_by(|left, right| left.id == right.id);
}

/// Keys of every symbol in a pool, for reporting.
pub fn keys(refs: &[SymbolRef]) -> BTreeSet<SymbolKey> {
    refs.iter().map(|symbol| symbol.key).collect()
}

/// Groups a pool by kind, keeping the pool order.
pub fn by_kind(refs: &[SymbolRef]) -> BTreeMap<review_core::symbol::SymbolKind, Vec<SymbolRef>> {
    let mut out: BTreeMap<review_core::symbol::SymbolKind, Vec<SymbolRef>> = BTreeMap::new();
    for symbol in refs {
        out.entry(symbol.kind).or_default().push(symbol.clone());
    }
    out
}
