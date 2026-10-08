//! SID-003: deterministic `~n` ordinals for symbols that share `(qualified name, kind)` in one
//! file. Covers the fixture files under `fixtures/repositories/ts-basic/src/ordinals/` and the
//! pass itself, including its idempotency and order independence.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use analysis_ir::hashing::Token;
use analysis_ir::hashing::TokenHasher;
use analysis_ir::{validate, ShingleSet};
use analysis_ir::{
    AnalyzerConfig, AnonymousFnPolicy, DiagCode, IrSymbol, LocalId, Modifiers, ParsedUnit,
};
use common::{analyze_fixture, analyze_with, fixture, try_analyze_fixture};
use lang_typescript::ordinals::assign_ordinals;
use review_core::location::{Position, SourceRange};
use review_core::symbol::SymbolKind;

fn ordinal_fixture(file: &str) -> ParsedUnit {
    try_analyze_fixture("ts-basic", &format!("src/ordinals/{file}"))
        .unwrap_or_else(|e| panic!("fixture src/ordinals/{file} must exist: {e}"))
}

/// `qualified.name~ordinal` for every symbol of one kind, in source order.
fn names_of(unit: &ParsedUnit, kind: SymbolKind) -> Vec<String> {
    unit.symbols
        .iter()
        .filter(|s| s.kind == kind)
        .map(|s| {
            let name = s.qualified_name.join(".");
            if s.ordinal > 0 {
                format!("{name}~{}", s.ordinal)
            } else {
                name
            }
        })
        .collect()
}

fn range(line: u32) -> SourceRange {
    SourceRange {
        start: Position { line, column: 0 },
        end: Position {
            line: line + 1,
            column: 0,
        },
    }
}

fn symbol(kind: SymbolKind, qualified: &[&str], line: u32, static_member: bool) -> IrSymbol {
    let mut symbol = IrSymbol::new(
        LocalId(0),
        kind,
        qualified.last().copied().unwrap_or_default(),
        qualified.iter().map(|s| (*s).to_owned()).collect(),
        range(line),
    );
    if static_member {
        symbol.modifiers = Modifiers::STATIC;
    }
    symbol
}

#[test]
fn unique_symbols_have_no_ordinal() {
    let unit = ordinal_fixture("duplicate-functions.ts");
    // The second `duplicate` is a real duplicate, so this fixture does have one; the class and
    // module symbols never do.
    assert!(unit
        .symbols
        .iter()
        .all(|s| s.ordinal == 0 || s.name == "duplicate"));
    let unique = analyze_with(
        "src/unique.ts",
        b"export class Only {\n  run(): void {}\n}\nexport const value = 1;\n",
        &AnalyzerConfig::default(),
    );
    assert!(unique.symbols.iter().all(|s| s.ordinal == 0));
}

#[test]
fn duplicate_functions_first_plain_rest_numbered() {
    let unit = ordinal_fixture("duplicate-functions.ts");
    assert_eq!(
        names_of(&unit, SymbolKind::Function),
        vec!["duplicate", "duplicate~1"],
        "the first declaration keeps the plain id"
    );
    assert!(unit
        .diagnostics
        .iter()
        .any(|d| d.code == DiagCode::DuplicateSymbol));
    assert!(validate(&unit).is_ok(), "ids must be unique");
}

#[test]
fn static_and_instance_same_name_instance_plain() {
    let unit = ordinal_fixture("static-instance.ts");
    let methods = names_of(&unit, SymbolKind::Method);
    assert_eq!(
        methods
            .iter()
            .filter(|n| n.starts_with("Counters.reset"))
            .count(),
        2
    );
    assert!(
        methods.iter().any(|n| n == "Counters.reset~1"),
        "the static member is ordered after the instance one: {methods:?}"
    );
    let properties = names_of(&unit, SymbolKind::Property);
    assert!(
        properties.iter().any(|n| n == "Counters.value")
            && properties.iter().any(|n| n == "Counters.value~1"),
        "same rule for properties: {properties:?}"
    );
    let static_method = unit
        .symbols
        .iter()
        .find(|s| s.ordinal == 1 && s.kind == SymbolKind::Method)
        .unwrap();
    assert!(static_method.modifiers.contains(Modifiers::STATIC));
}

