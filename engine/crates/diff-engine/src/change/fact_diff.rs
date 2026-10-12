//! Fact comparison shared by every classifier (CHG-002).
//!
//! Facts are compared by their position-free `key` only, so moving or reformatting code yields
//! no difference. [`diff_facts`] reports the multiset difference and whether the kept facts
//! appear in a different order; [`align`] pairs facts of one kind across the sides with an LCS so
//! a changed condition is reported as one replaced pair rather than an unrelated add and remove.

use std::collections::BTreeMap;

use analysis_ir::facts::{FactKind, SyntaxFact};
use gix::diff::blob::{Algorithm, Diff, InternedInput};
use review_core::location::DiffSide;

use super::{bounded, FactRef};

/// Multiset difference of two fact lists restricted to some kinds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FactDiff {
    /// Head facts beyond the base count of their key, in head order.
    pub added: Vec<FactRef>,
    /// Base facts beyond the head count of their key, in base order.
    pub removed: Vec<FactRef>,
    /// Facts present on both sides (by key, counted).
    pub kept: u32,
    /// The kept facts appear in a different relative order.
    pub reordered: bool,
}

impl FactDiff {
    /// No added or removed facts.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty()
    }
}

/// Evidence reference of `fact` on `side`.
pub fn fact_ref(side: DiffSide, fact: &SyntaxFact) -> FactRef {
    FactRef {
        side,
        kind: fact.kind,
        key: bounded(&fact.key),
        range: fact.range,
    }
}

fn filtered<'a>(facts: &'a [SyntaxFact], kinds: &[FactKind]) -> Vec<&'a SyntaxFact> {
    facts.iter().filter(|f| kinds.contains(&f.kind)).collect()
}

/// Compare `base` and `head` facts of the given kinds by `(kind, key)`.
pub fn diff_facts(base: &[SyntaxFact], head: &[SyntaxFact], kinds: &[FactKind]) -> FactDiff {
    let base = filtered(base, kinds);
    let head = filtered(head, kinds);
    let mut head_counts: BTreeMap<(FactKind, &str), u32> = BTreeMap::new();
    for f in &head {
        *head_counts.entry((f.kind, f.key.as_str())).or_insert(0) += 1;
    }
    let mut base_counts: BTreeMap<(FactKind, &str), u32> = BTreeMap::new();
    for f in &base {
        *base_counts.entry((f.kind, f.key.as_str())).or_insert(0) += 1;
    }

    let mut out = FactDiff::default();
    // Base facts: the first min(base, head) occurrences of a key are kept, the rest removed.
    let mut budget = head_counts.clone();
    let mut base_kept: Vec<(FactKind, &str)> = Vec::new();
    for f in &base {
        let k = (f.kind, f.key.as_str());
        match budget.get_mut(&k) {
            Some(n) if *n > 0 => {
                *n -= 1;
                base_kept.push(k);
            }
            _ => out.removed.push(fact_ref(DiffSide::Base, f)),
        }
    }
    let mut budget = base_counts;
    let mut head_kept: Vec<(FactKind, &str)> = Vec::new();
    for f in &head {
        let k = (f.kind, f.key.as_str());
        match budget.get_mut(&k) {
            Some(n) if *n > 0 => {
                *n -= 1;
                head_kept.push(k);
            }
            _ => out.added.push(fact_ref(DiffSide::Head, f)),
        }
    }
    out.kept = base_kept.len() as u32;
    out.reordered = base_kept != head_kept;
    out
}

/// One aligned position of two fact sequences.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Aligned<'a> {
    /// Same key on both sides.
    Same(&'a SyntaxFact, &'a SyntaxFact),
    /// A base fact replaced by a head fact at the same position.
    Replaced(&'a SyntaxFact, &'a SyntaxFact),
    /// Only on the base side.
    Removed(&'a SyntaxFact),
    /// Only on the head side.
    Added(&'a SyntaxFact),
}

/// Align the facts of `kind` on both sides with an LCS over their keys (Myers via imara-diff).
/// Within one changed region, facts are paired in order; the surplus is added or removed.
pub fn align<'a>(base: &'a [SyntaxFact], head: &'a [SyntaxFact], kind: FactKind) -> Vec<Aligned<'a>> {
    let b: Vec<&SyntaxFact> = base.iter().filter(|f| f.kind == kind).collect();
    let h: Vec<&SyntaxFact> = head.iter().filter(|f| f.kind == kind).collect();
    let mut input: InternedInput<&str> = InternedInput::default();
    input.update_before(b.iter().map(|f| f.key.as_str()));
    input.update_after(h.iter().map(|f| f.key.as_str()));
    let diff = Diff::compute(Algorithm::Myers, &input);

    let mut out = Vec::with_capacity(b.len().max(h.len()));
    let (mut i, mut j) = (0usize, 0usize);
    for hunk in diff.hunks() {
        let (bs, be) = (hunk.before.start as usize, hunk.before.end as usize);
        let (hs, he) = (hunk.after.start as usize, hunk.after.end as usize);
        while i < bs && j < hs {
            if let (Some(x), Some(y)) = (b.get(i), h.get(j)) {
                out.push(Aligned::Same(x, y));
            }
            i += 1;
            j += 1;
        }
        let pairs = (be - bs).min(he - hs);
        for k in 0..pairs {
            if let (Some(x), Some(y)) = (b.get(bs + k), h.get(hs + k)) {
                out.push(Aligned::Replaced(x, y));
            }
        }
        for k in bs + pairs..be {
            if let Some(x) = b.get(k) {
                out.push(Aligned::Removed(x));
            }
        }
        for k in hs + pairs..he {
            if let Some(y) = h.get(k) {
                out.push(Aligned::Added(y));
            }
        }
        i = be;
        j = he;
    }
    while i < b.len() && j < h.len() {
        if let (Some(x), Some(y)) = (b.get(i), h.get(j)) {
            out.push(Aligned::Same(x, y));
        }
        i += 1;
        j += 1;
    }
    out
}

/// The facts of one symbol on one side, in source order (empty when the unit has none).
pub fn facts_of(unit: &analysis_ir::unit::ParsedUnit, local: u32) -> &[SyntaxFact] {
    unit.facts
        .binary_search_by_key(&local, |f| f.symbol.0)
        .ok()
        .and_then(|i| unit.facts.get(i))
        .map_or(&[][..], |f| f.facts.as_slice())
}

/// Detail lookup helpers for analyzer-provided fact attributes.
pub mod detail {
    use analysis_ir::facts::SyntaxFact;
    use analysis_ir::symbol::AttrValue;

    /// A boolean detail, `None` when absent.
    pub fn flag(fact: &SyntaxFact, key: &str) -> Option<bool> {
        match fact.detail.get(key) {
            Some(AttrValue::Bool(b)) => Some(*b),
            Some(AttrValue::Str(s)) => Some(s == "true"),
            _ => None,
        }
    }

    /// A string detail.
    pub fn text<'a>(fact: &'a SyntaxFact, key: &str) -> Option<&'a str> {
        match fact.detail.get(key) {
            Some(AttrValue::Str(s)) => Some(s.as_str()),
            _ => None,
        }
    }

    /// A list-of-strings detail (a single string counts as a one-element list).
    pub fn list(fact: &SyntaxFact, key: &str) -> Vec<String> {
        match fact.detail.get(key) {
            Some(AttrValue::List(items)) => items
                .iter()
                .filter_map(|v| match v {
                    AttrValue::Str(s) => Some(s.clone()),
                    _ => None,
                })
                .collect(),
            Some(AttrValue::Str(s)) => vec![s.clone()],
            _ => Vec::new(),
        }
    }
}
