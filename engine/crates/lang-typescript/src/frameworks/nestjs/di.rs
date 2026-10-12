//! Dependency-injection facts: one per constructor parameter or injected property (NEST-003).

use std::collections::BTreeMap;

use analysis_ir::{AttrValue, DiagCode, DiagSeverity, FrameworkFactKind, IrExpr, IrSymbol};
use review_core::symbol::SymbolKind;

use super::tokens::{classify, nest_name, InjectedToken, TokenKind};
use super::ADAPTER;
use crate::frameworks::util::{expr_text, s, str_list, type_args};
use crate::frameworks::FrameworkCtx;
use crate::visit::db_heuristics::type_head;
use crate::visit::declarations::is_secret_name;

/// Class decorators that make a class a DI participant.
const CLASS_DECORATORS: [&str; 8] = [
    "Injectable",
    "Controller",
    "Module",
    "Catch",
    "Processor",
    "WebSocketGateway",
    "Resolver",
    "Global",
];

const MIDDLEWARE_SUFFIXES: [&str; 4] = ["Guard", "Interceptor", "Pipe", "Filter"];

const PRIMITIVES: [&str; 9] = [
    "string", "number", "boolean", "any", "unknown", "object", "symbol", "bigint", "Function",
];

fn eligible(ctx: &FrameworkCtx<'_>, class: &IrSymbol) -> bool {
    let decorated = class.decorators.iter().any(|d| {
        nest_name(ctx, &d.name).is_some_and(|(real, _)| CLASS_DECORATORS.contains(&real.as_str()))
    });
    if decorated {
        return true;
    }
    class
        .heritage
        .extends
        .iter()
        .chain(class.heritage.implements.iter())
        .any(|h| {
            let head = h.split(['<', '(']).next().unwrap_or("").trim();
            MIDDLEWARE_SUFFIXES
                .iter()
                .any(|suffix| head.ends_with(suffix))
        })
}

fn redact(expr: &IrExpr) -> IrExpr {
    match expr {
        IrExpr::Str(v) if is_secret_name(v) => IrExpr::Str("<redacted>".to_owned()),
        IrExpr::Ident(v) if is_secret_name(v) => IrExpr::Str("<redacted>".to_owned()),
        other => other.clone(),
    }
}

struct Param<'a> {
    index: i64,
    name: &'a str,
    type_text: Option<&'a str>,
    property: bool,
    token: InjectedToken,
}

fn is_interface(symbols: &[IrSymbol], name: &str) -> bool {
    symbols.iter().any(|sym| {
        sym.name == name && matches!(sym.kind, SymbolKind::Interface | SymbolKind::TypeAlias)
    })
}

fn param_fact(
    ctx: &mut FrameworkCtx<'_>,
    class: &IrSymbol,
    param: &Param<'_>,
    range: review_core::location::SourceRange,
) {
    let head = param.type_text.map(type_head).unwrap_or("");
    let mut attrs: BTreeMap<String, AttrValue> = BTreeMap::new();
    attrs.insert(
        "symbol".to_owned(),
        AttrValue::Str(ctx.qname(class.local_id)),
    );
    attrs.insert("param_index".to_owned(), AttrValue::Int(param.index));
    attrs.insert("param_name".to_owned(), s(param.name));
    if !head.is_empty() {
        attrs.insert("type".to_owned(), s(head));
    }
    attrs.insert(
        "type_args".to_owned(),
        str_list(param.type_text.map(type_args).unwrap_or_default()),
    );
    attrs.insert("optional".to_owned(), AttrValue::Bool(param.token.optional));
    attrs.insert(
        "scope_flags".to_owned(),
        str_list(param.token.scope_flags.clone()),
    );
    if param.property {
        attrs.insert("property".to_owned(), s(param.name));
    }
    if let Some(entity) = param.token.entity.as_ref().filter(|e| !e.is_empty()) {
        attrs.insert("entity".to_owned(), AttrValue::Str(entity.clone()));
    }
    if let Some(queue) = &param.token.queue {
        let name = ctx
            .const_str(queue)
            .unwrap_or_else(|| format!("{{{}}}", expr_text(queue)));
        attrs.insert("queue".to_owned(), AttrValue::Str(name));
    }
    if let Some(token) = &param.token.token {
        attrs.insert("token".to_owned(), AttrValue::Expr(redact(token)));
    }

    let primitive = PRIMITIVES.contains(&head);
    let (kind, confidence) = match param.token.kind {
        Some(kind) => {
            let base: f32 = match kind {
                TokenKind::Str | TokenKind::Symbol => 0.7,
                TokenKind::Unknown => 0.3,
                _ => 0.9,
            };
            let base: f32 = if param.token.unbound {
                base.min(0.7)
            } else {
                base
            };
            (kind, base)
        }
        None if head.is_empty() || primitive || is_interface(ctx.symbols, head) => {
            if head.is_empty() {
                ctx.sink.note(
                    DiagSeverity::Info,
                    DiagCode::UnsupportedConstruct,
                    "injected parameter has no usable type annotation",
                    Some(range),
                );
            }
            (TokenKind::Unknown, 0.3)
        }
        None => (TokenKind::Class, 0.9),
    };
    attrs.insert("token_kind".to_owned(), s(kind.as_str()));
    ctx.emit(
        ADAPTER,
        FrameworkFactKind::DiInjection,
        Some(class.local_id),
        attrs,
        range,
        confidence,
    );
}

pub fn collect(ctx: &mut FrameworkCtx<'_>) {
    let symbols = ctx.symbols;
    for class in symbols.iter().filter(|c| c.kind == SymbolKind::Class) {
        let class_eligible = eligible(ctx, class);
        let ctor = symbols
            .iter()
            .find(|c| c.parent == Some(class.local_id) && c.kind == SymbolKind::Constructor);
        if let Some(ctor) = ctor {
            for (index, param) in ctor.params.iter().enumerate() {
                let token = classify(ctx, &param.decorators);
                if !class_eligible && !token.explicit {
                    continue;
                }
                let item = Param {
                    index: i64::try_from(index).unwrap_or(i64::MAX),
                    name: &param.name,
                    type_text: param.type_text.as_deref(),
                    property: param.property.is_some(),
                    token,
                };
                param_fact(ctx, class, &item, ctor.range);
            }
        }
        for prop in symbols.iter().filter(|p| {
            p.parent == Some(class.local_id)
                && matches!(p.kind, SymbolKind::Property | SymbolKind::Field)
                && p.attrs.get("from_constructor_param").is_none()
        }) {
            let token = classify(ctx, &prop.decorators);
            if !token.explicit {
                continue;
            }
            let item = Param {
                index: -1,
                name: &prop.name,
                type_text: prop.declared_type.as_deref(),
                property: true,
                token,
            };
            param_fact(ctx, class, &item, prop.range);
        }
    }
}
