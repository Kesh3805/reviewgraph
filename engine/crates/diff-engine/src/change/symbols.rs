//! `ChangedSymbol` construction (CHG-001): joins the hunk-to-symbol map (where) with the
//! per-side symbol hashes (what), carrying base and head versions.
//!
//! Seeds come from two sources: the hits of the [`SymbolMap`] (symbols whose lines a hunk
//! touches) and the [`SymbolDelta`] (symbols whose hashes changed, even if no hunk touched their
//! own lines). Each seed becomes one [`SymbolRecord`].

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use analysis_ir::framework::FrameworkFactKind;
use analysis_ir::identity::symbol_ids;
use analysis_ir::symbol::{IrSymbol, LocalId, Modifiers, Visibility};
use analysis_ir::unit::ParsedUnit;
use codegraph::query::{EdgeFilter, GraphQuery};
use codegraph::{Direction, EdgeKind, EdgeKindSet, NodeFlags, NodeId};
use review_core::change::{ChangedSymbol, SymbolChange};
use review_core::ids::{SymbolId, SymbolKey};
use review_core::location::{DiffSide, Position, RepoPath, SourceRange};
use review_core::symbol::SymbolKind;
use serde::Serialize;

use super::delta::{DeltaEntry, SymbolDelta};
use super::graphs::GraphPair;
use super::versions::SymbolVersion;
use crate::disposition::FileDisposition;
use crate::metrics;
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
    /// Symbol lineage (SID-005): base symbol id to its head id.
    pub renames: BTreeMap<SymbolId, SymbolId>,
}

