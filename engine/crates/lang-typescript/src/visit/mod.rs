//! The syntax-tree visitor. TSA-002 provides the module symbol; later tasks add declarations,
//! imports/exports, references and syntax facts, all walking the tree iteratively with a depth
//! cap so no input can overflow the stack.

use analysis_ir::{
    DiagCode, DiagSeverity, IrExport, IrFrameworkFact, IrImport, IrReference, IrSymbol, LocalId,
    SymbolFacts,
};
use review_core::location::SourceRange;
use review_core::symbol::SymbolKind;

use crate::diagnostics::DiagnosticSink;
use crate::text::Source;

/// Deepest nesting the visitors descend into.
pub const MAX_VISIT_DEPTH: usize = 256;

/// Everything the visitors produce for one file.
#[derive(Debug, Default)]
pub struct Collected {
    pub symbols: Vec<IrSymbol>,
    pub references: Vec<IrReference>,
    pub imports: Vec<IrImport>,
    pub exports: Vec<IrExport>,
    pub framework: Vec<IrFrameworkFact>,
    pub facts: Vec<SymbolFacts>,
}

/// Inputs shared by the visitors.
#[derive(Debug)]
pub struct VisitCtx<'a> {
    pub source: Source<'a>,
    pub module_name: String,
    /// Ranges of ERROR/MISSING nodes; symbols overlapping one get `has_errors`.
    pub error_ranges: &'a [SourceRange],
}

/// Runs every visitor over `root`.
pub fn run(
    root: tree_sitter::Node<'_>,
    ctx: &VisitCtx<'_>,
    sink: &mut DiagnosticSink,
) -> Collected {
    let _ = (root, sink);
    let mut out = Collected::default();
    out.symbols.push(module_symbol(
        &ctx.source,
        &ctx.module_name,
        !ctx.error_ranges.is_empty(),
    ));
    out
}

/// `symbols[0]`: the module symbol covering the whole file.
pub fn module_symbol(source: &Source<'_>, name: &str, has_errors: bool) -> IrSymbol {
    let mut module = IrSymbol::new(
        LocalId(0),
        SymbolKind::Module,
        name,
        vec!["__module__".to_owned()],
        source.whole(),
    );
    module.has_errors = has_errors;
    module
}

/// Records that a subtree was skipped because it nests deeper than [`MAX_VISIT_DEPTH`].
pub fn depth_limit(sink: &mut DiagnosticSink, range: SourceRange) {
    sink.note(
        DiagSeverity::Warning,
        DiagCode::DepthLimit,
        "subtree skipped: nesting is too deep",
        Some(range),
    );
}
