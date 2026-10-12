//! Flask: HTTP routes from `@app.route("/x", methods=[...])` and the Flask 2 method shortcuts.

use analysis_ir::{FrameworkAdapter, FrameworkFactKind, FrameworkSignals, IrExpr, IrSymbol};

use super::{init_class, str_attrs, PyCtx, CONF_BOUND, CONF_UNBOUND};

pub const ADAPTER: &str = "flask";

const SHORTCUTS: &[&str] = &["get", "post", "put", "delete", "patch"];

#[derive(Debug, Clone, Copy, Default)]
pub struct Flask;

impl<'a> FrameworkAdapter<PyCtx<'a>> for Flask {
    fn name(&self) -> &'static str {
        ADAPTER
    }

    fn version(&self) -> u32 {
        1
    }

    fn detect(&self, signals: &FrameworkSignals) -> bool {
        signals.has(ADAPTER)
    }

    fn extract(&self, ctx: &mut PyCtx<'a>) {
        let symbols = ctx.symbols;
        for symbol in symbols {
            routes(ctx, symbol);
        }
    }
}

fn routes(ctx: &mut PyCtx<'_>, symbol: &IrSymbol) {
    for decorator in &symbol.decorators {
        let parts: Vec<&str> = decorator.name.split('.').collect();
        let [receiver, method] = parts.as_slice() else {
            continue;
        };
        let (receiver, method) = (*receiver, *method);
        let is_route = method == "route";
        if !is_route && !SHORTCUTS.contains(&method) {
            continue;
        }
        let confidence = match init_class(ctx.symbols, receiver).as_deref() {
            Some("Flask" | "Blueprint") => CONF_BOUND,
            _ if ctx.bindings.imports_from(ADAPTER) => CONF_UNBOUND,
            _ => continue,
        };
        let methods: Vec<String> = if is_route {
            match super::keyword(decorator, "methods") {
                Some(IrExpr::Array(items)) => items
                    .iter()
                    .filter_map(|item| match item {
                        IrExpr::Str(m) => Some(m.to_ascii_uppercase()),
                        _ => None,
                    })
                    .collect(),
                _ => vec!["GET".to_owned()],
            }
        } else {
            vec![method.to_ascii_uppercase()]
        };
        let path = super::first_str(decorator).unwrap_or_else(|| "?".to_owned());
        let qualified_name = super::qualified(symbol);
        for http_method in methods {
            ctx.emit(
                ADAPTER,
                FrameworkFactKind::HttpRoute,
                Some(symbol.local_id),
                str_attrs(&[
                    ("symbol", qualified_name.as_str()),
                    ("method", http_method.as_str()),
                    ("path", path.as_str()),
                    ("handler", symbol.name.as_str()),
                    ("router", receiver),
                ]),
                decorator.range,
                confidence,
            );
        }
    }
}
