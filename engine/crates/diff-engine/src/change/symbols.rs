//! `ChangedSymbol` construction (CHG-001): joins the hunk-to-symbol map (where) with the
//! per-side symbol hashes (what), carrying base and head versions.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use analysis_ir::symbol::IrSymbol;
use analysis_ir::unit::ParsedUnit;
use review_core::change::{ChangedSymbol, SymbolChange};
use review_core::ids::SymbolId;
use review_core::location::{DiffSide, Position, RepoPath, SourceRange};
use review_core::symbol::{Hash128, SymbolKind};
use serde::Serialize;

use crate::disposition::FileDisposition;
use crate::model::DiffModel;
use crate::symbol_map::{SymbolHit, SymbolMap, UnitSource};

/// Configuration of [`build_changed_symbols`].
#[derive(Debug, Clone)]
pub struct ChangeCfg {
    /// Output cap; extra records are dropped and `truncated` is set. Default 5,000.
    pub max_symbols: usize,
    /// Head path to base path for renamed files.
    pub base_paths: BTreeMap<RepoPath, RepoPath>,
    /// Files that are not analysed as source (generated, vendored, ...).
    pub generated: BTreeSet<RepoPath>,
}

impl Default for ChangeCfg {
    fn default() -> Self {
        Self {
            max_symbols: 5_000,
            base_paths: BTreeMap::new(),
            generated: BTreeSet::new(),
        }
    }
}

impl ChangeCfg {
    /// Derive renamed-file base paths and generated files from a [`DiffModel`].
    pub fn from_diff(diff: &DiffModel) -> Self {
        let mut cfg = Self::default();
        for f in &diff.files {
            if let Some(old) = &f.file.old_path {
                cfg.base_paths.insert(f.file.path.clone(), old.clone());
            }
            if f.disposition != FileDisposition::Analyze {
                cfg.generated.insert(f.file.path.clone());
            }
        }
        cfg
    }
}

/// One side of a changed symbol. Carries no source text.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SymbolVersion {
    /// Symbol id on this side.
    pub id: SymbolId,
    /// File on this side.
    pub path: RepoPath,
    /// Symbol kind.
    pub kind: SymbolKind,
    /// Full range.
    pub range: SourceRange,
    /// Body range, if the symbol has a body.
    pub body_range: Option<SourceRange>,
    /// Display signature (at most 512 characters).
    pub signature: Option<String>,
    /// Signature hash.
    pub signature_hash: Hash128,
    /// Body hash.
    pub body_hash: Hash128,
    /// Attribute hash (decorators, modifiers, heritage).
    pub attr_hash: Hash128,
    /// Index of the symbol in its unit.
    pub local: u32,
}

/// Flags of a changed symbol.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct SymbolFlags {
    /// The file is generated or otherwise not analysed as source.
    pub generated: bool,
}

/// A [`ChangedSymbol`] with both versions and flags.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SymbolRecord {
    /// The DOM-005 record (`side`, `range`, `change`).
    pub symbol: ChangedSymbol,
    /// Base version; `None` for added symbols.
    pub base: Option<SymbolVersion>,
    /// Head version; `None` for removed symbols.
    pub head: Option<SymbolVersion>,
    /// Only comments, whitespace or other hash-neutral edits.
    pub cosmetic: bool,
    /// Indices into the file's hunks that touch the symbol.
    pub hunk_ids: Vec<u32>,
    /// Symbol flags.
    pub flags: SymbolFlags,
}

/// Result of [`build_changed_symbols`].
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ChangedSymbolSet {
    /// Records sorted by `(path, range.start, symbol_id)`.
    pub symbols: Vec<SymbolRecord>,
    /// The cap was reached and records were dropped.
    pub truncated: bool,
    /// Files whose unit was unavailable (coverage note; versions are absent).
    pub missing_units: Vec<RepoPath>,
}

fn is_container(kind: SymbolKind) -> bool {
    matches!(
        kind,
        SymbolKind::Class | SymbolKind::Interface | SymbolKind::Namespace | SymbolKind::Enum
    )
}

fn version(
    unit: &Option<Arc<ParsedUnit>>,
    id: &SymbolId,
    path: &RepoPath,
    kind: SymbolKind,
    local: u32,
) -> Option<SymbolVersion> {
    let s: &IrSymbol = unit.as_ref()?.symbols.get(local as usize)?;
    Some(SymbolVersion {
        id: id.clone(),
        path: path.clone(),
        kind,
        range: s.range,
        body_range: s.body_range,
        signature: s.signature.clone(),
        signature_hash: s.signature_hash,
        body_hash: s.body_hash,
        attr_hash: s.attr_hash,
        local,
    })
}

