#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::sync::Arc;

use analysis_ir::diagnostic::{DiagCode, DiagSeverity, ParseDiagnostic, MAX_DIAGNOSTIC_CHARS};
use analysis_ir::symbol::{MAX_EXPR_DEPTH, MAX_EXPR_ELEMENTS, MAX_EXPR_STRING_BYTES};
use analysis_ir::*;
use review_core::language::{Dialect, Language};
use review_core::location::{ContentHash, Position, RepoPath, SourceRange};
use review_core::symbol::{Hash128, ModulePath, SymbolKind};
use review_core::version::AnalyzerVersion;

fn range(l1: u32, c1: u32, l2: u32, c2: u32) -> SourceRange {
    SourceRange::new(
        Position::new(l1, c1).unwrap(),
        Position::new(l2, c2).unwrap(),
    )
    .unwrap()
}

fn symbol(id: u32, kind: SymbolKind, qn: &[&str], parent: Option<u32>, r: SourceRange) -> IrSymbol {
    let mut s = IrSymbol::new(
        LocalId(id),
        kind,
        qn.last().copied().unwrap_or("x"),
        qn.iter().map(|s| (*s).to_owned()).collect(),
        r,
    );
    s.parent = parent.map(LocalId);
    s
}

fn sample_unit() -> ParsedUnit {
    let file = RepoPath::new("src/auth/auth.service.ts").unwrap();
    let mut class = symbol(
        1,
        SymbolKind::Class,
        &["AuthService"],
        Some(0),
        range(3, 1, 20, 2),
    );
    class.modifiers = Modifiers::EXPORTED | Modifiers::ABSTRACT;
    class.decorators.push(IrDecorator {
        name: "Injectable".to_owned(),
        args: vec![IrExpr::Object(vec![(
            "scope".to_owned(),
            IrExpr::Num("1".to_owned()),
        )])],
        range: range(2, 1, 2, 13),
    });
    class.heritage.implements.push("Auth".to_owned());
    class
        .attrs
        .insert("binding".to_owned(), AttrValue::Str("class".to_owned()));
    class.attrs.insert(
        "weight".to_owned(),
        AttrValue::Float(symbol::FloatBits::new(0.5)),
    );
    let mut method = symbol(
        2,
        SymbolKind::Method,
        &["AuthService", "authorize"],
        Some(1),
        range(5, 3, 10, 4),
    );
    method.params.push(IrParam {
        name: "user".to_owned(),
        type_text: Some("User".to_owned()),
        optional: false,
        rest: false,
        decorators: Vec::new(),
        property: Some(ParamProperty {
            visibility: Visibility::Private,
            readonly: true,
        }),
    });
    method.body_hash = Hash128::of("rg.body.v1", b"body");
    method.const_value = Some(ConstValue::Bool(true));
    ParsedUnit {
        ir_schema: IR_SCHEMA_VERSION,
        file: file.clone(),
        module_path: ModulePath::of(&file),
        language: Language::Typescript,
        dialect: Some(Dialect::Ts),
        content_hash: ContentHash::of(b"content"),
        analyzer: AnalyzerId {
            name: "lang-typescript".to_owned(),
            version: AnalyzerVersion::new(0, 1, 0),
        },
        status: ParseStatus::Partial {
            error_nodes: 1,
            missing_nodes: 0,
        },
        symbols: vec![
            symbol(
                0,
                SymbolKind::Module,
                &["__module__"],
                None,
                range(1, 1, 25, 1),
            ),
            class,
            method,
        ],
        references: vec![IrReference {
            from: LocalId(2),
            kind: RefKind::Call,
            name: "check".to_owned(),
            receiver: ReceiverHint::ThisField {
                field: "guard".to_owned(),
                declared_type: Some("Guard".to_owned()),
            },
            import_binding: Some(BindingRef {
                import: 0,
                binding: 0,
            }),
            range: range(6, 5, 6, 20),
            arg_count: 1,
            in_test_block: false,
            attrs: BTreeMap::new(),
        }],
        imports: vec![IrImport {
            specifier: "./guard".to_owned(),
            kind: ImportKind::Esm,
            type_only: false,
            bindings: vec![ImportBinding {
                local: "Guard".to_owned(),
                imported: Imported::Named("Guard".to_owned()),
                type_only: false,
            }],
            range: range(1, 1, 1, 30),
        }],
        exports: vec![IrExport::Local {
            symbol: LocalId(1),
            exported_as: "AuthService".to_owned(),
            type_only: false,
        }],
        framework: vec![IrFrameworkFact {
            adapter: "nestjs".to_owned(),
            kind: FrameworkFactKind::Custom("x".to_owned()),
            symbol: Some(LocalId(1)),
            attrs: BTreeMap::new(),
            range: range(2, 1, 2, 13),
            confidence: 0.9,
        }],
        facts: vec![SymbolFacts {
            symbol: LocalId(2),
            facts: vec![SyntaxFact {
                kind: FactKind::Call,
                key: "call:check".to_owned(),
                range: range(6, 5, 6, 20),
                detail: BTreeMap::new(),
            }],
        }],
        diagnostics: vec![ParseDiagnostic::new(
            DiagSeverity::Error,
            DiagCode::SyntaxError,
            "syntax error",
            Some(range(12, 1, 12, 5)),
        )
        .unwrap()],
        stats: UnitStats {
            bytes: 400,
            lines: 25,
            parse_micros: 12,
        },
    }
}

