//! `return_changed` (CHG-002): the multiset of `Return` keys differs (returned expression
//! shape, number of returns, early returns).

use analysis_ir::facts::FactKind;

use crate::change::classify::{ClassifyCx, Classifier};
use crate::change::fact_diff::diff_facts;
use crate::change::{Certainty, ChangeClass, ChangedSymbol, ClassifiedChange};

/// The classifier.
#[derive(Debug, Clone, Copy)]
pub struct Returns;

impl Classifier for Returns {
    fn name(&self) -> &'static str {
        "returns"
    }

    fn classify(&self, cx: &ClassifyCx<'_>, _sym: &ChangedSymbol) -> Vec<ClassifiedChange> {
        let diff = diff_facts(cx.base_facts, cx.head_facts, &[FactKind::Return]);
        if diff.is_empty() {
            return Vec::new();
        }
        let mut change = ClassifiedChange::new(ChangeClass::ReturnChanged, Certainty::Exact)
            .with("added", diff.added.len())
            .with("removed", diff.removed.len());
        change.evidence.extend(diff.removed);
        change.evidence.extend(diff.added);
        vec![change]
    }
}