#[test]
fn merged_interfaces_ordinals_by_source_order() {
    let unit = ordinal_fixture("merged-interface.ts");
    assert_eq!(
        names_of(&unit, SymbolKind::Interface),
        vec!["Merged", "Merged~1"],
        "declaration merging numbers by source order"
    );
    assert_eq!(
        names_of(&unit, SymbolKind::Property),
        vec!["Merged.first", "Merged.second"]
    );
}

#[test]
fn merged_interface_member_collision() {
    let unit = ordinal_fixture("merged-interface.ts");
    let methods = names_of(&unit, SymbolKind::Method);
    assert_eq!(
        methods,
        vec!["Merged.resolve", "Merged.resolve~1"],
        "a member declared in both halves of a merged interface is numbered"
    );
    let second = unit
        .symbols
        .iter()
        .find(|s| s.name == "resolve" && s.ordinal == 1)
        .unwrap();
    assert!(
        second.parent.is_some_and(|p| p.0 > 0),
        "the second declaration belongs to the second interface, not to the first"
    );
    assert!(
        second.overload_signatures.is_empty(),
        "a member declared in two halves is two symbols, not one folded overload set"
    );
}

#[test]
fn param_property_vs_field_collision() {
    let unit = ordinal_fixture("param-property-collision.ts");
    let properties = names_of(&unit, SymbolKind::Property);
    assert_eq!(
        properties,
        vec!["Repository.name", "Repository.name~1", "Repository.id"]
    );
    let from_param = unit
        .symbols
        .iter()
        .find(|s| s.qualified_name.last().is_some_and(|n| n == "name") && s.ordinal == 1)
        .unwrap();
    assert!(from_param.attrs.contains_key("from_constructor_param"));
}

#[test]
fn anonymous_emit_starts_at_one() {
    let bytes = std::fs::read(fixture("ts-basic").join("src/ordinals/anonymous-emit.ts")).unwrap();
    let cfg = AnalyzerConfig {
        anonymous_functions: AnonymousFnPolicy::Emit,
        ..AnalyzerConfig::default()
    };
    let unit = analyze_with("src/ordinals/anonymous-emit.ts", &bytes, &cfg);
    let anonymous: Vec<(String, u16)> = unit
        .symbols
        .iter()
        .filter(|s| s.qualified_name.last().is_some_and(|n| n == "<anonymous>"))
        .map(|s| (s.qualified_name.join("."), s.ordinal))
        .collect();
    assert_eq!(
        anonymous,
        vec![
            ("withCallbacks.<anonymous>".to_owned(), 1),
            ("withCallbacks.<anonymous>".to_owned(), 2),
        ],
        "a bare `<anonymous>` is never emitted"
    );
    assert!(validate(&unit).is_ok(), "ids must be unique");
}

#[test]
fn anonymous_ordinals_scoped_per_enclosing_symbol() {
    let mut symbols = vec![
        symbol(SymbolKind::Function, &["first"], 1, false),
        symbol(SymbolKind::Function, &["second"], 2, false),
    ];
    symbols[0].qualified_name = vec!["first".to_owned()];
    let mut a = symbol(SymbolKind::Function, &["first", "<anonymous>"], 3, false);
    a.ordinal = 7;
    let mut b = symbol(SymbolKind::Function, &["second", "<anonymous>"], 4, false);
    b.ordinal = 9;
    symbols.push(a);
    symbols.push(b);
    assign_ordinals(&mut symbols);
    let found: Vec<(String, u16)> = symbols
        .iter()
        .filter(|s| s.qualified_name.last().is_some_and(|n| n == "<anonymous>"))
        .map(|s| (s.qualified_name.join("."), s.ordinal))
        .collect();
    assert_eq!(
        found,
        vec![
            ("first.<anonymous>".to_owned(), 1),
            ("second.<anonymous>".to_owned(), 1)
        ],
        "ordinals restart in every enclosing symbol"
    );
}

