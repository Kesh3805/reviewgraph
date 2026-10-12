//! `return_changed`: the multiset of `return` expression shapes differs (a different
//! expression, a new early return, a removed return).

use analysis_ir::facts::FactKind;
use review_core::change::ChangeClass;

use crate::change::classify::{
    int, Certainty, ClassifiedChange, Classifier, ClassifyCx, FactsView,
};
use crate::change::fact_diff::diff_facts;
use crate::change::symbols::SymbolRecord;

/// Classifier for [`ChangeClass::ReturnChanged`].
#[derive(Debug, Clone, Copy, Default)]
pub struct ReturnClassifier;

impl Classifier for ReturnClassifier {
    fn name(&self) -> &'static str {
        "returns"
    }

    fn classify(
        &self,
        _cx: &ClassifyCx<'_>,
        _rec: &SymbolRecord,
        facts: &FactsView,
    ) -> Vec<ClassifiedChange> {
        let diff = diff_facts(facts.base(), facts.head(), &[FactKind::Return]);
        if diff.added.is_empty() && diff.removed.is_empty() {
            return Vec::new();
        }
        let evidence = diff
            .removed
            .iter()
            .chain(diff.added.iter())
            .cloned()
            .collect();
        vec![
            ClassifiedChange::new(ChangeClass::ReturnChanged, Certainty::Exact)
                .with("added", int(diff.added.len()))
                .with("removed", int(diff.removed.len()))
                .with_evidence(evidence),
        ]
    }
}
