//! `call_added` / `call_removed` (CHG-003) from the bound call delta. A callee text bound to a
//! different target on head is reported as one removal plus one addition with
//! `detail.retargeted = true`; calls to the same target that merely moved are ignored.

use serde_json::Value;

use super::{call_json, call_ref};
use crate::change::bind::BoundCall;
use crate::change::classify::{ClassifyCx, Classifier};
use crate::change::{Certainty, ChangeClass, ChangedSymbol, ClassifiedChange};

/// The classifier.
#[derive(Debug, Clone, Copy)]
pub struct Calls;

fn build(class: ChangeClass, calls: &[(&BoundCall, bool)]) -> Option<ClassifiedChange> {
    if calls.is_empty() {
        return None;
    }
    let certainty = if calls.iter().all(|(c, _)| c.target_id.is_some()) {
        Certainty::Exact
    } else {
        Certainty::Heuristic
    };
    let mut change = ClassifiedChange::new(class, certainty);
    let mut list = Vec::new();
    for (call, retargeted) in calls {
        change.evidence.push(call_ref(call));
        let mut json = call_json(call);
        if let Value::Object(map) = &mut json {
            map.insert("retargeted".to_owned(), Value::Bool(*retargeted));
        }
        list.push(json);
    }
    Some(change.with("calls", list))
}

impl Classifier for Calls {
    fn name(&self) -> &'static str {
        "calls"
    }

    fn classify(&self, cx: &ClassifyCx<'_>, _sym: &ChangedSymbol) -> Vec<ClassifiedChange> {
        let mut removed: Vec<(&BoundCall, bool)> =
            cx.calls.removed.iter().map(|c| (c, false)).collect();
        let mut added: Vec<(&BoundCall, bool)> = cx.calls.added.iter().map(|c| (c, false)).collect();
        for (base, head) in &cx.calls.retargeted {
            removed.push((base, true));
            added.push((head, true));
        }
        build(ChangeClass::CallRemoved, &removed)
            .into_iter()
            .chain(build(ChangeClass::CallAdded, &added))
            .collect()
    }
}
