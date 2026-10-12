//! Django: models (entities, columns, relations) and class based views.

use analysis_ir::{AttrValue, FrameworkAdapter, FrameworkFactKind, FrameworkSignals, IrSymbol};
use review_core::symbol::SymbolKind;

use super::{str_attrs, PyCtx, CONF_BOUND, CONF_UNBOUND};

pub const ADAPTER: &str = "django";

const RELATION_FIELDS: &[&str] = &["ForeignKey", "OneToOneField", "ManyToManyField"];

#[derive(Debug, Clone, Copy, Default)]
pub struct Django;

impl<'a> FrameworkAdapter<PyCtx<'a>> for Django {
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
        for class in symbols.iter().filter(|s| s.kind == SymbolKind::Class) {
            if extends_suffix(class, &["Model"]) {
                model(ctx, symbols, class);
            } else if extends_suffix(class, &["View", "APIView", "ViewSet", "ModelViewSet"]) {
                ctx.emit(
                    ADAPTER,
                    FrameworkFactKind::Controller,
                    Some(class.local_id),
                    str_attrs(&[
                        ("symbol", super::qualified(class).as_str()),
                        ("name", class.name.as_str()),
                    ]),
                    class.range,
                    CONF_UNBOUND,
                );
            }
        }
    }
}

fn extends_suffix(class: &IrSymbol, suffixes: &[&str]) -> bool {
    class.heritage.extends.iter().any(|base| {
        let last = base.rsplit('.').next().unwrap_or(base);
        suffixes.iter().any(|s| last.ends_with(s))
    })
}

fn model(ctx: &mut PyCtx<'_>, symbols: &[IrSymbol], class: &IrSymbol) {
    ctx.emit(
        ADAPTER,
        FrameworkFactKind::OrmEntity,
        Some(class.local_id),
        str_attrs(&[
            ("symbol", super::qualified(class).as_str()),
            ("name", class.name.as_str()),
            ("table", class.name.to_ascii_lowercase().as_str()),
        ]),
        class.range,
        CONF_BOUND,
    );
    for field in symbols
        .iter()
        .filter(|s| s.parent == Some(class.local_id) && s.kind == SymbolKind::Field)
    {
        let Some(AttrValue::Str(callee)) = field.attrs.get("init_call") else {
            continue;
        };
        let field_type = callee.rsplit('.').next().unwrap_or(callee);
        if !field_type.ends_with("Field") && !field_type.ends_with("Key") {
            continue;
        }
        if RELATION_FIELDS.contains(&field_type) {
            let target = match field.attrs.get("init_arg0") {
                Some(AttrValue::Str(target)) => target.as_str(),
                _ => "?",
            };
            ctx.emit(
                ADAPTER,
                FrameworkFactKind::OrmRelation,
                Some(field.local_id),
                str_attrs(&[
                    ("from", class.name.as_str()),
                    ("field", field.name.as_str()),
                    ("relation", field_type),
                    ("target", target),
                ]),
                field.range,
                CONF_BOUND,
            );
        } else {
            ctx.emit(
                ADAPTER,
                FrameworkFactKind::OrmColumn,
                Some(field.local_id),
                str_attrs(&[
                    ("entity", class.name.as_str()),
                    ("name", field.name.as_str()),
                    ("type", field_type),
                ]),
                field.range,
                CONF_BOUND,
            );
        }
    }
}
