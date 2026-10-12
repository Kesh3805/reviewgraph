//! `TestMock` facts: `jest.mock`, `jest.spyOn`, `jest.fn`, Nest testing modules and provider
//! overrides. Only the kind of a mocked value is recorded, never the value.

use std::collections::BTreeMap;

use analysis_ir::{AttrValue, FrameworkFactKind, IrExpr};
use tree_sitter::Node;

use super::{ADAPTER, JEST_GLOBALS};
use crate::frameworks::util::{
    arguments_of, expr_kind, expr_text, find_key, name_of, node_string, s,
};
use crate::frameworks::FrameworkCtx;
use crate::kinds::{field, kind};
use crate::visit::expr::{member_chain, to_ir_expr};
use crate::visit::fact_keys::callee_of;

const OVERRIDES: [&str; 5] = [
    "overrideProvider",
    "overrideGuard",
    "overridePipe",
    "overrideInterceptor",
    "overrideFilter",
];

fn is_jest(ctx: &FrameworkCtx<'_>) -> bool {
    match ctx.binding("jest") {
        None => true,
        Some(b) => b.specifier == JEST_GLOBALS,
    }
}

struct Mock<'a> {
    kind: &'a str,
    target: Option<String>,
    provider: Option<String>,
    value_kind: Option<&'a str>,
}

fn emit(ctx: &mut FrameworkCtx<'_>, node: Node<'_>, mock: &Mock<'_>, confidence: f32) {
    let range = ctx.src.range(node);
    let owner = ctx.owner(range.start);
    let mut attrs = BTreeMap::new();
    attrs.insert("symbol".to_owned(), AttrValue::Str(ctx.qname(owner)));
    attrs.insert("kind".to_owned(), s(mock.kind));
    if let Some(target) = &mock.target {
        attrs.insert("target".to_owned(), AttrValue::Str(target.clone()));
    }
    if let Some(provider) = &mock.provider {
        attrs.insert("provider".to_owned(), AttrValue::Str(provider.clone()));
    }
    if let Some(value_kind) = mock.value_kind {
        attrs.insert("value_kind".to_owned(), s(value_kind));
    }
    ctx.emit(
        ADAPTER,
        FrameworkFactKind::TestMock,
        Some(owner),
        attrs,
        range,
        confidence,
    );
}

/// The `use*` call chained directly after an `override*()` call: its kind of value.
fn chained_value_kind(ctx: &FrameworkCtx<'_>, call: Node<'_>) -> Option<&'static str> {
    let member = call.parent()?;
    if member.kind() != kind::MEMBER_EXPRESSION {
        return None;
    }
    let outer = member.parent()?;
    if outer.kind() != kind::CALL_EXPRESSION {
        return None;
    }
    let name = ctx.src.text(member.child_by_field_name(field::PROPERTY)?);
    match &*name {
        "useValue" => {
            let arg = arguments_of(outer).into_iter().next()?;
            Some(expr_kind(&to_ir_expr(arg, ctx.src)))
        }
        "useClass" | "useExisting" => Some("ident"),
        "useFactory" => Some("call"),
        _ => None,
    }
}

pub fn visit(ctx: &mut FrameworkCtx<'_>, call: Node<'_>) {
    let Some(function) = call.child_by_field_name(field::FUNCTION) else {
        return;
    };
    let Some(callee) = callee_of(function, ctx.src) else {
        return;
    };
    let receiver: Vec<&str> = callee.receiver.iter().map(String::as_str).collect();
    let args = arguments_of(call);
    match (receiver.as_slice(), callee.name.as_str()) {
        (["jest"], "mock" | "doMock") if is_jest(ctx) => {
            let target = args.first().and_then(|a| node_string(*a, ctx.src));
            emit(
                ctx,
                call,
                &Mock {
                    kind: "module_mock",
                    target,
                    provider: None,
                    value_kind: None,
                },
                0.95,
            );
        }
        (["jest"], "spyOn") if is_jest(ctx) => {
            let object = args
                .first()
                .and_then(|a| member_chain(*a, ctx.src))
                .map(|c| c.join("."))
                .or_else(|| args.first().map(|a| expr_text(&to_ir_expr(*a, ctx.src))))
                .unwrap_or_default();
            let method = args
                .get(1)
                .and_then(|a| node_string(*a, ctx.src))
                .unwrap_or_default();
            emit(
                ctx,
                call,
                &Mock {
                    kind: "spy",
                    target: Some(format!("{object}.{method}")),
                    provider: None,
                    value_kind: None,
                },
                0.95,
            );
        }
        (["jest"], "fn") if is_jest(ctx) => emit(
            ctx,
            call,
            &Mock {
                kind: "fn",
                target: None,
                provider: None,
                value_kind: None,
            },
            0.95,
        ),
        (["Test"], "createTestingModule") => {
            emit(
                ctx,
                call,
                &Mock {
                    kind: "testing_module",
                    target: None,
                    provider: None,
                    value_kind: None,
                },
                0.9,
            );
            let metadata = args.first().map(|a| to_ir_expr(*a, ctx.src));
            provide_values(ctx, call, metadata.as_ref());
        }
        (_, name) if OVERRIDES.contains(&name) => {
            let provider = args
                .first()
                .map(|a| name_of(&to_ir_expr(*a, ctx.src)))
                .filter(|p| !p.is_empty());
            let value_kind = chained_value_kind(ctx, call);
            emit(
                ctx,
                call,
                &Mock {
                    kind: "override_provider",
                    target: None,
                    provider,
                    value_kind,
                },
                0.9,
            );
        }
        _ => {}
    }
}

/// `{ provide: X, useValue: mock }` entries of a testing module's `providers`.
fn provide_values(ctx: &mut FrameworkCtx<'_>, call: Node<'_>, metadata: Option<&IrExpr>) {
    let Some(IrExpr::Object(pairs)) = metadata else {
        return;
    };
    let Some(IrExpr::Array(providers)) = find_key(pairs, "providers") else {
        return;
    };
    for provider in providers {
        let IrExpr::Object(p) = provider else {
            continue;
        };
        let Some(value) = find_key(p, "useValue") else {
            continue;
        };
        let token = match find_key(p, "provide") {
            Some(IrExpr::Str(t)) => t.clone(),
            Some(other) => expr_text(other),
            None => String::new(),
        };
        emit(
            ctx,
            call,
            &Mock {
                kind: "provide_value",
                target: None,
                provider: Some(token).filter(|t| !t.is_empty()),
                value_kind: Some(expr_kind(value)),
            },
            0.9,
        );
    }
}
