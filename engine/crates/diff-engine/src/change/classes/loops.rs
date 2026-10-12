//! `loop_changed`: a loop was added, removed, changed kind (`for_of` to `while`) or got a
//! different header or condition.

use analysis_ir::facts::FactKind;
use analysis_ir::symbol::AttrValue;
use review_core::change::ChangeClass;

use crate::change::classify::{
    int, Certainty, ClassifiedChange, Classifier, ClassifyCx, FactsView,
};
use crate::change::fact_diff::{diff_facts, FactRef};
use crate::change::symbols::SymbolRecord;

/// The loop label of a key `loop:<label>:<hash>`.
fn label(key: &str) -> &str {
    key.split(':').nth(1).unwrap_or("")
}

fn labels(refs: &[FactRef]) -> String {
    let mut out: Vec<&str> = refs.iter().map(|r| label(&r.key)).collect();
    out.sort_unstable();
    out.dedup();
    out.join(",")
}

/// Classifier for [`ChangeClass::LoopChanged`].
#[derive(Debug, Clone, Copy, Default)]
pub struct LoopClassifier;

impl Classifier for LoopClassifier {
    fn name(&self) -> &'static str {
        "loops"
    }

    fn classify(
        &self,
        _cx: &ClassifyCx<'_>,
        _rec: &SymbolRecord,
        facts: &FactsView,
    ) -> Vec<ClassifiedChange> {
        let diff = diff_facts(facts.base(), facts.head(), &[FactKind::Loop]);
        if diff.added.is_empty() && diff.removed.is_empty() {
            return Vec::new();
        }
        let (old, new) = (labels(&diff.removed), labels(&diff.added));
        let subtype = match (diff.removed.is_empty(), diff.added.is_empty()) {
            (true, _) => "added",
            (_, true) => "removed",
            _ if old != new => "kind_changed",
            _ => "header_changed",
        };
        let evidence = diff
            .removed
            .iter()
            .chain(diff.added.iter())
            .cloned()
            .collect();
        vec![
            ClassifiedChange::new(ChangeClass::LoopChanged, Certainty::Exact)
                .with("subtype", AttrValue::Str(subtype.to_owned()))
                .with("added", int(diff.added.len()))
                .with("removed", int(diff.removed.len()))
                .with("old", AttrValue::Str(old))
                .with("new", AttrValue::Str(new))
                .with_evidence(evidence),
        ]
    }
}
