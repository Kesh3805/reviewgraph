//! `authorization_changed` (CHG-005). Any of, each contributing evidence:
//!
//! * (a) guard decorators (`GuardDecorator` facts or decorators matching the policy) or
//!   `AUTHORIZES` in-edges differ between the sides;
//! * (b) a removed/added bound call that is an authorization call by name or whose target
//!   carries `AUTHORIZES` edges;
//! * (c) a condition added, removed or changed whose identifiers include an authorization
//!   identifier (`role`, `permissions`, ...);
//! * (d) a `throw` of a `Forbidden*`/`Unauthorized*` class added or removed.
//!
//! `detail.direction` is `weakened` when guards/authorization calls/throws were removed with
//! none added, `strengthened` for the opposite, `unknown` otherwise (heuristic).

use std::collections::BTreeSet;

use analysis_ir::facts::{FactKind, SyntaxFact};
use review_core::location::DiffSide;
use serde_json::json;

use super::call_ref;
use crate::change::bind::BoundCall;
use crate::change::classify::{ClassifyCx, Classifier};
use crate::change::fact_diff::{align, detail, diff_facts, fact_ref, Aligned};
use crate::change::graph::{Dir, GraphView, Rel};
use crate::change::policy::ClassifierPolicy;
use crate::change::{Certainty, ChangeClass, ChangedSymbol, ClassifiedChange};
use crate::metrics;

/// The classifier.
#[derive(Debug, Clone, Copy)]
pub struct Authorization;

fn guards(cx: &ClassifyCx<'_>, side: DiffSide) -> BTreeSet<String> {
    let (symbol, facts) = match side {
        DiffSide::Base => (cx.base, cx.base_facts),
        DiffSide::Head => (cx.head, cx.head_facts),
    };
    let mut out: BTreeSet<String> = symbol
        .decorators
        .iter()
        .filter(|d| cx.policy.is_guard_decorator(&d.name))
        .map(|d| format!("{}({:?})", d.name, d.args))
        .collect();
    out.extend(
        facts
            .iter()
            .filter(|f| f.kind == FactKind::GuardDecorator)
            .map(|f| f.key.clone()),
    );
    out
}

fn authorizers(graph: Option<&dyn GraphView>, id: &str) -> BTreeSet<String> {
    graph
        .map(|g| {
            g.edges(id, Rel::Authorizes, Dir::In)
                .into_iter()
                .map(|e| e.other)
                .collect()
        })
        .unwrap_or_default()
}

fn is_authz_call(policy: &ClassifierPolicy, graph: Option<&dyn GraphView>, call: &BoundCall) -> bool {
    if policy.is_authz_call(&call.callee_text) {
        return true;
    }
    match (graph, &call.target_id) {
        (Some(g), Some(target)) => !g.edges(target, Rel::Authorizes, Dir::Out).is_empty(),
        _ => false,
    }
}

fn authz_condition(policy: &ClassifierPolicy, fact: &SyntaxFact) -> bool {
    detail::list(fact, "idents")
        .iter()
        .any(|i| policy.is_authz_identifier(i))
}

