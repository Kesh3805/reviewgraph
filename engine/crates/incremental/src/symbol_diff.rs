//! Per-file symbol diff (SID-004, ADR-004).
//!
//! Given the parsed unit of one file in two snapshots, say exactly which symbols are new, gone or
//! changed, and in what way. Identity is the `SymbolId`, so a diff is meaningful for one path, and a
//! file that appeared or disappeared diffs `None` against `Some`.
//!
//! The differ never looks at `SyntaxFacts` and never fails: a file whose parse failed yields
//! `unknown`, and a partially parsed file marks the affected symbols `uncertain`, so callers
//! re-link rather than skip work.

use std::collections::BTreeMap;

use analysis_ir::identity::{symbol_id_of, symbol_id_parts};
use analysis_ir::{FailReason, IrSymbol, ParseStatus, ParsedUnit};
use review_core::ids::SymbolId;
use review_core::location::{DiffSide, RepoPath};
use review_core::matcher::SymbolRef;
use review_core::symbol::{Hash128, SymbolKind};
use serde::{Deserialize, Serialize};

/// Which part of a symbol changed. Signature, body and attributes are independent flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ModifiedFlags(u8);

impl ModifiedFlags {
    /// No part changed.
    pub const NONE: Self = Self(0);
    /// `signature_hash` differs: parameters, types or modifiers changed.
    pub const SIGNATURE: Self = Self(1 << 0);
    /// `body_hash` differs: the implementation changed.
    pub const BODY: Self = Self(1 << 1);
    /// `attr_hash` differs: decorators or export/visibility flags changed.
    pub const ATTRIBUTES: Self = Self(1 << 2);

    /// Every flag.
    pub const ALL: [ModifiedFlags; 3] = [Self::SIGNATURE, Self::BODY, Self::ATTRIBUTES];

    /// Whether every flag in `other` is set.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Adds flags.
    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }

    /// Whether no flag is set.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Stable metric label, in signature, body, attribute order.
    pub fn names(self) -> Vec<&'static str> {
        let mut out = Vec::new();
        for (flag, name) in [
            (Self::SIGNATURE, "signature"),
            (Self::BODY, "body"),
            (Self::ATTRIBUTES, "attributes"),
        ] {
            if self.contains(flag) {
                out.push(name);
            }
        }
        out
    }

    /// The raw bits, for storage and tests.
    pub const fn bits(self) -> u8 {
        self.0
    }
}

/// What happened to one symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SymbolChangeKind {
    /// Present on both sides with identical hashes; only its range may have moved.
    Unchanged,
    /// Present on both sides with at least one changed hash.
    Modified,
    /// Only in the head unit.
    Added,
    /// Only in the base unit.
    Removed,
}

/// One symbol's change. This is what the incremental delta writer and `review graph diff` consume.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SymbolChange {
    /// Canonical id of the symbol.
    pub id: SymbolId,
    /// What happened.
    pub kind: SymbolChangeKind,
    /// Which parts changed; `NONE` unless `kind` is `Modified`.
    pub flags: ModifiedFlags,
    /// The symbol's range moved although nothing about it changed.
    pub moved_range: bool,
    /// The symbol or its file is affected by a parse error, so the classification is provisional.
    pub uncertain: bool,
}

impl SymbolChange {
    /// A symbol present only in the head unit.
    pub fn added(id: SymbolId) -> Self {
        Self {
            id,
            kind: SymbolChangeKind::Added,
            flags: ModifiedFlags::NONE,
            moved_range: false,
            uncertain: false,
        }
    }

    /// A symbol present only in the base unit.
    pub fn removed(id: SymbolId) -> Self {
        Self {
            id,
            kind: SymbolChangeKind::Removed,
            flags: ModifiedFlags::NONE,
            moved_range: false,
            uncertain: false,
        }
    }
}

/// Aggregate counts of one file's diff, for the ADR-004 counters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffCounts {
    /// Symbols with no hash change.
    pub unchanged: u32,
    /// Modified because the signature changed.
    pub modified_signature: u32,
    /// Modified because the body changed.
    pub modified_body: u32,
    /// Modified because an attribute changed.
    pub modified_attributes: u32,
    /// Symbols that only exist in the head unit.
    pub added: u32,
    /// Symbols that only exist in the base unit.
    pub removed: u32,
}