impl Default for ChangeCfg {
    fn default() -> Self {
        Self {
            max_symbols: 5_000,
            base_paths: BTreeMap::new(),
            generated: BTreeSet::new(),
            renames: BTreeMap::new(),
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

/// Flags of a changed symbol.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct SymbolFlags {
    /// The file is generated or otherwise not analysed as source.
    pub generated: bool,
    /// The symbol lives in a test file or test suite.
    pub test: bool,
    /// The symbol, or the class that contains it, is exported.
    pub exported: bool,
    /// Exported and publicly visible: other modules may depend on its signature.
    pub public_api: bool,
    /// Handles an HTTP route or a queue job on the head graph.
    pub entrypoint: bool,
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

/// A symbol to turn into a record, from a hunk hit or a delta entry.
#[derive(Debug, Clone)]
struct Seed {
    key: SymbolKey,
    id: SymbolId,
    kind: SymbolKind,
    side: DiffSide,
    path: RepoPath,
    local: u32,
    base_local: Option<u32>,
    base_id: Option<SymbolId>,
    base_path: Option<RepoPath>,
    hunk_ids: Vec<u32>,
    touches_code: bool,
    first_line: u32,
}

impl Seed {
    fn from_hit(hit: &SymbolHit, cfg: &ChangeCfg) -> Self {
        let base_path = match hit.side {
            DiffSide::Base => Some(hit.path.clone()),
            DiffSide::Head => hit
                .base_local
                .map(|_| cfg.base_paths.get(&hit.path).unwrap_or(&hit.path).clone()),
        };
        let first_line = hit
            .ranges
            .first()
            .or_else(|| hit.ranges_old.first())
            .map_or(1, |r| r.start);
        Self {
            key: hit.key,
            id: hit.symbol_id.clone(),
            kind: hit.kind,
            side: hit.side,
            path: hit.path.clone(),
            local: hit.local,
            base_local: hit.base_local,
            base_id: hit.base_symbol_id.clone(),
            base_path,
            hunk_ids: hit.hunk_ids.clone(),
            touches_code: hit.touches_code,
            first_line,
        }
    }

    fn from_entry(e: &DeltaEntry) -> Self {
        Self {
            key: SymbolKey::of(&e.id),
            id: e.id.clone(),
            kind: e.kind,
            side: e.side,
            path: e.path.clone(),
            local: e.local,
            base_local: e.base_local,
            base_id: e.base_id.clone(),
            base_path: e.base_path.clone(),
            hunk_ids: Vec::new(),
            touches_code: false,
            first_line: 1,
        }
    }
}

fn is_container(kind: SymbolKind) -> bool {
    matches!(
        kind,
        SymbolKind::Class | SymbolKind::Interface | SymbolKind::Namespace | SymbolKind::Enum
    )
}

/// Build the changed-symbol records from the hunk map, the hash delta and the units of both
/// sides. `pair` supplies graph-derived flags (entrypoint, test, exported) when available.
pub fn build_changed_symbols(
    map: &SymbolMap,
    delta: &SymbolDelta,
    pair: &GraphPair<'_>,
    units: &dyn UnitSource,
    cfg: &ChangeCfg,
) -> ChangedSymbolSet {
    let span = tracing::info_span!(
        "change.symbols",
        seeds = tracing::field::Empty,
        changed = tracing::field::Empty,
        cosmetic = tracing::field::Empty,
        added = tracing::field::Empty,
        removed = tracing::field::Empty,
        renamed = tracing::field::Empty,
        modified = tracing::field::Empty,
    );
    let _enter = span.enter();

    let mut seeds: BTreeMap<(SymbolKey, DiffSide), Seed> = BTreeMap::new();
    for hit in &map.hits {
        seeds.insert((hit.key, hit.side), Seed::from_hit(hit, cfg));
    }
    for entry in &delta.entries {
        let seed = Seed::from_entry(entry);
        seeds.entry((seed.key, seed.side)).or_insert(seed);
    }
    span.record("seeds", seeds.len());

    // The hunk map loads the base unit only when old-side lines changed, so an insert-only edit
    // arrives without its base symbol; look it up by id (or lineage) here.
    let reverse: BTreeMap<&SymbolId, &SymbolId> =
        cfg.renames.iter().map(|(b, h)| (h, b)).collect();
    let mut base_index: HashMap<RepoPath, Option<HashMap<SymbolId, u32>>> = HashMap::new();
    for seed in seeds.values_mut() {
        if seed.side != DiffSide::Head || seed.base_local.is_some() {
            continue;
        }
        let base_path = cfg
            .base_paths
            .get(&seed.path)
            .unwrap_or(&seed.path)
            .clone();
        let index = base_index.entry(base_path.clone()).or_insert_with(|| {
            units.unit(DiffSide::Base, &base_path).map(|u| {
                symbol_ids(&u)
                    .into_iter()
                    .enumerate()
                    .map(|(i, id)| (id, i as u32))
                    .collect()
            })
        });
        let Some(index) = index else { continue };
        let base_id = reverse.get(&seed.id).map_or_else(|| seed.id.clone(), |b| (*b).clone());
        if let Some(&local) = index.get(&base_id) {
            seed.base_local = Some(local);
            seed.base_path = Some(base_path);
            if base_id != seed.id {
                seed.base_id = Some(base_id);
            }
        }
    }

    let mut out = ChangedSymbolSet::default();
    for seed in seeds.values() {
        if let Some(rec) = record_for(seed, units, pair, cfg, &mut out.missing_units) {
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

    let mut counts = [0usize; 5];
    for rec in &out.symbols {
        let (slot, label) = match (&rec.symbol.change, rec.cosmetic) {
            (_, true) => (0, "cosmetic"),
            (SymbolChange::Added, _) => (1, "added"),
            (SymbolChange::Removed, _) => (2, "removed"),
            (SymbolChange::Renamed { .. }, _) => (3, "renamed"),
            (SymbolChange::Modified { .. }, _) => (4, "modified"),
        };
        if let Some(c) = counts.get_mut(slot) {
            *c += 1;
        }
        metrics::record_changed_symbol(label);
    }
    span.record("changed", out.symbols.len() - counts[0]);
    span.record("cosmetic", counts[0]);
    span.record("added", counts[1]);
    span.record("removed", counts[2]);
    span.record("renamed", counts[3]);
    span.record("modified", counts[4]);
    out
}

/// Similarity of a renamed symbol's two bodies in `[0, 1]`: 1.0 for an identical body, the
/// shingle Jaccard estimate otherwise (0.5 when no shingles were recorded).
fn rename_similarity(base: &IrSymbol, head: &IrSymbol) -> f32 {
    if base.body_hash == head.body_hash {
        return 1.0;
    }
    if base.body_shingles.is_empty() || head.body_shingles.is_empty() {
        return 0.5;
    }
    base.body_shingles
        .jaccard(&head.body_shingles)
        .clamp(0.0, 1.0)
}

fn record_for(
    seed: &Seed,
    units: &dyn UnitSource,
    pair: &GraphPair<'_>,
    cfg: &ChangeCfg,
    missing: &mut Vec<RepoPath>,
) -> Option<SymbolRecord> {
    let generated = cfg.generated.contains(&seed.path);
    match seed.side {
        DiffSide::Base => {
            let unit = units.unit(DiffSide::Base, &seed.path);
            if unit.is_none() {
                missing.push(seed.path.clone());
            }
            let base = unit
                .as_deref()
                .and_then(|u| SymbolVersion::of(u, &seed.id, &seed.path, seed.local));
            let flags = flags_for(seed, DiffSide::Base, unit.as_deref(), pair, generated);
            Some(finish(
                seed,
                base,
                None,
                SymbolChange::Removed,
                false,
                flags,
            ))
        }
        DiffSide::Head => {
            let head_unit = units.unit(DiffSide::Head, &seed.path);
            if head_unit.is_none() {
                missing.push(seed.path.clone());
            }
            let head = head_unit
                .as_deref()
                .and_then(|u| SymbolVersion::of(u, &seed.id, &seed.path, seed.local));
            let flags = flags_for(seed, DiffSide::Head, head_unit.as_deref(), pair, generated);
            let Some(base_local) = seed.base_local else {
                return Some(finish(seed, None, head, SymbolChange::Added, false, flags));
            };
            let base_path = seed.base_path.clone().unwrap_or_else(|| seed.path.clone());
            let base_unit: Option<Arc<ParsedUnit>> = units.unit(DiffSide::Base, &base_path);
            if base_unit.is_none() {
                missing.push(base_path.clone());
            }
            let base_id = seed.base_id.clone().unwrap_or_else(|| seed.id.clone());
            let base = base_unit
                .as_deref()
                .and_then(|u| SymbolVersion::of(u, &base_id, &base_path, base_local));

            let (sig, body, attrs) = match (&base, &head) {
                (Some(b), Some(h)) => (
                    b.signature_hash != h.signature_hash,
                    b.body_hash != h.body_hash,
                    b.attr_hash != h.attr_hash,
                ),
                // Hashes unavailable: trust the line mapping.
                _ => (false, seed.touches_code, false),
            };
            let renamed = base_id != seed.id;
            // A container is changed only by its own header, decorators or heritage.
            if is_container(seed.kind) && !renamed && !sig && !attrs {
                return None;
            }
            let change = if renamed {
                let similarity = match (
                    base_unit
                        .as_deref()
                        .and_then(|u| u.symbols.get(base_local as usize)),
                    head_unit
                        .as_deref()
                        .and_then(|u| u.symbols.get(seed.local as usize)),
                ) {
                    (Some(b), Some(h)) => rename_similarity(b, h),
                    _ => {
                        if seed.touches_code {
                            0.5
                        } else {
                            1.0
                        }
                    }
                };
                SymbolChange::Renamed {
                    from: base_id,
                    similarity,
                }
            } else if sig || body || attrs {
                SymbolChange::Modified {
                    signature: sig,
                    body,
                    attrs,
                }
            } else {
                // Hash-neutral edit (comments, whitespace): kept as a cosmetic record.
                SymbolChange::Modified {
                    signature: false,
                    body: false,
                    attrs: false,
                }
            };
            let cosmetic = !renamed && !sig && !body && !attrs;
            Some(finish(seed, base, head, change, cosmetic, flags))
        }
    }
}

fn finish(
    seed: &Seed,
    base: Option<SymbolVersion>,
    head: Option<SymbolVersion>,
    change: SymbolChange,
    cosmetic: bool,
    flags: SymbolFlags,
) -> SymbolRecord {
    let side = seed.side;
    let range = match side {
        DiffSide::Head => head.as_ref().map(|v| v.range),
        DiffSide::Base => base.as_ref().map(|v| v.range),
    }
    .unwrap_or_else(|| {
        // No unit: fall back to the first changed line.
        let pos = Position {
            line: seed.first_line,
            column: 0,
        };
        SourceRange {
            start: pos,
            end: pos,
        }
    });
    SymbolRecord {
        symbol: ChangedSymbol {
            symbol_key: seed.key,
            symbol_id: seed.id.clone(),
            path: seed.path.clone(),
            side,
            range,
            change,
        },
        base,
        head,
        cosmetic,
        hunk_ids: seed.hunk_ids.clone(),
        flags,
    }
}

/// Whether a path is a test file by convention.
pub(crate) fn is_test_path(path: &str) -> bool {
    path.contains(".spec.")
        || path.contains(".test.")
        || path.contains(".e2e-spec.")
        || path.contains("/__tests__/")
        || path.contains("/test/")
        || path.contains("/tests/")
        || path.starts_with("test/")
        || path.starts_with("tests/")
        || path.starts_with("__tests__/")
}

/// Exported / public-API flags of `local` in `unit`: the symbol itself or an enclosing class is
/// exported, and the symbol is publicly visible.
fn exposure(unit: &ParsedUnit, local: u32) -> (bool, bool) {
    let Some(symbol) = unit.symbols.get(local as usize) else {
        return (false, false);
    };
    if symbol.kind == SymbolKind::Module {
        return (false, false);
    }
    let mut exported = symbol.modifiers.contains(Modifiers::EXPORTED);
    let mut cursor = symbol.parent;
    let mut hops = 0;
    let mut visible = symbol.visibility == Visibility::Public;
    while let Some(LocalId(parent)) = cursor {
        hops += 1;
        if hops > 16 {
            break;
        }
        let Some(p) = unit.symbols.get(parent as usize) else {
            break;
        };
        if p.kind == SymbolKind::Module {
            break;
        }
        if p.modifiers.contains(Modifiers::EXPORTED) {
            exported = true;
        }
        visible &= p.visibility == Visibility::Public;
        cursor = p.parent;
    }
    (exported, exported && visible)
}

fn unit_entrypoint(unit: &ParsedUnit, local: u32) -> bool {
    unit.framework.iter().any(|f| {
        f.symbol == Some(LocalId(local))
            && matches!(
                f.kind,
                FrameworkFactKind::HttpRoute
                    | FrameworkFactKind::QueueConsumer
                    | FrameworkFactKind::QueueJobHandler
            )
    })
}

fn unit_test(unit: &ParsedUnit, local: u32) -> bool {
    unit.framework.iter().any(|f| {
        f.symbol == Some(LocalId(local))
            && matches!(
                f.kind,
                FrameworkFactKind::TestSuite | FrameworkFactKind::TestCase
            )
    })
}

fn graph_flags(g: &dyn GraphQuery, id: &SymbolId) -> (NodeFlags, bool) {
    let flags = g
        .node_by_id(id.as_str())
        .map_or(NodeFlags::EMPTY, |n| n.attrs.flags);
    let kinds = EdgeKindSet::of(EdgeKind::HandledBy).union(EdgeKindSet::of(EdgeKind::ConsumesJob));
    let key = NodeId::from_canonical(id.as_str()).key();
    let entry = g.degree(key, Direction::Both, &EdgeFilter::kinds(kinds)) > 0;
    (flags, entry)
}

fn flags_for(
    seed: &Seed,
    side: DiffSide,
    unit: Option<&ParsedUnit>,
    pair: &GraphPair<'_>,
    generated: bool,
) -> SymbolFlags {
    let mut flags = SymbolFlags {
        generated,
        test: is_test_path(seed.path.as_str()),
        ..SymbolFlags::default()
    };
    if let Some(u) = unit {
        let (exported, public_api) = exposure(u, seed.local);
        flags.exported = exported;
        flags.public_api = public_api;
        flags.test |= unit_test(u, seed.local);
        flags.entrypoint = unit_entrypoint(u, seed.local);
    }
    if let Some(g) = pair.graph(side) {
        let (node_flags, entry) = graph_flags(g, &seed.id);
        flags.test |= node_flags.contains(NodeFlags::TEST);
        flags.generated |= node_flags.contains(NodeFlags::GENERATED);
        if node_flags.contains(NodeFlags::EXPORTED) {
            flags.exported = true;
        }
        flags.entrypoint |= entry;
    }
    flags
}
