//! Change classification framework (CHG-002): the [`Classifier`] trait, the registry and the
//! parallel driver that fills [`SymbolRecord::classes`].
//!
//! Classifiers compare the syntax facts of one symbol on both sides. They never read source
//! text and never guess from text: when facts are unavailable (parse failure, analyzer without
//! facts) the symbol gets no classes and is listed in [`ChangedSymbolSet::facts_unavailable`].

use std::collections::BTreeMap;
use std::sync::Arc;

use analysis_ir::facts::SyntaxFact;
use analysis_ir::symbol::{AttrValue, LocalId};
use analysis_ir::unit::{ParseStatus, ParsedUnit};
use rayon::prelude::*;
use review_core::change::{ChangeClass, SymbolChange};
use review_core::location::DiffSide;
use review_core::symbol::SymbolKind;
use serde::Serialize;

use super::classes;
pub use super::fact_diff::FactRef;
use super::graphs::GraphPair;
use super::symbols::{ChangedSymbolSet, SymbolRecord};
use crate::metrics;
use crate::symbol_map::UnitSource;

/// How sure a classification is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Certainty {
    /// Derived from facts or edges without guessing.
    Exact,
    /// Rule-of-thumb match (names, patterns, missing type information).
    Heuristic,
}

/// One class assigned to a symbol, with the facts that justify it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ClassifiedChange {
    /// The class.
    pub class: ChangeClass,
    /// Facts behind the class: removed ones on the base side, added ones on the head side.
    pub evidence: Vec<FactRef>,
    /// Small, bounded details (counts, normalized fragments, hashes; never source text).
    pub detail: BTreeMap<String, AttrValue>,
    /// Exact or heuristic.
    pub certainty: Certainty,
}

impl ClassifiedChange {
    /// A change of `class` with no evidence or detail yet.
    pub fn new(class: ChangeClass, certainty: Certainty) -> Self {
        Self {
            class,
            evidence: Vec::new(),
            detail: BTreeMap::new(),
            certainty,
        }
    }

    /// Set a detail entry.
    pub fn with(mut self, key: &str, value: AttrValue) -> Self {
        self.detail.insert(key.to_owned(), value);
        self
    }

    /// Set the evidence.
    pub fn with_evidence(mut self, evidence: Vec<FactRef>) -> Self {
        self.evidence = evidence;
        self
    }
}

/// The two fact lists of one symbol.
#[derive(Debug, Clone)]
pub struct FactsView {
    base_unit: Arc<ParsedUnit>,
    head_unit: Arc<ParsedUnit>,
    base_local: u32,
    head_local: u32,
}

fn slice(unit: &ParsedUnit, local: u32) -> &[SyntaxFact] {
    unit.facts
        .iter()
        .find(|g| g.symbol == LocalId(local))
        .map(|g| g.facts.as_slice())
        .unwrap_or(&[])
}

impl FactsView {
    /// Facts of the symbol in the base unit, in source order.
    pub fn base(&self) -> &[SyntaxFact] {
        slice(&self.base_unit, self.base_local)
    }

    /// Facts of the symbol in the head unit, in source order.
    pub fn head(&self) -> &[SyntaxFact] {
        slice(&self.head_unit, self.head_local)
    }

    /// The base unit.
    pub fn base_unit(&self) -> &ParsedUnit {
        &self.base_unit
    }

    /// The head unit.
    pub fn head_unit(&self) -> &ParsedUnit {
        &self.head_unit
    }

    /// Local id of the symbol in the base unit.
    pub fn base_local(&self) -> u32 {
        self.base_local
    }

    /// Local id of the symbol in the head unit.
    pub fn head_local(&self) -> u32 {
        self.head_local
    }
}

/// Whether the facts of a symbol can be compared.
#[derive(Debug, Clone)]
pub enum FactsAccess {
    /// Added, removed or cosmetic symbols are not classified.
    NotApplicable,
    /// A unit is missing or failed to parse, or the analyzer produced no facts.
    Unavailable,
    /// Both fact lists are available.
    Ready(FactsView),
}

/// Everything a classifier may read besides the record.
#[derive(Clone, Copy)]
pub struct ClassifyCx<'a> {
    /// Units of both sides.
    pub units: &'a dyn UnitSource,
    /// Graphs of both sides, when available.
    pub pair: GraphPair<'a>,
}

impl std::fmt::Debug for ClassifyCx<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClassifyCx")
            .field("pair", &self.pair)
            .finish_non_exhaustive()
    }
}

