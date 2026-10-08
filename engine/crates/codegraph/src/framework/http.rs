//! Controllers and routes → `ApiEndpoint`, `HANDLED_BY`, `ROUTES_TO` (CG-006).
//!
//! A `controller` fact refines its class to [`NodeKind::Controller`]. A `route` fact creates the
//! endpoint node, points `HANDLED_BY` at the handler and `ROUTES_TO` at the controller when the
//! adapter named one. A handler is refined to [`NodeKind::Handler`] only when it is a function:
//! a method stays a `Method`, because "is this callable a route handler" is the framework's
//! question, not a change of what the symbol is.

use analysis_ir::symbol::AttrValue;

use crate::edge::EdgeFlags;
use crate::edge_kind::EdgeKind;
use crate::node_id::NodeId;
use crate::node_kind::NodeKind;

use super::contract::{self, FactCategory};
use super::{join_prefix, GlobalFact, Mapping};

/// `controller` → refine the class to `Controller`.
pub(crate) fn apply_controller(ctx: &mut Mapping<'_>) {
    let Some(symbol) = ctx.require_symbol("symbol") else {
        return;
    };
    if !NodeKind::Controller.refines_from(ctx.kind_of(&symbol).unwrap_or(NodeKind::Class)) {
        ctx.issue(
            contract::FactIssueCode::InvalidAttribute,
            format!(
                "symbol is a {}, which cannot refine to Controller",
                ctx.kind_of(&symbol).map_or(NodeKind::Class, |k| k)
            ),
        );
        return;
    }
    ctx.refine(symbol, NodeKind::Controller);
}

/// `route` → the endpoint node plus its two edges.
pub(crate) fn apply_route(ctx: &mut Mapping<'_>) {
    let Some(handler) = ctx.require_symbol("symbol") else {
        return;
    };
    let mut issues = Vec::new();
    let method = contract::require_str(ctx.fact, ctx.category, ctx.path, "method", &mut issues);
    let path = contract::require_str(ctx.fact, ctx.category, ctx.path, "path", &mut issues);
    let controller =
        contract::optional_str(ctx.fact, ctx.category, ctx.path, "controller", &mut issues);
    let prefix = contract::optional_str(ctx.fact, ctx.category, ctx.path, "prefix", &mut issues);
    for issue in issues {
        ctx.out.issues.push(issue);
    }
    let (Some(method), Some(path)) = (method, path) else {
        return;
    };
    let full_path = match &prefix {
        Some(prefix) => join_prefix(prefix, &path),
        None => path.clone(),
    };
    let Ok(id) = NodeId::http(&method, &full_path) else {
        ctx.issue(
            contract::FactIssueCode::InvalidAttribute,
            format!("method {method:?} is not a valid HTTP method token"),
        );
        return;
    };
    let display = id
        .as_str()
        .strip_prefix("http:")
        .unwrap_or(id.as_str())
        .to_owned();
    let endpoint = ctx.node(
        id,
        NodeKind::ApiEndpoint,
        display.clone(),
        format!("{} {}", method.to_ascii_uppercase(), full_path),
    );
    ctx.edge(EdgeKind::HandledBy, endpoint, handler, EdgeFlags::EMPTY);

    if let Some(controller) = controller {
        match ctx.symbol_key(&controller) {
            Some(class) => ctx.edge(EdgeKind::RoutesTo, endpoint, class, EdgeFlags::EMPTY),
            None => ctx.issue(
                contract::FactIssueCode::UnknownSymbol,
                format!(
                    "controller attribute names {controller:?}, which this file does not declare"
                ),
            ),
        }
    }

    if NodeKind::Handler.refines_from(ctx.kind_of(&handler).unwrap_or(NodeKind::Function)) {
        ctx.refine(handler, NodeKind::Handler);
    }

    // A controller-wide prefix is a global fact: routes declared in other files inherit it.
    if let Some(prefix) = prefix.filter(|p| !p.trim().is_empty()) {
        if matches!(ctx.fact.attrs.get("global"), Some(AttrValue::Bool(true))) {
            ctx.out
                .global_facts
                .push(GlobalFact::GlobalPrefix { prefix });
        }
    }
}

/// `http_global_config` → a [`GlobalFact::GlobalPrefix`] the snapshot-wide id owner applies.
pub(crate) fn apply_global_prefix(ctx: &mut Mapping<'_>) {
    let mut issues = Vec::new();
    let Some(prefix) = contract::require_str(
        ctx.fact,
        FactCategory::GlobalPrefix,
        ctx.path,
        "prefix",
        &mut issues,
    ) else {
        for issue in issues {
            ctx.out.issues.push(issue);
        }
        return;
    };
    for issue in issues {
        ctx.out.issues.push(issue);
    }
    ctx.out
        .global_facts
        .push(GlobalFact::GlobalPrefix { prefix });
}
