//! The built-in classifiers (CHG-002..CHG-005). Semantics of each class and its evidence are
//! documented in `docs/reviewers/change-classes.md`.

pub mod api_contract;
pub mod asyncs;
pub mod authorization;
pub mod calls;
pub mod condition;
pub mod control_flow;
pub mod db_write;
pub mod dependencies;
pub mod exceptions;
pub mod loops;
pub mod return_type;
pub mod returns;
pub mod transaction;
pub mod validation;

use analysis_ir::facts::SyntaxFact;
use review_core::location::DiffSide;
use serde_json::Value;

use super::bind::BoundCall;
use super::fact_diff::fact_ref;
use super::FactRef;

/// Evidence for a list of facts on one side.
pub(crate) fn refs(side: DiffSide, facts: &[&SyntaxFact]) -> Vec<FactRef> {
    facts.iter().map(|f| fact_ref(side, f)).collect()
}

/// JSON description of a bound call for `detail`.
pub(crate) fn call_json(call: &BoundCall) -> Value {
    serde_json::json!({
        "callee_text": call.callee_text,
        "target_id": call.target_id,
        "count": call.count,
    })
}

/// Evidence pointing at a bound call's first occurrence.
pub(crate) fn call_ref(call: &BoundCall) -> FactRef {
    FactRef {
        side: call.side,
        kind: if call.constructor {
            analysis_ir::facts::FactKind::New
        } else {
            analysis_ir::facts::FactKind::Call
        },
        key: call.callee_text.clone(),
        range: call.range,
    }
}
