//! `control_flow_changed` (CHG-002): branch points (`Condition`, `Loop`, `Throw`, `TryCatch`,
//! `Return`) inserted, removed or reordered. A condition whose expression changed in place is
//! `condition_changed` only.

use analysis_ir::facts::FactKind;

use crate::change::classify::{ClassifyCx, Classifier};
use crate::change::fact_diff::{diff_facts, fact_ref};
use crate::change::{Certainty, ChangeClass, ChangedSymbol, ClassifiedChange};
use review_core::location::DiffSide;

/// Branch-point kinds.
pub const BRANCH_KINDS: [FactKind; 5] = [
    FactKind::Condition,
    FactKind::Loop,
    FactKind::Throw,
    FactKind::TryCatch,
    FactKind::Return,
];

/// The classifier.
#[derive(Debug, Clone, Copy)]
pub struct ControlFlow;

impl Classifier for ControlFlow {
    fn name(&self) -> &'static str {
        "control_flow"
    }

    fn classify(&self, cx: &ClassifyCx<'_>, _sym: &ChangedSymbol) -> Vec<ClassifiedChange> {
        let shape = |facts: &[analysis_ir::facts::SyntaxFact]| -> Vec<FactKind> {
            facts
                .iter()
                .filter(|f| BRANCH_KINDS.contains(&f.kind))
                .map(|f| f.kind)
                .collect()
        };
        let base_shape = shape(cx.base_facts);
        let head_shape = shape(cx.head_facts);
        let diff = diff_facts(cx.base_facts, cx.head_facts, &BRANCH_KINDS);
        let structural = base_shape != head_shape;
        if !structural && !diff.reordered {
            return Vec::new();
        }
        let subtype = if diff.reordered && !structural {
            "reordered"
        } else if head_shape.len() > base_shape.len() {
            "inserted"
        } else if head_shape.len() < base_shape.len() {
            "removed"
        } else {
            "restructured"
        };
        let mut change = ClassifiedChange::new(ChangeClass::ControlFlowChanged, Certainty::Exact)
            .with("subtype", subtype)
            .with("base_branch_points", base_shape.len())
            .with("head_branch_points", head_shape.len());
        change.evidence.extend(diff.removed);
        change.evidence.extend(diff.added);
        if change.evidence.is_empty() {
            // Pure reorder: point at the head branch points.
            change.evidence = cx
                .head_facts
                .iter()
                .filter(|f| BRANCH_KINDS.contains(&f.kind))
                .map(|f| fact_ref(DiffSide::Head, f))
                .collect();
        }
        vec![change]
    }
}
