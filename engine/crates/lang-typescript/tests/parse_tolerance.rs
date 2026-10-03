#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use analysis_ir::diagnostic::DiagCode;
use analysis_ir::{AnalyzerConfig, FailReason, ParseStatus};
use common::{analyze, analyze_fixture, analyze_with};
use proptest::prelude::*;
use review_core::language::{Dialect, Language};
use review_core::symbol::SymbolKind;

fn is_clean(unit: &analysis_ir::ParsedUnit) -> bool {
    unit.status == ParseStatus::Ok && unit.diagnostics.is_empty()
}

#[test]
fn syntax_error_yields_partial_with_diagnostics() {
    let unit = analyze_fixture("ts-edge", "src/syntax-error-mid-class.ts");
    match unit.status {
        ParseStatus::Partial {
            error_nodes,
            missing_nodes,
        } => assert!(error_nodes + missing_nodes >= 1),
        other => panic!("{other:?}"),
    }
    assert!(!unit.diagnostics.is_empty());
    assert!(unit.symbols[0].has_errors, "the module symbol is marked");
    assert_eq!(unit.symbols[0].kind, SymbolKind::Module);
}

#[test]
fn missing_node_reported() {
    let unit = analyze("a.ts", "function f() {\n");
    assert!(
        unit.diagnostics
            .iter()
            .any(|d| d.code == DiagCode::MissingNode && d.message == "missing }"),
        "{:?}",
        unit.diagnostics
    );
    assert!(matches!(
        unit.status,
        ParseStatus::Partial {
            missing_nodes: 1..,
            ..
        }
    ));
}

#[test]
fn oversize_file_failed_too_large_module_symbol_only() {
    let cfg = AnalyzerConfig {
        max_file_bytes: 16,
        ..AnalyzerConfig::default()
    };
    let unit = analyze_with("a.ts", b"export const a = 1;\nexport const b = 2;\n", &cfg);
    assert_eq!(
        unit.status,
        ParseStatus::Failed {
            reason: FailReason::TooLarge
        }
    );
    assert_eq!(unit.symbols.len(), 1);
    assert!(unit
        .diagnostics
        .iter()
        .any(|d| d.code == DiagCode::FileTooLarge));
}

#[test]
fn binary_input_failed() {
    let unit = analyze_with("a.ts", b"abc\0def", &AnalyzerConfig::default());
    assert_eq!(
        unit.status,
        ParseStatus::Failed {
            reason: FailReason::Binary
        }
    );
    assert_eq!(unit.symbols.len(), 1);
}

fn huge_source() -> String {
    let line = "export function f(a: number, b: string): number { return a + b.length; }\n";
    line.repeat(3 * 1024 * 1024 / line.len() + 1)
}

#[test]
fn timeout_returns_failed_and_parser_is_reusable() {
    let big = huge_source();
    let cfg = AnalyzerConfig {
        max_file_bytes: 16 * 1024 * 1024,
        parse_timeout_ms: 1,
        ..AnalyzerConfig::default()
    };
    let before = analyze("small.ts", "export const a = 1;\n");
    let timed_out = analyze_with("huge.ts", big.as_bytes(), &cfg);
    assert_eq!(
        timed_out.status,
        ParseStatus::Failed {
            reason: FailReason::Timeout
        }
    );
    assert!(timed_out
        .diagnostics
        .iter()
        .any(|d| d.code == DiagCode::ParseTimeout));
    // The parser of this thread must not resume the cancelled parse.
    let after = analyze("small.ts", "export const a = 1;\n");
    assert!(is_clean(&after));
    assert_eq!(before.symbols, after.symbols);
    assert_eq!(before.status, after.status);
}

#[test]
fn tsx_grammar_for_tsx_and_jsx() {
    assert!(is_clean(&analyze_fixture("ts-edge", "src/component.tsx")));
    assert!(is_clean(&analyze_fixture("ts-edge", "src/legacy.jsx")));
    // The plain TypeScript grammar rejects JSX.
    let ts = analyze("a.ts", "export const x = <div>hi</div>;\n");
    assert!(matches!(ts.status, ParseStatus::Partial { .. }));
}

#[test]
fn ts_grammar_for_ts_and_dts() {
    // An angle-bracket cast only parses with the TS grammar.
    let cast = analyze("a.ts", "export const x = <string>value;\n");
    assert!(is_clean(&cast), "{:?}", cast.diagnostics);
    let tsx = analyze("a.tsx", "export const x = <string>value;\n");
    assert!(matches!(tsx.status, ParseStatus::Partial { .. }));
    let dts = analyze(
        "types/global.d.ts",
        "declare const version: string;\nexport {};\n",
    );
    assert!(is_clean(&dts));
    assert_eq!(dts.dialect, Some(Dialect::Dts));
    assert_eq!(dts.language, Language::Typescript);
}

