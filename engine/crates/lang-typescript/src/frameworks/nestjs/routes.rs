//! Controllers, routes and application-level HTTP configuration (NEST-002).

use std::collections::{BTreeMap, BTreeSet};

use analysis_ir::{AttrValue, DiagCode, DiagSeverity, FrameworkFactKind, IrDecorator, IrExpr};
use review_core::symbol::SymbolKind;
use tree_sitter::Node;

use super::{ADAPTER, COMMON};
use crate::frameworks::util::{
    arguments_of, expr_text, find_key, last_segment, nodes_of_kinds, receiver_types, s, str_list,
};
use crate::frameworks::FrameworkCtx;
use crate::kinds::{field, kind};
use crate::visit::expr::{member_chain, to_ir_expr};
use crate::visit::fact_keys::callee_of;

/// `(decorator, HTTP method)`; `Sse` is a GET with a flag.
const ROUTE_DECORATORS: [(&str, &str); 9] = [
    ("Get", "GET"),
    ("Post", "POST"),
    ("Put", "PUT"),
    ("Patch", "PATCH"),
    ("Delete", "DELETE"),
    ("All", "ALL"),
    ("Options", "OPTIONS"),
    ("Head", "HEAD"),
    ("Sse", "GET"),
];

#[derive(Debug, Clone)]
struct PathArg {
    path: String,
    dynamic: bool,
}

#[derive(Debug, Clone)]
struct ControllerInfo {
    prefixes: Vec<PathArg>,
    version: Option<AttrValue>,
}

/// Joins segments into a normalized route path: leading `/`, no repeated or trailing `/`,
/// parameter regexes removed (`:id(\\d+)` becomes `:id`), literals keep their case.
pub fn normalize(parts: &[&str]) -> String {
    let stripped = strip_param_regex(&parts.join("/"));
    let mut out = String::from("/");
    let mut last_slash = true;
    for c in stripped.chars() {
        if c == '/' {
            if !last_slash {
                out.push('/');
                last_slash = true;
            }
        } else {
            out.push(c);
            last_slash = false;
        }
    }
    if out.len() > 1 && out.ends_with('/') {
        out.pop();
    }
    out
}

fn strip_param_regex(path: &str) -> String {
    let chars: Vec<char> = path.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    let mut in_param = false;
    while i < chars.len() {
        let c = chars[i];
        if c == ':' {
            in_param = true;
            out.push(c);
            i += 1;
            continue;
        }
        if in_param && c == '(' {
            let mut depth = 0i32;
            while i < chars.len() {
                if chars[i] == '(' {
                    depth += 1;
                } else if chars[i] == ')' {
                    depth -= 1;
                    if depth == 0 {
                        i += 1;
                        break;
                    }
                }
                i += 1;
            }
            in_param = false;
            continue;
        }
        if in_param && !(c.is_alphanumeric() || c == '_') {
            in_param = false;
        }
        out.push(c);
        i += 1;
    }
    out
}

fn one_path(ctx: &FrameworkCtx<'_>, expr: &IrExpr) -> PathArg {
    match ctx.const_str(expr) {
        Some(path) => PathArg {
            path,
            dynamic: false,
        },
        None => PathArg {
            path: format!("{{{}}}", expr_text(expr)),
            dynamic: true,
        },
    }
}

/// Paths of a route or controller argument: none is `""`, an array is several.
fn paths_of(ctx: &FrameworkCtx<'_>, expr: Option<&IrExpr>) -> Vec<PathArg> {
    match expr {
        None => vec![PathArg {
            path: String::new(),
            dynamic: false,
        }],
        Some(IrExpr::Array(items)) => items.iter().map(|e| one_path(ctx, e)).collect(),
        Some(e) => vec![one_path(ctx, e)],
    }
}

