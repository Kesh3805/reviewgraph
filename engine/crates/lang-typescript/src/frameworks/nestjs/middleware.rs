//! Guards, interceptors, pipes and filters: classes, bindings, global providers and route
//! metadata (NEST-004).
//!
//! Conservative by design: a binding that cannot be parsed is emitted as `dynamic_expr` with low
//! confidence rather than dropped, so a verifier never concludes "no guard" from a gap.

use std::collections::{BTreeMap, BTreeSet};

use analysis_ir::{AttrValue, FrameworkFactKind, IrDecorator, IrExpr, IrSymbol, LocalId};
use review_core::symbol::SymbolKind;

use super::tokens::nest_name;
use super::{ADAPTER, COMMON};
use crate::frameworks::util::{arguments_of, expr_text, name_of, nodes_of_kinds, s, str_list};
use crate::frameworks::FrameworkCtx;
use crate::kinds::{field, kind};
use crate::visit::db_heuristics::type_head;
use crate::visit::expr::to_ir_expr;
use crate::visit::fact_keys::callee_of;

const USE_DECORATORS: [(&str, &str); 4] = [
    ("UseGuards", "guard"),
    ("UseInterceptors", "interceptor"),
    ("UsePipes", "pipe"),
    ("UseFilters", "filter"),
];

const GLOBAL_TOKENS: [(&str, &str); 4] = [
    ("APP_GUARD", "guard"),
    ("APP_INTERCEPTOR", "interceptor"),
    ("APP_PIPE", "pipe"),
    ("APP_FILTER", "filter"),
];

const GLOBAL_CALLS: [(&str, &str); 4] = [
    ("useGlobalGuards", "guard"),
    ("useGlobalInterceptors", "interceptor"),
    ("useGlobalPipes", "pipe"),
    ("useGlobalFilters", "filter"),
];

const ENTRY_METHODS: [&str; 5] = ["canActivate", "intercept", "transform", "catch", "use"];

const PARAM_DECORATORS: [&str; 4] = ["Body", "Query", "Param", "Headers"];

/// Conventional metadata decorators and their keys, used when the definition is in another file.
const CONVENTIONS: [(&str, &str); 3] = [
    ("Public", "isPublic"),
    ("Roles", "roles"),
    ("Permissions", "permissions"),
];

fn head(text: &str) -> &str {
    text.split(['<', '(']).next().unwrap_or("").trim()
}

fn role_by_suffix(name: &str) -> Option<&'static str> {
    [
        ("Guard", "guard"),
        ("Interceptor", "interceptor"),
        ("Pipe", "pipe"),
        ("Filter", "filter"),
    ]
    .iter()
    .find(|(suffix, _)| name.ends_with(suffix))
    .map(|(_, role)| *role)
}

struct MiddlewareClass {
    role: &'static str,
    confidence: f32,
    implements: Vec<String>,
}

fn middleware_class(ctx: &FrameworkCtx<'_>, class: &IrSymbol) -> Option<MiddlewareClass> {
    let implements: Vec<String> = class
        .heritage
        .implements
        .iter()
        .map(|i| head(i).to_owned())
        .collect();
    for name in &implements {
        let role = match name.as_str() {
            "CanActivate" => "guard",
            "NestInterceptor" => "interceptor",
            "PipeTransform" => "pipe",
            "ExceptionFilter" => "filter",
            "NestMiddleware" => "middleware",
            _ => continue,
        };
        return Some(MiddlewareClass {
            role,
            confidence: 0.95,
            implements: implements.clone(),
        });
    }
    let catches = class
        .decorators
        .iter()
        .any(|d| matches!(nest_name(ctx, &d.name), Some((real, _)) if real == "Catch"));
    if catches {
        return Some(MiddlewareClass {
            role: "filter",
            confidence: 0.95,
            implements,
        });
    }
    if let Some(base) = class.heritage.extends.first().map(|e| head(e)) {
        if base == "AuthGuard" {
            return Some(MiddlewareClass {
                role: "guard",
                confidence: 0.9,
                implements,
            });
        }
        if let Some(role) = role_by_suffix(base) {
            return Some(MiddlewareClass {
                role,
                confidence: 0.8,
                implements,
            });
        }
    }
    let role = role_by_suffix(&class.name)?;
    let has_entry = ctx.symbols.iter().any(|m| {
        m.parent == Some(class.local_id)
            && m.kind == SymbolKind::Method
            && ENTRY_METHODS.contains(&m.name.as_str())
    });
    Some(MiddlewareClass {
        role,
        confidence: if has_entry { 0.8 } else { 0.6 },
        implements,
    })
}

