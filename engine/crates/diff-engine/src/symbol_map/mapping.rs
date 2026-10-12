//! [`map_hunks`] and the per-file [`map_file`] (DIFF-006).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ops::Range;

use analysis_ir::identity::symbol_ids;
use analysis_ir::unit::{ParseStatus, ParsedUnit};
use rayon::prelude::*;
use review_core::change::FileChangeStatus;
use review_core::ids::{SymbolId, SymbolKey};
use review_core::location::{DiffSide, RepoPath};
use review_core::symbol::SymbolKind;

use super::intervals::{line_span, HitScope, LineIndex};
use super::{MapConfig, SymbolHit, SymbolMap, UnitSource, UnmappedRange, UnmappedReason};
use crate::hunks::{changed_ranges, DiffHunk, LineKind};
use crate::metrics;
use crate::model::{DiffModel, FileDiff};

/// Map every changed file of `diff`. Files are processed in parallel and merged in path order.
pub fn map_hunks(diff: &DiffModel, units: &dyn UnitSource, cfg: &MapConfig) -> SymbolMap {
    let span = tracing::info_span!(
        "symbol_mapping",
        files = diff.files.len(),
        hits = tracing::field::Empty,
        deleted_symbols = tracing::field::Empty,
        module_level = tracing::field::Empty,
        unmapped = tracing::field::Empty,
    );
    let _enter = span.enter();
    let started = std::time::Instant::now();

    let per_file: Vec<SymbolMap> = diff
        .files
        .par_iter()
        .map(|file| {
            let head = (file.file.status != FileChangeStatus::Deleted)
                .then(|| units.unit(DiffSide::Head, &file.file.path))
                .flatten();
            let base_path = file.file.old_path.as_ref().unwrap_or(&file.file.path);
            let base = (file.file.status != FileChangeStatus::Added)
                .then(|| units.unit(DiffSide::Base, base_path))
                .flatten();
            map_file(file, head.as_deref(), base.as_deref(), cfg)
        })
        .collect();

    let mut out = SymbolMap::default();
    for part in per_file {
        out.hits.extend(part.hits);
        out.unmapped.extend(part.unmapped);
    }
    out.hits
        .sort_by(|a, b| (a.path.as_str(), a.side, a.key).cmp(&(b.path.as_str(), b.side, b.key)));
    out.unmapped.sort_by(|a, b| {
        (a.path.as_str(), a.side, a.range.start).cmp(&(b.path.as_str(), b.side, b.range.start))
    });
    for u in &out.unmapped {
        metrics::record_unmapped(u.reason.label());
    }
    span.record("hits", out.hits.len());
    span.record(
        "deleted_symbols",
        out.hits.iter().filter(|h| h.side == DiffSide::Base).count(),
    );
    span.record(
        "module_level",
        out.hits
            .iter()
            .filter(|h| h.scope == HitScope::ModuleLevel)
            .count(),
    );
    span.record("unmapped", out.unmapped.len());
    metrics::record_symbol_map_duration(started.elapsed().as_secs_f64());
    out
}

/// Accumulated lines of one symbol on one side.
#[derive(Debug, Default)]
struct Acc {
    lines: BTreeSet<u32>,
    old_lines: BTreeSet<u32>,
    scope: Option<HitScope>,
    base_local: Option<u32>,
}

impl Acc {
    fn scope(&mut self, scope: HitScope) {
        self.scope = Some(self.scope.map_or(scope, |s| s.max(scope)));
    }
}

/// Usable unit or the reason it is not.
fn usable(unit: Option<&ParsedUnit>) -> Result<&ParsedUnit, UnmappedReason> {
    match unit {
        None => Err(UnmappedReason::NoUnit),
        Some(u) if matches!(u.status, ParseStatus::Failed { .. }) => {
            Err(UnmappedReason::ParseFailed)
        }
        Some(u) => Ok(u),
    }
}