fn version_value(expr: &IrExpr) -> AttrValue {
    match expr {
        IrExpr::Array(items) => AttrValue::List(items.iter().map(version_value).collect()),
        IrExpr::Str(v) | IrExpr::Num(v) => AttrValue::Str(v.clone()),
        other => {
            if last_segment(other).as_deref() == Some("VERSION_NEUTRAL") {
                s("NEUTRAL")
            } else {
                AttrValue::Str(expr_text(other))
            }
        }
    }
}

fn controller_info(ctx: &FrameworkCtx<'_>, decorator: &IrDecorator) -> (ControllerInfo, bool) {
    let mut version = None;
    let prefixes = match decorator.args.first() {
        Some(IrExpr::Object(pairs)) => {
            version = find_key(pairs, "version").map(version_value);
            paths_of(ctx, find_key(pairs, "path"))
        }
        other => paths_of(ctx, other),
    };
    let dynamic = prefixes.iter().any(|p| p.dynamic);
    (ControllerInfo { prefixes, version }, dynamic)
}

fn head_of(text: &str) -> &str {
    let end = text.find(['<', '(']).unwrap_or(text.len());
    text[..end].trim()
}

pub fn collect(ctx: &mut FrameworkCtx<'_>) {
    controllers_and_routes(ctx);
    global_config(ctx);
}

