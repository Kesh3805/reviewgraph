//! Per-file symbol classification and the global rename/move pairing (INC-003).
//!
//! Every changed file is diffed by `NodeKey`: a key only at head is `added`, only at base is
//! `removed`, and a key on both sides is compared by its three hashes. The removed x added pools of
//! *all* changed files then go through the SID-005 matcher once, so a symbol moved between files is
//! paired. Both keys of a pair stay in `added`/`removed` (the delta still removes the old node and
//! adds the new one); only the counters and the lineage treat the pair as one rename.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use analysis_ir::{IrSymbol, ParsedUnit};
use codegraph::linker::symbol_node_id;
use codegraph::{LineageRecord, LineageTransition, NodeKey};
use rayon::prelude::*;
use review_core::lineage::SymbolTransition;
use review_core::location::RepoPath;
use review_core::matcher::{MatcherConfig, SymbolRef};
use review_core::symbol::ModulePath;

use crate::changeset::ChangeSet;
use crate::error::{Error, Result};
use crate::matcher::{match_symbols, RenameHints};
use crate::metrics::IncrementalCounters;
use crate::reparse::{FileOutcome, ReparseResult};
use crate::symbol_diff::{symbol_ref, ModifiedFlags};

/// What the name index needs to know about a symbol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameEntry {
    pub name: String,
    /// Nested symbols (class members, parameters) are indexed as members, the rest as top-level.
    pub is_member: bool,
}

/// One added or removed symbol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolRecord {
    pub key: NodeKey,
    pub file: RepoPath,
    pub entry: NameEntry,
}

/// The two sides of one changed file.
#[derive(Debug, Clone)]
pub struct FileChangeInput {
    pub path: RepoPath,
    /// The unit at the base commit, when the file existed there.
    pub base: Option<Arc<ParsedUnit>>,
    /// The unit at the head commit, when the file exists there and is parsed.
    pub head: Option<Arc<ParsedUnit>>,
}

/// The artifact other stages consume: which symbols changed and how.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SymbolChangeSet {
    /// Keys only at head, sorted by key.
    pub added: Vec<SymbolRecord>,
    /// Keys only at base, sorted by key.
    pub removed: Vec<SymbolRecord>,
    /// Keys on both sides with at least one changed hash.
    pub modified: Vec<(NodeKey, ModifiedFlags)>,
    /// Keys whose only change is the source range.
    pub moved_range: Vec<NodeKey>,
    /// Pairs the matcher decided are renames or moves.
    pub renamed: Vec<LineageRecord>,
    /// Symbols of changed files with no change at all.
    pub unchanged_in_changed_files: u32,
    /// Symbols the matcher left unpaired because candidates tied.
    pub ambiguous: u32,
    /// Every changed path (including deleted ones).
    pub changed_files: BTreeSet<RepoPath>,
}

/// Identity-level counts (renamed pairs are not double counted).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SymbolCounts {
    pub added: u64,
    pub removed: u64,
    pub modified: u64,
    pub renamed: u64,
}

impl SymbolChangeSet {
    /// Counters for ADR-004: added/removed exclude the symbols that were paired as renames.
    pub fn counts(&self) -> SymbolCounts {
        let renamed = self.renamed.len() as u64;
        SymbolCounts {
            added: (self.added.len() as u64).saturating_sub(renamed),
            removed: (self.removed.len() as u64).saturating_sub(renamed),
            modified: self.modified.len() as u64,
            renamed,
        }
    }

    /// Whether the signature or export flags of `key` changed (consumers re-link type users).
    pub fn signature_or_attr_changed(&self, key: &NodeKey) -> bool {
        self.modified.iter().any(|(k, flags)| {
            k == key
                && (flags.contains(ModifiedFlags::SIGNATURE)
                    || flags.contains(ModifiedFlags::ATTRIBUTES))
        })
    }
}

/// Rename hints for the matcher from the git-level renames.
pub fn rename_hints(changes: &ChangeSet) -> RenameHints {
    let pairs: Vec<(String, String)> = changes
        .rename_pairs()
        .into_iter()
        .map(|(old, new)| {
            (
                ModulePath::of(old).as_str().to_owned(),
                ModulePath::of(new).as_str().to_owned(),
            )
        })
        .collect();
    RenameHints::from_pairs(pairs.iter().map(|(a, b)| (a.as_str(), b.as_str())))
}

/// Turns reparse outcomes into per-file inputs. `base_unit` supplies the unit a path had at the
/// base commit (from the IR cache); `Unchanged` outcomes produce nothing.
///
/// A `Skipped` path is treated as removed: a full build would not contain it either.
pub fn change_inputs(
    reparse: &ReparseResult,
    base_unit: &dyn Fn(&RepoPath) -> Option<Arc<ParsedUnit>>,
) -> Vec<FileChangeInput> {
    let mut out = Vec::new();
    for outcome in &reparse.outcomes {
        match outcome {
            FileOutcome::Unchanged { .. } => {}
            FileOutcome::Reparsed { path, unit, .. } => out.push(FileChangeInput {
                path: path.clone(),
                base: base_unit(path),
                head: Some(Arc::clone(unit)),
            }),
            FileOutcome::Deleted { path } | FileOutcome::Skipped { path, .. } => {
                out.push(FileChangeInput {
                    path: path.clone(),
                    base: base_unit(path),
                    head: None,
                });
            }
        }
    }
    out
}

struct Indexed<'a> {
    symbol: &'a IrSymbol,
    unit: &'a ParsedUnit,
}