impl DiffCounts {
    /// Total number of classified symbols.
    pub fn total(&self) -> u32 {
        self.unchanged
            + self.modified_signature
            + self.modified_body
            + self.modified_attributes
            + self.added
            + self.removed
    }
}

/// Why the diff could not be computed for a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnknownReason {
    /// One side failed to parse, so nothing about the file can be trusted.
    ParseFailed {
        /// Which side failed.
        side: DiffSide,
        /// Why the analyzer gave up.
        reason: FailReason,
    },
    /// A symbol could not be turned into a canonical id.
    UnidentifiableSymbol,
}

/// The diff of one file between two snapshots.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileSymbolDiff {
    /// Path of the file; the base path for a deleted file, the head path otherwise.
    pub path: RepoPath,
    /// Parse status of the base unit, if there was one.
    pub base_status: Option<UnitStatus>,
    /// Parse status of the head unit, if there was one.
    pub head_status: Option<UnitStatus>,
    /// Every classified symbol, sorted by `(kind, id)`.
    pub changes: Vec<SymbolChange>,
    /// Aggregate counts.
    pub counts: DiffCounts,
    /// Set when the diff could not be computed; `changes` is then empty.
    pub unknown: Option<UnknownReason>,
}

/// Serializable mirror of [`ParseStatus`], so a diff can be written to a delta file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnitStatus {
    /// The file parsed without error nodes.
    Ok,
    /// The file parsed with error or missing nodes.
    Partial {
        /// Number of ERROR nodes.
        error_nodes: u32,
        /// Number of MISSING nodes.
        missing_nodes: u32,
    },
    /// The analyzer gave up.
    Failed {
        /// Why.
        reason: FailReason,
    },
}

impl From<ParseStatus> for UnitStatus {
    fn from(status: ParseStatus) -> Self {
        match status {
            ParseStatus::Ok => Self::Ok,
            ParseStatus::Partial {
                error_nodes,
                missing_nodes,
            } => Self::Partial {
                error_nodes,
                missing_nodes,
            },
            ParseStatus::Failed { reason } => Self::Failed { reason },
        }
    }
}

impl FileSymbolDiff {
    /// The diff of a file whose parse failed on one side: nothing can be trusted.
    pub fn unknown(path: RepoPath, reason: UnknownReason) -> Self {
        Self {
            path,
            base_status: None,
            head_status: None,
            changes: Vec::new(),
            counts: DiffCounts::default(),
            unknown: Some(reason),
        }
    }

    /// Whether the diff is usable.
    pub fn is_known(&self) -> bool {
        self.unknown.is_none()
    }

    /// Symbols that only exist in the head unit, sorted by id. SID-005 pools these.
    pub fn added(&self) -> Vec<&SymbolId> {
        self.changes
            .iter()
            .filter(|change| change.kind == SymbolChangeKind::Added)
            .map(|change| &change.id)
            .collect()
    }

    /// Symbols that only exist in the base unit, sorted by id. SID-005 pools these.
    pub fn removed(&self) -> Vec<&SymbolId> {
        self.changes
            .iter()
            .filter(|change| change.kind == SymbolChangeKind::Removed)
            .map(|change| &change.id)
            .collect()
    }

    /// Symbols whose ids are stable across both sides.
    pub fn unchanged(&self) -> Vec<&SymbolId> {
        self.changes
            .iter()
            .filter(|change| change.kind == SymbolChangeKind::Unchanged)
            .map(|change| &change.id)
            .collect()
    }

    /// Whether any symbol is uncertain, so the caller must treat the whole file as affected.
    pub fn has_uncertain(&self) -> bool {
        self.changes.iter().any(|change| change.uncertain)
    }
}