impl Classifier for Authorization {
    fn name(&self) -> &'static str {
        "authorization"
    }

    fn classify(&self, cx: &ClassifyCx<'_>, _sym: &ChangedSymbol) -> Vec<ClassifiedChange> {
        let mut change = ClassifiedChange::new(ChangeClass::AuthorizationChanged, Certainty::Exact);
        let (mut weakening, mut strengthening) = (false, false);
        let mut reasons: BTreeSet<&'static str> = BTreeSet::new();

        // (a) guards and AUTHORIZES edges.
        let (bg, hg) = (guards(cx, DiffSide::Base), guards(cx, DiffSide::Head));
        let removed_guards: Vec<&String> = bg.difference(&hg).collect();
        let added_guards: Vec<&String> = hg.difference(&bg).collect();
        let (ba, ha) = (
            authorizers(cx.graph(DiffSide::Base), cx.base_id.as_str()),
            authorizers(cx.graph(DiffSide::Head), cx.head_id.as_str()),
        );
        if !removed_guards.is_empty() || ba.difference(&ha).next().is_some() {
            weakening = true;
            reasons.insert("guard_removed");
        }
        if !added_guards.is_empty() || ha.difference(&ba).next().is_some() {
            strengthening = true;
            reasons.insert("guard_added");
        }
        for f in cx.base_facts.iter().filter(|f| f.kind == FactKind::GuardDecorator) {
            if removed_guards.iter().any(|g| **g == f.key) {
                change.evidence.push(fact_ref(DiffSide::Base, f));
            }
        }
        for f in cx.head_facts.iter().filter(|f| f.kind == FactKind::GuardDecorator) {
            if added_guards.iter().any(|g| **g == f.key) {
                change.evidence.push(fact_ref(DiffSide::Head, f));
            }
        }

        // (b) authorization calls.
        let mut calls = Vec::new();
        for call in cx
            .calls
            .removed
            .iter()
            .chain(cx.calls.retargeted.iter().map(|(b, _)| b))
        {
            if is_authz_call(cx.policy, cx.graph(DiffSide::Base), call) {
                weakening = true;
                reasons.insert("authz_call_removed");
                change.evidence.push(call_ref(call));
                calls.push(json!({ "removed": call.callee_text, "target_id": call.target_id }));
            }
        }
        for call in cx
            .calls
            .added
            .iter()
            .chain(cx.calls.retargeted.iter().map(|(_, h)| h))
        {
            if is_authz_call(cx.policy, cx.graph(DiffSide::Head), call) {
                strengthening = true;
                reasons.insert("authz_call_added");
                change.evidence.push(call_ref(call));
                calls.push(json!({ "added": call.callee_text, "target_id": call.target_id }));
            }
        }

        // (c) authorization-relevant conditions.
        for a in align(cx.base_facts, cx.head_facts, FactKind::Condition) {
            match a {
                Aligned::Same(..) => {}
                Aligned::Replaced(b, h) => {
                    if authz_condition(cx.policy, b) || authz_condition(cx.policy, h) {
                        reasons.insert("authz_condition_changed");
                        change.evidence.push(fact_ref(DiffSide::Base, b));
                        change.evidence.push(fact_ref(DiffSide::Head, h));
                    }
                }
                Aligned::Removed(b) => {
                    if authz_condition(cx.policy, b) {
                        reasons.insert("authz_condition_removed");
                        change.evidence.push(fact_ref(DiffSide::Base, b));
                    }
                }
                Aligned::Added(h) => {
                    if authz_condition(cx.policy, h) {
                        reasons.insert("authz_condition_added");
                        change.evidence.push(fact_ref(DiffSide::Head, h));
                    }
                }
            }
        }

        // (d) authorization exceptions.
        let throws = diff_facts(cx.base_facts, cx.head_facts, &[FactKind::Throw]);
        for f in &throws.removed {
            if cx.policy.is_authz_exception(f.key.strip_prefix("throw:").unwrap_or(&f.key)) {
                weakening = true;
                reasons.insert("authz_throw_removed");
                change.evidence.push(f.clone());
            }
        }
        for f in &throws.added {
            if cx.policy.is_authz_exception(f.key.strip_prefix("throw:").unwrap_or(&f.key)) {
                strengthening = true;
                reasons.insert("authz_throw_added");
                change.evidence.push(f.clone());
            }
        }

        if reasons.is_empty() {
            return Vec::new();
        }
        let direction = match (weakening, strengthening) {
            (true, false) => "weakened",
            (false, true) => "strengthened",
            _ => "unknown",
        };
        metrics::record_authz_direction(direction);
        let certainty = if direction == "unknown" {
            Certainty::Heuristic
        } else {
            Certainty::Exact
        };
        change.certainty = certainty;
        vec![change
            .with("direction", direction)
            .with("reasons", reasons.into_iter().collect::<Vec<&str>>())
            .with("calls", calls)
            .with(
                "guards_removed",
                removed_guards.iter().map(|g| g.as_str()).collect::<Vec<&str>>(),
            )
            .with(
                "guards_added",
                added_guards.iter().map(|g| g.as_str()).collect::<Vec<&str>>(),
            )]
    }
}