#[test]
fn sample_unit_is_valid() {
    validate(&sample_unit()).unwrap();
}

#[test]
fn parsed_unit_json_roundtrip() {
    let unit = sample_unit();
    let json = serde_json::to_string(&unit).unwrap();
    assert_eq!(serde_json::from_str::<ParsedUnit>(&json).unwrap(), unit);
    // Deterministic: serializing twice gives identical text.
    assert_eq!(json, serde_json::to_string(&unit).unwrap());
}

#[test]
fn parsed_unit_bincode_roundtrip() {
    let unit = sample_unit();
    let bytes = bincode::serialize(&unit).unwrap();
    assert_eq!(bincode::deserialize::<ParsedUnit>(&bytes).unwrap(), unit);
    assert_eq!(bytes, bincode::serialize(&unit).unwrap());
}

#[test]
fn symbol_kind_id_strings_are_stable() {
    let ids: Vec<&str> = SymbolKindList::all()
        .iter()
        .map(|k| k.as_id_str())
        .collect();
    assert_eq!(
        ids,
        [
            "module",
            "namespace",
            "class",
            "interface",
            "enum",
            "enum_member",
            "type_alias",
            "function",
            "method",
            "get",
            "set",
            "constructor",
            "property",
            "field",
            "variable",
            "constant",
            "parameter"
        ]
    );
}

struct SymbolKindList;

impl SymbolKindList {
    fn all() -> [SymbolKind; 17] {
        SymbolKind::ALL
    }
}

#[test]
fn ref_kind_strings_stable() {
    let strings: Vec<&str> = RefKind::ALL.iter().map(|k| k.as_str()).collect();
    assert_eq!(
        strings,
        [
            "call",
            "new",
            "type_ref",
            "extends",
            "implements",
            "decorator",
            "di_injection",
            "framework_ref",
            "value_read",
            "jsx_element"
        ]
    );
    let facts: Vec<&str> = FactKind::ALL.iter().map(|k| k.as_str()).collect();
    assert_eq!(facts.len(), 14);
    assert_eq!(facts[0], "call");
}

#[test]
fn modifiers_report_their_names() {
    let m = Modifiers::EXPORTED | Modifiers::ASYNC;
    assert_eq!(m.names(), vec!["exported", "async"]);
    assert!(m.contains(Modifiers::ASYNC));
    assert!(!m.contains(Modifiers::STATIC));
}

