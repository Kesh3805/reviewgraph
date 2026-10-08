//! Guards and middleware → `Middleware`, `AUTHORIZES` (CG-006).
//!
//! This is the edge the auth-bypass golden scenario (PRD §151) depends on: a guard must appear
//! in the graph as a node with an `AUTHORIZES` edge to the endpoints it actually protects, so a
//! reviewer can see "this route has no guard" as a fact rather than as an absence.
//!
//! Two ways a guard reaches an endpoint:
//!
//! * the guard names `targets` (handler symbols) or a `controller`, and this file produced those
//!   endpoints, so the fan-out happens here;
//! * the guard is `global` (`APP_GUARD`-style), in which case the endpoints it protects may live
//!   in any file and the edge is deferred to
//!   [`crate::framework::FrameworkMapper::apply_globals`].

use crate::edge::EdgeFlags;
use crate::edge_kind::EdgeKind;
use crate::node_id::NodeKey;
use crate::node_kind::NodeKind;

use super::contract::{self, FactCategory, FactIssueCode};
use super::{GlobalFact, Mapping};

/// `guard` → refine to `Middleware` and authorize every endpoint it protects.
pub(crate) fn apply_guard(ctx: &mut Mapping<'_>) {
    let Some(guard) = ctx.require_symbol("symbol") else {
        return;
    };
    let mut issues = Vec::new();
    let targets = contract::str_list(ctx.fact, ctx.category, ctx.path, "targets", &mut issues);
    let controller = contract::optional_str(
        ctx.fact,
        FactCategory::Guard,
        ctx.path,
        "controller",
        &mut issues,
    );
    let is_global = contract::flag(ctx.fact, "global");
    for issue in issues {
        ctx.out.issues.push(issue);
    }

    let current = ctx.kind_of(&guard).unwrap_or(NodeKind::Class);
    if !NodeKind::Middleware.refines_from(current) {
        ctx.issue(
            FactIssueCode::InvalidAttribute,
            format!("a guard must be a class or a function, not a {current}"),
        );
        return;
    }
    ctx.refine(guard, NodeKind::Middleware);

    if is_global {
        ctx.out.global_facts.push(GlobalFact::GlobalGuard { guard });
    }

    // Resolve the named targets to keys, keeping the ones this file knows about.
    let mut named: Vec<NodeKey> = Vec::new();
    for name in targets.iter().chain(controller.iter()) {
        match ctx.symbol_key(name) {
            Some(key) => named.push(key),
            None => ctx.issue(
                FactIssueCode::UnknownSymbol,
                format!("guard target {name:?} is not declared in this file"),
            ),
        }
    }
    named.sort();
    named.dedup();

    if named.is_empty() && !is_global {
        ctx.issue(
            FactIssueCode::MissingAttribute,
            "a guard needs targets, a controller or global=true".to_owned(),
        );
        return;
    }

    for (endpoint, handlers, controllers) in endpoints_of(ctx) {
        let protects_handler = named.iter().any(|key| handlers.contains(key));
        let protects_controller = named.iter().any(|key| controllers.contains(key));
        if protects_handler || protects_controller {
            ctx.edge(EdgeKind::Authorizes, guard, endpoint, EdgeFlags::EMPTY);
        }
    }
}

/// The endpoints this file's facts produced, with the handlers and controllers each is wired to.
///
/// Snapshot of the accumulator taken *before* the guard's own edges are added, so a guard can
/// never authorize itself through a `ROUTES_TO` it just created.
fn endpoints_of(ctx: &Mapping<'_>) -> Vec<(NodeKey, Vec<NodeKey>, Vec<NodeKey>)> {
    let mut out: Vec<(NodeKey, Vec<NodeKey>, Vec<NodeKey>)> = Vec::new();
    for node in &ctx.out.nodes {
        if node.kind != NodeKind::ApiEndpoint {
            continue;
        }
        let endpoint = node.id.key();
        let handlers: Vec<NodeKey> = ctx
            .out
            .edges
            .iter()
            .filter(|edge| edge.kind == EdgeKind::HandledBy && edge.source == endpoint)
            .map(|edge| edge.target)
            .collect();
        let controllers: Vec<NodeKey> = ctx
            .out
            .edges
            .iter()
            .filter(|edge| edge.kind == EdgeKind::RoutesTo && edge.source == endpoint)
            .map(|edge| edge.target)
            .collect();
        out.push((endpoint, handlers, controllers));
    }
    out.sort();
    out
}
