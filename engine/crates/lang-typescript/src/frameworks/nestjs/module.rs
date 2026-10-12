//! `@Module({...})` facts (NEST-001).

use std::collections::BTreeMap;

use analysis_ir::{
    AttrValue, DiagCode, DiagSeverity, FrameworkFactKind, IrExpr, IrFrameworkFact, Modifiers,
};
use review_core::location::SourceRange;
use review_core::symbol::SymbolKind;

use super::{ADAPTER, COMMON};
use crate::frameworks::util::{name_of, nodes_of_kinds, s};
use crate::frameworks::FrameworkCtx;
use crate::kinds::kind;
use crate::visit::expr::to_ir_expr;

const LISTS: [&str; 4] = ["imports", "providers", "controllers", "exports"];

pub fn collect(ctx: &mut FrameworkCtx<'_>) {
    let symbols = ctx.symbols;
    let returns = nodes_of_kinds(ctx.root, &[kind::RETURN_STATEMENT]);
    for class in symbols.iter().filter(|s| s.kind == SymbolKind::Class) {
        let Some((decorator, confidence)) = class
            .decorators
            .iter()
            .find_map(|d| ctx.decorator_origin(d, COMMON, "Module").map(|c| (d, c)))
        else {
            continue;
        };
        let global = class
            .decorators
            .iter()
            .any(|d| ctx.decorator_origin(d, COMMON, "Global").is_some());
        // `static forRoot(...) { return { module: X, providers: [...] }; }`
        let mut dynamic_returns: Vec<(String, IrExpr, SourceRange)> = Vec::new();
        for method in symbols.iter().filter(|s| {
            s.parent == Some(class.local_id)
                && s.kind == SymbolKind::Method
                && s.modifiers.contains(Modifiers::STATIC)
        }) {
            for ret in &returns {
                let range = ctx.src.range(*ret);
                if ctx.owner(range.start) != method.local_id {
                    continue;
                }
                let Some(value) = ret.named_child(0) else {
                    continue;
                };
                if value.kind() != kind::OBJECT {
                    continue;
                }
                let expr = to_ir_expr(value, ctx.src);
                let has_module = match &expr {
                    IrExpr::Object(pairs) => pairs.iter().any(|(k, _)| k == "module"),
                    _ => false,
                };
                if has_module {
                    dynamic_returns.push((method.name.clone(), expr, range));
                }
            }
        }
        let mut attrs = BTreeMap::new();
        attrs.insert("module_name".to_owned(), AttrValue::Str(class.name.clone()));
        attrs.insert("global".to_owned(), AttrValue::Bool(global));
        attrs.insert(
            "dynamic".to_owned(),
            AttrValue::Bool(!dynamic_returns.is_empty()),
        );
        let mut uncertain = false;
        let metadata = match decorator.args.first() {
            Some(IrExpr::Object(pairs)) => Some(pairs.as_slice()),
            Some(other) => {
                attrs.insert("metadata_ref".to_owned(), AttrValue::Expr(other.clone()));
                ctx.sink.note(
                    DiagSeverity::Warning,
                    DiagCode::UnsupportedConstruct,
                    "module metadata is not an object literal",
                    Some(decorator.range),
                );
                None
            }
            None => None,
        };
        fill_lists(ctx, metadata, &mut attrs, &mut uncertain);
        let fact_confidence = if uncertain {
            confidence.min(0.7)
        } else {
            confidence
        };
        ctx.facts.push(IrFrameworkFact {
            adapter: ADAPTER.to_owned(),
            kind: FrameworkFactKind::ModuleDeclaration,
            symbol: Some(class.local_id),
            attrs,
            range: decorator.range,
            confidence: fact_confidence,
        });
        for (method, expr, range) in dynamic_returns {
            let IrExpr::Object(pairs) = &expr else {
                continue;
            };
            let mut dyn_attrs = BTreeMap::new();
            dyn_attrs.insert("module_name".to_owned(), AttrValue::Str(class.name.clone()));
            dyn_attrs.insert("global".to_owned(), AttrValue::Bool(global));
            dyn_attrs.insert("dynamic".to_owned(), AttrValue::Bool(true));
            dyn_attrs.insert("dynamic_method".to_owned(), AttrValue::Str(method));
            let mut dyn_uncertain = false;
            fill_lists(
                ctx,
                Some(pairs.as_slice()),
                &mut dyn_attrs,
                &mut dyn_uncertain,
            );
            ctx.facts.push(IrFrameworkFact {
                adapter: ADAPTER.to_owned(),
                kind: FrameworkFactKind::ModuleDeclaration,
                symbol: Some(class.local_id),
                attrs: dyn_attrs,
                range,
                confidence: if dyn_uncertain {
                    confidence.min(0.7)
                } else {
                    confidence
                },
            });
        }
    }
}

