//! `validation_removed` (CHG-005): a validation call (by name, or a call fact flagged
//! `validation` by TSA-006), a validation decorator or pipe (on the symbol or its parameters),
//! or a guard clause (a condition immediately followed by a `throw`, or flagged `early_exit`)
//! disappeared. Only removals are classified; adding validation is benign.

use std::collections::BTreeSet;

use analysis_ir::facts::{FactKind, SyntaxFact};
use analysis_ir::symbol::{IrExpr, IrSymbol};
use review_core::location::DiffSide;
use serde_json::json;

use super::call_ref;
use crate::change::classify::{ClassifyCx, Classifier};
use crate::change::fact_diff::{detail, fact_ref};
use crate::change::policy::ClassifierPolicy;
use crate::change::{Certainty, ChangeClass, ChangedSymbol, ClassifiedChange};

/// The classifier.
#[derive(Debug, Clone, Copy)]
pub struct Validation;

fn expr_mentions(expr: &IrExpr, needle: &str) -> bool {
    match expr {
        IrExpr::Ident(s) | IrExpr::Other(s) => s.contains(needle),
        IrExpr::Member(parts) => parts.iter().any(|p| p.contains(needle)),
        IrExpr::Call { callee, args } => {
            callee.iter().any(|c| c.contains(needle)) || args.iter().any(|a| expr_mentions(a, needle))
        }
        IrExpr::Array(items) => items.iter().any(|a| expr_mentions(a, needle)),
        IrExpr::Object(pairs) => pairs.iter().any(|(_, v)| expr_mentions(v, needle)),
        IrExpr::Arrow { returns } => expr_mentions(returns, needle),
        _ => false,
    }
}

/// Validation decorators and pipes of a symbol and its parameters, as stable strings.
fn validators(policy: &ClassifierPolicy, symbol: &IrSymbol) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for d in &symbol.decorators {
        if policy.is_validation_decorator(&d.name)
            || d.args.iter().any(|a| expr_mentions(a, "ValidationPipe"))
        {
            out.insert(format!("@{}", d.name));
        }
    }
    for p in &symbol.params {
        for d in &p.decorators {
            let piped = d
                .args
                .iter()
                .any(|a| expr_mentions(a, "Pipe") || expr_mentions(a, "Validation"));
            if policy.is_validation_decorator(&d.name) || piped {
                out.insert(format!("{}:@{}", p.name, d.name));
            }
        }
    }
    out
}

/// Keys of conditions that guard a `throw` (or are flagged as early exits).
fn guard_clauses(facts: &[SyntaxFact]) -> Vec<&SyntaxFact> {
    let mut out = Vec::new();
    for (i, f) in facts.iter().enumerate() {
        if f.kind != FactKind::Condition {
            continue;
        }
        let next_is_throw = facts
            .get(i + 1)
            .is_some_and(|n| n.kind == FactKind::Throw);
        if next_is_throw || detail::flag(f, "early_exit") == Some(true) && next_is_throw {
            out.push(f);
        }
    }
    out
}

impl Classifier for Validation {
    fn name(&self) -> &'static str {
        "validation"
    }

    fn classify(&self, cx: &ClassifyCx<'_>, _sym: &ChangedSymbol) -> Vec<ClassifiedChange> {
        let mut change = ClassifiedChange::new(ChangeClass::ValidationRemoved, Certainty::Exact);
        let mut removed: Vec<serde_json::Value> = Vec::new();

        for call in &cx.calls.removed {
            let flagged = cx.base_facts.iter().any(|f| {
                matches!(f.kind, FactKind::Call | FactKind::New)
                    && crate::change::bind::callee_text(&f.key) == call.callee_text
                    && detail::flag(f, "validation") == Some(true)
            });
            if flagged || cx.policy.is_validation_call(&call.callee_text) {
                change.evidence.push(call_ref(call));
                removed.push(json!({ "kind": "call", "callee_text": call.callee_text }));
            }
        }

        let (bv, hv) = (validators(cx.policy, cx.base), validators(cx.policy, cx.head));
        for v in bv.difference(&hv) {
            removed.push(json!({ "kind": "decorator", "name": v }));
        }

        let head_keys: BTreeSet<&str> = cx
            .head_facts
            .iter()
            .filter(|f| f.kind == FactKind::Condition)
            .map(|f| f.key.as_str())
            .collect();
        for clause in guard_clauses(cx.base_facts) {
            if !head_keys.contains(clause.key.as_str()) {
                change.evidence.push(fact_ref(DiffSide::Base, clause));
                removed.push(json!({ "kind": "guard_clause", "key": clause.key }));
            }
        }

        if removed.is_empty() {
            return Vec::new();
        }
        vec![change.with("removed", removed)]
    }
}
