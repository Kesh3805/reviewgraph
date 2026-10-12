//! Call binding (CHG-003): `Call`/`New` facts of a symbol joined with the `CALLS` edges of that
//! side's graph, and the call delta between base and head.
//!
//! Removed calls are resolved against the **base** graph and added calls against the **head**
//! graph, so a callee deleted by the pull request still resolves. Unresolved calls are kept and
//! compared by their normalized callee text.

use std::collections::BTreeMap;

use analysis_ir::facts::{FactKind, SyntaxFact};
use review_core::location::{DiffSide, SourceRange};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::bounded;
use super::graph::{Dir, GraphView, Rel};
use super::policy::last_segment;
use crate::metrics;

/// One distinct call of a symbol on one side.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BoundCall {
    /// Normalized callee text (`this.permissionService.check`), at most 120 characters.
    pub callee_text: String,
    /// Resolved target node id.
    pub target_id: Option<String>,
    /// Owner of the target: its class/module, or the target itself for packages.
    pub owner_id: Option<String>,
    /// Terminal name of the owner (`PermissionService`), for dependency comparison.
    pub owner_name: Option<String>,
    /// The target is an external package/API.
    pub external: bool,
    /// The target is an `async` callable.
    pub target_async: bool,
    /// Occurrences on this side.
    pub count: u16,
    /// Range of the first occurrence.
    pub range: SourceRange,
    /// Side the call was read from.
    pub side: DiffSide,
    /// `New` (constructor call) rather than `Call`.
    pub constructor: bool,
}

impl BoundCall {
    /// Terminal callee name (`check`).
    pub fn name(&self) -> &str {
        last_segment(&self.callee_text)
    }
}

/// Callee text of a `Call`/`New` fact key: `call:this.a.b/2` -> `this.a.b`, `new:Foo/1` -> `Foo`.
pub fn callee_text(key: &str) -> String {
    let body = key
        .strip_prefix("call:")
        .or_else(|| key.strip_prefix("new:"))
        .unwrap_or(key);
    let body = match body.rsplit_once('/') {
        Some((callee, argc)) if argc.bytes().all(|b| b.is_ascii_digit()) => callee,
        _ => body,
    };
    bounded(body)
}

/// Bind the `Call`/`New` facts of `symbol_id` on `side`. Calls with the same callee text are
/// merged with a count; the target is the `CALLS` edge whose target name equals the callee's
/// terminal name, preferring an edge located inside the fact's lines.
pub fn bind_calls(
    facts: &[SyntaxFact],
    symbol_id: &str,
    graph: Option<&dyn GraphView>,
    side: DiffSide,
) -> Vec<BoundCall> {
    let edges: Vec<(String, Option<u32>, String)> = graph
        .map(|g| {
            g.edges(symbol_id, Rel::Calls, Dir::Out)
                .into_iter()
                .filter_map(|e| {
                    let name = g.node(&e.other).map(|n| n.name)?;
                    Some((e.other, e.line, name))
                })
                .collect()
        })
        .unwrap_or_default();

    let mut by_text: BTreeMap<String, BoundCall> = BTreeMap::new();
    for fact in facts
        .iter()
        .filter(|f| matches!(f.kind, FactKind::Call | FactKind::New))
    {
        let text = callee_text(&fact.key);
        if let Some(existing) = by_text.get_mut(&text) {
            existing.count = existing.count.saturating_add(1);
            continue;
        }
        let terminal = last_segment(&text).to_owned();
        let candidates: Vec<&(String, Option<u32>, String)> =
            edges.iter().filter(|(_, _, name)| *name == terminal).collect();
        let (start, end) = (fact.range.start.line, fact.range.end.line);
        let chosen = candidates
            .iter()
            .find(|(_, line, _)| line.is_some_and(|l| l >= start && l <= end))
            .or_else(|| candidates.first())
            .map(|(id, _, _)| id.clone());
        let info = chosen.as_deref().and_then(|id| graph.and_then(|g| g.node(id)));
        let (owner_id, owner_name) = match &info {
            Some(node) if node.external => (Some(node.id.clone()), Some(node.name.clone())),
            Some(node) => match node.parent.as_deref().and_then(|p| graph.and_then(|g| g.node(p))) {
                Some(owner) => (Some(owner.id), Some(owner.name)),
                None => (Some(node.id.clone()), Some(node.name.clone())),
            },
            None => (None, None),
        };
        if chosen.is_none() {
            metrics::record_unresolved_call(match side {
                DiffSide::Base => "base",
                DiffSide::Head => "head",
            });
        }
        by_text.insert(
            text.clone(),
            BoundCall {
                callee_text: text,
                target_id: chosen,
                owner_id,
                owner_name,
                external: info.as_ref().is_some_and(|n| n.external),
                target_async: info.as_ref().is_some_and(|n| n.is_async),
                count: 1,
                range: fact.range,
                side,
                constructor: fact.kind == FactKind::New,
            },
        );
    }
    by_text.into_values().collect()
}

/// Calls added, removed and retargeted between base and head.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CallDelta {
    /// Calls only on head (or more often on head), bound against the head graph.
    pub added: Vec<BoundCall>,
    /// Calls only on base (or more often on base), bound against the base graph.
    pub removed: Vec<BoundCall>,
    /// Same callee text bound to a different target: `(base, head)`.
    pub retargeted: Vec<(BoundCall, BoundCall)>,
}

impl CallDelta {
    /// No call changed.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.retargeted.is_empty()
    }
}

/// Compare the bound calls of both sides.
pub fn call_delta(base: &[BoundCall], head: &[BoundCall]) -> CallDelta {
    let mut delta = CallDelta::default();
    let head_by_text: BTreeMap<&str, &BoundCall> =
        head.iter().map(|c| (c.callee_text.as_str(), c)).collect();
    let base_by_text: BTreeMap<&str, &BoundCall> =
        base.iter().map(|c| (c.callee_text.as_str(), c)).collect();

    let mut removed: Vec<BoundCall> = Vec::new();
    let mut added: Vec<BoundCall> = Vec::new();
    for b in base {
        match head_by_text.get(b.callee_text.as_str()) {
            None => removed.push(b.clone()),
            Some(h) => {
                let retargeted = matches!((&b.target_id, &h.target_id), (Some(x), Some(y)) if x != y);
                if retargeted {
                    delta.retargeted.push((b.clone(), (*h).clone()));
                } else if b.count > h.count {
                    let mut extra = b.clone();
                    extra.count = b.count - h.count;
                    removed.push(extra);
                } else if h.count > b.count {
                    let mut extra = (*h).clone();
                    extra.count = h.count - b.count;
                    added.push(extra);
                }
            }
        }
    }
    for h in head {
        if !base_by_text.contains_key(h.callee_text.as_str()) {
            added.push(h.clone());
        }
    }
    // A different callee text bound to the same target (receiver renamed) is not a change.
    let mut kept_added = Vec::new();
    for a in added {
        let same = if a.target_id.is_some() {
            removed
                .iter()
                .position(|r| r.target_id == a.target_id && r.count == a.count)
        } else {
            None
        };
        match same {
            Some(i) => {
                removed.remove(i);
            }
            None => kept_added.push(a),
        }
    }
    delta.added = kept_added;
    delta.removed = removed;
    delta
}