/// `(entry map, unknown)` for one middleware expression.
fn middleware_entry(ctx: &FrameworkCtx<'_>, expr: &IrExpr) -> (AttrValue, bool) {
    let mut m = BTreeMap::new();
    let mut unknown = false;
    let (name, form): (String, &str) = match expr {
        IrExpr::Ident(n) => (n.clone(), "class"),
        IrExpr::Member(parts) => (parts.join("."), "class"),
        IrExpr::Call { callee, args } if callee.len() == 1 && callee[0] == "forwardRef" => {
            let name = match args.first() {
                Some(IrExpr::Arrow { returns }) => name_of(returns),
                _ => String::new(),
            };
            (name, "forward_ref")
        }
        IrExpr::Call { callee, args } => {
            m.insert(
                "args".to_owned(),
                AttrValue::List(args.iter().cloned().map(AttrValue::Expr).collect()),
            );
            (callee.join("."), "call")
        }
        IrExpr::Other(text) if text.starts_with("new ") => {
            let name = head(text.trim_start_matches("new ")).to_owned();
            (name, "instance")
        }
        other => {
            unknown = true;
            (expr_text(other), "dynamic_expr")
        }
    };
    if let Some(binding) = ctx.binding(name.split('.').next().unwrap_or("")) {
        m.insert("import_specifier".to_owned(), s(&binding.specifier));
        m.insert("imported".to_owned(), s(&binding.imported));
    }
    m.insert("name".to_owned(), AttrValue::Str(name));
    m.insert("form".to_owned(), s(form));
    (AttrValue::Map(m), unknown)
}

fn entries(ctx: &FrameworkCtx<'_>, args: &[IrExpr]) -> (Vec<AttrValue>, bool) {
    let mut unknown = false;
    let mut out = Vec::new();
    for (order, arg) in args.iter().enumerate() {
        let (mut entry, bad) = middleware_entry(ctx, arg);
        unknown |= bad;
        if let AttrValue::Map(m) = &mut entry {
            m.insert(
                "order".to_owned(),
                AttrValue::Int(i64::try_from(order).unwrap_or(i64::MAX)),
            );
        }
        out.push(entry);
    }
    (out, unknown)
}

fn first_class_name(list: &[AttrValue]) -> Option<String> {
    list.iter().find_map(|e| match e {
        AttrValue::Map(m) => match (m.get("form"), m.get("name")) {
            (Some(AttrValue::Str(form)), Some(AttrValue::Str(name)))
                if form == "class" || form == "instance" || form == "call" =>
            {
                Some(name.clone())
            }
            _ => None,
        },
        _ => None,
    })
}

pub fn collect(ctx: &mut FrameworkCtx<'_>) {
    let global_guards = global_provider_facts(ctx);
    classes(ctx, &global_guards);
    bindings(ctx);
    global_calls(ctx);
    metadata(ctx);
}

/// `APP_*` providers from the module facts. Returns the names of globally bound guard classes.
fn global_provider_facts(ctx: &mut FrameworkCtx<'_>) -> BTreeSet<String> {
    let modules: Vec<analysis_ir::IrFrameworkFact> = ctx
        .facts
        .iter()
        .filter(|f| f.kind == FrameworkFactKind::ModuleDeclaration)
        .cloned()
        .collect();
    let mut guards = BTreeSet::new();
    for module in modules {
        let Some(AttrValue::List(providers)) = module.attrs.get("providers") else {
            continue;
        };
        let module_name = match module.attrs.get("module_name") {
            Some(AttrValue::Str(n)) => n.clone(),
            _ => String::new(),
        };
        for provider in providers {
            let AttrValue::Map(p) = provider else {
                continue;
            };
            let Some(AttrValue::Expr(provide)) = p.get("provide") else {
                continue;
            };
            let token = name_of(provide);
            let Some((_, role)) = GLOBAL_TOKENS.iter().find(|(t, _)| *t == token) else {
                continue;
            };
            let class = match (p.get("use_class"), p.get("use_existing")) {
                (Some(AttrValue::Str(n)), _) | (None, Some(AttrValue::Str(n))) if !n.is_empty() => {
                    Some(n.clone())
                }
                _ => None,
            };
            let mut entry = BTreeMap::new();
            let confidence = match &class {
                Some(name) => {
                    if let Some(binding) = ctx.binding(name) {
                        entry.insert("import_specifier".to_owned(), s(&binding.specifier));
                        entry.insert("imported".to_owned(), s(&binding.imported));
                    }
                    entry.insert("name".to_owned(), AttrValue::Str(name.clone()));
                    entry.insert("form".to_owned(), s("class"));
                    if *role == "guard" {
                        guards.insert(name.clone());
                    }
                    module.confidence
                }
                None => {
                    entry.insert("name".to_owned(), s(""));
                    entry.insert("form".to_owned(), s("dynamic_expr"));
                    0.4
                }
            };
            let mut attrs = BTreeMap::new();
            attrs.insert("role".to_owned(), s(role));
            attrs.insert(
                "middleware".to_owned(),
                AttrValue::List(vec![AttrValue::Map(entry)]),
            );
            attrs.insert("via".to_owned(), s("module_provider"));
            attrs.insert("module".to_owned(), AttrValue::Str(module_name.clone()));
            ctx.emit(
                ADAPTER,
                FrameworkFactKind::GlobalProvider,
                module.symbol,
                attrs,
                module.range,
                confidence,
            );
        }
    }
    guards
}

