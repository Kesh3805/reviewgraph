//! Maps parse outcomes onto stored diagnostic rows (IDX-003): codes and ranges only.

use analysis_ir::{DiagCode, DiagSeverity};
use graph_storage::diagnostics::{ParseDiagnosticRow, MAX_DIAGNOSTICS_PER_FILE};

use super::parse::{FileOutcome, ParsedFile};

/// Stable code stored for an analyzer diagnostic.
pub const fn code_of(code: DiagCode) -> &'static str {
    match code {
        DiagCode::SyntaxError => "syntax_error",
        DiagCode::MissingNode => "missing_node",
        DiagCode::FileTooLarge => "file_too_large",
        DiagCode::ParseTimeout => "timeout",
        DiagCode::ComputedMemberName => "computed_member_name",
        DiagCode::UnsupportedConstruct => "unsupported_construct",
        DiagCode::DynamicImportNonLiteral => "dynamic_import_non_literal",
        DiagCode::DuplicateSymbol => "duplicate_symbol",
        DiagCode::DepthLimit => "depth_limit",
        DiagCode::ExprTruncated => "expr_truncated",
    }
}

const fn severity_of(severity: DiagSeverity) -> u8 {
    match severity {
        DiagSeverity::Error => 2,
        DiagSeverity::Warning => 1,
        DiagSeverity::Info => 0,
    }
}

/// The rows to store for `file`: at most [`MAX_DIAGNOSTICS_PER_FILE`], never message text.
/// A failed file without a unit (analyzer panic, unreadable file) gets one row naming the reason.
pub fn rows_for(file: &ParsedFile) -> Vec<ParseDiagnosticRow> {
    if let Some(unit) = file.outcome.unit() {
        return unit
            .diagnostics
            .iter()
            .take(MAX_DIAGNOSTICS_PER_FILE)
            .map(|d| ParseDiagnosticRow {
                severity: severity_of(d.severity),
                code: code_of(d.code).to_owned(),
                line: d.range.map(|r| r.start.line),
                col: d.range.map(|r| r.start.column),
                end_line: d.range.map(|r| r.end.line),
                end_col: d.range.map(|r| r.end.column),
            })
            .collect();
    }
    match &file.outcome {
        FileOutcome::Failed { reason, .. } => vec![ParseDiagnosticRow {
            severity: 2,
            code: reason.as_str().to_owned(),
            line: None,
            col: None,
            end_line: None,
            end_col: None,
        }],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_snake_case_and_unique() {
        let all = [
            DiagCode::SyntaxError,
            DiagCode::MissingNode,
            DiagCode::FileTooLarge,
            DiagCode::ParseTimeout,
            DiagCode::ComputedMemberName,
            DiagCode::UnsupportedConstruct,
            DiagCode::DynamicImportNonLiteral,
            DiagCode::DuplicateSymbol,
            DiagCode::DepthLimit,
            DiagCode::ExprTruncated,
        ];
        let mut seen = std::collections::BTreeSet::new();
        for code in all {
            let text = code_of(code);
            assert!(text.chars().all(|c| c.is_ascii_lowercase() || c == '_'));
            assert!(seen.insert(text));
        }
    }
}
