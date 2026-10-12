//! Framework adapters (ADR-006). They run inside the analyzer after declarations, read the
//! decorators, class attributes and import bindings already extracted, and emit neutral
//! `IrFrameworkFact`s. Framework knowledge lives only in this module (target-architecture
//! section 3.1).

pub mod celery;
pub mod django;
pub mod fastapi;
pub mod flask;
pub mod pytest;

use std::collections::BTreeMap;

use analysis_ir::{
    AnalyzerConfig, AttrValue, FrameworkAdapter, FrameworkFactKind, IrDecorator, IrExpr,
    IrFrameworkFact, IrImport, IrSymbol, LocalId,
};
use review_core::location::SourceRange;

use crate::imports::BindingTable;

/// Confidence when the receiver is known to be an instance of the framework's application class.
pub const CONF_BOUND: f32 = 0.95;
/// Confidence when only the import of the framework package supports the match.
pub const CONF_UNBOUND: f32 = 0.7;

/// Shared state for one file's adapters.
#[derive(Debug)]
pub struct PyCtx<'a> {
    pub file: &'a str,
    pub symbols: &'a [IrSymbol],
    pub imports: &'a [IrImport],
    pub bindings: &'a BindingTable,
    pub facts: Vec<IrFrameworkFact>,
}

impl PyCtx<'_> {
    /// Adds one fact.
    pub fn emit(
        &mut self,
        adapter: &str,
        kind: FrameworkFactKind,
        symbol: Option<LocalId>,
        attrs: BTreeMap<String, AttrValue>,
        range: SourceRange,
        confidence: f32,
    ) {
        self.facts.push(IrFrameworkFact {
            adapter: adapter.to_owned(),
            kind,
            symbol,
            attrs,
            range,
            confidence,
        });
    }
}

/// `{"k": Str(v), ...}` from pairs.
pub fn str_attrs(pairs: &[(&str, &str)]) -> BTreeMap<String, AttrValue> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), AttrValue::Str((*v).to_owned())))
        .collect()
}

/// The dotted qualified name of a symbol, which is how `symbol` attributes name it.
pub fn qualified(symbol: &IrSymbol) -> String {
    symbol.qualified_name.join(".")
}

/// The first positional argument of a decorator when it is a string literal.
pub fn first_str(decorator: &IrDecorator) -> Option<String> {
    match decorator.args.first() {
        Some(IrExpr::Str(value)) => Some(value.clone()),
        _ => None,
    }
}

/// A keyword argument of a decorator call (keywords are the trailing `Object` argument).
pub fn keyword<'d>(decorator: &'d IrDecorator, name: &str) -> Option<&'d IrExpr> {
    match decorator.args.last() {
        Some(IrExpr::Object(pairs)) => pairs.iter().find(|(k, _)| k == name).map(|(_, v)| v),
        _ => None,
    }
}

/// The class a module level variable is initialized with (`app = FastAPI()` gives `FastAPI`),
/// without its module prefix.
pub fn init_class(symbols: &[IrSymbol], variable: &str) -> Option<String> {
    symbols
        .iter()
        .find(|s| s.parent == Some(LocalId(0)) && s.name == variable)
        .and_then(|s| match s.attrs.get("init_call") {
            Some(AttrValue::Str(callee)) => callee.rsplit('.').next().map(str::to_owned),
            _ => None,
        })
}

/// Whether the path names a pytest test module or conftest.
pub fn is_test_path(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.starts_with("test_")
        || name.ends_with("_test.py")
        || name == "conftest.py"
        || path.split('/').any(|segment| segment == "tests")
}

/// Runs every enabled and detected adapter over one parsed file.
pub fn run(
    file: &str,
    symbols: &[IrSymbol],
    imports: &[IrImport],
    bindings: &BindingTable,
    cfg: &AnalyzerConfig,
) -> Vec<IrFrameworkFact> {
    let mut ctx = PyCtx {
        file,
        symbols,
        imports,
        bindings,
        facts: Vec::new(),
    };
    let adapters: [&dyn FrameworkAdapter<PyCtx<'_>>; 5] = [
        &fastapi::FastApi,
        &flask::Flask,
        &django::Django,
        &pytest::Pytest,
        &celery::Celery,
    ];
    for adapter in adapters {
        let name = adapter.name();
        let enabled = cfg
            .enabled_adapters
            .as_ref()
            .is_none_or(|set| set.contains(name));
        let detected = adapter.detect(&cfg.frameworks)
            || bindings.imports_from(name)
            || (name == "pytest" && is_test_path(file));
        if enabled && detected {
            adapter.extract(&mut ctx);
        }
    }
    ctx.facts
}
