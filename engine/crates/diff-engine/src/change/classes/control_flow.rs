//! `control_flow_changed`: the structure of branch points differs.
//!
//! Branch points are statement conditions (`if`, ternary, `switch`), loops, `throw`, `try` and
//! `return`. A comparison used as a value (`return a === b`) is not a branch point. The class
//! fires when branch points are inserted or removed (the ordered sequence of branch-point
//! kinds differs) or when the same branch points appear in a different order. A branch point
//! whose own key changed in place (an edited condition) is `condition_changed`, not this.

use analysis_ir::facts::{FactKind, SyntaxFact};
use analysis_ir::symbol::AttrValue;
use review_core::change::ChangeClass;
use review_core::location::DiffSide;

use crate::change::classify::{
    int, Certainty, ClassifiedChange, Classifier, ClassifyCx, FactRef, FactsView,
};
use crate::change::fact_diff::diff_facts;
use crate::change::symbols::SymbolRecord;

const KINDS: [FactKind; 5] = [
    FactKind::Condition,
    FactKind::Loop,
    FactKind::Throw,
    FactKind::TryCatch,
    FactKind::Return,
];

/// Evidence cap for a pure reorder.
const MAX_REORDER_EVIDENCE: usize = 16;

fn is_branch_point(fact: &SyntaxFact) -> bool {
    fact.kind != FactKind::Condition
        || !matches!(fact.detail.get("expression"), Some(AttrValue::Bool(true)))
}

/// Classifier for [`ChangeClass::ControlFlowChanged`].
#[derive(Debug, Clone, Copy, Default)]
pub struct ControlFlowClassifier;

impl Classifier for ControlFlowClassifier {
    fn name(&self) -> &'static str {
        "control_flow"
    }

    fn classify(
        &self,
        _cx: &ClassifyCx<'_>,
        _rec: &SymbolRecord,
        facts: &FactsView,
    ) -> Vec<ClassifiedChange> {
        let base: Vec<SyntaxFact> = facts
            .base()
            .iter()
            .filter(|f| KINDS.contains(&f.kind) && is_branch_point(f))
            .cloned()
            .collect();
        let head: Vec<SyntaxFact> = facts
            .head()
            .iter()
            .filter(|f| KINDS.contains(&f.kind) && is_branch_point(f))
            .cloned()
            .collect();
        let diff = diff_facts(&base, &head, &KINDS);
        if diff.is_empty() {
            return Vec::new();
        }
        let multiset_equal = diff.added.is_empty() && diff.removed.is_empty();
        let kinds = |list: &[SyntaxFact]| list.iter().map(|f| f.kind).collect::<Vec<_>>();
        let structural = kinds(&base) != kinds(&head);
        if !multiset_equal && !structural {
            return Vec::new();
        }
        let evidence: Vec<FactRef> = if multiset_equal {
            head.iter()
                .take(MAX_REORDER_EVIDENCE)
                .map(|f| FactRef::of(DiffSide::Head, f))
                .collect()
        } else {
            diff.removed
                .iter()
                .chain(diff.added.iter())
                .cloned()
                .collect()
        };
        vec![
            ClassifiedChange::new(ChangeClass::ControlFlowChanged, Certainty::Exact)
                .with("added", int(diff.added.len()))
                .with("removed", int(diff.removed.len()))
                .with("reordered", AttrValue::Bool(multiset_equal))
                .with_evidence(evidence),
        ]
    }
}