fn classes(ctx: &mut FrameworkCtx<'_>, global_guards: &BTreeSet<String>) {
    let symbols = ctx.symbols;
    let calls = nodes_of_kinds(ctx.root, &[kind::CALL_EXPRESSION]);
    for class in symbols.iter().filter(|c| c.kind == SymbolKind::Class) {
        let Some(found) = middleware_class(ctx, class) else {
            continue;
        };
        let entry = symbols.iter().find(|m| {
            m.parent == Some(class.local_id)
                && m.kind == SymbolKind::Method
                && ENTRY_METHODS.contains(&m.name.as_str())
        });
        let uses_reflector = symbols.iter().any(|c| {
            c.parent == Some(class.local_id)
                && c.kind == SymbolKind::Constructor
                && c.params
                    .iter()
                    .any(|p| p.type_text.as_deref().map(type_head) == Some("Reflector"))
        });
        let mut keys: Vec<String> = Vec::new();
        for call in &calls {
            let Some(function) = call.child_by_field_name(field::FUNCTION) else {
                continue;
            };
            let Some(callee) = callee_of(function, ctx.src) else {
                continue;
            };
            if !matches!(
                callee.name.as_str(),
                "get" | "getAll" | "getAllAndOverride" | "getAllAndMerge"
            ) {
                continue;
            }
            if !callee
                .receiver_name()
                .is_some_and(|r| r.to_ascii_lowercase().contains("reflector"))
            {
                continue;
            }
            let range = ctx.src.range(*call);
            let owner = ctx.owner(range.start);
            if ctx.class_of(owner).map(|c| c.local_id) != Some(class.local_id) {
                continue;
            }
            if let Some(arg) = arguments_of(*call).into_iter().next() {
                let expr = to_ir_expr(arg, ctx.src);
                let key = ctx.const_str(&expr).unwrap_or_else(|| expr_text(&expr));
                if !keys.contains(&key) {
                    keys.push(key);
                }
            }
        }
        let mut attrs = BTreeMap::new();
        attrs.insert(
            "symbol".to_owned(),
            AttrValue::Str(ctx.qname(class.local_id)),
        );
        attrs.insert("role".to_owned(), s(found.role));
        attrs.insert("name".to_owned(), AttrValue::Str(class.name.clone()));
        if let Some(entry) = entry {
            attrs.insert(
                "entry_symbol".to_owned(),
                AttrValue::Int(i64::from(entry.local_id.0)),
            );
        }
        attrs.insert("uses_reflector".to_owned(), AttrValue::Bool(uses_reflector));
        attrs.insert("reads_metadata_keys".to_owned(), str_list(keys));
        attrs.insert("implements".to_owned(), str_list(found.implements.clone()));
        if found.role == "guard" && global_guards.contains(&class.name) {
            attrs.insert("global".to_owned(), AttrValue::Bool(true));
        }
        ctx.emit(
            ADAPTER,
            FrameworkFactKind::Middleware,
            Some(class.local_id),
            attrs,
            class.range,
            found.confidence,
        );
    }
}

