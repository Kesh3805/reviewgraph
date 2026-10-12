//! `condition_changed` (CHG-002): `Condition` facts aligned by LCS; a replaced pair, or an
//! unaligned added/removed condition, is a change. Keys are normalized expression hashes
//! (TSA-006), so the detail records keys, never expression text.

use analysis_ir::facts::FactKind;
use review_core::location::DiffSide;
use serde_json::json;

use crate::change::classify::{ClassifyCx, Classifier};
use crate::change::fact_diff::{align, fact_ref, Aligned};
use crate::change::{bounded, Certainty, ChangeClass, ChangedSymbol, ClassifiedChange};

/// The classifier.
#[derive(Debug, Clone, Copy)]
pub struct Condition;

impl Classifier for Condition {
    fn name(&self) -> &'static str {
        "condition"
    }

    fn classify(&self, cx: &ClassifyCx<'_>, _sym: &ChangedSymbol) -> Vec<ClassifiedChange> {
        let mut change = ClassifiedChange::new(ChangeClass::ConditionChanged, Certainty::Exact);
        let mut pairs = Vec::new();
        let (mut added, mut removed) = (0u32, 0u32);
        for a in align(cx.base_facts, cx.head_facts, FactKind::Condition) {
            match a {
                Aligned::Same(..) => {}
                Aligned::Replaced(b, h) => {
                    change.evidence.push(fact_ref(DiffSide::Base, b));
                    change.evidence.push(fact_ref(DiffSide::Head, h));
                    pairs.push(json!({ "old": bounded(&b.key), "new": bounded(&h.key) }));
                }
                Aligned::Removed(b) => {
                    removed += 1;
                    change.evidence.push(fact_ref(DiffSide::Base, b));
                    pairs.push(json!({ "old": bounded(&b.key), "new": null }));
                }
                Aligned::Added(h) => {
                    added += 1;
                    change.evidence.push(fact_ref(DiffSide::Head, h));
                    pairs.push(json!({ "old": null, "new": bounded(&h.key) }));
                }
            }
        }
        if pairs.is_empty() {
            return Vec::new();
        }
        vec![change
            .with("changes", pairs)
            .with("added", added)
            .with("removed", removed)]
    }
}