fn controllers_and_routes(ctx: &mut FrameworkCtx<'_>) {
    let symbols = ctx.symbols;
    let mut controllers: BTreeMap<String, ControllerInfo> = BTreeMap::new();
    for class in symbols.iter().filter(|c| c.kind == SymbolKind::Class) {
        let Some((decorator, conf)) = class.decorators.iter().find_map(|d| {
            ctx.decorator_origin(d, COMMON, "Controller")
                .map(|c| (d, c))
        }) else {
            continue;
        };
        let (info, dynamic) = controller_info(ctx, decorator);
        let mut attrs = BTreeMap::new();
        attrs.insert(
            "symbol".to_owned(),
            AttrValue::Str(ctx.qname(class.local_id)),
        );
        attrs.insert(
            "prefixes".to_owned(),
            str_list(info.prefixes.iter().map(|p| p.path.clone())),
        );
        if let Some(version) = &info.version {
            attrs.insert("version".to_owned(), version.clone());
        }
        if let Some(IrExpr::Object(pairs)) = decorator.args.first() {
            if let Some(host) = find_key(pairs, "host").and_then(|h| ctx.const_str(h)) {
                attrs.insert("host".to_owned(), AttrValue::Str(host));
            }
        }
        ctx.emit(
            ADAPTER,
            FrameworkFactKind::Controller,
            Some(class.local_id),
            attrs,
            decorator.range,
            if dynamic { 0.5 } else { conf },
        );
        controllers.insert(class.name.clone(), info);
    }

    for method in symbols.iter().filter(|m| m.kind == SymbolKind::Method) {
        let Some(class) = method.parent.and_then(|p| symbols.get(p.0 as usize)) else {
            continue;
        };
        let routes: Vec<(&IrDecorator, &str, &str, f32)> = method
            .decorators
            .iter()
            .filter_map(|d| {
                ROUTE_DECORATORS.iter().find_map(|(wanted, http)| {
                    ctx.decorator_origin(d, COMMON, wanted)
                        .map(|c| (d, *http, *wanted, c))
                })
            })
            .collect();
        if routes.is_empty() {
            continue;
        }
        let inherited = class
            .heritage
            .extends
            .first()
            .and_then(|e| controllers.get(head_of(e)));
        let Some(info) = controllers.get(&class.name).or(inherited) else {
            ctx.sink.note(
                DiagSeverity::Info,
                DiagCode::UnsupportedConstruct,
                "route decorator on a class that is not a controller",
                Some(method.range),
            );
            continue;
        };
        let handler_version = method.decorators.iter().find_map(|d| {
            ctx.decorator_origin(d, COMMON, "Version")
                .and_then(|_| d.args.first().map(version_value))
        });
        let status_code = method.decorators.iter().find_map(|d| {
            ctx.decorator_origin(d, COMMON, "HttpCode")?;
            match d.args.first() {
                Some(IrExpr::Num(n)) => n.parse::<i64>().ok(),
                _ => None,
            }
        });
        let redirect = method
            .decorators
            .iter()
            .find(|d| ctx.decorator_origin(d, COMMON, "Redirect").is_some());
        let header_names: Vec<String> = method
            .decorators
            .iter()
            .filter(|d| ctx.decorator_origin(d, COMMON, "Header").is_some())
            .filter_map(|d| d.args.first().and_then(|a| ctx.const_str(a)))
            .collect();
        for (decorator, http, wanted, conf) in routes {
            for prefix in &info.prefixes {
                for route in paths_of(ctx, decorator.args.first()) {
                    let dynamic = prefix.dynamic || route.dynamic;
                    let mut attrs = BTreeMap::new();
                    attrs.insert(
                        "symbol".to_owned(),
                        AttrValue::Str(ctx.qname(method.local_id)),
                    );
                    attrs.insert("controller".to_owned(), AttrValue::Str(class.name.clone()));
                    attrs.insert("method".to_owned(), s(http));
                    attrs.insert(
                        "path".to_owned(),
                        AttrValue::Str(normalize(&[&prefix.path, &route.path])),
                    );
                    attrs.insert("raw_path".to_owned(), AttrValue::Str(route.path.clone()));
                    attrs.insert(
                        "controller_prefix".to_owned(),
                        AttrValue::Str(prefix.path.clone()),
                    );
                    if let Some(version) = handler_version.as_ref().or(info.version.as_ref()) {
                        attrs.insert("version".to_owned(), version.clone());
                    }
                    if let Some(code) = status_code {
                        attrs.insert("status_code".to_owned(), AttrValue::Int(code));
                    }
                    if let Some(redirect) = redirect {
                        attrs.insert("redirect".to_owned(), AttrValue::Bool(true));
                        if let Some(IrExpr::Num(n)) = redirect.args.get(1) {
                            if let Ok(code) = n.parse::<i64>() {
                                attrs.insert("redirect_status".to_owned(), AttrValue::Int(code));
                            }
                        }
                    }
                    attrs.insert("header_names".to_owned(), str_list(header_names.clone()));
                    attrs.insert("sse".to_owned(), AttrValue::Bool(wanted == "Sse"));
                    attrs.insert("dynamic".to_owned(), AttrValue::Bool(dynamic));
                    if dynamic {
                        attrs.insert("path_dynamic".to_owned(), AttrValue::Bool(true));
                    }
                    ctx.emit(
                        ADAPTER,
                        FrameworkFactKind::HttpRoute,
                        Some(method.local_id),
                        attrs,
                        decorator.range,
                        if dynamic { 0.5 } else { conf },
                    );
                }
            }
        }
    }
}

/// Identifiers assigned from `NestFactory.create(...)`.
fn nest_app_vars(ctx: &FrameworkCtx<'_>) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for declarator in nodes_of_kinds(ctx.root, &[kind::VARIABLE_DECLARATOR]) {
        let (Some(name), Some(value)) = (
            declarator.child_by_field_name(field::NAME),
            declarator.child_by_field_name(field::VALUE),
        ) else {
            continue;
        };
        if name.kind() != kind::IDENTIFIER {
            continue;
        }
        let mut current: Node<'_> = value;
        for _ in 0..8 {
            if matches!(
                current.kind(),
                kind::AWAIT_EXPRESSION
                    | kind::PARENTHESIZED_EXPRESSION
                    | kind::AS_EXPRESSION
                    | kind::NON_NULL_EXPRESSION
            ) {
                match current.named_child(0) {
                    Some(inner) => current = inner,
                    None => break,
                }
            } else {
                break;
            }
        }
        if current.kind() != kind::CALL_EXPRESSION {
            continue;
        }
        let chain = current
            .child_by_field_name(field::FUNCTION)
            .and_then(|f| member_chain(f, ctx.src));
        if chain.is_some_and(|c| c.len() == 2 && c[0] == "NestFactory" && c[1] == "create") {
            out.insert(ctx.src.text(name).into_owned());
        }
    }
    out
}

