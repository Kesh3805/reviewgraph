//! The classifier framework (CHG-002): a [`Classifier`] trait, the default registry and
//! [`classify_symbols`], which runs every classifier over every changed symbol that exists on
//! both sides.
//!
//! Classes come only from comparing syntax facts, IR symbol data and graph edges of the two
//! versions; nothing is guessed from text. A symbol whose units carry no syntax facts at all
//! (analyzer without TSA-006 or `syntax_facts = false`) gets no fact-based classes and is
//! reported in `facts_unavailable`.

use std::collections::BTreeMap;

use analysis_ir::facts::SyntaxFact;
use analysis_ir::symbol::IrSymbol;
use analysis_ir::unit::ParsedUnit;
use rayon::prelude::*;
use review_core::ids::SymbolId;
use review_core::location::DiffSide;

use super::bind::{bind_calls, call_delta, BoundCall, CallDelta};
use super::classes;
use super::fact_diff::facts_of;
use super::graph::{GraphPair, GraphView};
use super::policy::ClassifierPolicy;
use super::{ChangeKind, ChangedSymbol, ClassifiedChange};
use crate::metrics;
use crate::symbol_map::UnitSource;

/// Everything a classifier may look at for one symbol.
pub struct ClassifyCx<'a> {
    /// Base unit of the symbol's file.
    pub base_unit: &'a ParsedUnit,
    /// Head unit of the symbol's file.
    pub head_unit: &'a ParsedUnit,
    /// Base IR symbol.
    pub base: &'a IrSymbol,
    /// Head IR symbol.
    pub head: &'a IrSymbol,
    /// Base symbol id.
    pub base_id: SymbolId,
    /// Head symbol id.
    pub head_id: SymbolId,
    /// Base facts of the symbol, in source order.
    pub base_facts: &'a [SyntaxFact],
    /// Head facts of the symbol, in source order.
    pub head_facts: &'a [SyntaxFact],
    /// Whether the units carry syntax facts at all.
    pub facts_available: bool,
    /// Graphs of both sides (either may be absent).
    pub graphs: GraphPair<'a>,
    /// Policy tables.
    pub policy: &'a ClassifierPolicy,
    /// Bound calls of the base version.
    pub base_calls: Vec<BoundCall>,
    /// Bound calls of the head version.
    pub head_calls: Vec<BoundCall>,
    /// Call delta between the two.
    pub calls: CallDelta,
}

impl std::fmt::Debug for ClassifyCx<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClassifyCx")
            .field("base", &self.base.name)
            .field("head", &self.head.name)
            .field("base_facts", &self.base_facts.len())
            .field("head_facts", &self.head_facts.len())
            .finish_non_exhaustive()
    }
}

impl<'a> ClassifyCx<'a> {
    /// Build the context for one symbol (binds calls on both sides).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        base_unit: &'a ParsedUnit,
        base_local: u32,
        base_id: &SymbolId,
        head_unit: &'a ParsedUnit,
        head_local: u32,
        head_id: &SymbolId,
        graphs: GraphPair<'a>,
        policy: &'a ClassifierPolicy,
    ) -> Option<Self> {
        let base = base_unit.symbols.get(base_local as usize)?;
        let head = head_unit.symbols.get(head_local as usize)?;
        let base_facts = facts_of(base_unit, base_local);
        let head_facts = facts_of(head_unit, head_local);
        let base_calls = bind_calls(base_facts, base_id.as_str(), graphs.base, DiffSide::Base);
        let head_calls = bind_calls(head_facts, head_id.as_str(), graphs.head, DiffSide::Head);
        let calls = call_delta(&base_calls, &head_calls);
        Some(Self {
            base_unit,
            head_unit,
            base,
            head,
            base_id: base_id.clone(),
            head_id: head_id.clone(),
            base_facts,
            head_facts,
            facts_available: !base_unit.facts.is_empty() || !head_unit.facts.is_empty(),
            graphs,
            policy,
            base_calls,
            head_calls,
            calls,
        })
    }

    /// The graph of one side.
    pub fn graph(&self, side: DiffSide) -> Option<&'a dyn GraphView> {
        self.graphs.side(side)
    }
}

