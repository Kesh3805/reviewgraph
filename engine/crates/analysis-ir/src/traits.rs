//! The analyzer protocol (ADR-006).

use review_core::language::Language;
use review_core::location::RepoPath;

use crate::framework::FrameworkSignals;
use crate::unit::{AnalyzerConfig, AnalyzerId, ParsedUnit, SourceInput};

/// Infrastructure failures only. Bad source never errors: it yields `ParseStatus::Partial` or
/// `Failed` with diagnostics (ADR-006: parse errors are tolerated).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AnalyzeError {
    /// The grammar could not be loaded, which is an ABI mismatch and therefore a deployment bug.
    #[error("grammar could not be loaded: {0}")]
    GrammarLoad(String),
    #[error("analyzer failed: {0}")]
    Internal(String),
}

/// Parses one file into IR.
pub trait LanguageAnalyzer: Send + Sync {
    fn id(&self) -> AnalyzerId;
    fn language(&self) -> Language;
    fn supports(&self, path: &RepoPath) -> bool;
    fn analyze(
        &self,
        input: &SourceInput<'_>,
        cfg: &AnalyzerConfig,
    ) -> Result<ParsedUnit, AnalyzeError>;
}

/// Extracts framework facts inside an analyzer. `Ctx` is the language-specific context (for
/// TypeScript, `lang_typescript::FrameworkCtx`), so this crate stays free of tree-sitter while
/// adapters keep typed access to the syntax tree. The IR output is framework-neutral.
pub trait FrameworkAdapter<Ctx>: Send + Sync {
    fn name(&self) -> &'static str;
    fn version(&self) -> u32;
    fn detect(&self, signals: &FrameworkSignals) -> bool;
    fn extract(&self, ctx: &mut Ctx);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolveKind {
    Import,
    TypeImport,
    Require,
    Dynamic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolutionMethod {
    Relative,
    TsPaths,
    BaseUrl,
    WorkspacePackage,
    PackageEntry,
    Index,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnresolvedReason {
    NotFound,
    OutsideRepository,
    Ambiguous,
    NonLiteral,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Resolution {
    File {
        path: RepoPath,
        method: ResolutionMethod,
        confidence: f32,
    },
    External {
        ecosystem: String,
        name: String,
        subpath: Option<String>,
        version_range: Option<String>,
    },
    Builtin {
        name: String,
    },
    Unresolved {
        reason: UnresolvedReason,
    },
}

/// Turns import specifiers into files, packages or built-ins.
pub trait ModuleResolver: Send + Sync {
    fn resolve(&self, from: &RepoPath, specifier: &str, kind: ResolveKind) -> Resolution;
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SemanticCapabilities {
    pub resolves_calls: bool,
    pub resolves_types: bool,
    pub languages: Vec<Language>,
}

/// A reference the syntactic linker could not settle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmbiguousRef {
    pub id: u64,
    pub file: RepoPath,
    /// 1-based line and column of the reference.
    pub line: u32,
    pub column: u32,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SemanticBudget {
    pub max_refs: u32,
    pub timeout_ms: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SemanticResolution {
    pub ref_id: u64,
    pub target_file: Option<RepoPath>,
    /// 1-based line and column of the declaration, when known.
    pub target_line: Option<u32>,
    pub target_column: Option<u32>,
    pub confidence: f32,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SemanticError {
    #[error("semantic provider is unavailable: {0}")]
    Unavailable(String),
    #[error("semantic provider timed out")]
    Timeout,
    #[error("semantic provider protocol error: {0}")]
    Protocol(String),
}

/// Optional type-checker enrichment (ADR-007). Never required for correctness.
pub trait SemanticProvider: Send + Sync {
    fn capabilities(&self) -> SemanticCapabilities;
    fn resolve_ambiguous(
        &self,
        refs: &[AmbiguousRef],
        budget: SemanticBudget,
    ) -> Result<Vec<SemanticResolution>, SemanticError>;
}
