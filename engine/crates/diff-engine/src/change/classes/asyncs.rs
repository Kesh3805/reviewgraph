//! `async_behavior_changed` (CHG-004).
//!
//! `Await` keys are `await:{callee}` / `await:expr` / `await:for_of` (TSA-006). Subtypes:
//! `await_added`, `await_removed`, `floating_promise_introduced` (an awaited call still made
//! but no longer awaited, on an async target), `floating_promise_resolved`, `async_added`,
//! `async_removed`, `parallelization_changed` (sequential awaits <-> `Promise.all/allSettled/
//! race`, `for await` <-> `Promise.all(map)`) and `then_chain_changed`. Floating-promise and
//! parallelization verdicts are heuristic (no type information without a semantic provider).

use std::collections::BTreeSet;

use analysis_ir::facts::{FactKind, SyntaxFact};
use analysis_ir::symbol::Modifiers;
use review_core::location::DiffSide;
use serde_json::json;

use crate::change::bind::{callee_text, BoundCall};
use crate::change::classify::{ClassifyCx, Classifier};
use crate::change::fact_diff::{diff_facts, fact_ref};
use crate::change::{Certainty, ChangeClass, ChangedSymbol, ClassifiedChange};

/// The classifier.
#[derive(Debug, Clone, Copy)]
pub struct Async;

fn awaited(facts: &[SyntaxFact]) -> BTreeSet<String> {
    facts
        .iter()
        .filter(|f| f.kind == FactKind::Await)
        .filter_map(|f| f.key.strip_prefix("await:"))
        .filter(|c| *c != "expr" && *c != "for_of")
        .map(str::to_owned)
        .collect()
}

fn called(facts: &[SyntaxFact]) -> BTreeSet<String> {
    facts
        .iter()
        .filter(|f| matches!(f.kind, FactKind::Call | FactKind::New))
        .map(|f| callee_text(&f.key))
        .collect()
}

fn count_calls(facts: &[SyntaxFact], pred: impl Fn(&str) -> bool) -> usize {
    facts
        .iter()
        .filter(|f| f.kind == FactKind::Call && pred(&callee_text(&f.key)))
        .count()
}

fn is_parallel(callee: &str) -> bool {
    matches!(
        callee,
        "Promise.all" | "Promise.allSettled" | "Promise.race" | "Promise.any"
    )
}

fn is_then(callee: &str) -> bool {
    callee.ends_with(".then") || callee.ends_with(".catch") || callee.ends_with(".finally")
}

fn for_await(facts: &[SyntaxFact]) -> usize {
    facts
        .iter()
        .filter(|f| f.kind == FactKind::Await && f.key == "await:for_of")
        .count()
}

fn is_async_target(calls: &[BoundCall], callee: &str) -> bool {
    calls
        .iter()
        .any(|c| c.callee_text == callee && c.target_async)
}

impl Classifier for Async {
    fn name(&self) -> &'static str {
        "async"
    }

    fn classify(&self, cx: &ClassifyCx<'_>, _sym: &ChangedSymbol) -> Vec<ClassifiedChange> {
        let mut subtypes: BTreeSet<&'static str> = BTreeSet::new();
        let mut heuristic = false;
        let mut callees: Vec<serde_json::Value> = Vec::new();

        let base_async = cx.base.modifiers.contains(Modifiers::ASYNC);
        let head_async = cx.head.modifiers.contains(Modifiers::ASYNC);
        if base_async && !head_async {
            subtypes.insert("async_removed");
        } else if head_async && !base_async {
            subtypes.insert("async_added");
        }

        let (base_awaited, head_awaited) = (awaited(cx.base_facts), awaited(cx.head_facts));
        let head_called = called(cx.head_facts);
        let base_called = called(cx.base_facts);
        for callee in base_awaited.difference(&head_awaited) {
            if head_called.contains(callee) {
                // Still called, no longer awaited.
                let async_target = is_async_target(&cx.head_calls, callee)
                    || is_async_target(&cx.base_calls, callee);
                if async_target {
                    subtypes.insert("floating_promise_introduced");
                    heuristic = true;
                } else {
                    subtypes.insert("await_removed");
                }
            } else {
                subtypes.insert("await_removed");
            }
            callees.push(json!({ "callee": callee, "change": "unawaited" }));
        }
        for callee in head_awaited.difference(&base_awaited) {
            if base_called.contains(callee) {
                subtypes.insert("floating_promise_resolved");
            } else {
                subtypes.insert("await_added");
            }
            callees.push(json!({ "callee": callee, "change": "awaited" }));
        }

        let parallel_base = count_calls(cx.base_facts, is_parallel);
        let parallel_head = count_calls(cx.head_facts, is_parallel);
        if parallel_base != parallel_head || for_await(cx.base_facts) != for_await(cx.head_facts)
        {
            subtypes.insert("parallelization_changed");
            heuristic = true;
        }

        let thens = diff_facts(cx.base_facts, cx.head_facts, &[FactKind::Call]);
        let then_changed = thens.added.iter().chain(&thens.removed).any(|f| is_then(&callee_text(&f.key)));
        if then_changed {
            subtypes.insert("then_chain_changed");
        }

        if subtypes.is_empty() {
            return Vec::new();
        }
        let mut evidence = Vec::new();
        let awaits = diff_facts(cx.base_facts, cx.head_facts, &[FactKind::Await]);
        evidence.extend(awaits.removed);
        evidence.extend(awaits.added);
        if evidence.is_empty() {
            evidence.extend(
                cx.head_facts
                    .iter()
                    .filter(|f| f.kind == FactKind::Await)
                    .map(|f| fact_ref(DiffSide::Head, f)),
            );
        }
        let certainty = if heuristic {
            Certainty::Heuristic
        } else {
            Certainty::Exact
        };
        let mut change = ClassifiedChange::new(ChangeClass::AsyncBehaviorChanged, certainty)
            .with("subtypes", subtypes.iter().copied().collect::<Vec<&str>>())
            .with("callees", callees);
        if let Some(first) = subtypes.iter().next() {
            change = change.with("subtype", *first);
        }
        change.evidence = evidence;
        vec![change]
    }
}