/// One deterministic classifier.
pub trait Classifier: Send + Sync {
    /// Stable name (span attribute, mutation tests).
    fn name(&self) -> &'static str;
    /// Classes for one symbol. Return nothing when the classifier does not apply.
    fn classify(&self, cx: &ClassifyCx<'_>, sym: &ChangedSymbol) -> Vec<ClassifiedChange>;
}

/// Every built-in classifier, in a fixed order.
pub fn default_classifiers() -> Vec<Box<dyn Classifier>> {
    vec![
        Box::new(classes::control_flow::ControlFlow),
        Box::new(classes::condition::Condition),
        Box::new(classes::loops::Loops),
        Box::new(classes::returns::Returns),
        Box::new(classes::exceptions::Exceptions),
        Box::new(classes::asyncs::Async),
        Box::new(classes::calls::Calls),
        Box::new(classes::dependencies::Dependencies),
        Box::new(classes::authorization::Authorization),
        Box::new(classes::validation::Validation),
        Box::new(classes::db_write::DbWrite),
        Box::new(classes::transaction::Transaction),
        Box::new(classes::api_contract::ApiContract),
        Box::new(classes::return_type::ReturnType),
    ]
}

/// Result of [`classify_symbols`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClassifyReport {
    /// Symbols that could not be classified because no syntax facts exist for them.
    pub facts_unavailable: Vec<SymbolId>,
}

/// Run `classifiers` over every non-cosmetic symbol present on both sides, in parallel, and
/// store the classes sorted by `(class, evidence order)`.
pub fn classify_symbols(
    symbols: &mut [ChangedSymbol],
    units: &dyn UnitSource,
    graphs: GraphPair<'_>,
    policy: &ClassifierPolicy,
    classifiers: &[Box<dyn Classifier>],
) -> ClassifyReport {
    let span = tracing::info_span!(
        "change.classify",
        symbols = symbols.len(),
        classifiers = classifiers.len(),
        classes = tracing::field::Empty,
    );
    let _enter = span.enter();

    let results: Vec<(Vec<ClassifiedChange>, bool)> = symbols
        .par_iter()
        .map(|sym| classify_one(sym, units, graphs, policy, classifiers))
        .collect();
    let mut report = ClassifyReport::default();
    let mut total = 0usize;
    for (sym, (classes, unavailable)) in symbols.iter_mut().zip(results) {
        if unavailable {
            report.facts_unavailable.push(sym.symbol_id.clone());
        }
        total += classes.len();
        for c in &classes {
            metrics::record_change_class(c.class.as_str());
            if c.certainty == super::Certainty::Heuristic {
                metrics::record_heuristic(c.class.as_str());
            }
        }
        sym.classes = classes;
    }
    report.facts_unavailable.sort();
    span.record("classes", total);
    report
}

fn classify_one(
    sym: &ChangedSymbol,
    units: &dyn UnitSource,
    graphs: GraphPair<'_>,
    policy: &ClassifierPolicy,
    classifiers: &[Box<dyn Classifier>],
) -> (Vec<ClassifiedChange>, bool) {
    if sym.cosmetic || matches!(sym.change, ChangeKind::Added | ChangeKind::Removed) {
        return (Vec::new(), false);
    }
    let (Some(base), Some(head)) = (&sym.base, &sym.head) else {
        return (Vec::new(), false);
    };
    let (Some(base_unit), Some(head_unit)) = (
        units.unit(DiffSide::Base, &base.path),
        units.unit(DiffSide::Head, &head.path),
    ) else {
        return (Vec::new(), true);
    };
    let Some(cx) = ClassifyCx::new(
        &base_unit,
        base.local,
        &base.id,
        &head_unit,
        head.local,
        &head.id,
        graphs,
        policy,
    ) else {
        return (Vec::new(), true);
    };
    let mut out: Vec<ClassifiedChange> = Vec::new();
    for classifier in classifiers {
        out.extend(classifier.classify(&cx, sym));
    }
    // Stable order: by class, then the order classifiers produced them.
    let mut indexed: Vec<(usize, ClassifiedChange)> = out.into_iter().enumerate().collect();
    indexed.sort_by(|a, b| a.1.class.cmp(&b.1.class).then(a.0.cmp(&b.0)));
    let mut merged: BTreeMap<super::ChangeClass, ClassifiedChange> = BTreeMap::new();
    for (_, change) in indexed {
        match merged.get_mut(&change.class) {
            Some(existing) => {
                existing.evidence.extend(change.evidence);
                for (k, v) in change.detail {
                    existing.detail.entry(k).or_insert(v);
                }
                if change.certainty == super::Certainty::Exact {
                    existing.certainty = super::Certainty::Exact;
                }
            }
            None => {
                merged.insert(change.class, change);
            }
        }
    }
    (merged.into_values().collect(), !cx.facts_available)
}
