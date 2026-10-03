#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use analysis_ir::{AnalyzerConfig, LanguageAnalyzer, ParsedUnit, SourceInput};
use lang_typescript::TypeScriptAnalyzer;
use review_core::location::{ContentHash, RepoPath};
use review_core::symbol::ModulePath;

pub fn analyze_with(path: &str, bytes: &[u8], cfg: &AnalyzerConfig) -> ParsedUnit {
    let path = RepoPath::new(path).unwrap();
    let input = SourceInput {
        module_path: ModulePath::of(&path),
        content_hash: ContentHash::of(bytes),
        path,
        bytes,
        is_generated: false,
    };
    let unit = TypeScriptAnalyzer::new().analyze(&input, cfg).unwrap();
    analysis_ir::validate(&unit).unwrap();
    unit
}

pub fn analyze(path: &str, source: &str) -> ParsedUnit {
    analyze_with(path, source.as_bytes(), &AnalyzerConfig::default())
}

pub fn fixture(name: &str) -> std::path::PathBuf {
    review_test_support::fixture_repo(name)
}

pub fn analyze_fixture(repo: &str, rel: &str) -> ParsedUnit {
    let bytes = std::fs::read(fixture(repo).join(rel)).unwrap();
    analyze_with(rel, &bytes, &AnalyzerConfig::default())
}
