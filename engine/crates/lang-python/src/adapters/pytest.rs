//! pytest: test suites, test cases (linked to the module under test by name) and fixtures.

use analysis_ir::{AttrValue, FrameworkAdapter, FrameworkFactKind, FrameworkSignals};
use review_core::symbol::SymbolKind;

use super::{is_test_path, str_attrs, PyCtx, CONF_BOUND, CONF_UNBOUND};

pub const ADAPTER: &str = "pytest";

#[derive(Debug, Clone, Copy, Default)]
pub struct Pytest;

/// The module a test file is named after: `tests/test_billing.py` and `billing_test.py` both give
/// `billing`.
pub fn subject_hint(file: &str) -> String {
    let name = file.rsplit('/').next().unwrap_or(file);
    let stem = name.strip_suffix(".py").unwrap_or(name);
    let stem = stem.strip_prefix("test_").unwrap_or(stem);
    stem.strip_suffix("_test").unwrap_or(stem).to_owned()
}

impl<'a> FrameworkAdapter<PyCtx<'a>> for Pytest {
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
        let test_file = is_test_path(ctx.file);
        let subject = subject_hint(ctx.file);
        let imported: Vec<AttrValue> = ctx
            .imports
            .iter()
            .filter(|i| !i.specifier.starts_with("pytest") && !i.type_only)
            .map(|i| AttrValue::Str(i.specifier.clone()))
            .collect();
        for symbol in symbols {
            let is_fixture = symbol
                .decorators
                .iter()
                .any(|d| d.name.rsplit('.').next() == Some("fixture"));
            if is_fixture {
                ctx.emit(
                    ADAPTER,
                    FrameworkFactKind::Custom("pytest_fixture".to_owned()),
                    Some(symbol.local_id),
                    str_attrs(&[("name", symbol.name.as_str())]),
                    symbol.range,
                    CONF_BOUND,
                );
                continue;
            }
            if !test_file {
                continue;
            }
            match symbol.kind {
                SymbolKind::Class if symbol.name.starts_with("Test") => ctx.emit(
                    ADAPTER,
                    FrameworkFactKind::TestSuite,
                    Some(symbol.local_id),
                    str_attrs(&[
                        ("symbol", super::qualified(symbol).as_str()),
                        ("suite_path", ctx.file),
                        ("name", symbol.name.as_str()),
                    ]),
                    symbol.range,
                    CONF_UNBOUND,
                ),
                SymbolKind::Function | SymbolKind::Method if symbol.name.starts_with("test") => {
                    let suite_path = match symbol.parent.and_then(|p| symbols.get(p.0 as usize)) {
                        Some(parent) if parent.kind == SymbolKind::Class => {
                            super::qualified(parent)
                        }
                        _ => ctx.file.to_owned(),
                    };
                    let mut attrs = str_attrs(&[
                        ("symbol", super::qualified(symbol).as_str()),
                        ("suite_path", suite_path.as_str()),
                        ("name", symbol.name.as_str()),
                        ("subject_hint", subject.as_str()),
                    ]);
                    let (start, end) = (symbol.range.start, symbol.range.end);
                    attrs.insert(
                        "range".to_owned(),
                        AttrValue::List(
                            [start.line, start.column, end.line, end.column]
                                .iter()
                                .map(|n| AttrValue::Int(i64::from(*n)))
                                .collect(),
                        ),
                    );
                    attrs.insert("imports".to_owned(), AttrValue::List(imported.clone()));
                    ctx.emit(
                        ADAPTER,
                        FrameworkFactKind::TestCase,
                        Some(symbol.local_id),
                        attrs,
                        symbol.range,
                        CONF_BOUND,
                    );
                }
                _ => {}
            }
        }
    }
}
