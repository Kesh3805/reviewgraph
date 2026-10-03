//! Parse diagnostics. Messages never embed source text: constructors accept only static strings
//! and numbers, and cap the length.

use review_core::location::SourceRange;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const MAX_DIAGNOSTIC_CHARS: usize = 200;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
pub enum DiagSeverity {
    Error,
    Warning,
    Info,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
pub enum DiagCode {
    SyntaxError,
    MissingNode,
    FileTooLarge,
    ParseTimeout,
    ComputedMemberName,
    UnsupportedConstruct,
    DynamicImportNonLiteral,
    DuplicateSymbol,
    DepthLimit,
    ExprTruncated,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("diagnostic message is {len} characters; the limit is {MAX_DIAGNOSTIC_CHARS}")]
pub struct DiagnosticTooLong {
    pub len: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ParseDiagnostic {
    pub severity: DiagSeverity,
    pub code: DiagCode,
    /// At most 200 characters; never source text.
    pub message: String,
    pub range: Option<SourceRange>,
}

impl ParseDiagnostic {
    /// `message` must be a static string so source text cannot leak into diagnostics.
    pub fn new(
        severity: DiagSeverity,
        code: DiagCode,
        message: &'static str,
        range: Option<SourceRange>,
    ) -> Result<Self, DiagnosticTooLong> {
        let len = message.chars().count();
        if len > MAX_DIAGNOSTIC_CHARS {
            return Err(DiagnosticTooLong { len });
        }
        Ok(Self {
            severity,
            code,
            message: message.to_owned(),
            range,
        })
    }

    /// `"<prefix> <kind>"` where `kind` is a grammar node kind (a `'static` string owned by the
    /// parser tables, never source text).
    pub fn with_kind(
        severity: DiagSeverity,
        code: DiagCode,
        prefix: &'static str,
        kind: &'static str,
        range: Option<SourceRange>,
    ) -> Result<Self, DiagnosticTooLong> {
        let message = format!("{prefix} {kind}");
        let len = message.chars().count();
        if len > MAX_DIAGNOSTIC_CHARS {
            return Err(DiagnosticTooLong { len });
        }
        Ok(Self {
            severity,
            code,
            message,
            range,
        })
    }

    /// `"<prefix> <n>"`, for counts such as `"more diagnostics: 12"`.
    pub fn with_count(
        severity: DiagSeverity,
        code: DiagCode,
        prefix: &'static str,
        count: u64,
        range: Option<SourceRange>,
    ) -> Result<Self, DiagnosticTooLong> {
        let message = format!("{prefix} {count}");
        let len = message.chars().count();
        if len > MAX_DIAGNOSTIC_CHARS {
            return Err(DiagnosticTooLong { len });
        }
        Ok(Self {
            severity,
            code,
            message,
            range,
        })
    }
}