/// Map one file. `head` is the unit of `file.path` on the head side, `base` the unit of
/// `old_path` (or `path`) on the base side.
pub fn map_file(
    file: &FileDiff,
    head: Option<&ParsedUnit>,
    base: Option<&ParsedUnit>,
    cfg: &MapConfig,
) -> SymbolMap {
    let path = &file.file.path;
    let base_path = file.file.old_path.as_ref().unwrap_or(path);
    let (changed_old, changed_new) = changed_ranges(&file.hunks);
    let mut out = SymbolMap::default();

    if !file.disposition.is_analyze() {
        let reason = UnmappedReason::Disposition {
            disposition: file.disposition,
        };
        push_unmapped(&mut out, path, DiffSide::Head, &changed_new, reason);
        push_unmapped(&mut out, base_path, DiffSide::Base, &changed_old, reason);
        return out;
    }

    let trivia_new = trivia_by_line(&file.hunks, LineKind::Add);
    let trivia_old = trivia_by_line(&file.hunks, LineKind::Del);
    let hunk_new = hunk_by_line(&file.hunks, LineKind::Add);
    let hunk_old = hunk_by_line(&file.hunks, LineKind::Del);

    // Head side.
    let head_unit = if changed_new.is_empty() && changed_old.is_empty() {
        None
    } else {
        match usable(head) {
            Ok(u) => Some(u),
            Err(reason) => {
                push_unmapped(&mut out, path, DiffSide::Head, &changed_new, reason);
                None
            }
        }
    };
    let head_ids: Vec<SymbolId> = head_unit.map(symbol_ids).unwrap_or_default();
    let head_by_id: HashMap<&SymbolId, u32> = head_ids
        .iter()
        .enumerate()
        .map(|(i, id)| (id, i as u32))
        .collect();
    let head_index = head_unit.map(|u| LineIndex::build(u, u.stats.lines));
    let mut head_acc: BTreeMap<u32, Acc> = BTreeMap::new();
    if let Some(index) = &head_index {
        for line in lines_of(&changed_new) {
            match index.owner(line) {
                None => push_unmapped_line(&mut out, path, DiffSide::Head, line),
                Some(owner) => {
                    let (local, scope) = owner.unwrap_or((0, HitScope::ModuleLevel));
                    let acc = head_acc.entry(local).or_default();
                    acc.lines.insert(line);
                    acc.scope(scope);
                }
            }
        }
    }

    // Base side.
    let mut base_acc: BTreeMap<u32, Acc> = BTreeMap::new();
    let base_unit = if changed_old.is_empty() {
        None
    } else {
        match usable(base) {
            Ok(u) => Some(u),
            Err(reason) => {
                push_unmapped(&mut out, base_path, DiffSide::Base, &changed_old, reason);
                None
            }
        }
    };
    let base_ids: Vec<SymbolId> = base_unit.map(symbol_ids).unwrap_or_default();
    if let Some(unit) = base_unit {
        let index = LineIndex::build(unit, unit.stats.lines);
        for line in lines_of(&changed_old) {
            let Some(owner) = index.owner(line) else {
                push_unmapped_line(&mut out, base_path, DiffSide::Base, line);
                continue;
            };
            let (local, scope) = owner.unwrap_or((0, HitScope::ModuleLevel));
            // Where does this base symbol live in head?
            let head_local = if head_unit.is_none() {
                None
            } else if local == 0 {
                Some(0)
            } else {
                base_ids.get(local as usize).and_then(|id| {
                    let target = cfg.renames.get(id).unwrap_or(id);
                    head_by_id.get(target).copied()
                })
            };
            match head_local {
                Some(h) => {
                    let acc = head_acc.entry(h).or_default();
                    acc.old_lines.insert(line);
                    if acc.base_local.is_none() {
                        acc.base_local = Some(local);
                    }
                    acc.scope(scope);
                }
                None => {
                    let acc = base_acc.entry(local).or_default();
                    acc.lines.insert(line);
                    acc.scope(scope);
                }
            }
        }
    }

    let changed_new_set: BTreeSet<u32> = lines_of(&changed_new).collect();
    let changed_old_set: BTreeSet<u32> = lines_of(&changed_old).collect();
    if let Some(unit) = head_unit {
        for (local, acc) in head_acc {
            let Some(id) = head_ids.get(local as usize) else {
                continue;
            };
            let kind = unit
                .symbols
                .get(local as usize)
                .map_or(SymbolKind::Module, |s| s.kind);
            let whole = local != 0
                && acc.old_lines.is_empty()
                && unit.symbols.get(local as usize).is_some_and(|s| {
                    let (a, b) = line_span(&s.range);
                    (a..=b).all(|l| changed_new_set.contains(&l))
                });
            let base_symbol_id = acc
                .base_local
                .and_then(|b| base_ids.get(b as usize))
                .filter(|b| *b != id)
                .cloned();
            let base_local = acc.base_local.or_else(|| {
                // A head symbol touched only on the new side may still exist in base.
                let target = cfg
                    .renames
                    .iter()
                    .find(|(_, h)| *h == id)
                    .map(|(b, _)| b)
                    .unwrap_or(id);
                base_ids.iter().position(|b| b == target).map(|p| p as u32)
            });
            out.hits.push(SymbolHit {
                key: SymbolKey::of(id),
                symbol_id: id.clone(),
                kind,
                side: DiffSide::Head,
                path: path.clone(),
                ranges: to_ranges(&acc.lines),
                ranges_old: to_ranges(&acc.old_lines),
                hunk_ids: hunk_ids(&acc.lines, &hunk_new, &acc.old_lines, &hunk_old),
                scope: if local == 0 {
                    HitScope::ModuleLevel
                } else {
                    acc.scope.unwrap_or(HitScope::Body)
                },
                whole_symbol: whole,
                touches_code: touches(&acc.lines, &trivia_new)
                    || touches(&acc.old_lines, &trivia_old),
                local,
                base_local: if base_unit.is_some() {
                    base_local
                } else {
                    None
                },
                base_symbol_id,
            });
        }
    }
    if let Some(unit) = base_unit {
        for (local, acc) in base_acc {
            let Some(id) = base_ids.get(local as usize) else {
                continue;
            };
            let kind = unit
                .symbols
                .get(local as usize)
                .map_or(SymbolKind::Module, |s| s.kind);
            let whole = local != 0
                && unit.symbols.get(local as usize).is_some_and(|s| {
                    let (a, b) = line_span(&s.range);
                    (a..=b).all(|l| changed_old_set.contains(&l))
                });
            out.hits.push(SymbolHit {
                key: SymbolKey::of(id),
                symbol_id: id.clone(),
                kind,
                side: DiffSide::Base,
                path: base_path.clone(),
                ranges: to_ranges(&acc.lines),
                ranges_old: Vec::new(),
                hunk_ids: hunk_ids(&BTreeSet::new(), &hunk_new, &acc.lines, &hunk_old),
                scope: if local == 0 {
                    HitScope::ModuleLevel
                } else {
                    acc.scope.unwrap_or(HitScope::Body)
                },
                whole_symbol: whole,
                touches_code: touches(&acc.lines, &trivia_old),
                local,
                base_local: Some(local),
                base_symbol_id: None,
            });
        }
    }
    out.unmapped = merge_unmapped(std::mem::take(&mut out.unmapped));
    out
}

