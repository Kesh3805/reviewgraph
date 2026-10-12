//! Class-driven signals (RISK-003): what the classified changes of a symbol say about risk.
//!
//! One table row per rule of `RISK-003`; the subject is the changed symbol and the confidence
//! is the classification confidence. `exception_handling_changed` (including swallowed
//! exceptions) deliberately produces **no** signal: PRD §37 has no error-handling category, so
//! it influences reviewer routing through the change class only.

use super::new_signal;
use super::policy::RiskPolicy;
use super::signal::{dedup_signals, RiskSignal, SignalEvidence, SignalSubject};
use crate::input::{tags, ChangeClass, ChangeSet, ClassInput, SymbolInput};

/// The rule ids a classified change maps to.
fn rules_for(class: &ClassInput) -> Vec<&'static str> {
    match class.class {
        ChangeClass::AuthorizationChanged => {
            let mut ids = Vec::new();
            if class.has(tags::AUTHZ_CALL_REMOVED) {
                ids.push("authz.call_removed");
            }
            if class.has(tags::DIRECT_ROLE_COMPARISON_ADDED) {
                ids.push("authz.direct_role_check");
            }
            if class.has(tags::GUARD_REMOVED) {
                ids.push("authz.guard_removed");
            }
            if class.has(tags::GUARD_ADDED) {
                ids.push("authz.guard_added");
            }
            if ids.is_empty() {
                ids.push("authz.logic_changed");
            }
            ids
        }
        ChangeClass::ValidationRemoved => vec!["validation.removed"],
        ChangeClass::DatabaseWriteChanged => {
            if class.has(tags::INSIDE_TRANSACTION) {
                vec!["db.write_changed_in_tx"]
            } else {
                vec!["db.write_changed"]
            }
        }
        ChangeClass::TransactionBoundaryChanged => vec!["tx.boundary_changed"],
        ChangeClass::ApiContractChanged => {
            if class.has(tags::ADDITIVE) && !class.has(tags::BREAKING) {
                vec!["api.additive_change"]
            } else {
                vec!["api.breaking_change"]
            }
        }
        ChangeClass::AsyncBehaviorChanged => {
            if class.has(tags::FLOATING_PROMISE) || class.has(tags::AWAIT_REMOVED_ON_DB_WRITE) {
                vec!["async.floating_promise"]
            } else {
                Vec::new()
            }
        }
        _ => Vec::new(),
    }
}

fn evidence(symbol: &SymbolInput, detail: String) -> Vec<SignalEvidence> {
    vec![SignalEvidence {
        file: Some(symbol.path().clone()),
        symbol: Some(symbol.key()),
        node_id: Some(symbol.id().to_owned()),
        detail,
        ..SignalEvidence::default()
    }]
}

/// Signals from the change classes of every changed, non-cosmetic, non-test symbol.
pub fn detect_change_signals(changes: &ChangeSet, policy: &RiskPolicy) -> Vec<RiskSignal> {
    let mut out = Vec::new();
    for symbol in &changes.symbols {
        if symbol.is_skipped() || symbol.test {
            continue;
        }
        let subject = SignalSubject::Symbol(symbol.key());
        for class in &symbol.classes {
            for rule_id in rules_for(class) {
                out.extend(new_signal(
                    rule_id,
                    subject.clone(),
                    class.confidence,
                    evidence(symbol, format!("change class {}", class.class.as_str())),
                ));
            }
        }
        // A removed call to a configured authorization symbol is an authorization call
        // removal even when the classifier did not tag it.
        for call in &symbol.removed_calls {
            if policy.is_authorization_symbol(&call.callee_text) {
                out.extend(new_signal(
                    "authz.call_removed",
                    subject.clone(),
                    call.confidence,
                    evidence(
                        symbol,
                        "removed call to a configured authorization symbol".to_owned(),
                    ),
                ));
            }
        }
    }
    dedup_signals(out)
}