/// Build the changed-symbol records from the hunk map and the units of both sides.
pub fn build_changed_symbols(
    map: &SymbolMap,
    units: &dyn UnitSource,
    cfg: &ChangeCfg,
) -> ChangedSymbolSet {
    let mut out = ChangedSymbolSet::default();
    for hit in &map.hits {
        if let Some(rec) = record_for(hit, units, cfg, &mut out.missing_units) {
            out.symbols.push(rec);
        }
    }
    out.symbols.sort_by(|a, b| {
        (
            a.symbol.path.as_str(),
            a.symbol.range.start,
            a.symbol.symbol_id.as_str(),
        )
            .cmp(&(
                b.symbol.path.as_str(),
                b.symbol.range.start,
                b.symbol.symbol_id.as_str(),
            ))
    });
    out.missing_units.sort();
    out.missing_units.dedup();
    if out.symbols.len() > cfg.max_symbols {
        out.symbols.truncate(cfg.max_symbols);
        out.truncated = true;
    }
    out
}

fn record_for(
    hit: &SymbolHit,
    units: &dyn UnitSource,
    cfg: &ChangeCfg,
    missing: &mut Vec<RepoPath>,
) -> Option<SymbolRecord> {
    let generated = cfg.generated.contains(&hit.path);
    let (base, head, change, cosmetic) = match hit.side {
        DiffSide::Base => {
            let unit = units.unit(DiffSide::Base, &hit.path);
            if unit.is_none() {
                missing.push(hit.path.clone());
            }
            let base = version(&unit, &hit.symbol_id, &hit.path, hit.kind, hit.local);
            (base, None, SymbolChange::Removed, false)
        }
        DiffSide::Head => {
            let head_unit = units.unit(DiffSide::Head, &hit.path);
            if head_unit.is_none() {
                missing.push(hit.path.clone());
            }
            let head = version(&head_unit, &hit.symbol_id, &hit.path, hit.kind, hit.local);
            let Some(base_local) = hit.base_local else {
                return Some(finish(
                    hit,
                    None,
                    head,
                    SymbolChange::Added,
                    false,
                    generated,
                ));
            };
            let base_path = cfg.base_paths.get(&hit.path).unwrap_or(&hit.path);
            let base_unit = units.unit(DiffSide::Base, base_path);
            if base_unit.is_none() {
                missing.push(base_path.clone());
            }
            let base_id = hit
                .base_symbol_id
                .clone()
                .unwrap_or_else(|| hit.symbol_id.clone());
            let base = version(&base_unit, &base_id, base_path, hit.kind, base_local);

            let (sig, body, attrs) = match (&base, &head) {
                (Some(b), Some(h)) => (
                    b.signature_hash != h.signature_hash,
                    b.body_hash != h.body_hash,
                    b.attr_hash != h.attr_hash,
                ),
                // Hashes unavailable: trust the line mapping.
                _ => (false, hit.touches_code, false),
            };
            let renamed = base_id != hit.symbol_id;
            // A container is changed only by its own header, decorators or heritage.
            if is_container(hit.kind) && !renamed && !sig && !attrs {
                return None;
            }
            let change = if renamed {
                SymbolChange::Renamed {
                    from: base_id,
                    // Coarse: identical body is a pure rename, otherwise body changed too.
                    similarity: if body { 0.5 } else { 1.0 },
                }
            } else {
                SymbolChange::Modified {
                    signature: sig,
                    body,
                    attrs,
                }
            };
            (base, head, change, !renamed && !sig && !body && !attrs)
        }
    };
    Some(finish(hit, base, head, change, cosmetic, generated))
}

fn finish(
    hit: &SymbolHit,
    base: Option<SymbolVersion>,
    head: Option<SymbolVersion>,
    change: SymbolChange,
    cosmetic: bool,
    generated: bool,
) -> SymbolRecord {
    let side = hit.side;
    let range = match side {
        DiffSide::Head => head.as_ref().map(|v| v.range),
        DiffSide::Base => base.as_ref().map(|v| v.range),
    }
    .unwrap_or_else(|| {
        // No unit: fall back to the first changed line.
        let line = hit.ranges.first().map_or(1, |r| r.start);
        let pos = Position { line, column: 0 };
        SourceRange {
            start: pos,
            end: pos,
        }
    });
    SymbolRecord {
        symbol: ChangedSymbol {
            symbol_key: hit.key,
            symbol_id: hit.symbol_id.clone(),
            path: hit.path.clone(),
            side,
            range,
            change,
        },
        base,
        head,
        cosmetic,
        hunk_ids: hit.hunk_ids.clone(),
        flags: SymbolFlags { generated },
    }
}
