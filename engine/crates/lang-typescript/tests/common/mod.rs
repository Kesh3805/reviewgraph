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

/// Like [`analyze_fixture`] but reports a missing or unreadable file instead of panicking, so a
/// test can assert that a fixture exists and what it contains.
pub fn try_analyze_fixture(repo: &str, rel: &str) -> std::result::Result<ParsedUnit, String> {
    let path = fixture(repo).join(rel);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    Ok(analyze_with(rel, &bytes, &AnalyzerConfig::default()))
}
