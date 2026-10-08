//! SID-001: the identity view over `ParsedUnit` — ids, keys and namespaces derived from IR.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use analysis_ir::identity::{
    lang_of, module_id_of, symbol_by_id, symbol_id_of, symbol_id_parts, symbol_ids, symbol_key_of,
    unit_namespace,
};
use analysis_ir::{LocalId, ParsedUnit};
use review_core::ids::SymbolId;
use review_core::language::{Dialect, Language};
use review_core::location::{ContentHash, Position, RepoPath, SourceRange};
use review_core::symbol::{ModulePath, SymbolKind};
use review_core::symbol_id::module_path_for;
use review_core::version::AnalyzerVersion;

fn range(start_line: u32, end_line: u32) -> SourceRange {
    SourceRange {
        start: Position {
            line: start_line,
            column: 0,
        },
        end: Position {
            line: end_line,
            column: 0,
        },
    }
}

fn symbol(
    local_id: u32,
    kind: SymbolKind,
    name: &str,
    qualified: &[&str],
    ordinal: u16,
) -> analysis_ir::IrSymbol {
    let mut symbol = analysis_ir::IrSymbol::new(
        LocalId(local_id),
        kind,
        name,
        qualified.iter().map(|s| (*s).to_owned()).collect(),
        range(local_id + 1, local_id + 2),
    );
    symbol.ordinal = ordinal;
    symbol
}

fn unit(language: Language, module: &str) -> ParsedUnit {
    let file = RepoPath::new(format!("{module}.ts")).unwrap();
    ParsedUnit {
        ir_schema: analysis_ir::IR_SCHEMA_VERSION,
        file: file.clone(),
        module_path: ModulePath::of(&file),
        language,
        dialect: Some(Dialect::Ts),
        content_hash: ContentHash::of(module.as_bytes()),
        analyzer: analysis_ir::AnalyzerId {
            name: "test".to_owned(),
            version: AnalyzerVersion::new(0, 1, 0),
        },
        status: analysis_ir::ParseStatus::Ok,
        symbols: vec![
            symbol(0, SymbolKind::Module, "auth.service", &["__module__"], 0),
            symbol(1, SymbolKind::Class, "AuthService", &["AuthService"], 0),
            symbol(
                2,
                SymbolKind::Method,
                "authorize",
                &["AuthService", "authorize"],
                0,
            ),
            symbol(
                3,
                SymbolKind::Method,
                "authorize",
                &["AuthService", "authorize"],
                1,
            ),
        ],
        references: vec![],
        imports: vec![],
        exports: vec![],
        framework: vec![],
        facts: vec![],
        diagnostics: vec![],
        stats: Default::default(),
    }
}

#[test]
fn identity_symbol_ids_match_the_canonical_grammar() {
    let unit = unit(Language::Typescript, "src/auth/auth.service");
    assert_eq!(lang_of(&unit), "ts");
    assert_eq!(
        unit_namespace(&unit),
        ("ts".to_owned(), ModulePath::of(&unit.file))
    );
    assert_eq!(
        symbol_id_of(&unit, 2).unwrap().as_str(),
        "ts:src/auth/auth.service#AuthService.authorize/method"
    );
    assert_eq!(
        symbol_id_of(&unit, 3).unwrap().as_str(),
        "ts:src/auth/auth.service#AuthService.authorize/method~1",
        "the ordinal keeps colliding names unique"
    );
    assert_eq!(
        module_id_of(&unit).as_str(),
        "ts:src/auth/auth.service#__module__/module"
    );
    assert!(symbol_id_of(&unit, 4).is_none());
    assert_eq!(symbol_ids(&unit).len(), unit.symbols.len());
}

#[test]
fn identity_keys_follow_the_ids() {
    let unit = unit(Language::Typescript, "src/a");
    let id = symbol_id_of(&unit, 1).unwrap();
    assert_eq!(
        symbol_key_of(&unit, 1).map(|k| k.to_string()),
        Some(review_core::ids::SymbolKey::of(&id).to_string())
    );
    assert_eq!(id.as_str(), "ts:src/a#AuthService/class");
    assert_eq!(
        symbol_by_id(&unit, &id).map(|s| s.kind),
        Some(SymbolKind::Class)
    );
    let missing = SymbolId::parse("ts:src/other#Nope/class").unwrap();
    assert!(symbol_by_id(&unit, &missing).is_none());
}

#[test]
fn identity_language_tag_follows_the_unit() {
    let js = unit(Language::Javascript, "src/legacy");
    assert_eq!(lang_of(&js), "js");
    assert_eq!(
        symbol_id_of(&js, 1).unwrap().as_str(),
        "js:src/legacy#AuthService/class"
    );
    // The same declarations in a TypeScript file are different symbols.
    let ts = unit(Language::Typescript, "src/legacy");
    assert_ne!(symbol_id_of(&js, 1), symbol_id_of(&ts, 1));
}

#[test]
fn identity_parts_match_the_parser() {
    let unit = unit(Language::Typescript, "src/a");
    let parts = symbol_id_parts(&unit, &unit.symbols[3]);
    assert_eq!(parts.kind, SymbolKind::Method);
    assert_eq!(parts.ordinal, 1);
    assert_eq!(parts.qualified_name, vec!["AuthService", "authorize"]);
    let from_parts = parts.format().unwrap();
    assert_eq!(from_parts, symbol_id_of(&unit, 3).unwrap());
    assert_eq!(SymbolId::parse(from_parts.as_str()).unwrap(), from_parts);
}

#[test]
fn identity_module_path_is_the_units_namespace() {
    // The id namespace comes from the unit's `module_path`, which the caller computes with
    // `review_core::symbol_id::module_path_for`, not from the file name at id time.
    let unit = unit(Language::Typescript, "src/a");
    let parts = symbol_id_parts(&unit, &unit.symbols[1]);
    let renamed_path = RepoPath::new("src/renamed.ts").unwrap();
    let mut renamed = unit.clone();
    renamed.file = renamed_path.clone();
    renamed.module_path = module_path_for(&renamed_path, Language::Typescript);
    assert_eq!(parts.module_path, unit.module_path);
    assert_eq!(
        symbol_id_of(&renamed, 1).unwrap().as_str(),
        "ts:src/renamed#AuthService/class"
    );
    assert_ne!(symbol_id_of(&unit, 1), symbol_id_of(&renamed, 1));
}
