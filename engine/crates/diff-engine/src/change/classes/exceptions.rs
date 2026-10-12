//! `exception_handling_changed` (CHG-004).
//!
//! `TryCatch` keys are `try:{catch,nocatch}:{finally,nofinally}` with detail `empty_catch`,
//! `rethrows`, `catch_param` (TSA-006); `Throw` keys are `throw:{ClassName}`. Subtypes:
//! `try_added`, `try_removed`, `catch_added`, `catch_removed`, `catch_type_changed`,
//! `catch_body_emptied` (swallow), `finally_added`, `finally_removed`, `throw_added`,
//! `throw_removed`, `throw_type_changed`, `rethrow_removed`, and `edges_changed` when only the
//! `THROWS`/`CATCHES` edges differ. Without detail attributes the comparison degrades to keys
//! (`certainty = heuristic`).

use std::collections::BTreeSet;

use analysis_ir::facts::{FactKind, SyntaxFact};
use review_core::location::DiffSide;

use crate::change::classify::{ClassifyCx, Classifier};
use crate::change::fact_diff::{detail, diff_facts, fact_ref};
use crate::change::graph::{Dir, Rel};
use crate::change::{Certainty, ChangeClass, ChangedSymbol, ClassifiedChange, FactRef};

/// The classifier.
#[derive(Debug, Clone, Copy)]
pub struct Exceptions;

fn tries(facts: &[SyntaxFact]) -> Vec<&SyntaxFact> {
    facts.iter().filter(|f| f.kind == FactKind::TryCatch).collect()
}

fn has_catch(f: &SyntaxFact) -> bool {
    f.key.starts_with("try:catch")
}

fn has_finally(f: &SyntaxFact) -> bool {
    f.key.ends_with(":finally")
}

fn throw_class(key: &str) -> &str {
    key.strip_prefix("throw:").unwrap_or(key)
}

impl Classifier for Exceptions {
    fn name(&self) -> &'static str {
        "exceptions"
    }

    fn classify(&self, cx: &ClassifyCx<'_>, _sym: &ChangedSymbol) -> Vec<ClassifiedChange> {
        let mut subtypes: BTreeSet<&'static str> = BTreeSet::new();
        let mut evidence: Vec<FactRef> = Vec::new();
        let mut heuristic = false;

        // Try/catch blocks, aligned in order.
        let base = tries(cx.base_facts);
        let head = tries(cx.head_facts);
        if head.len() > base.len() {
            subtypes.insert("try_added");
            evidence.extend(head[base.len()..].iter().map(|f| fact_ref(DiffSide::Head, f)));
        } else if base.len() > head.len() {
            subtypes.insert("try_removed");
            evidence.extend(base[head.len()..].iter().map(|f| fact_ref(DiffSide::Base, f)));
        }
        for (b, h) in base.iter().zip(head.iter()) {
            let before = subtypes.len();
            match (has_catch(b), has_catch(h)) {
                (false, true) => {
                    subtypes.insert("catch_added");
                }
                (true, false) => {
                    subtypes.insert("catch_removed");
                }
                _ => {}
            }
            match (has_finally(b), has_finally(h)) {
                (false, true) => {
                    subtypes.insert("finally_added");
                }
                (true, false) => {
                    subtypes.insert("finally_removed");
                }
                _ => {}
            }
            match (detail::flag(b, "empty_catch"), detail::flag(h, "empty_catch")) {
                (Some(false), Some(true)) => {
                    subtypes.insert("catch_body_emptied");
                }
                (None, _) | (_, None) => heuristic = true,
                _ => {}
            }
            if detail::flag(b, "rethrows") == Some(true) && detail::flag(h, "rethrows") == Some(false)
            {
                subtypes.insert("rethrow_removed");
            }
            let (bt, ht) = (
                detail::list(b, "catch_types"),
                detail::list(h, "catch_types"),
            );
            if !bt.is_empty() && !ht.is_empty() && bt != ht {
                subtypes.insert("catch_type_changed");
            }
            if subtypes.len() > before {
                evidence.push(fact_ref(DiffSide::Base, b));
                evidence.push(fact_ref(DiffSide::Head, h));
            }
        }

        // Throws, compared by thrown class.
        let throws = diff_facts(cx.base_facts, cx.head_facts, &[FactKind::Throw]);
        if !throws.is_empty() {
            let removed: BTreeSet<&str> = throws.removed.iter().map(|f| throw_class(&f.key)).collect();
            let added: BTreeSet<&str> = throws.added.iter().map(|f| throw_class(&f.key)).collect();
            if !removed.is_empty() && !added.is_empty() && throws.removed.len() == throws.added.len() {
                subtypes.insert("throw_type_changed");
            } else {
                if !removed.is_empty() {
                    subtypes.insert("throw_removed");
                }
                if !added.is_empty() {
                    subtypes.insert("throw_added");
                }
            }
            evidence.extend(throws.removed);
            evidence.extend(throws.added);
        }

        // THROWS/CATCHES edges catch what facts missed; only reported when facts saw nothing.
        if subtypes.is_empty() {
            if let (Some(bg), Some(hg)) = (cx.graph(DiffSide::Base), cx.graph(DiffSide::Head)) {
                let (base_id, head_id) = (cx.base_id.as_str(), cx.head_id.as_str());
                let set = |g: &dyn crate::change::graph::GraphView, id: &str, rel: Rel| -> BTreeSet<String> {
                    g.edges(id, rel, Dir::Out).into_iter().map(|e| e.other).collect()
                };
                let changed = [Rel::Throws, Rel::Catches]
                    .iter()
                    .any(|rel| set(bg, base_id, *rel) != set(hg, head_id, *rel));
                if changed {
                    subtypes.insert("edges_changed");
                    heuristic = true;
                }
            }
        }

        if subtypes.is_empty() {
            return Vec::new();
        }
        let certainty = if heuristic {
            Certainty::Heuristic
        } else {
            Certainty::Exact
        };
        let mut change = ClassifiedChange::new(ChangeClass::ExceptionHandlingChanged, certainty)
            .with("subtypes", subtypes.iter().copied().collect::<Vec<&str>>());
        if let Some(first) = subtypes.iter().next() {
            change = change.with("subtype", *first);
        }
        change.evidence = evidence;
        vec![change]
    }
}
