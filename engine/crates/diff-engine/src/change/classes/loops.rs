//! `loop_changed` (CHG-002): `Loop` facts added, removed or changed kind/header
//! (`loop:for_of:…` to `loop:foreach:…`), plus iterator calls (`map`, `forEach`, `reduce`, …)
//! appearing or disappearing when they replace a loop.

use analysis_ir::facts::FactKind;

use crate::change::classify::{ClassifyCx, Classifier};
use crate::change::fact_diff::diff_facts;
use crate::change::{Certainty, ChangeClass, ChangedSymbol, ClassifiedChange};

/// Loop kind of a `loop:{kind}:{h8}` key.
fn loop_kind(key: &str) -> &str {
    key.strip_prefix("loop:")
        .and_then(|rest| rest.split(':').next())
        .unwrap_or("loop")
}

/// The classifier.
#[derive(Debug, Clone, Copy)]
pub struct Loops;

impl Classifier for Loops {
    fn name(&self) -> &'static str {
        "loops"
    }

    fn classify(&self, cx: &ClassifyCx<'_>, _sym: &ChangedSymbol) -> Vec<ClassifiedChange> {
        let diff = diff_facts(cx.base_facts, cx.head_facts, &[FactKind::Loop]);
        if diff.is_empty() {
            return Vec::new();
        }
        let removed_kinds: Vec<&str> = diff.removed.iter().map(|f| loop_kind(&f.key)).collect();
        let added_kinds: Vec<&str> = diff.added.iter().map(|f| loop_kind(&f.key)).collect();
        let subtype = match (diff.removed.len(), diff.added.len()) {
            (0, _) => "loop_added",
            (_, 0) => "loop_removed",
            _ if removed_kinds != added_kinds => "loop_kind_changed",
            _ => "loop_header_changed",
        };
        let mut change = ClassifiedChange::new(ChangeClass::LoopChanged, Certainty::Exact)
            .with("subtype", subtype)
            .with("removed_kinds", removed_kinds)
            .with("added_kinds", added_kinds);
        change.evidence.extend(diff.removed);
        change.evidence.extend(diff.added);
        vec![change]
    }
}
