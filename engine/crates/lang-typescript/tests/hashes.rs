//! TSA-007 normalization rules, golden vectors and the reformat check over `ts-basic`. The
//! analyzer fills the hashes itself, so nothing here calls the hash pass explicitly.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::path::{Path, PathBuf};

use analysis_ir::{IrSymbol, ParsedUnit};
use common::{analyze, fixture};

fn symbol<'a>(unit: &'a ParsedUnit, name: &str) -> &'a IrSymbol {
    unit.symbols
        .iter()
        .find(|s| s.qualified_name.join(".") == name)
        .unwrap_or_else(|| panic!("no symbol `{name}`"))
}

fn body(path: &str, source: &str, name: &str) -> String {
    symbol(&analyze(path, source), name).body_hash.to_string()
}

fn sig(path: &str, source: &str, name: &str) -> String {
    symbol(&analyze(path, source), name)
        .signature_hash
        .to_string()
}

fn attr(path: &str, source: &str, name: &str) -> String {
    symbol(&analyze(path, source), name).attr_hash.to_string()
}

#[test]
fn analyzer_fills_every_hash() {
    let unit = analyze(
        "src/a.ts",
        "export function f(a: number): number { return a; }\n",
    );
    for s in &unit.symbols {
        assert!(!s.body_hash.is_zero(), "{} body", s.name);
        assert!(!s.signature_hash.is_zero(), "{} signature", s.name);
        assert!(!s.attr_hash.is_zero(), "{} attr", s.name);
    }
}

#[test]
fn reformat_does_not_change_body_hash() {
    let a = "function f(a: number) { const x = a + 1; return x * 2; }\n";
    let b = "function f(a: number) {\n    const x =\n        a + 1;\n\n    return x * 2;\n}\n";
    assert_eq!(body("src/a.ts", a, "f"), body("src/a.ts", b, "f"));
}

#[test]
fn comments_do_not_change_body_hash() {
    let a = "function f() { return 1; }\n";
    let b = "function f() {\n  // note\n  /* block */ return 1; // trailing\n}\n";
    assert_eq!(body("src/a.ts", a, "f"), body("src/a.ts", b, "f"));
}

#[test]
fn quote_style_normalized() {
    let a = "function f() { return 'a b'; }\n";
    let b = "function f() { return \"a b\"; }\n";
    assert_eq!(body("src/a.ts", a, "f"), body("src/a.ts", b, "f"));
}

#[test]
fn trailing_comma_and_semicolon_normalized() {
    let a = "function f() { return g(1, 2); }\n";
    let b = "function f() { return g(1, 2,) }\n";
    assert_eq!(body("src/a.ts", a, "f"), body("src/a.ts", b, "f"));
}

#[test]
fn crlf_vs_lf_same_hash() {
    let lf = "function f() {\n  return 1;\n}\n";
    let crlf = lf.replace('\n', "\r\n");
    assert_eq!(body("src/a.ts", lf, "f"), body("src/a.ts", &crlf, "f"));
}

#[test]
fn semantic_change_changes_body_hash() {
    let a = "function f() { return 1 + 2; }\n";
    let b = "function f() { return 1 - 2; }\n";
    assert_ne!(body("src/a.ts", a, "f"), body("src/a.ts", b, "f"));
}

#[test]
fn local_rename_changes_body_hash() {
    let a = "function f() { const total = 1; return total; }\n";
    let b = "function f() { const sum = 1; return sum; }\n";
    assert_ne!(body("src/a.ts", a, "f"), body("src/a.ts", b, "f"));
}

#[test]
fn param_type_change_changes_signature_not_body() {
    let a = "function f(a: number) { return a; }\n";
    let b = "function f(a: string) { return a; }\n";
    assert_ne!(sig("src/a.ts", a, "f"), sig("src/a.ts", b, "f"));
    assert_eq!(body("src/a.ts", a, "f"), body("src/a.ts", b, "f"));
}

#[test]
fn body_change_keeps_signature_hash() {
    let a = "function f(a: number): number { return a; }\n";
    let b = "function f(a: number): number { return a + 1; }\n";
    assert_eq!(sig("src/a.ts", a, "f"), sig("src/a.ts", b, "f"));
}

#[test]
fn rename_keeps_signature_hash() {
    let a = "function f(a: number): number { return a; }\n";
    let b = "function g(a: number): number { return a; }\n";
    assert_eq!(sig("src/a.ts", a, "f"), sig("src/a.ts", b, "g"));
}

