//! The syntax-tree visitor: declarations (TSA-003), imports/exports (TSA-004), references
//! (TSA-005), syntax facts (TSA-006) and hashes (TSA-007), all walking the tree with bounded
//! depth so no input can overflow the stack.

pub mod declarations;
pub mod expr;

use analysis_ir::{
    AnalyzerConfig, DiagCode, DiagSeverity, IrExport, IrFrameworkFact, IrImport, IrReference,
    IrSymbol, LocalId, SymbolFacts,
};
use review_core::location::SourceRange;
use review_core::symbol::SymbolKind;

use crate::diagnostics::DiagnosticSink;
use crate::ordinals::assign_ordinals;
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
    pub cfg: &'a AnalyzerConfig,
}

/// Runs every visitor over `root`.
pub fn run(
    root: tree_sitter::Node<'_>,
    ctx: &VisitCtx<'_>,
    sink: &mut DiagnosticSink,
) -> Collected {
    let module = module_symbol(&ctx.source, &ctx.module_name, !ctx.error_ranges.is_empty());
    let mut table = declarations::collect(root, &ctx.source, ctx.cfg, sink, module);
    declarations::mark_errors(&mut table.symbols, ctx.error_ranges);
    let report = assign_ordinals(&mut table.symbols);
    if report.dropped > 0 {
        sink.note(
            DiagSeverity::Error,
            DiagCode::DuplicateSymbol,
            "too many duplicate symbols; some were dropped",
            None,
        );
    }
    for _ in &report.duplicate_groups {
        sink.note(
            DiagSeverity::Info,
            DiagCode::DuplicateSymbol,
            "duplicate symbol name in one scope",
            None,
        );
    }
    Collected {
        symbols: table.symbols,
        ..Collected::default()
    }
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
