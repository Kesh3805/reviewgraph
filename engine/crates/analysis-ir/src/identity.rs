//! Identity view over a [`ParsedUnit`] (ADR-005, SID-001).
//!
//! The canonical string grammar lives in `review-core::symbol_id`; this module is the only place
//! that knows how an IR symbol maps onto it, so every analyzer produces identical ids for the same
//! declarations.

use review_core::ids::{SymbolId, SymbolKey};
use review_core::symbol::{ModulePath, SymbolKind};
use review_core::symbol_id::{SymbolIdError, SymbolIdParts};

use crate::symbol::IrSymbol;
use crate::unit::ParsedUnit;

/// The language tag of one unit (`ts`, `js`, ...), which is the first component of every id.
pub fn lang_of(unit: &ParsedUnit) -> &'static str {
    unit.language.id_prefix()
}

/// The parts of one symbol's id, straight from the IR.
pub fn symbol_id_parts(unit: &ParsedUnit, symbol: &IrSymbol) -> SymbolIdParts {
    SymbolIdParts::new(
        lang_of(unit),
        unit.module_path.clone(),
        symbol.qualified_name.clone(),
        symbol.kind,
    )
    .with_ordinal(symbol.ordinal)
}

/// The canonical id of `local_id` inside `unit`.
pub fn symbol_id_of(unit: &ParsedUnit, local_id: u32) -> Option<SymbolId> {
    let symbol = unit.symbols.get(local_id as usize)?;
    let parts = symbol_id_parts(unit, symbol);
    Some(parts.format().unwrap_or_else(|_| parts.format_lossy()))
}

/// Every symbol id of one unit, indexed by `LocalId`.
pub fn symbol_ids(unit: &ParsedUnit) -> Vec<SymbolId> {
    unit.symbols
        .iter()
        .map(|symbol| {
            let parts = symbol_id_parts(unit, symbol);
            parts.format().unwrap_or_else(|_| parts.format_lossy())
        })
        .collect()
}

/// The id of the module symbol (`ts:src/a#__module__/module`), which is `symbols[0]`.
pub fn module_id_of(unit: &ParsedUnit) -> SymbolId {
    let parts = SymbolIdParts::module(lang_of(unit), unit.module_path.clone());
    parts.format().unwrap_or_else(|_| parts.format_lossy())
}

/// The storage key of a symbol id (ADR-005).
pub fn symbol_key_of(unit: &ParsedUnit, local_id: u32) -> Option<SymbolKey> {
    symbol_id_of(unit, local_id).map(|id| SymbolKey::of(&id))
}

/// `language + module_path` of a unit, the namespace one unit's symbols live in.
pub fn unit_namespace(unit: &ParsedUnit) -> (String, ModulePath) {
    (lang_of(unit).to_owned(), unit.module_path.clone())
}

/// Looks a symbol up by its canonical id.
pub fn symbol_by_id<'a>(unit: &'a ParsedUnit, id: &SymbolId) -> Option<&'a IrSymbol> {
    unit.symbols.iter().find(|symbol| {
        symbol_id_parts(unit, symbol)
            .format()
            .map(|candidate| &candidate == id)
            .unwrap_or(false)
    })
}

/// Whether the id could be produced from these parts. Convenience for callers that want to reject
/// ids without parsing them.
pub fn is_canonical(parts: &SymbolIdParts) -> Result<(), SymbolIdError> {
    parts.format().map(|_| ())
}

/// The kind of a symbol as it appears in an id (`method`, `get`, ...).
pub fn kind_str(kind: SymbolKind) -> &'static str {
    kind.as_id_str()
}

#[cfg(test)]
mod tests {
    use super::*;
    use review_core::location::{ContentHash, Position, RepoPath, SourceRange};
    use review_core::version::AnalyzerVersion;

    use crate::symbol::LocalId;
    use crate::unit::{AnalyzerId, ParseStatus, UnitStats, IR_SCHEMA_VERSION};

    fn range() -> SourceRange {
        SourceRange {
            start: Position { line: 1, column: 0 },
            end: Position { line: 2, column: 1 },
        }
    }

    fn unit() -> ParsedUnit {
        let path = RepoPath::new("src/auth/auth.service.ts").unwrap();
        let module = IrSymbol::new(
            LocalId(0),
            SymbolKind::Module,
            "auth.service",
            vec!["__module__".to_owned()],
            range(),
        );
        let class = IrSymbol::new(
            LocalId(1),
            SymbolKind::Class,
            "AuthService",
            vec!["AuthService".to_owned()],
            range(),
        );
        let method = IrSymbol::new(
            LocalId(2),
            SymbolKind::Method,
            "authorize",
            vec!["AuthService".to_owned(), "authorize".to_owned()],
            range(),
        );
        ParsedUnit {
            ir_schema: IR_SCHEMA_VERSION,
            file: path.clone(),
            module_path: ModulePath::of(&path),
            language: review_core::language::Language::Typescript,
            dialect: None,
            content_hash: ContentHash::of(b"x"),
            analyzer: AnalyzerId {
                name: "test".to_owned(),
                version: AnalyzerVersion::new(0, 1, 0),
            },
            status: ParseStatus::Ok,
            symbols: vec![module, class, method],
            references: vec![],
            imports: vec![],
            exports: vec![],
            framework: vec![],
            facts: vec![],
            diagnostics: vec![],
            stats: UnitStats::default(),
        }
    }

    #[test]
    fn symbol_id_of_assembles_from_ir_symbol() {
        let unit = unit();
        assert_eq!(
            symbol_id_of(&unit, 2).unwrap().as_str(),
            "ts:src/auth/auth.service#AuthService.authorize/method"
        );
        assert_eq!(
            module_id_of(&unit).as_str(),
            "ts:src/auth/auth.service#__module__/module"
        );
        assert!(symbol_id_of(&unit, 9).is_none());
        let ids = symbol_ids(&unit);
        assert_eq!(ids.len(), 3);
        assert_eq!(
            ids[1].as_str(),
            "ts:src/auth/auth.service#AuthService/class"
        );
        assert_eq!(
            symbol_by_id(&unit, &ids[1]).map(|s| s.name.as_str()),
            Some("AuthService")
        );
        assert_eq!(
            symbol_key_of(&unit, 2).map(|k| k.to_string()),
            Some(SymbolKey::of(&ids[2]).to_string())
        );
        assert_eq!(
            unit_namespace(&unit).0,
            "ts",
            "the language tag is the id prefix"
        );
    }

    #[test]
    fn ordinal_and_kind_survive() {
        let mut unit = unit();
        unit.symbols[2].ordinal = 2;
        assert_eq!(
            symbol_id_of(&unit, 2).unwrap().as_str(),
            "ts:src/auth/auth.service#AuthService.authorize/method~2"
        );
        assert_eq!(kind_str(SymbolKind::Getter), "get");
        assert!(is_canonical(&symbol_id_parts(&unit, &unit.symbols[1])).is_ok());
        assert_eq!(unit.status, ParseStatus::Ok);
    }
}