fn index(unit: Option<&ParsedUnit>) -> Result<BTreeMap<NodeKey, Indexed<'_>>> {
    let mut out = BTreeMap::new();
    let Some(unit) = unit else {
        return Ok(out);
    };
    for symbol in &unit.symbols {
        let Some(id) = symbol_node_id(unit, symbol) else {
            continue;
        };
        if out.insert(id.key(), Indexed { symbol, unit }).is_some() {
            return Err(Error::KeyCollision(id.as_str().to_owned()));
        }
    }
    Ok(out)
}

#[derive(Default)]
struct FileDiffOut {
    added: Vec<(SymbolRecord, Option<SymbolRef>)>,
    removed: Vec<(SymbolRecord, Option<SymbolRef>)>,
    modified: Vec<(NodeKey, ModifiedFlags)>,
    moved_range: Vec<NodeKey>,
    unchanged: u32,
}

fn record(path: &RepoPath, key: NodeKey, item: &Indexed<'_>) -> (SymbolRecord, Option<SymbolRef>) {
    (
        SymbolRecord {
            key,
            file: path.clone(),
            entry: NameEntry {
                name: item.symbol.name.clone(),
                is_member: item.symbol.parent.is_some(),
            },
        },
        symbol_ref(item.unit, item.symbol),
    )
}

fn diff_file(input: &FileChangeInput) -> Result<FileDiffOut> {
    let base = index(input.base.as_deref())?;
    let head = index(input.head.as_deref())?;
    let mut out = FileDiffOut::default();
    for (key, head_item) in &head {
        match base.get(key) {
            None => out.added.push(record(&input.path, *key, head_item)),
            Some(base_item) => {
                let mut flags = ModifiedFlags::NONE;
                if base_item.symbol.signature_hash != head_item.symbol.signature_hash {
                    flags.insert(ModifiedFlags::SIGNATURE);
                }
                if base_item.symbol.body_hash != head_item.symbol.body_hash {
                    flags.insert(ModifiedFlags::BODY);
                }
                if base_item.symbol.attr_hash != head_item.symbol.attr_hash {
                    flags.insert(ModifiedFlags::ATTRIBUTES);
                }
                if !flags.is_empty() {
                    out.modified.push((*key, flags));
                } else if base_item.symbol.range != head_item.symbol.range {
                    out.moved_range.push(*key);
                } else {
                    out.unchanged += 1;
                }
            }
        }
    }
    for (key, base_item) in &base {
        if !head.contains_key(key) {
            out.removed.push(record(&input.path, *key, base_item));
        }
    }
    Ok(out)
}

/// Classifies the symbols of every changed file, pairs renames across files and updates the
/// symbol counters.
pub fn diff_changed_files(
    inputs: &[FileChangeInput],
    hints: &RenameHints,
    cfg: &MatcherConfig,
    counters: &IncrementalCounters,
) -> Result<SymbolChangeSet> {
    let span = tracing::info_span!("incremental.symbol_diff", files = inputs.len());
    let _guard = span.enter();

    let per_file: Vec<Result<FileDiffOut>> = inputs.par_iter().map(diff_file).collect();

    let mut set = SymbolChangeSet::default();
    let mut removed_refs: Vec<SymbolRef> = Vec::new();
    let mut added_refs: Vec<SymbolRef> = Vec::new();
    let mut removed_by_id: BTreeMap<String, SymbolRecord> = BTreeMap::new();
    let mut added_by_id: BTreeMap<String, SymbolRecord> = BTreeMap::new();

    for (input, diff) in inputs.iter().zip(per_file) {
        let diff = diff?;
        set.changed_files.insert(input.path.clone());
        set.unchanged_in_changed_files += diff.unchanged;
        set.modified.extend(diff.modified);
        set.moved_range.extend(diff.moved_range);
        for (record, sym_ref) in diff.added {
            if let Some(sym_ref) = sym_ref {
                added_by_id.insert(sym_ref.id.as_str().to_owned(), record.clone());
                added_refs.push(sym_ref);
            }
            set.added.push(record);
        }
        for (record, sym_ref) in diff.removed {
            if let Some(sym_ref) = sym_ref {
                removed_by_id.insert(sym_ref.id.as_str().to_owned(), record.clone());
                removed_refs.push(sym_ref);
            }
            set.removed.push(record);
        }
    }
    set.added.sort_by_key(|r| r.key);
    set.removed.sort_by_key(|r| r.key);
    set.modified.sort_by_key(|(key, _)| *key);
    set.moved_range.sort();

    removed_refs.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
    added_refs.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
    let matched = match_symbols(&removed_refs, &added_refs, cfg, hints);
    set.ambiguous = matched.ambiguous.len() as u32;
    for pair in &matched.matches {
        let (Some(from), Some(to)) = (
            removed_by_id.get(pair.from_id.as_str()),
            added_by_id.get(pair.to_id.as_str()),
        ) else {
            continue;
        };
        let transition = match pair.transition {
            SymbolTransition::Renamed => LineageTransition::Renamed {
                from: from.entry.name.clone(),
                to: to.entry.name.clone(),
            },
            SymbolTransition::Moved | SymbolTransition::RenamedAndMoved => {
                LineageTransition::Moved {
                    from: from.file.clone(),
                    to: to.file.clone(),
                }
            }
        };
        set.renamed.push(LineageRecord {
            from: Some(from.key),
            to: to.key,
            transition,
            similarity: pair.similarity,
        });
    }
    set.renamed.sort_by_key(|r| (r.to, r.from));

    let counts = set.counts();
    counters.add_symbols_added(counts.added);
    counters.add_symbols_removed(counts.removed);
    counters.add_symbols_modified(counts.modified);
    counters.add_symbols_renamed(counts.renamed);
    Ok(set)
}