fn is_callable(kind: SymbolKind) -> bool {
    matches!(
        kind,
        SymbolKind::Function
            | SymbolKind::Method
            | SymbolKind::Constructor
            | SymbolKind::Getter
            | SymbolKind::Setter
    )
}

impl ClassifyCx<'_> {
    /// Locate the facts of `rec` on both sides.
    pub fn facts_access(&self, rec: &SymbolRecord) -> FactsAccess {
        if rec.cosmetic
            || matches!(
                rec.symbol.change,
                SymbolChange::Added | SymbolChange::Removed
            )
        {
            return FactsAccess::NotApplicable;
        }
        let (Some(base), Some(head)) = (&rec.base, &rec.head) else {
            return FactsAccess::Unavailable;
        };
        let (Some(base_unit), Some(head_unit)) = (
            self.units.unit(DiffSide::Base, &base.path),
            self.units.unit(DiffSide::Head, &head.path),
        ) else {
            return FactsAccess::Unavailable;
        };
        let failed = |u: &ParsedUnit| matches!(u.status, ParseStatus::Failed { .. });
        if failed(&base_unit) || failed(&head_unit) {
            return FactsAccess::Unavailable;
        }
        // An analyzer run without `syntax_facts` leaves the whole unit without facts although
        // callable symbols have bodies.
        let facts_missing = |u: &ParsedUnit, local: u32, kind: SymbolKind| {
            u.facts.is_empty()
                && is_callable(kind)
                && u.symbols
                    .get(local as usize)
                    .is_some_and(|s| s.body_token_count > 0)
        };
        if facts_missing(&base_unit, base.local, base.kind)
            || facts_missing(&head_unit, head.local, head.kind)
        {
            return FactsAccess::Unavailable;
        }
        FactsAccess::Ready(FactsView {
            base_unit,
            head_unit,
            base_local: base.local,
            head_local: head.local,
        })
    }
}

/// A rule set producing classes for one symbol.
pub trait Classifier: Send + Sync {
    /// Stable name, used as the `classifier` span attribute.
    fn name(&self) -> &'static str;

    /// The classes of `rec` given both fact lists. Output order does not matter; the driver
    /// sorts by `(class, evidence range)`.
    fn classify(
        &self,
        cx: &ClassifyCx<'_>,
        rec: &SymbolRecord,
        facts: &FactsView,
    ) -> Vec<ClassifiedChange>;
}

/// The structural classifiers of CHG-002, extended by CHG-003..005.
pub fn default_classifiers() -> Vec<Box<dyn Classifier>> {
    classes::registry()
}

fn sort_key(c: &ClassifiedChange) -> (ChangeClass, Option<(DiffSide, u32, u32)>) {
    (
        c.class,
        c.evidence
            .first()
            .map(|e| (e.side, e.range.start.line, e.range.start.column)),
    )
}

/// Run every classifier over every applicable symbol of `set`, in parallel, and store the
/// classes sorted by `(class, evidence order)`. Symbols whose facts are unavailable are listed
/// in `set.facts_unavailable`.
pub fn classify_all(
    set: &mut ChangedSymbolSet,
    cx: &ClassifyCx<'_>,
    classifiers: &[Box<dyn Classifier>],
) {
    set.symbols.par_iter_mut().for_each(|rec| {
        rec.classes.clear();
        rec.facts_unavailable = false;
        match cx.facts_access(rec) {
            FactsAccess::NotApplicable => {}
            FactsAccess::Unavailable => rec.facts_unavailable = true,
            FactsAccess::Ready(view) => {
                let mut all = Vec::new();
                for classifier in classifiers {
                    let span =
                        tracing::info_span!("change.classify", classifier = classifier.name());
                    let _enter = span.enter();
                    all.extend(classifier.classify(cx, rec, &view));
                }
                all.sort_by_key(sort_key);
                rec.classes = all;
            }
        }
    });
    set.facts_unavailable = set
        .symbols
        .iter()
        .filter(|r| r.facts_unavailable)
        .map(|r| r.symbol.symbol_key)
        .collect();
    for rec in &set.symbols {
        for class in &rec.classes {
            metrics::record_change_class(class.class.as_str());
            if class.certainty == Certainty::Heuristic {
                metrics::record_heuristic(class.class.as_str());
            }
        }
    }
}

/// Integer detail value.
pub fn int(n: usize) -> AttrValue {
    AttrValue::Int(i64::try_from(n).unwrap_or(i64::MAX))
}