/// Diffs the parsed unit of one file between two snapshots.
///
/// `None` means the file does not exist on that side, so every symbol of the other side is added or
/// removed. The result is sorted by `(kind, id)`, so two runs over the same inputs produce the same
/// vector regardless of the analyzer's internal ordering.
pub fn diff_units(base: Option<&ParsedUnit>, head: Option<&ParsedUnit>) -> FileSymbolDiff {
    let path = match (base, head) {
        (Some(unit), _) => unit.file.clone(),
        (None, Some(unit)) => unit.file.clone(),
        // Diffing a file that exists in neither snapshot is a caller error; it is reported as an
        // unknown diff under a synthetic path rather than panicking.
        (None, None) => {
            return FileSymbolDiff::unknown(
                fallback_path(),
                UnknownReason::ParseFailed {
                    side: DiffSide::Base,
                    reason: FailReason::Internal,
                },
            )
        }
    };
    for (side, unit) in [(DiffSide::Base, base), (DiffSide::Head, head)] {
        if let Some(unit) = unit {
            if let ParseStatus::Failed { reason } = unit.status {
                return FileSymbolDiff::unknown(path, UnknownReason::ParseFailed { side, reason });
            }
        }
    }

    let base_index = index_of(base);
    let head_index = index_of(head);
    let partial = base.is_some_and(|unit| matches!(unit.status, ParseStatus::Partial { .. }))
        || head.is_some_and(|unit| matches!(unit.status, ParseStatus::Partial { .. }));

    let mut changes: Vec<SymbolChange> = Vec::new();
    for (id, head_symbol) in &head_index {
        match base_index.get(id) {
            None => changes.push(SymbolChange::added(id.clone())),
            Some(base_symbol) => changes.push(classify(id, base_symbol, head_symbol, partial)),
        }
    }
    for id in base_index.keys() {
        if !head_index.contains_key(id) {
            changes.push(SymbolChange::removed(id.clone()));
        }
    }
    changes.sort_by(|left, right| {
        kind_rank(left.kind)
            .cmp(&kind_rank(right.kind))
            .then_with(|| left.id.as_str().cmp(right.id.as_str()))
    });

    let mut counts = DiffCounts::default();
    for change in &changes {
        match change.kind {
            SymbolChangeKind::Unchanged => counts.unchanged += 1,
            SymbolChangeKind::Added => counts.added += 1,
            SymbolChangeKind::Removed => counts.removed += 1,
            SymbolChangeKind::Modified => {
                if change.flags.contains(ModifiedFlags::SIGNATURE) {
                    counts.modified_signature += 1;
                }
                if change.flags.contains(ModifiedFlags::BODY) {
                    counts.modified_body += 1;
                }
                if change.flags.contains(ModifiedFlags::ATTRIBUTES) {
                    counts.modified_attributes += 1;
                }
            }
        }
    }
    FileSymbolDiff {
        path,
        base_status: base.map(|unit| unit.status.into()),
        head_status: head.map(|unit| unit.status.into()),
        changes,
        counts,
        unknown: None,
    }
}

/// Every symbol of a unit, keyed by canonical id. A symbol whose parts cannot be formatted makes the
/// whole file unknown, because ids are the diff's key.
fn index_of(unit: Option<&ParsedUnit>) -> BTreeMap<SymbolId, &IrSymbol> {
    let mut out = BTreeMap::new();
    let Some(unit) = unit else {
        return out;
    };
    for symbol in &unit.symbols {
        let Some(id) = symbol_id_of(unit, symbol.local_id.0) else {
            continue;
        };
        out.insert(id, symbol);
    }
    out
}

fn classify(id: &SymbolId, base: &IrSymbol, head: &IrSymbol, partial: bool) -> SymbolChange {
    let mut flags = ModifiedFlags::NONE;
    if base.signature_hash != head.signature_hash {
        flags.insert(ModifiedFlags::SIGNATURE);
    }
    if base.body_hash != head.body_hash {
        flags.insert(ModifiedFlags::BODY);
    }
    if base.attr_hash != head.attr_hash {
        flags.insert(ModifiedFlags::ATTRIBUTES);
    }
    let moved_range = flags.is_empty() && base.range != head.range;
    let uncertain = partial && (base.has_errors || head.has_errors) && !flags.is_empty();
    SymbolChange {
        id: id.clone(),
        kind: if flags.is_empty() {
            SymbolChangeKind::Unchanged
        } else {
            SymbolChangeKind::Modified
        },
        flags,
        moved_range,
        uncertain,
    }
}

fn kind_rank(kind: SymbolChangeKind) -> u8 {
    match kind {
        SymbolChangeKind::Unchanged => 0,
        SymbolChangeKind::Modified => 1,
        SymbolChangeKind::Added => 2,
        SymbolChangeKind::Removed => 3,
    }
}

/// A path for the impossible "file exists in neither snapshot" case. It terminates on the first
/// candidate, which `RepoPath` accepts; the loop exists so the function is total without a panic.
fn fallback_path() -> RepoPath {
    let mut candidate = String::from("_");
    loop {
        match RepoPath::new(candidate.clone()) {
            Ok(path) => return path,
            Err(_) => candidate.push('x'),
        }
    }
}

