//! FastAPI: HTTP routes from `@app.get("/x")` style decorators and `Depends(...)` parameters.

use analysis_ir::{
    AttrValue, FrameworkAdapter, FrameworkFactKind, FrameworkSignals, IrExpr, IrSymbol,
};

use super::{init_class, str_attrs, PyCtx, CONF_BOUND, CONF_UNBOUND};

pub const ADAPTER: &str = "fastapi";

const ROUTE_METHODS: &[&str] = &["get", "post", "put", "delete", "patch", "options", "head"];

#[derive(Debug, Clone, Copy, Default)]
pub struct FastApi;

impl<'a> FrameworkAdapter<PyCtx<'a>> for FastApi {
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
            injections(ctx, symbol);
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
        if !ROUTE_METHODS.contains(&method) {
            continue;
        }
        let confidence = match init_class(ctx.symbols, receiver).as_deref() {
            Some("FastAPI" | "APIRouter") => CONF_BOUND,
            _ if ctx.bindings.imports_from(ADAPTER) => CONF_UNBOUND,
            _ => continue,
        };
        let path = super::first_str(decorator).unwrap_or_else(|| "?".to_owned());
        let upper = method.to_ascii_uppercase();
        let qualified_name = super::qualified(symbol);
        let mut attrs = str_attrs(&[
            ("symbol", qualified_name.as_str()),
            ("method", upper.as_str()),
            ("path", path.as_str()),
            ("handler", symbol.name.as_str()),
            ("router", receiver),
        ]);
        if let Some(IrExpr::Str(model) | IrExpr::Ident(model)) =
            super::keyword(decorator, "response_model")
        {
            attrs.insert("response_model".to_owned(), AttrValue::Str(model.clone()));
        }
        ctx.emit(
            ADAPTER,
            FrameworkFactKind::HttpRoute,
            Some(symbol.local_id),
            attrs,
            decorator.range,
            confidence,
        );
    }
}

fn injections(ctx: &mut PyCtx<'_>, symbol: &IrSymbol) {
    let Some(AttrValue::Map(defaults)) = symbol.attrs.get("param_defaults") else {
        return;
    };
    for (param, value) in defaults {
        let AttrValue::Expr(IrExpr::Call { callee, args }) = value else {
            continue;
        };
        if callee.last().map(String::as_str) != Some("Depends") {
            continue;
        }
        let target = match args.first() {
            Some(IrExpr::Ident(name)) => name.clone(),
            Some(IrExpr::Member(parts)) => parts.join("."),
            _ => "?".to_owned(),
        };
        ctx.emit(
            ADAPTER,
            FrameworkFactKind::DiInjection,
            Some(symbol.local_id),
            str_attrs(&[("param", param.as_str()), ("target", target.as_str())]),
            symbol.range,
            CONF_BOUND,
        );
    }
}