fn is_app_type(declared: &str) -> bool {
    declared.contains("NestApplication") || declared.contains("INestApplication")
}

fn global_config(ctx: &mut FrameworkCtx<'_>) {
    let app_vars = nest_app_vars(ctx);
    let types = receiver_types(ctx.symbols);
    let calls = nodes_of_kinds(ctx.root, &[kind::CALL_EXPRESSION]);
    for call in calls {
        let Some(function) = call.child_by_field_name(field::FUNCTION) else {
            continue;
        };
        let Some(callee) = callee_of(function, ctx.src) else {
            continue;
        };
        if callee.name != "setGlobalPrefix" && callee.name != "enableVersioning" {
            continue;
        }
        let [receiver] = callee.receiver.as_slice() else {
            continue;
        };
        let typed = types.get(receiver).is_some_and(|t| is_app_type(t));
        if !(app_vars.contains(receiver) || typed) {
            continue;
        }
        let args: Vec<IrExpr> = arguments_of(call)
            .into_iter()
            .map(|a| to_ir_expr(a, ctx.src))
            .collect();
        let range = ctx.src.range(call);
        let owner = ctx.owner(range.start);
        let mut attrs = BTreeMap::new();
        attrs.insert("symbol".to_owned(), AttrValue::Str(ctx.qname(owner)));
        if callee.name == "setGlobalPrefix" {
            let Some(prefix) = args.first().and_then(|a| ctx.const_str(a)) else {
                ctx.sink.note(
                    DiagSeverity::Info,
                    DiagCode::UnsupportedConstruct,
                    "global prefix is not a literal",
                    Some(range),
                );
                continue;
            };
            attrs.insert("kind".to_owned(), s("global_prefix"));
            attrs.insert("prefix".to_owned(), AttrValue::Str(prefix));
            let mut exclude = Vec::new();
            if let Some(IrExpr::Object(pairs)) = args.get(1) {
                if let Some(IrExpr::Array(items)) = find_key(pairs, "exclude") {
                    for item in items {
                        match item {
                            IrExpr::Object(p) => {
                                if let Some(path) =
                                    find_key(p, "path").and_then(|v| ctx.const_str(v))
                                {
                                    exclude.push(path);
                                }
                            }
                            other => {
                                if let Some(path) = ctx.const_str(other) {
                                    exclude.push(path);
                                }
                            }
                        }
                    }
                }
            }
            attrs.insert("exclude".to_owned(), str_list(exclude));
        } else {
            attrs.insert("kind".to_owned(), s("versioning"));
            if let Some(IrExpr::Object(pairs)) = args.first() {
                if let Some(t) = find_key(pairs, "type").and_then(last_segment) {
                    attrs.insert("type".to_owned(), AttrValue::Str(t));
                }
                if let Some(v) = find_key(pairs, "defaultVersion") {
                    attrs.insert("default_version".to_owned(), version_value(v));
                }
                if let Some(p) = find_key(pairs, "prefix").and_then(|v| ctx.const_str(v)) {
                    attrs.insert("version_prefix".to_owned(), AttrValue::Str(p));
                }
                if let Some(h) = find_key(pairs, "header").and_then(|v| ctx.const_str(v)) {
                    attrs.insert("header".to_owned(), AttrValue::Str(h));
                }
                if let Some(k) = find_key(pairs, "key").and_then(|v| ctx.const_str(v)) {
                    attrs.insert("key".to_owned(), AttrValue::Str(k));
                }
            }
        }
        ctx.emit(
            ADAPTER,
            FrameworkFactKind::HttpGlobalConfig,
            Some(owner),
            attrs,
            range,
            0.95,
        );
    }
}