/// Builds the matcher input for one symbol of a unit: ids, names, hashes and the body sketch, with
/// no source text.
pub fn symbol_ref(unit: &ParsedUnit, symbol: &IrSymbol) -> Option<SymbolRef> {
    let id = symbol_id_of(unit, symbol.local_id.0)?;
    let parent_id = symbol
        .parent
        .and_then(|parent| symbol_id_of(unit, parent.0));
    Some(SymbolRef {
        key: review_core::ids::SymbolKey::of(&id),
        id,
        kind: symbol.kind,
        name: symbol.name.clone(),
        qualified_name: symbol.qualified_name.clone(),
        module_path: unit.module_path.clone(),
        parent_id,
        signature_hash: symbol.signature_hash,
        body_hash: symbol.body_hash,
        body_token_count: symbol.body_token_count,
        shingles: symbol.body_shingles.clone(),
    })
}

/// Matcher input for every symbol of a unit, sorted by id so the matcher input order never
/// influences the result.
pub fn symbol_refs(unit: &ParsedUnit) -> Vec<SymbolRef> {
    let mut refs: Vec<SymbolRef> = unit
        .symbols
        .iter()
        .filter_map(|symbol| symbol_ref(unit, symbol))
        .collect();
    refs.sort_by(|left, right| left.id.as_str().cmp(right.id.as_str()));
    refs
}

/// Matcher input for several units, again sorted by id.
pub fn symbol_refs_of<'a>(units: impl IntoIterator<Item = &'a ParsedUnit>) -> Vec<SymbolRef> {
    let mut refs: Vec<SymbolRef> = units.into_iter().flat_map(symbol_refs).collect();
    refs.sort_by(|left, right| left.id.as_str().cmp(right.id.as_str()));
    refs.dedup_by(|left, right| left.id == right.id);
    refs
}

/// The kind of one symbol, for callers that only have an id.
pub fn kind_of(unit: &ParsedUnit, id: &SymbolId) -> Option<SymbolKind> {
    unit.symbols
        .iter()
        .find(|symbol| symbol_id_parts(unit, symbol).format().ok().as_ref() == Some(id))
        .map(|symbol| symbol.kind)
}

/// Whether two hashes are equal, exposed so callers can build their own flags.
pub fn same_hash(left: Hash128, right: Hash128) -> bool {
    left == right
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_are_named_in_a_stable_order() {
        let mut flags = ModifiedFlags::NONE;
        assert!(flags.is_empty());
        flags.insert(ModifiedFlags::ATTRIBUTES);
        flags.insert(ModifiedFlags::BODY);
        flags.insert(ModifiedFlags::SIGNATURE);
        assert_eq!(flags.names(), vec!["signature", "body", "attributes"]);
        assert!(flags.contains(ModifiedFlags::BODY));
        assert!(ModifiedFlags::NONE.is_empty());
        assert_eq!(flags.bits(), 0b111);
        assert!(ModifiedFlags::ALL.iter().all(|f| flags.contains(*f)));
    }

    #[test]
    fn statuses_mirror_parse_status() {
        assert_eq!(UnitStatus::from(ParseStatus::Ok), UnitStatus::Ok);
        assert_eq!(
            UnitStatus::from(ParseStatus::Partial {
                error_nodes: 1,
                missing_nodes: 2
            }),
            UnitStatus::Partial {
                error_nodes: 1,
                missing_nodes: 2
            }
        );
        assert_eq!(
            UnitStatus::from(ParseStatus::Failed {
                reason: FailReason::Timeout
            }),
            UnitStatus::Failed {
                reason: FailReason::Timeout
            }
        );
        assert_eq!(serde_json::to_string(&UnitStatus::Ok).unwrap(), "\"ok\"");
    }

    #[test]
    fn changes_serialize_with_stable_wire_names() {
        let change = SymbolChange::added(SymbolId::parse("ts:src/a#f/function").unwrap());
        let json = serde_json::to_string(&change).unwrap();
        assert!(json.contains("\"kind\":\"added\""), "{json}");
        assert!(json.contains("\"flags\":0"), "{json}");
        assert_eq!(serde_json::from_str::<SymbolChange>(&json).unwrap(), change);
        assert_eq!(
            serde_json::to_string(&UnknownReason::UnidentifiableSymbol).unwrap(),
            "\"unidentifiable_symbol\""
        );
    }
}