#[test]
fn validate_rejects_child_outside_parent() {
    let mut unit = sample_unit();
    unit.symbols[2].range = range(30, 1, 31, 1);
    unit.stats.lines = 40;
    let violations = validate(&unit).unwrap_err();
    assert!(violations.iter().any(|v| matches!(
        v,
        IrViolation::ChildOutsideParent {
            symbol: 2,
            parent: 1
        }
    )));
}

#[test]
fn validate_rejects_dangling_local_id() {
    let mut unit = sample_unit();
    unit.references[0].from = LocalId(99);
    unit.exports.push(IrExport::Local {
        symbol: LocalId(77),
        exported_as: "x".to_owned(),
        type_only: false,
    });
    let violations = validate(&unit).unwrap_err();
    assert_eq!(
        violations
            .iter()
            .filter(|v| matches!(v, IrViolation::DanglingLocalId { .. }))
            .count(),
        2,
        "every violation is reported, not just the first: {violations:?}"
    );
}

#[test]
fn validate_rejects_duplicate_identity() {
    let mut unit = sample_unit();
    let mut dup = unit.symbols[2].clone();
    dup.local_id = LocalId(3);
    unit.symbols.push(dup);
    let violations = validate(&unit).unwrap_err();
    assert!(violations.contains(&IrViolation::DuplicateIdentity {
        first: 2,
        second: 3
    }));
    // A different ordinal makes it unique again.
    unit.symbols[3].ordinal = 1;
    validate(&unit).unwrap();
}

#[test]
fn validate_requires_module_symbol_first() {
    let mut unit = sample_unit();
    unit.symbols[0].kind = SymbolKind::Namespace;
    assert!(validate(&unit)
        .unwrap_err()
        .contains(&IrViolation::FirstSymbolNotModule));
    let mut empty = sample_unit();
    empty.symbols.clear();
    empty.references.clear();
    empty.exports.clear();
    empty.facts.clear();
    empty.framework.clear();
    assert!(validate(&empty)
        .unwrap_err()
        .contains(&IrViolation::FirstSymbolNotModule));
}

#[test]
fn validate_checks_parent_order_facts_and_ranges() {
    let mut unit = sample_unit();
    unit.symbols[1].parent = Some(LocalId(2));
    assert!(validate(&unit)
        .unwrap_err()
        .iter()
        .any(|v| matches!(v, IrViolation::ParentDoesNotPrecede { .. })));

    let mut unit = sample_unit();
    unit.facts.push(SymbolFacts {
        symbol: LocalId(1),
        facts: Vec::new(),
    });
    assert!(validate(&unit)
        .unwrap_err()
        .contains(&IrViolation::FactsNotSorted));

    let mut unit = sample_unit();
    unit.stats.lines = 3;
    assert!(validate(&unit)
        .unwrap_err()
        .iter()
        .any(|v| matches!(v, IrViolation::RangeBeyondFile { .. })));
}

#[test]
fn ir_expr_depth_and_size_bounded() {
    // Depth: nest arrays 20 deep.
    let mut expr = IrExpr::Num("1".to_owned());
    for _ in 0..20 {
        expr = IrExpr::Array(vec![expr]);
    }
    assert!(expr.depth() > MAX_EXPR_DEPTH);
    let bounded = expr.bounded();
    assert!(
        bounded.depth() <= MAX_EXPR_DEPTH + 1,
        "depth {}",
        bounded.depth()
    );

    // Elements.
    let wide = IrExpr::Array((0..200).map(|i| IrExpr::Num(i.to_string())).collect());
    match wide.bounded() {
        IrExpr::Array(items) => {
            assert_eq!(items.len(), MAX_EXPR_ELEMENTS);
            assert_eq!(items.last().unwrap(), &IrExpr::truncated());
        }
        other => panic!("{other:?}"),
    }

    // Strings.
    match IrExpr::Str("é".repeat(2000)).bounded() {
        IrExpr::Str(s) => assert!(s.len() <= MAX_EXPR_STRING_BYTES),
        other => panic!("{other:?}"),
    }
    match IrExpr::Other("x".repeat(500)).bounded() {
        IrExpr::Other(s) => assert_eq!(s.chars().count(), 120),
        other => panic!("{other:?}"),
    }
}