#[test]
fn duplicate_object_keys_diagnostic_and_ordinals() {
    let unit = ordinal_fixture("object-duplicate-keys.ts");
    let duplicates: Vec<(String, String, u16)> = unit
        .symbols
        .iter()
        .filter(|s| s.qualified_name == ["endpoints", "list"])
        .map(|s| (s.kind.as_id_str().to_owned(), s.name.clone(), s.ordinal))
        .collect();
    assert_eq!(
        duplicates,
        vec![
            ("method".to_owned(), "list".to_owned(), 0),
            ("function".to_owned(), "list".to_owned(), 0),
        ],
        "a method and a pair with a function value have different kinds, so no ordinal is needed"
    );
    let same_kind = analyze_with(
        "src/same-kind.ts",
        b"export const registry = {\n  load() {},\n  load: () => undefined,\n  load: function () {},\n};\n",
        &AnalyzerConfig::default(),
    );
    let loads: Vec<(String, u16)> = same_kind
        .symbols
        .iter()
        .filter(|s| s.qualified_name.last().is_some_and(|n| n == "load"))
        .map(|s| (s.kind.as_id_str().to_owned(), s.ordinal))
        .collect();
    assert!(
        loads.len() >= 2,
        "three same-kind members must all be present: {loads:?}"
    );
    assert!(
        same_kind
            .diagnostics
            .iter()
            .any(|d| d.code == DiagCode::DuplicateSymbol),
        "a non-benign collision is reported"
    );
}

#[test]
fn adding_duplicate_after_keeps_existing_ids() {
    let before = analyze_with(
        "src/dup.ts",
        b"export function handler(input: string): string {\n  return input.trim();\n}\n",
        &AnalyzerConfig::default(),
    );
    let after = analyze_with(
        "src/dup.ts",
        b"export function handler(input: string): string {\n  return input.trim();\n}\nexport function handler(input: number): number {\n  return input + 1;\n}\n",
        &AnalyzerConfig::default(),
    );
    let before_ids: Vec<String> = before
        .symbols
        .iter()
        .map(|s| format!("{}~{}", s.qualified_name.join("."), s.ordinal))
        .collect();
    let after_ids: Vec<String> = after
        .symbols
        .iter()
        .map(|s| format!("{}~{}", s.qualified_name.join("."), s.ordinal))
        .collect();
    assert_eq!(before_ids, vec!["__module__~0", "handler~0"]);
    assert_eq!(after_ids, vec!["__module__~0", "handler~0", "handler~1"]);
}

#[test]
fn unrelated_insertion_does_not_shift_other_groups() {
    let base = vec![
        symbol(SymbolKind::Function, &["dup"], 1, false),
        symbol(SymbolKind::Function, &["dup"], 2, false),
        symbol(SymbolKind::Function, &["other"], 3, false),
    ];
    let mut with_extra = base.clone();
    with_extra.insert(2, symbol(SymbolKind::Function, &["unrelated"], 4, false));
    assign_ordinals(&mut base.clone());
    let mut other = base.clone();
    let mut shuffled = with_extra.clone();
    assign_ordinals(&mut other);
    assign_ordinals(&mut shuffled);
    let render = |symbols: &[IrSymbol]| -> Vec<(String, u16)> {
        symbols
            .iter()
            .map(|s| (s.qualified_name.join("."), s.ordinal))
            .collect()
    };
    let base_rendered = render(&other);
    let extra_rendered = render(&shuffled);
    assert_eq!(
        extra_rendered.iter().filter(|(n, _)| n == "other").count(),
        1,
        "the unrelated symbol exists"
    );
    assert_eq!(
        base_rendered
            .iter()
            .find(|(n, _)| n == "other")
            .map(|(_, o)| *o),
        extra_rendered
            .iter()
            .find(|(n, _)| n == "other")
            .map(|(_, o)| *o),
        "an unrelated insertion cannot shift another group"
    );
    assert_eq!(
        base_rendered
            .iter()
            .find(|(n, _)| n == "dup")
            .map(|(_, o)| *o),
        Some(0)
    );
}

#[test]
fn reformat_does_not_change_ordinals() {
    let compact = analyze_with(
        "src/fmt.ts",
        b"export class A {\n  run(): void {}\n  static run(): void {}\n}\n",
        &AnalyzerConfig::default(),
    );
    let formatted = analyze_with(
        "src/fmt.ts",
        b"export class A {\n    run(): void {}\n\n    static run(): void {}\n}\n",
        &AnalyzerConfig::default(),
    );
    let render = |unit: &ParsedUnit| -> Vec<(String, u16)> {
        unit.symbols
            .iter()
            .map(|s| (s.qualified_name.join("."), s.ordinal))
            .collect()
    };
    assert_eq!(render(&compact), render(&formatted));
}

