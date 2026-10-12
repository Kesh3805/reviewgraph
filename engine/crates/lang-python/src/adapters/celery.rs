//! Celery: task definitions from `@app.task` and `@shared_task` decorators.

use analysis_ir::{AttrValue, FrameworkAdapter, FrameworkFactKind, FrameworkSignals, IrExpr};

use super::{init_class, str_attrs, PyCtx, CONF_BOUND, CONF_UNBOUND};

pub const ADAPTER: &str = "celery";

#[derive(Debug, Clone, Copy, Default)]
pub struct Celery;

impl<'a> FrameworkAdapter<PyCtx<'a>> for Celery {
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
            for decorator in &symbol.decorators {
                let parts: Vec<&str> = decorator.name.split('.').collect();
                let confidence = match parts.as_slice() {
                    ["shared_task"] => CONF_BOUND,
                    [receiver, "task"] => match init_class(ctx.symbols, receiver).as_deref() {
                        Some("Celery") => CONF_BOUND,
                        _ => CONF_UNBOUND,
                    },
                    _ => continue,
                };
                let mut attrs = str_attrs(&[
                    ("symbol", super::qualified(symbol).as_str()),
                    ("handler", symbol.name.as_str()),
                    ("queue", "celery"),
                ]);
                for key in ["name", "queue"] {
                    if let Some(IrExpr::Str(value)) = super::keyword(decorator, key) {
                        attrs.insert(key.to_owned(), AttrValue::Str(value.clone()));
                    }
                }
                ctx.emit(
                    ADAPTER,
                    FrameworkFactKind::QueueJobHandler,
                    Some(symbol.local_id),
                    attrs,
                    decorator.range,
                    confidence,
                );
            }
        }
    }
}
