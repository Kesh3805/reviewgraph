//! `lang-typescript` crate. See docs/architecture/target-architecture.md §2.
//!
//! The TypeScript/JavaScript analyzer: tree-sitter parsing (ADR-007), IR extraction
//! (ADR-006), framework adapters and the module resolver. This is the only crate that knows
//! TypeScript.

pub mod analyzer;
pub mod diagnostics;
pub mod error;
pub mod hashing;
pub mod kinds;
pub mod naming;
pub mod ordinals;
pub mod parser_pool;
pub mod text;
pub mod tokens;
pub mod visit;

use review_core::version::AnalyzerVersion;

pub use analyzer::{dialect_of, language_of, TypeScriptAnalyzer};
pub use error::{Error, Result};

/// Registered name of the analyzer in `AnalyzerId`.
pub const ANALYZER_NAME: &str = "lang-typescript";

/// Minor: new facts extracted. Major: identity or hash rule change (forces a re-parse of
/// TS/JS files only, ADR-015).
pub const ANALYZER_VERSION: AnalyzerVersion = AnalyzerVersion::new(0, 1, 0);

/// Exact versions of the parser stack, part of the repository fingerprint (INIT-012).
/// `tests/parser_versions.rs` checks them against `engine/Cargo.lock`.
pub const PARSER_VERSIONS: &[(&str, &str)] = &[
    ("tree-sitter", "0.25.10"),
    ("tree-sitter-typescript", "0.23.2"),
];