#[test]
fn return_type_change_changes_signature_hash() {
    let a = "function f(a: number): number { return a; }\n";
    let b = "function f(a: number): string { return a; }\n";
    assert_ne!(sig("src/a.ts", a, "f"), sig("src/a.ts", b, "f"));
}

#[test]
fn default_value_expression_in_body_domain_not_signature() {
    let a = "function f(a = 1) { return a; }\n";
    let b = "function f(a = 2) { return a; }\n";
    assert_eq!(sig("src/a.ts", a, "f"), sig("src/a.ts", b, "f"));
}

#[test]
fn decorator_change_changes_attr_hash_only() {
    let a = "class C {\n  @Get('a')\n  run() { return 1; }\n}\n";
    let b = "class C {\n  @Get('b')\n  run() { return 1; }\n}\n";
    assert_ne!(attr("src/a.ts", a, "C.run"), attr("src/a.ts", b, "C.run"));
    assert_eq!(body("src/a.ts", a, "C.run"), body("src/a.ts", b, "C.run"));
    assert_eq!(sig("src/a.ts", a, "C.run"), sig("src/a.ts", b, "C.run"));
}

#[test]
fn class_hash_ignores_method_body_edits() {
    let a = "class C {\n  run() { return 1; }\n}\n";
    let b = "class C {\n  run() { return 2; }\n}\n";
    assert_eq!(body("src/a.ts", a, "C"), body("src/a.ts", b, "C"));
    assert_ne!(body("src/a.ts", a, "C.run"), body("src/a.ts", b, "C.run"));
}

#[test]
fn class_hash_changes_when_member_added() {
    let a = "class C {\n  run() { return 1; }\n}\n";
    let b = "class C {\n  run() { return 1; }\n  stop() { return 0; }\n}\n";
    assert_ne!(body("src/a.ts", a, "C"), body("src/a.ts", b, "C"));
}

#[test]
fn overload_signatures_in_signature_hash() {
    let a = "function f(a: number): number;\nfunction f(a: string): string;\nfunction f(a: any): any { return a; }\n";
    let b = "function f(a: number): number;\nfunction f(a: any): any { return a; }\n";
    assert_ne!(sig("src/a.ts", a, "f"), sig("src/a.ts", b, "f"));
}

#[test]
fn empty_body_hash_is_non_zero_and_stable() {
    let a = "export interface Shape { width: number }\n";
    let h1 = analyze("src/a.ts", a);
    let h2 = analyze("src/b.ts", a);
    for unit in [&h1, &h2] {
        assert!(!symbol(unit, "Shape.width").body_hash.is_zero());
    }
    assert_eq!(
        symbol(&h1, "Shape.width").body_hash,
        symbol(&h2, "Shape.width").body_hash
    );
}

#[test]
fn hash_golden_vectors() {
    // The body `{ return a + 1; }` is the token stream `{ return a + 1 }`: the semicolon before
    // the closing brace is normalized away.
    assert_eq!(
        body(
            "src/a.ts",
            "function f(a: number): number { return a + 1; }\n",
            "f"
        ),
        "619a71edbf0853aba1eae9aca21a9055"
    );
}

fn collect_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect_sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "ts") {
            out.push(path);
        }
    }
}

/// A fixed "formatter": every indent is doubled, every line gets trailing blanks and a blank
/// line follows it. Nothing the lexer keeps changes.
fn reformat(source: &str) -> String {
    let mut out = String::new();
    for line in source.lines() {
        let indent = line.len() - line.trim_start().len();
        out.push_str(&line[..indent]);
        out.push_str(line);
        out.push_str("   \n\n");
    }
    out
}

fn hashes(unit: &ParsedUnit) -> Vec<(String, String, String, String)> {
    unit.symbols
        .iter()
        .map(|s| {
            (
                format!("{:?}:{}", s.kind, s.qualified_name.join(".")),
                s.body_hash.to_string(),
                s.signature_hash.to_string(),
                s.attr_hash.to_string(),
            )
        })
        .collect()
}

#[test]
fn reformatting_every_ts_basic_file_keeps_all_hashes() {
    let root = fixture("ts-basic");
    let mut files = Vec::new();
    collect_sources(&root.join("src"), &mut files);
    assert!(files.len() > 20, "found {} files", files.len());
    for file in files {
        let rel = file
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let text = std::fs::read_to_string(&file).unwrap();
        let before = analyze(&rel, &text);
        let after = analyze(&rel, &reformat(&text));
        assert_eq!(hashes(&before), hashes(&after), "{rel}");
    }
}
