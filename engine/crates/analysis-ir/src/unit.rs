//! `ParsedUnit`: the cached, serializable result of analyzing one file.

use std::collections::BTreeSet;

use review_core::language::{Dialect, Language};
use review_core::location::{ContentHash, RepoPath};
use review_core::symbol::ModulePath;
use review_core::version::AnalyzerVersion;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::diagnostic::ParseDiagnostic;
use crate::facts::SymbolFacts;
use crate::framework::{FrameworkSignals, IrFrameworkFact};
use crate::module::{IrExport, IrImport};
use crate::reference::IrReference;
use crate::symbol::IrSymbol;

/// Bumped on any change to the serialized shape of [`ParsedUnit`]. Part of the parse-cache key.
pub const IR_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AnalyzerId {
    pub name: String,
    pub version: AnalyzerVersion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum FailReason {
    TooLarge,
    Timeout,
    Binary,
    NotUtf8Decodable,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum ParseStatus {
    Ok,
    Partial {
        error_nodes: u32,
        missing_nodes: u32,
    },
    Failed {
        reason: FailReason,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct UnitStats {
    pub bytes: u64,
    /// Number of newline characters plus one: the line count as tree-sitter sees it, so the end
    /// line of a range never exceeds it.
    pub lines: u32,
    pub parse_micros: u64,
}

// Not `Eq`: framework facts carry an f32 confidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ParsedUnit {
    pub ir_schema: u16,
    pub file: RepoPath,
    pub module_path: ModulePath,
    pub language: Language,
    pub dialect: Option<Dialect>,
    pub content_hash: ContentHash,
    pub analyzer: AnalyzerId,
    pub status: ParseStatus,
    /// Index equals `LocalId`; `symbols[0]` is always the module symbol.
    pub symbols: Vec<IrSymbol>,
    pub references: Vec<IrReference>,
    pub imports: Vec<IrImport>,
    pub exports: Vec<IrExport>,
    pub framework: Vec<IrFrameworkFact>,
    /// Sorted by symbol.
    pub facts: Vec<SymbolFacts>,
    pub diagnostics: Vec<ParseDiagnostic>,
    pub stats: UnitStats,
}

/// What an analyzer is given for one file.
#[derive(Debug, Clone)]
pub struct SourceInput<'a> {
    pub path: RepoPath,
    /// Computed by the caller (SID-001).
    pub module_path: ModulePath,
    pub bytes: &'a [u8],
    pub content_hash: ContentHash,
    pub is_generated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub enum AnonymousFnPolicy {
    /// Anonymous functions are not symbols; their references belong to the enclosing symbol.
    #[default]
    Attribute,
    /// Anonymous functions become `<anonymous>` function symbols (experiments only).
    Emit,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AnalyzerConfig {
    pub anonymous_functions: AnonymousFnPolicy,
    /// Larger files are `Failed { TooLarge }`. Default 1 MiB.
    pub max_file_bytes: u64,
    /// Default 2000 ms.
    pub parse_timeout_ms: u32,
    pub frameworks: FrameworkSignals,
    /// `None` runs every adapter whose `detect` passes.
    pub enabled_adapters: Option<BTreeSet<String>>,
    /// Compute per-symbol syntax facts. Default true.
    pub syntax_facts: bool,
    /// Decorator names (last segment) that produce `GuardDecorator` facts (TSA-006). `None` uses
    /// the default pattern `^(UseGuards|Roles?|Permissions?|Public|Auth\w*|Authorize\w*|Skip\w*Auth\w*)$`.
    pub guard_decorator_names: Option<Vec<String>>,
}

impl Default for AnalyzerConfig {
    fn default() -> Self {
        Self {
            anonymous_functions: AnonymousFnPolicy::Attribute,
            max_file_bytes: 1024 * 1024,
            parse_timeout_ms: 2000,
            frameworks: FrameworkSignals::default(),
            enabled_adapters: None,
            syntax_facts: true,
            guard_decorator_names: None,
        }
    }
}
