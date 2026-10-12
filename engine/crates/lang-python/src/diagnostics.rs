//! Diagnostics collection: capped, never carrying source text.

use analysis_ir::diagnostic::{DiagCode, DiagSeverity, ParseDiagnostic};
use review_core::location::SourceRange;
use tree_sitter::Node;

use crate::text::Source;

/// Diagnostics kept per file; further ones are summarized.
pub const MAX_DIAGNOSTICS: usize = 50;

#[derive(Debug, Default)]
pub struct DiagnosticSink {
    items: Vec<ParseDiagnostic>,
    dropped: u64,
}

impl DiagnosticSink {
    pub fn push(
        &mut self,
        diagnostic: Result<ParseDiagnostic, analysis_ir::diagnostic::DiagnosticTooLong>,
    ) {
        let Ok(diagnostic) = diagnostic else { return };
        if self.items.len() < MAX_DIAGNOSTICS {
            self.items.push(diagnostic);
        } else {
            self.dropped += 1;
        }
    }

    pub fn note(
        &mut self,
        severity: DiagSeverity,
        code: DiagCode,
        message: &'static str,
        range: Option<SourceRange>,
    ) {
        self.push(ParseDiagnostic::new(severity, code, message, range));
    }

    /// Appends the `+ N more` summary when diagnostics were dropped.
    pub fn finish(mut self) -> Vec<ParseDiagnostic> {
        if self.dropped > 0 {
            if let Ok(summary) = ParseDiagnostic::with_count(
                DiagSeverity::Info,
                DiagCode::SyntaxError,
                "more diagnostics:",
                self.dropped,
                None,
            ) {
                self.items.push(summary);
            }
        }
        self.items
    }
}

/// Result of scanning a tree for error and missing nodes.
#[derive(Debug, Default)]
pub struct ErrorScan {
    pub error_nodes: u32,
    pub missing_nodes: u32,
    /// Ranges of ERROR and MISSING nodes, used to mark symbols that overlap them.
    pub ranges: Vec<SourceRange>,
}

/// Walks the tree iteratively (no recursion, so deep trees cannot overflow the stack) and turns
/// ERROR and MISSING nodes into diagnostics. Subtrees without errors are skipped.
pub fn scan_errors(root: Node<'_>, source: &Source<'_>, sink: &mut DiagnosticSink) -> ErrorScan {
    let mut scan = ErrorScan::default();
    if !root.has_error() {
        return scan;
    }
    let mut cursor = root.walk();
    loop {
        let node = cursor.node();
        let mut descend = node.has_error();
        if node.is_error() {
            scan.error_nodes += 1;
            let range = source.range(node);
            scan.ranges.push(range);
            sink.note(
                DiagSeverity::Error,
                DiagCode::SyntaxError,
                "syntax error",
                Some(range),
            );
        } else if node.is_missing() {
            scan.missing_nodes += 1;
            let range = source.range(node);
            scan.ranges.push(range);
            sink.push(ParseDiagnostic::with_kind(
                DiagSeverity::Error,
                DiagCode::MissingNode,
                "missing",
                node.kind(),
                Some(range),
            ));
            descend = false;
        }
        if descend && cursor.goto_first_child() {
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return scan;
            }
        }
    }
}