fn bindings(ctx: &mut FrameworkCtx<'_>) {
    let symbols = ctx.symbols;
    for symbol in symbols {
        let scope = match symbol.kind {
            SymbolKind::Class => "class",
            SymbolKind::Method | SymbolKind::Getter | SymbolKind::Setter => "method",
            _ => continue,
        };
        for decorator in &symbol.decorators {
            let Some((role, conf)) = USE_DECORATORS.iter().find_map(|(wanted, role)| {
                ctx.decorator_origin(decorator, COMMON, wanted)
                    .map(|c| (*role, c))
            }) else {
                continue;
            };
            let (list, unknown) = entries(ctx, &decorator.args);
            let mut attrs = BTreeMap::new();
            let target = ctx.qname(symbol.local_id);
            attrs.insert("role".to_owned(), s(role));
            attrs.insert("scope".to_owned(), s(scope));
            attrs.insert("target".to_owned(), AttrValue::Str(target.clone()));
            if let Some(name) = first_class_name(&list) {
                attrs.insert("symbol".to_owned(), AttrValue::Str(name));
            } else {
                attrs.insert("symbol".to_owned(), AttrValue::Str(target.clone()));
            }
            if scope == "class" {
                attrs.insert("controller".to_owned(), AttrValue::Str(target));
            } else {
                attrs.insert("targets".to_owned(), str_list(vec![target]));
            }
            attrs.insert("middleware".to_owned(), AttrValue::List(list));
            ctx.emit(
                ADAPTER,
                FrameworkFactKind::MiddlewareBinding,
                Some(symbol.local_id),
                attrs,
                decorator.range,
                if unknown { 0.4 } else { conf },
            );
        }
        if scope == "method" {
            param_pipes(ctx, symbol);
        }
    }
}

/// `@Body(ValidationPipe)`, `@Query('id', ParseIntPipe)`.
fn param_pipes(ctx: &mut FrameworkCtx<'_>, method: &IrSymbol) {
    for param in &method.params {
        for decorator in &param.decorators {
            let bound = PARAM_DECORATORS
                .iter()
                .any(|wanted| ctx.decorator_origin(decorator, COMMON, wanted).is_some());
            if !bound {
                continue;
            }
            let pipe_args: Vec<IrExpr> = decorator
                .args
                .iter()
                .filter(|a| !matches!(a, IrExpr::Str(_) | IrExpr::Template { .. }))
                .cloned()
                .collect();
            if pipe_args.is_empty() {
                continue;
            }
            let (list, unknown) = entries(ctx, &pipe_args);
            let target = ctx.qname(method.local_id);
            let mut attrs = BTreeMap::new();
            attrs.insert("role".to_owned(), s("pipe"));
            attrs.insert("scope".to_owned(), s("param"));
            attrs.insert("target".to_owned(), AttrValue::Str(target.clone()));
            attrs.insert("param_name".to_owned(), s(&param.name));
            attrs.insert(
                "symbol".to_owned(),
                AttrValue::Str(first_class_name(&list).unwrap_or_else(|| target.clone())),
            );
            attrs.insert("targets".to_owned(), str_list(vec![target]));
            attrs.insert("middleware".to_owned(), AttrValue::List(list));
            ctx.emit(
                ADAPTER,
                FrameworkFactKind::MiddlewareBinding,
                Some(method.local_id),
                attrs,
                decorator.range,
                if unknown { 0.4 } else { 0.95 },
            );
        }
    }
}

/// `app.useGlobalGuards(...)` and friends.
fn global_calls(ctx: &mut FrameworkCtx<'_>) {
    for call in nodes_of_kinds(ctx.root, &[kind::CALL_EXPRESSION]) {
        let Some(function) = call.child_by_field_name(field::FUNCTION) else {
            continue;
        };
        let Some(callee) = callee_of(function, ctx.src) else {
            continue;
        };
        let Some((_, role)) = GLOBAL_CALLS.iter().find(|(n, _)| *n == callee.name) else {
            continue;
        };
        let args: Vec<IrExpr> = arguments_of(call)
            .into_iter()
            .map(|a| to_ir_expr(a, ctx.src))
            .collect();
        let (list, unknown) = entries(ctx, &args);
        let range = ctx.src.range(call);
        let owner = ctx.owner(range.start);
        let mut attrs = BTreeMap::new();
        attrs.insert("role".to_owned(), s(role));
        attrs.insert("middleware".to_owned(), AttrValue::List(list));
        attrs.insert("via".to_owned(), s("main"));
        attrs.insert("symbol".to_owned(), AttrValue::Str(ctx.qname(owner)));
        ctx.emit(
            ADAPTER,
            FrameworkFactKind::GlobalProvider,
            Some(owner),
            attrs,
            range,
            if unknown { 0.4 } else { 0.95 },
        );
    }
}

fn metadata_key(ctx: &FrameworkCtx<'_>, expr: Option<&IrExpr>) -> String {
    match expr {
        Some(e) => ctx.const_str(e).unwrap_or_else(|| expr_text(e)),
        None => String::new(),
    }
}