fn lines_of(ranges: &[Range<u32>]) -> impl Iterator<Item = u32> + '_ {
    ranges.iter().flat_map(|r| r.clone())
}

fn to_ranges(lines: &BTreeSet<u32>) -> Vec<Range<u32>> {
    let mut out: Vec<Range<u32>> = Vec::new();
    for &l in lines {
        match out.last_mut() {
            Some(last) if last.end == l => last.end = l + 1,
            _ => out.push(l..l + 1),
        }
    }
    out
}

fn trivia_by_line(hunks: &[DiffHunk], kind: LineKind) -> HashMap<u32, bool> {
    let mut out = HashMap::new();
    for hunk in hunks {
        for line in hunk.lines.iter().filter(|l| l.kind == kind) {
            let no = match kind {
                LineKind::Del => line.old_no,
                LineKind::Add | LineKind::Context => line.new_no,
            };
            if let Some(no) = no {
                out.insert(no, line.trivia);
            }
        }
    }
    out
}

fn hunk_by_line(hunks: &[DiffHunk], kind: LineKind) -> HashMap<u32, u32> {
    let mut out = HashMap::new();
    for (i, hunk) in hunks.iter().enumerate() {
        for line in hunk.lines.iter().filter(|l| l.kind == kind) {
            let no = match kind {
                LineKind::Del => line.old_no,
                LineKind::Add | LineKind::Context => line.new_no,
            };
            if let Some(no) = no {
                out.insert(no, i as u32);
            }
        }
    }
    out
}

fn hunk_ids(
    new_lines: &BTreeSet<u32>,
    by_new: &HashMap<u32, u32>,
    old_lines: &BTreeSet<u32>,
    by_old: &HashMap<u32, u32>,
) -> Vec<u32> {
    let ids: BTreeSet<u32> = new_lines
        .iter()
        .filter_map(|l| by_new.get(l).copied())
        .chain(old_lines.iter().filter_map(|l| by_old.get(l).copied()))
        .collect();
    ids.into_iter().collect()
}

fn touches(lines: &BTreeSet<u32>, trivia: &HashMap<u32, bool>) -> bool {
    lines.iter().any(|l| trivia.get(l) != Some(&true))
}

fn push_unmapped(
    out: &mut SymbolMap,
    path: &RepoPath,
    side: DiffSide,
    ranges: &[Range<u32>],
    reason: UnmappedReason,
) {
    for range in ranges {
        out.unmapped.push(UnmappedRange {
            path: path.clone(),
            side,
            range: range.clone(),
            reason,
        });
    }
}

fn push_unmapped_line(out: &mut SymbolMap, path: &RepoPath, side: DiffSide, line: u32) {
    out.unmapped.push(UnmappedRange {
        path: path.clone(),
        side,
        range: line..line + 1,
        reason: UnmappedReason::BeyondEof,
    });
}

/// Merge adjacent unmapped lines with the same path, side and reason.
fn merge_unmapped(mut items: Vec<UnmappedRange>) -> Vec<UnmappedRange> {
    items.sort_by(|a, b| {
        (a.path.as_str(), a.side, a.range.start).cmp(&(b.path.as_str(), b.side, b.range.start))
    });
    let mut out: Vec<UnmappedRange> = Vec::with_capacity(items.len());
    for item in items {
        if let Some(last) = out.last_mut() {
            if last.path == item.path
                && last.side == item.side
                && last.reason == item.reason
                && last.range.end == item.range.start
            {
                last.range.end = item.range.end;
                continue;
            }
        }
        out.push(item);
    }
    out
}
