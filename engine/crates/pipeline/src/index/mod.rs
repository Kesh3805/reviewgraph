//! Repository indexing (phase 7): configuration, versions, progress, parse tolerance, the parse
//! cache and the full index pipeline.

use std::collections::BTreeMap;
use std::sync::Arc;

use analysis_ir::AnalyzerRegistry;

pub mod config;
pub mod error;
pub mod full;
pub mod parse;
pub mod parse_cache;
pub mod progress;
pub mod resolver;
pub mod source;
pub mod tolerance;
pub mod versions;

pub use config::IndexConfig;
pub use error::IndexError;
pub use full::{FullIndexOutcome, FullIndexRequest, FullIndexer, IndexStats};
pub use parse::{FailReason, FileOutcome, ParsedFile, SkipReason};
pub use parse_cache::{
    CacheLookup, InMemoryParseCache, MissReason, ParseCache, ParseCacheError, ParseCacheKey,
};
pub use progress::{IndexPhase, NoopProgress, ProgressSink, ProgressThrottle};
pub use resolver::FileSetResolver;
pub use source::{SourceFile, SourceSpec};
pub use tolerance::{FileOutcomeKind, OutcomeCounts, ToleranceExceeded, TolerancePolicy};
pub use versions::{compute_versions, ConfigComponents, VersionInputs};

/// The production analyzer set (TypeScript/JavaScript) and its parser stack versions, as
/// recorded in the snapshot fingerprint.
pub fn typescript_registry() -> (AnalyzerRegistry, BTreeMap<String, String>) {
    let mut registry = AnalyzerRegistry::new();
    registry.register(Arc::new(lang_typescript::TypeScriptAnalyzer::new()));
    let parsers = lang_typescript::PARSER_VERSIONS
        .iter()
        .map(|(name, version)| ((*name).to_owned(), (*version).to_owned()))
        .collect();
    (registry, parsers)
}