fn metadata(ctx: &mut FrameworkCtx<'_>) {
    let symbols = ctx.symbols;
    // Definitions: `const Roles = (...r) => SetMetadata('roles', r)`.
    let mut definitions: BTreeMap<String, String> = BTreeMap::new();
    for call in nodes_of_kinds(ctx.root, &[kind::CALL_EXPRESSION]) {
        if call.parent().is_some_and(|p| p.kind() == kind::DECORATOR) {
            continue;
        }
        let Some(function) = call.child_by_field_name(field::FUNCTION) else {
            continue;
        };
        let Some(callee) = callee_of(function, ctx.src) else {
            continue;
        };
        let text = callee.text();
        let (def_kind, key) = if callee.name == "SetMetadata"
            && matches!(nest_name(ctx, &text), Some((ref real, _)) if real == "SetMetadata")
        {
            let first = arguments_of(call)
                .into_iter()
                .next()
                .map(|a| to_ir_expr(a, ctx.src));
            ("set_metadata", metadata_key(ctx, first.as_ref()))
        } else if text == "Reflector.createDecorator" {
            ("create_decorator", String::new())
        } else {
            continue;
        };
        let range = ctx.src.range(call);
        let owner = ctx.owner(range.start);
        let Some(owner_symbol) = ctx.symbol(owner) else {
            continue;
        };
        if !matches!(
            owner_symbol.kind,
            SymbolKind::Function | SymbolKind::Constant | SymbolKind::Variable
        ) {
            continue;
        }
        let name = owner_symbol.name.clone();
        let mut attrs = BTreeMap::new();
        attrs.insert("symbol".to_owned(), AttrValue::Str(ctx.qname(owner)));
        attrs.insert("name".to_owned(), AttrValue::Str(name.clone()));
        attrs.insert("key".to_owned(), AttrValue::Str(key.clone()));
        attrs.insert("kind".to_owned(), s(def_kind));
        definitions.insert(name, key);
        ctx.emit(
            ADAPTER,
            FrameworkFactKind::MetadataDecoratorDefinition,
            Some(owner),
            attrs,
            range,
            0.95,
        );
    }

    // Usages on classes and methods.
    for symbol in symbols {
        let scope = match symbol.kind {
            SymbolKind::Class => "class",
            SymbolKind::Method => "method",
            _ => continue,
        };
        for decorator in &symbol.decorators {
            if let Some((key, convention, conf)) = metadata_usage(ctx, &definitions, decorator) {
                emit_route_metadata(
                    ctx,
                    symbol.local_id,
                    scope,
                    decorator,
                    &key,
                    convention,
                    conf,
                );
            }
        }
    }
}

fn metadata_usage(
    ctx: &FrameworkCtx<'_>,
    definitions: &BTreeMap<String, String>,
    decorator: &IrDecorator,
) -> Option<(String, bool, f32)> {
    let real = nest_name(ctx, &decorator.name);
    if matches!(&real, Some((r, _)) if r == "SetMetadata") {
        return Some((metadata_key(ctx, decorator.args.first()), false, 0.95));
    }
    if let Some(key) = definitions.get(&decorator.name) {
        return Some((key.clone(), false, 0.95));
    }
    let (_, key) = CONVENTIONS.iter().find(|(n, _)| *n == decorator.name)?;
    Some(((*key).to_owned(), true, 0.6))
}

fn emit_route_metadata(
    ctx: &mut FrameworkCtx<'_>,
    symbol: LocalId,
    scope: &str,
    decorator: &IrDecorator,
    key: &str,
    convention: bool,
    confidence: f32,
) {
    let is_set_metadata = decorator.name.ends_with("SetMetadata");
    let value = if is_set_metadata {
        decorator.args.get(1).cloned().unwrap_or(IrExpr::Bool(true))
    } else {
        match decorator.args.as_slice() {
            [] => IrExpr::Bool(true),
            [one] => one.clone(),
            many => IrExpr::Array(many.to_vec()),
        }
    };
    let mut attrs = BTreeMap::new();
    attrs.insert("symbol".to_owned(), AttrValue::Str(ctx.qname(symbol)));
    attrs.insert("scope".to_owned(), s(scope));
    attrs.insert("key".to_owned(), s(key));
    attrs.insert("value".to_owned(), AttrValue::Expr(value));
    attrs.insert("via_decorator".to_owned(), s(&decorator.name));
    attrs.insert("convention_based".to_owned(), AttrValue::Bool(convention));
    ctx.emit(
        ADAPTER,
        FrameworkFactKind::RouteMetadata,
        Some(symbol),
        attrs,
        decorator.range,
        confidence,
    );
}
