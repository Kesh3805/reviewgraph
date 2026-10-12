//! Fact-level comparison of one symbol's base and head [`SyntaxFact`]s (CHG-002).
//!
//! Fact keys are position-free (TSA-001/TSA-006), so a moved or reformatted block produces no
//! difference. The comparison is a multiset diff over `(kind, key)` plus a reorder flag.

use std::collections::BTreeMap;

use analysis_ir::facts::{FactKind, SyntaxFact};
use review_core::location::{DiffSide, SourceRange};
use serde::Serialize;

/// A pointer to one fact, with the side it lives on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FactRef {
    /// `Base` for removed facts, `Head` for added ones.
    pub side: DiffSide,
    /// Fact kind.
    pub kind: FactKind,
    /// Position-free comparison key.
    pub key: String,
    /// Where the fact is on its side.
    pub range: SourceRange,
}

impl FactRef {
    /// Reference `fact` on `side`.
    pub fn of(side: DiffSide, fact: &SyntaxFact) -> Self {
        Self {
            side,
            kind: fact.kind,
            key: fact.key.clone(),
            range: fact.range,
        }
    }
}

/// Result of [`diff_facts`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct FactDiff {
    /// Facts present more often in head, in head source order (`side = Head`).
    pub added: Vec<FactRef>,
    /// Facts present more often in base, in base source order (`side = Base`).
    pub removed: Vec<FactRef>,
    /// Facts present on both sides.
    pub kept: u32,
    /// The kept facts appear in a different order on the two sides.
    pub reordered: bool,
}

impl FactDiff {
    /// Nothing added, removed or reordered.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && !self.reordered
    }
}

type Key<'a> = (FactKind, &'a str);

/// Compare `base` and `head` restricted to `kinds`.
///
/// 1. Multiset diff on `(kind, key)`: O(n log n).
/// 2. `reordered`: the multiset intersection, read in source order on each side, differs.
pub fn diff_facts(base: &[SyntaxFact], head: &[SyntaxFact], kinds: &[FactKind]) -> FactDiff {
    let base: Vec<&SyntaxFact> = base.iter().filter(|f| kinds.contains(&f.kind)).collect();
    let head: Vec<&SyntaxFact> = head.iter().filter(|f| kinds.contains(&f.kind)).collect();

    let mut counts: BTreeMap<Key<'_>, (u32, u32)> = BTreeMap::new();
    for f in &base {
        counts.entry((f.kind, f.key.as_str())).or_default().0 += 1;
    }
    for f in &head {
        counts.entry((f.kind, f.key.as_str())).or_default().1 += 1;
    }
    let shared: BTreeMap<Key<'_>, u32> = counts
        .iter()
        .map(|(k, (b, h))| (*k, (*b).min(*h)))
        .collect();

    let mut out = FactDiff::default();
    let kept_base = split(&base, &shared, DiffSide::Base, &mut out.removed);
    let kept_head = split(&head, &shared, DiffSide::Head, &mut out.added);
    out.kept = kept_base.len() as u32;
    out.reordered = kept_base != kept_head;
    out
}

/// Walk `facts` in order; the first `shared[key]` occurrences of each key are kept, the rest go
/// to `rest`. Returns the kept keys in order.
fn split<'a>(
    facts: &[&'a SyntaxFact],
    shared: &BTreeMap<Key<'a>, u32>,
    side: DiffSide,
    rest: &mut Vec<FactRef>,
) -> Vec<Key<'a>> {
    let mut allowance = shared.clone();
    let mut kept = Vec::new();
    for fact in facts {
        let key = (fact.kind, fact.key.as_str());
        match allowance.get_mut(&key) {
            Some(n) if *n > 0 => {
                *n -= 1;
                kept.push(key);
            }
            _ => rest.push(FactRef::of(side, fact)),
        }
    }
    kept
}

/// Truncate `text` to at most `max` characters on a character boundary.
pub fn short(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}
