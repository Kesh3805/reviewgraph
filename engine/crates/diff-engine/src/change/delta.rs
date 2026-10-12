//! [`SymbolDelta`]: symbols whose hashes changed between base and head, independent of any hunk.
//!
//! The hunk map says where lines changed; the delta says which symbols changed. A symbol can
//! change without a hunk inside its own lines (a lineage rename moves an unedited symbol; a
//! hash-affecting edit whose lines belong to another symbol), so
//! [`crate::change::build_changed_symbols`] seeds from both.
//!
//! `diff-engine` must not depend on `incremental` (the dependency-direction table), so this type
//! is the neutral hand-off: the composition root converts the incremental `FileSymbolDiff`s
//! into [`DeltaEntry`] values, and [`SymbolDelta::from_units`] computes the same hash comparison
//! directly from the parsed units of the changed files.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use analysis_ir::identity::symbol_ids;
use analysis_ir::unit::{ParseStatus, ParsedUnit};
use review_core::change::FileChangeStatus;
use review_core::ids::SymbolId;
use review_core::location::{DiffSide, RepoPath};
use review_core::symbol::SymbolKind;

use crate::model::DiffModel;
use crate::symbol_map::UnitSource;

/// How a symbol changed according to the hashes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeltaKind {
    /// Only in head.
    Added,
    /// Only in base.
    Removed,
    /// On both sides with at least one different hash.
    Modified,
    /// Same symbol under a new id (lineage); hashes may or may not differ.
    Renamed,
}

/// One changed symbol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeltaEntry {
    /// Id on `side` (the head id for everything except `Removed`).
    pub id: SymbolId,
    /// Symbol kind.
    pub kind: SymbolKind,
    /// `Base` for removed symbols, `Head` otherwise.
    pub side: DiffSide,
    /// File on `side`.
    pub path: RepoPath,
    /// Index in the unit of `side`.
    pub local: u32,
    /// Index in the base unit, for `Head` entries that exist in base.
    pub base_local: Option<u32>,
    /// Base id when it differs from `id`.
    pub base_id: Option<SymbolId>,
    /// Base file when the symbol exists in base.
    pub base_path: Option<RepoPath>,
    /// What happened.
    pub change: DeltaKind,
}

/// The hash-level symbol changes of a diff.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SymbolDelta {
    /// Entries sorted by `(path, local, side)`.
    pub entries: Vec<DeltaEntry>,
}

fn usable(unit: &Option<Arc<ParsedUnit>>) -> Option<&ParsedUnit> {
    unit.as_deref()
        .filter(|u| !matches!(u.status, ParseStatus::Failed { .. }))
}

fn removed_entry(
    unit: &ParsedUnit,
    ids: &[SymbolId],
    index: usize,
    base_path: &RepoPath,
) -> Option<DeltaEntry> {
    let id = ids.get(index)?;
    let kind = unit
        .symbols
        .get(index)
        .map_or(SymbolKind::Module, |s| s.kind);
    Some(DeltaEntry {
        id: id.clone(),
        kind,
        side: DiffSide::Base,
        path: base_path.clone(),
        local: index as u32,
        base_local: Some(index as u32),
        base_id: None,
        base_path: Some(base_path.clone()),
        change: DeltaKind::Removed,
    })
}

impl SymbolDelta {
    /// Compare the units of every analysable changed file. `renames` maps a base symbol id to
    /// its head id (SID-005 lineage). The module symbol is never part of the delta; module-level
    /// edits are found through the hunk map. Files whose unit is missing or failed to parse on
    /// either side contribute nothing.
    pub fn from_units(
        diff: &DiffModel,
        units: &dyn UnitSource,
        renames: &BTreeMap<SymbolId, SymbolId>,
    ) -> Self {
        let mut entries = Vec::new();
        for file in &diff.files {
            if !file.disposition.is_analyze() {
                continue;
            }
            let path = &file.file.path;
            let base_path = file.file.old_path.as_ref().unwrap_or(path);
            let status = file.file.status;
            let head = (status != FileChangeStatus::Deleted)
                .then(|| units.unit(DiffSide::Head, path))
                .flatten();
            let base = (status != FileChangeStatus::Added)
                .then(|| units.unit(DiffSide::Base, base_path))
                .flatten();
            let (head, base) = (usable(&head), usable(&base));
            if (status != FileChangeStatus::Deleted && head.is_none())
                || (status != FileChangeStatus::Added && base.is_none())
            {
                continue;
            }
            Self::compare_file(&mut entries, path, base_path, head, base, renames);
        }
        entries.sort_by(|a, b| {
            (a.path.as_str(), a.local, a.side).cmp(&(b.path.as_str(), b.local, b.side))
        });
        Self { entries }
    }

    fn compare_file(
        out: &mut Vec<DeltaEntry>,
        path: &RepoPath,
        base_path: &RepoPath,
        head: Option<&ParsedUnit>,
        base: Option<&ParsedUnit>,
        renames: &BTreeMap<SymbolId, SymbolId>,
    ) {
        let head_ids: Vec<SymbolId> = head.map(symbol_ids).unwrap_or_default();
        let base_ids: Vec<SymbolId> = base.map(symbol_ids).unwrap_or_default();
        let head_by_id: HashMap<&SymbolId, u32> = head_ids
            .iter()
            .enumerate()
            .map(|(i, id)| (id, i as u32))
            .collect();
        let mut matched: Vec<bool> = vec![false; head_ids.len()];

        if let Some(base_unit) = base {
            for (bi, bid) in base_ids.iter().enumerate().skip(1) {
                let target = renames.get(bid).unwrap_or(bid);
                let found = head.zip(head_by_id.get(target).copied().filter(|&hi| hi != 0));
                let Some((head_unit, hi)) = found else {
                    out.extend(removed_entry(base_unit, &base_ids, bi, base_path));
                    continue;
                };
                if let Some(m) = matched.get_mut(hi as usize) {
                    *m = true;
                }
                let (Some(b), Some(h)) = (
                    base_unit.symbols.get(bi),
                    head_unit.symbols.get(hi as usize),
                ) else {
                    continue;
                };
                let differs = b.signature_hash != h.signature_hash
                    || b.body_hash != h.body_hash
                    || b.attr_hash != h.attr_hash;
                let renamed = target != bid;
                if differs || renamed {
                    out.push(DeltaEntry {
                        id: target.clone(),
                        kind: h.kind,
                        side: DiffSide::Head,
                        path: path.clone(),
                        local: hi,
                        base_local: Some(bi as u32),
                        base_id: renamed.then(|| bid.clone()),
                        base_path: Some(base_path.clone()),
                        change: if renamed {
                            DeltaKind::Renamed
                        } else {
                            DeltaKind::Modified
                        },
                    });
                }
            }
        }

        if let Some(head_unit) = head {
            for (hi, hid) in head_ids.iter().enumerate().skip(1) {
                if matched.get(hi).copied().unwrap_or(false) {
                    continue;
                }
                let kind = head_unit
                    .symbols
                    .get(hi)
                    .map_or(SymbolKind::Module, |s| s.kind);
                out.push(DeltaEntry {
                    id: hid.clone(),
                    kind,
                    side: DiffSide::Head,
                    path: path.clone(),
                    local: hi as u32,
                    base_local: None,
                    base_id: None,
                    base_path: None,
                    change: DeltaKind::Added,
                });
            }
        }
    }
}
