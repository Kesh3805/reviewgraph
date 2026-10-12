//! `condition_changed`: a condition was added, removed or edited.
//!
//! Conditions are keyed by a hash of their normalized expression, so an edited condition
//! appears as one removed and one added key; a moved one appears as nothing.

use analysis_ir::facts::FactKind;
use analysis_ir::symbol::AttrValue;
use review_core::change::ChangeClass;

use crate::change::classify::{
    int, Certainty, ClassifiedChange, Classifier, ClassifyCx, FactsView,
};
use crate::change::fact_diff::{diff_facts, short, FactRef};
use crate::change::symbols::SymbolRecord;

/// Longest `old` / `new` detail text.
const MAX_DETAIL: usize = 120;

fn joined(refs: &[FactRef]) -> String {
    short(
        &refs
            .iter()
            .map(|r| r.key.as_str())
            .collect::<Vec<_>>()
            .join(","),
        MAX_DETAIL,
    )
}

/// Classifier for [`ChangeClass::ConditionChanged`].
#[derive(Debug, Clone, Copy, Default)]
pub struct ConditionClassifier;

impl Classifier for ConditionClassifier {
    fn name(&self) -> &'static str {
        "condition"
    }

    fn classify(
        &self,
        _cx: &ClassifyCx<'_>,
        _rec: &SymbolRecord,
        facts: &FactsView,
    ) -> Vec<ClassifiedChange> {
        let diff = diff_facts(facts.base(), facts.head(), &[FactKind::Condition]);
        if diff.added.is_empty() && diff.removed.is_empty() {
            return Vec::new();
        }
        let change = ClassifiedChange::new(ChangeClass::ConditionChanged, Certainty::Exact)
            .with("added", int(diff.added.len()))
            .with("removed", int(diff.removed.len()))
            .with("old", AttrValue::Str(joined(&diff.removed)))
            .with("new", AttrValue::Str(joined(&diff.added)));
        let evidence = diff
            .removed
            .iter()
            .chain(diff.added.iter())
            .cloned()
            .collect();
        vec![change.with_evidence(evidence)]
    }
}
