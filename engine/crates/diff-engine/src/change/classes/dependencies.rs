//! `dependency_added` / `dependency_removed` (CHG-003): the set of owners (classes, modules,
//! packages) a symbol uses through bound calls, plus constructor-injected types for
//! constructors. A dependency is removed only when *no* use of that owner remains, so swapping
//! one method of a service for another is a call change, not a dependency change.

use std::collections::BTreeMap;

use analysis_ir::symbol::IrSymbol;
use analysis_ir::unit::ParsedUnit;
use review_core::symbol::SymbolKind;
use serde_json::json;

use crate::change::bind::BoundCall;
use crate::change::classify::{ClassifyCx, Classifier};
use crate::change::{Certainty, ChangeClass, ChangedSymbol, ClassifiedChange, FactRef};

/// How a dependency is used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Via {
    Call,
    Di,
}

impl Via {
    fn as_str(self) -> &'static str {
        match self {
            Self::Call => "call",
            Self::Di => "di",
        }
    }
}

/// Owner name -> (owner id if resolved, via, evidence).
type Deps = BTreeMap<String, (Option<String>, Via, Option<FactRef>)>;

/// `Foo<Bar>[]` -> `Foo`.
fn type_name(text: &str) -> String {
    let t = text.trim();
    let end = t
        .find(|c: char| c == '<' || c == '[' || c == '|' || c == ' ')
        .unwrap_or(t.len());
    t[..end].trim().to_owned()
}

fn own_class(unit: &ParsedUnit, symbol: &IrSymbol) -> Option<String> {
    symbol
        .parent
        .and_then(|p| unit.symbols.get(p.0 as usize))
        .map(|p| p.name.clone())
}

fn deps(unit: &ParsedUnit, symbol: &IrSymbol, calls: &[BoundCall]) -> Deps {
    let own = own_class(unit, symbol);
    let mut out = Deps::new();
    for call in calls {
        let Some(name) = call.owner_name.clone() else {
            continue;
        };
        if Some(&name) == own.as_ref() || name == symbol.name {
            continue;
        }
        out.entry(name).or_insert((
            call.owner_id.clone(),
            Via::Call,
            Some(super::call_ref(call)),
        ));
    }
    if symbol.kind == SymbolKind::Constructor {
        for param in symbol.params.iter().filter(|p| p.property.is_some()) {
            if let Some(t) = param.type_text.as_deref() {
                let name = type_name(t);
                if !name.is_empty() {
                    out.entry(name).or_insert((None, Via::Di, None));
                }
            }
        }
    }
    out
}

/// The classifier.
#[derive(Debug, Clone, Copy)]
pub struct Dependencies;

fn build(class: ChangeClass, names: Vec<(&String, &(Option<String>, Via, Option<FactRef>))>) -> Option<ClassifiedChange> {
    if names.is_empty() {
        return None;
    }
    let mut change = ClassifiedChange::new(class, Certainty::Exact);
    let mut list = Vec::new();
    for (name, (id, via, evidence)) in names {
        list.push(json!({ "owner": name, "owner_id": id, "via": via.as_str() }));
        if let Some(e) = evidence {
            change.evidence.push(e.clone());
        }
    }
    Some(change.with("dependencies", list))
}

impl Classifier for Dependencies {
    fn name(&self) -> &'static str {
        "dependencies"
    }

    fn classify(&self, cx: &ClassifyCx<'_>, _sym: &ChangedSymbol) -> Vec<ClassifiedChange> {
        let base = deps(cx.base_unit, cx.base, &cx.base_calls);
        let head = deps(cx.head_unit, cx.head, &cx.head_calls);
        let removed: Vec<_> = base.iter().filter(|(n, _)| !head.contains_key(*n)).collect();
        let added: Vec<_> = head.iter().filter(|(n, _)| !base.contains_key(*n)).collect();
        build(ChangeClass::DependencyRemoved, removed)
            .into_iter()
            .chain(build(ChangeClass::DependencyAdded, added))
            .collect()
    }
}