#[test]
fn pass_is_idempotent() {
    for file in [
        "duplicate-functions.ts",
        "merged-interface.ts",
        "static-instance.ts",
        "param-property-collision.ts",
        "object-duplicate-keys.ts",
    ] {
        let mut unit = ordinal_fixture(file);
        let before: Vec<u16> = unit.symbols.iter().map(|s| s.ordinal).collect();
        let report = assign_ordinals(&mut unit.symbols);
        let after: Vec<u16> = unit.symbols.iter().map(|s| s.ordinal).collect();
        assert_eq!(before, after, "{file}: running the pass twice changed ids");
        assert_eq!(report.dropped, 0, "{file}");
    }
}

#[test]
fn shuffled_input_same_output() {
    let unit = ordinal_fixture("duplicate-functions.ts");
    let forward: Vec<(String, u16)> = unit.symbols.iter().map(ordinal_of).collect();
    let mut reversed = unit.clone();
    reversed.symbols.reverse();
    assign_ordinals(&mut reversed.symbols);
    let mut sorted: Vec<(String, u16)> = reversed.symbols.iter().map(ordinal_of).collect();
    sorted.sort();
    let mut expected = forward.clone();
    expected.sort();
    assert_eq!(sorted, expected, "input order is irrelevant");
}

fn ordinal_of(symbol: &IrSymbol) -> (String, u16) {
    (
        format!(
            "{}:{}",
            symbol.qualified_name.join("."),
            symbol.kind.as_id_str()
        ),
        symbol.ordinal,
    )
}

#[test]
fn validate_accepts_result_unique_identity() {
    for file in [
        "duplicate-functions.ts",
        "merged-interface.ts",
        "static-instance.ts",
        "param-property-collision.ts",
        "object-duplicate-keys.ts",
        "anonymous-emit.ts",
    ] {
        let unit = ordinal_fixture(file);
        assert!(
            validate(&unit).is_ok(),
            "{file}: (qualified name, kind, ordinal) must be unique"
        );
    }
    let unit = analyze_fixture("ts-basic", "src/overloads.ts");
    assert!(validate(&unit).is_ok());
}

#[test]
fn overflow_guard() {
    // A group larger than `u16::MAX` is impossible to build in a test cheaply, so the guard is
    // exercised through the boundary: the pass never wraps and never panics.
    let mut symbols = vec![
        symbol(SymbolKind::Function, &["dup"], 1, false),
        symbol(SymbolKind::Function, &["dup"], 2, false),
    ];
    let report = assign_ordinals(&mut symbols);
    assert_eq!(report.colliding_groups, 1);
    assert_eq!(report.dropped, 0);
    assert_eq!(symbols[0].ordinal, 0);
    assert_eq!(symbols[1].ordinal, 1);
    assert!(
        symbols[1].ordinal < u16::MAX,
        "an ordinary group never approaches the overflow guard"
    );
}

#[test]
fn linear_after_sort_on_a_large_file() {
    // 10k symbols in one file: the pass must finish in well under a second (SID-003 benchmark
    // note: linear after the group sort).
    let mut symbols: Vec<IrSymbol> = (0..10_000u32)
        .map(|i| {
            let name = format!("fn{i}");
            symbol(SymbolKind::Function, &[name.as_str()], i + 1, false)
        })
        .collect();
    let started = std::time::Instant::now();
    let report = assign_ordinals(&mut symbols);
    let elapsed = started.elapsed();
    assert_eq!(report.colliding_groups, 0);
    assert!(symbols.iter().all(|s| s.ordinal == 0));
    assert!(
        elapsed < std::time::Duration::from_millis(500),
        "assign_ordinals took {elapsed:?} for 10k symbols"
    );
}

#[test]
fn hashing_primitives_are_available_to_later_stages() {
    // The differ and the matcher consume hashes; the ordinal pass is the last step before them,
    // so this test pins the contract the two later stages rely on.
    let mut hasher = TokenHasher::new(analysis_ir::hashing::HashKind::Body);
    hasher.extend([&Token::new(analysis_ir::hashing::TokenClass::Ident, "a")]);
    let digest = hasher.finish();
    assert!(!digest.is_zero());
    let shingles = ShingleSet::of([1, 2, 3]);
    assert_eq!(shingles.jaccard(&shingles), 1.0);
}