struct FakeAnalyzer {
    name: &'static str,
    ext: &'static str,
}

impl LanguageAnalyzer for FakeAnalyzer {
    fn id(&self) -> AnalyzerId {
        AnalyzerId {
            name: self.name.to_owned(),
            version: AnalyzerVersion::new(0, 1, 0),
        }
    }

    fn language(&self) -> Language {
        Language::Typescript
    }

    fn supports(&self, path: &RepoPath) -> bool {
        path.extension() == Some(self.ext)
    }

    fn analyze(
        &self,
        _input: &SourceInput<'_>,
        _cfg: &AnalyzerConfig,
    ) -> Result<ParsedUnit, AnalyzeError> {
        Ok(sample_unit())
    }
}

#[test]
fn registry_picks_first_supporting_analyzer() {
    let mut registry = AnalyzerRegistry::new();
    registry.register(Arc::new(FakeAnalyzer {
        name: "first",
        ext: "ts",
    }));
    registry.register(Arc::new(FakeAnalyzer {
        name: "second",
        ext: "ts",
    }));
    registry.register(Arc::new(FakeAnalyzer {
        name: "js",
        ext: "js",
    }));
    let ts = RepoPath::new("a/b.ts").unwrap();
    let js = RepoPath::new("a/b.js").unwrap();
    let md = RepoPath::new("a/b.md").unwrap();
    assert_eq!(registry.for_path(&ts).unwrap().id().name, "first");
    assert_eq!(registry.for_path(&js).unwrap().id().name, "js");
    assert!(registry.for_path(&md).is_none());
    assert_eq!(registry.len(), 3);
}

#[test]
fn diagnostic_message_constructor_rejects_long_text() {
    let ok = ParseDiagnostic::new(DiagSeverity::Info, DiagCode::DepthLimit, "deep", None).unwrap();
    assert_eq!(ok.message, "deep");
    let counted = ParseDiagnostic::with_count(
        DiagSeverity::Warning,
        DiagCode::SyntaxError,
        "more diagnostics:",
        12,
        None,
    )
    .unwrap();
    assert_eq!(counted.message, "more diagnostics: 12");
    // A static string longer than the cap is refused.
    const LONG: &str = "0123456789012345678901234567890123456789012345678901234567890123456789\
                        0123456789012345678901234567890123456789012345678901234567890123456789\
                        0123456789012345678901234567890123456789012345678901234567890123456789";
    assert!(LONG.chars().count() > MAX_DIAGNOSTIC_CHARS);
    assert!(ParseDiagnostic::new(DiagSeverity::Error, DiagCode::SyntaxError, LONG, None).is_err());
}

#[test]
fn framework_signals_lookup() {
    let mut signals = FrameworkSignals::default();
    signals.frameworks.insert(
        "nestjs".to_owned(),
        FrameworkPresence {
            scope_dirs: vec![String::new()],
            major: Some(10),
            confidence: 1.0,
        },
    );
    assert!(signals.has("nestjs"));
    assert_eq!(signals.major("nestjs"), Some(10));
    assert!(!signals.has("express"));
}

#[test]
fn default_config_matches_the_spec() {
    let cfg = AnalyzerConfig::default();
    assert_eq!(cfg.max_file_bytes, 1024 * 1024);
    assert_eq!(cfg.parse_timeout_ms, 2000);
    assert!(cfg.syntax_facts);
    assert_eq!(cfg.anonymous_functions, AnonymousFnPolicy::Attribute);
}
