//! Shared helpers for the incremental integration tests: analyze a file with the TypeScript
//! analyzer, fill the hash fields and look symbols up by qualified name.

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

pub mod lineage_harness;

use analysis_ir::identity::symbol_id_of;
use analysis_ir::validate;
use analysis_ir::{AnalyzerConfig, IrSymbol, LanguageAnalyzer, ParsedUnit, SourceInput};
use lang_typescript::hashing::assign_hashes;
use lang_typescript::text::Source;
use lang_typescript::TypeScriptAnalyzer;
use review_core::ids::SymbolId;
use review_core::language::Language;
use review_core::location::{ContentHash, RepoPath};
use review_core::symbol::{ModulePath, SymbolKind};
use review_core::symbol_id::module_path_for;

/// Analyzes one file and fills the hash fields, which is what a caller does today; TSA-007 will
/// move the hash pass into the analyzer.
pub fn analyze(path: &str, source: &str) -> ParsedUnit {
    analyze_with(path, source, &AnalyzerConfig::default())
}

/// Analyzes one file with an explicit configuration.
pub fn analyze_with(path: &str, source: &str, cfg: &AnalyzerConfig) -> ParsedUnit {
    let path = RepoPath::new(path).unwrap();
    let module_path: ModulePath = module_path_for(&path, Language::Typescript);
    let bytes = source.as_bytes();
    let input = SourceInput {
        module_path,
        content_hash: ContentHash::of(bytes),
        path,
        bytes,
        is_generated: false,
    };
    let mut unit = TypeScriptAnalyzer::new().analyze(&input, cfg).unwrap();
    assign_hashes(&mut unit.symbols, &Source::new(bytes));
    assert!(
        validate(&unit).is_ok(),
        "the analyzer produced an invalid unit: {:?}",
        validate(&unit)
    );
    unit
}

/// The canonical id of the symbol with this qualified name and kind.
pub fn id_of(unit: &ParsedUnit, qualified_name: &str, kind: SymbolKind) -> SymbolId {
    let symbol = symbol_of(unit, qualified_name);
    assert_eq!(symbol.kind, kind, "{qualified_name} has another kind");
    symbol_id_of(unit, symbol.local_id.0).expect("every symbol has an id")
}

/// The symbol with this qualified name.
pub fn symbol_of<'a>(unit: &'a ParsedUnit, qualified_name: &str) -> &'a IrSymbol {
    unit.symbols
        .iter()
        .find(|symbol| symbol.qualified_name.join(".") == qualified_name)
        .unwrap_or_else(|| {
            panic!(
                "no symbol `{qualified_name}` in {:?}",
                unit.symbols
                    .iter()
                    .map(|symbol| symbol.qualified_name.join("."))
                    .collect::<Vec<_>>()
            )
        })
}

/// Every symbol with this qualified name, in source order (for ordinal collisions).
pub fn symbols_of<'a>(unit: &'a ParsedUnit, qualified_name: &str) -> Vec<&'a IrSymbol> {
    unit.symbols
        .iter()
        .filter(|symbol| symbol.qualified_name.join(".") == qualified_name)
        .collect()
}
