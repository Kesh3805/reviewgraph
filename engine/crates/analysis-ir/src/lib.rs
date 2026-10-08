//! `analysis-ir` crate. See docs/architecture/target-architecture.md §2 and ADR-006.
//!
//! The language-neutral intermediate representation every analyzer produces, plus the analyzer
//! protocol traits. Pure types: no parsing, no I/O, no dependency on tree-sitter, `repository`,
//! `sqlx` or `tokio`.

pub mod diagnostic;
pub mod error;
pub mod facts;
pub mod framework;
pub mod identity;
pub mod module;
pub mod reference;
pub mod registry;
pub mod symbol;
pub mod traits;
pub mod unit;
pub mod validate;

pub use diagnostic::{DiagCode, DiagSeverity, ParseDiagnostic};
pub use error::{Error, Result};
pub use facts::{FactKind, SymbolFacts, SyntaxFact};
pub use framework::{FrameworkFactKind, FrameworkPresence, FrameworkSignals, IrFrameworkFact};
pub use module::{ImportBinding, ImportKind, Imported, IrExport, IrImport};
pub use reference::{BindingRef, IrReference, ReceiverHint, RefKind};
pub use registry::AnalyzerRegistry;
pub use symbol::{
    AttrValue, ConstValue, Heritage, IrDecorator, IrExpr, IrParam, IrSymbol, LocalId, Modifiers,
    ParamProperty, QualifiedName, ShingleSet, Visibility,
};
pub use traits::{
    AnalyzeError, FrameworkAdapter, LanguageAnalyzer, ModuleResolver, Resolution, ResolutionMethod,
    ResolveKind, SemanticProvider, UnresolvedReason,
};
pub use unit::{
    AnalyzerConfig, AnalyzerId, AnonymousFnPolicy, FailReason, ParseStatus, ParsedUnit,
    SourceInput, UnitStats, IR_SCHEMA_VERSION,
};
pub use validate::{validate, IrViolation};