/// Fills `imports`, `providers`, `controllers` and `exports` from the metadata pairs.
fn fill_lists(
    ctx: &FrameworkCtx<'_>,
    metadata: Option<&[(String, IrExpr)]>,
    attrs: &mut BTreeMap<String, AttrValue>,
    uncertain: &mut bool,
) {
    for key in LISTS {
        let found = metadata.and_then(|pairs| pairs.iter().find(|(k, _)| k == key));
        let entries: Vec<AttrValue> = match found.map(|(_, v)| v) {
            Some(IrExpr::Array(items)) => items
                .iter()
                .map(|item| entry(ctx, item, uncertain))
                .collect(),
            Some(other) => vec![entry(ctx, other, uncertain)],
            None => Vec::new(),
        };
        attrs.insert(key.to_owned(), AttrValue::List(entries));
    }
}

/// One `{ name, form, import_specifier?, imported?, args? }` entry.
pub fn entry(ctx: &FrameworkCtx<'_>, expr: &IrExpr, uncertain: &mut bool) -> AttrValue {
    let mut m = BTreeMap::new();
    let (name, form): (String, &str) = match expr {
        IrExpr::Ident(n) => (n.clone(), "class"),
        IrExpr::Call { callee, args } if callee.len() == 1 && callee[0] == "forwardRef" => {
            let name = match args.first() {
                Some(IrExpr::Arrow { returns }) => name_of(returns),
                _ => String::new(),
            };
            (name, "forward_ref")
        }
        IrExpr::Call { callee, args } => {
            let root = callee.first().cloned().unwrap_or_default();
            let form = match callee.last().map(String::as_str) {
                Some("forRoot") => "for_root",
                Some("forRootAsync") => "for_root_async",
                Some("forFeature") => "for_feature",
                Some("register" | "registerAsync") => "register",
                Some("registerQueue" | "registerQueueAsync") => "register_queue",
                _ => {
                    *uncertain = true;
                    "dynamic_expr"
                }
            };
            m.insert(
                "args".to_owned(),
                AttrValue::List(args.iter().cloned().map(AttrValue::Expr).collect()),
            );
            (root, form)
        }
        IrExpr::Object(pairs) => {
            provider_object(pairs, &mut m);
            (String::new(), "provider_object")
        }
        IrExpr::Other(text) if text.starts_with("...") => {
            *uncertain = true;
            (text.trim_start_matches("...").to_owned(), "spread")
        }
        _ => {
            *uncertain = true;
            (String::new(), "dynamic_expr")
        }
    };
    if let Some(binding) = ctx.binding(name.split('.').next().unwrap_or("")) {
        m.insert("import_specifier".to_owned(), s(&binding.specifier));
        m.insert("imported".to_owned(), s(&binding.imported));
    }
    m.insert("name".to_owned(), AttrValue::Str(name));
    m.insert("form".to_owned(), s(form));
    AttrValue::Map(m)
}

/// `{ provide, useClass | useExisting | useFactory | useValue, inject, scope }`. `useValue`
/// literals may hold secrets and are never stored, only their kind.
fn provider_object(pairs: &[(String, IrExpr)], m: &mut BTreeMap<String, AttrValue>) {
    for (key, value) in pairs {
        match key.as_str() {
            "provide" => {
                m.insert("provide".to_owned(), AttrValue::Expr(value.clone()));
            }
            "useClass" => {
                m.insert("use_class".to_owned(), AttrValue::Str(name_of(value)));
            }
            "useExisting" => {
                m.insert("use_existing".to_owned(), AttrValue::Str(name_of(value)));
            }
            "useFactory" => {
                m.insert(
                    "use_factory".to_owned(),
                    AttrValue::Expr(IrExpr::Other("factory".to_owned())),
                );
            }
            "useValue" => {
                let kind = match value {
                    IrExpr::Str(_) | IrExpr::Template { .. } => "string",
                    IrExpr::Num(_) => "number",
                    IrExpr::Object(_) => "object",
                    IrExpr::Ident(_) | IrExpr::Member(_) => "ident",
                    _ => "other",
                };
                m.insert("use_value".to_owned(), s(kind));
            }
            "inject" => {
                let names = match value {
                    IrExpr::Array(items) => {
                        items.iter().map(|i| AttrValue::Str(name_of(i))).collect()
                    }
                    _ => Vec::new(),
                };
                m.insert("inject".to_owned(), AttrValue::List(names));
            }
            "scope" => {
                m.insert("scope".to_owned(), AttrValue::Expr(value.clone()));
            }
            _ => {}
        }
    }
}