#[test]
fn js_uses_tsx_grammar_and_js_tag() {
    let unit = analyze("a.js", "export const el = <b>x</b>;\n");
    assert!(is_clean(&unit), "{:?}", unit.diagnostics);
    assert_eq!(unit.language, Language::Javascript);
    assert_eq!(unit.dialect, Some(Dialect::Js));
    let mjs = analyze_fixture("ts-edge", "src/module.mjs");
    assert_eq!(mjs.dialect, Some(Dialect::Mjs));
    assert!(is_clean(&mjs));
    let cjs = analyze_fixture("ts-edge", "src/common.cjs");
    assert_eq!(cjs.dialect, Some(Dialect::Cjs));
    assert!(is_clean(&cjs));
}

#[test]
fn bom_offsets_are_file_offsets() {
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(b"let = ;\n");
    let unit = analyze_with("a.ts", &bytes, &AnalyzerConfig::default());
    let range = unit
        .diagnostics
        .iter()
        .find_map(|d| d.range)
        .expect("a ranged diagnostic");
    assert_eq!(range.start.line, 1);
    assert!(
        range.start.column >= 3,
        "column {} ignores the BOM",
        range.start.column
    );
    // Second line columns are unaffected by the BOM.
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(b"let a = 1;\nlet = ;\n");
    let unit = analyze_with("a.ts", &bytes, &AnalyzerConfig::default());
    let range = unit.diagnostics.iter().find_map(|d| d.range).unwrap();
    assert_eq!(range.start.line, 2);
    assert!(range.start.column < 8);
}

#[test]
fn crlf_lines_counted_correctly() {
    let unit = analyze_with(
        "a.ts",
        b"let a = 1;\r\nlet b = 2;\r\nlet = ;",
        &AnalyzerConfig::default(),
    );
    assert_eq!(unit.stats.lines, 3);
    let range = unit.diagnostics.iter().find_map(|d| d.range).unwrap();
    assert_eq!(range.start.line, 3);
}

#[test]
fn lossy_utf8_diagnostic_once() {
    let mut bytes = b"export const s = \"caf".to_vec();
    bytes.push(0xE9);
    bytes.extend_from_slice(b"\";\nexport const t = \"x");
    bytes.push(0xFF);
    bytes.extend_from_slice(b"\";\n");
    let unit = analyze_with("latin1.js", &bytes, &AnalyzerConfig::default());
    let lossy = unit
        .diagnostics
        .iter()
        .filter(|d| d.message == "invalid utf-8 decoded lossily")
        .count();
    assert_eq!(lossy, 1);
    assert_eq!(unit.status, ParseStatus::Ok);
}

#[test]
fn deep_nesting_no_stack_overflow() {
    let unit = analyze_fixture("ts-edge", "src/deep-nesting.ts");
    assert_eq!(unit.symbols[0].kind, SymbolKind::Module);
    assert!(matches!(
        unit.status,
        ParseStatus::Ok | ParseStatus::Partial { .. }
    ));
    // 5,000 levels, far beyond the fixture, still must not overflow.
    let deeper = format!("{}x{}", "(".repeat(5000), ")".repeat(5000));
    let unit = analyze("a.ts", &deeper);
    assert_eq!(unit.symbols[0].kind, SymbolKind::Module);
}

#[test]
fn diagnostics_capped_at_50() {
    let unit = analyze("a.ts", &"let = ;\n".repeat(200));
    assert_eq!(unit.diagnostics.len(), 51, "{}", unit.diagnostics.len());
    let last = unit.diagnostics.last().unwrap();
    assert!(
        last.message.starts_with("more diagnostics:"),
        "{}",
        last.message
    );
    match unit.status {
        ParseStatus::Partial {
            error_nodes,
            missing_nodes,
        } => {
            assert!(error_nodes + missing_nodes > 50)
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn diagnostics_never_contain_source_text() {
    let secret = "SUPERSECRETVALUE";
    let unit = analyze(
        "a.ts",
        &format!("let {secret} = ;\nfunction ({secret} {{\n"),
    );
    for d in &unit.diagnostics {
        assert!(!d.message.contains(secret), "{}", d.message);
    }
}

#[test]
fn unterminated_template_is_partial() {
    let unit = analyze_fixture("ts-edge", "src/unterminated-template.ts");
    assert!(matches!(unit.status, ParseStatus::Partial { .. }));
}

#[test]
fn same_bytes_give_identical_units() {
    let a = analyze_fixture("ts-edge", "src/component.tsx");
    let b = analyze_fixture("ts-edge", "src/component.tsx");
    let strip = |mut u: analysis_ir::ParsedUnit| {
        u.stats.parse_micros = 0;
        u
    };
    assert_eq!(strip(a), strip(b));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn random_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..2048)) {
        for path in ["a.ts", "a.tsx", "a.js"] {
            let unit = analyze_with(path, &bytes, &AnalyzerConfig::default());
            prop_assert_eq!(unit.symbols[0].kind, SymbolKind::Module);
        }
    }
}
