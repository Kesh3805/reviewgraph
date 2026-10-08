//! The per-file hash pass that the symbol differ (SID-004) and the rename matcher (SID-005) read.
//!
//! Scope note: this covers the field-filling pass and the token normalization it needs. TSA-007
//! additionally owns calling the pass from the analyzer, the `docs/languages/typescript.md`
//! normalization section and the golden hash vectors, none of which are done here.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use analysis_ir::{AnalyzerConfig, IrSymbol, ParsedUnit};
use common::{analyze, analyze_with};
use lang_typescript::hashing::{assign_hashes, is_container};
use lang_typescript::text::Source;
use review_core::symbol::{Hash128, SymbolKind};

fn hashed(path: &str, source: &str) -> ParsedUnit {
    let mut unit = analyze(path, source);
    let text = Source::new(source.as_bytes());
    assign_hashes(&mut unit.symbols, &text);
    unit
}

fn symbol<'a>(unit: &'a ParsedUnit, name: &str) -> &'a IrSymbol {
    unit.symbols
        .iter()
        .find(|s| s.qualified_name.join(".") == name)
        .unwrap_or_else(|| {
            panic!(
                "no symbol `{name}` in {:?}",
                unit.symbols
                    .iter()
                    .map(|s| s.qualified_name.join("."))
                    .collect::<Vec<_>>()
            )
        })
}

#[test]
fn every_symbol_gets_a_body_signature_and_attribute_hash() {
    let unit = hashed(
        "src/svc.ts",
        "export class Service {\n  run(input: string): number {\n    return input.length;\n  }\n}\n",
    );
    for s in &unit.symbols {
        assert!(!s.body_hash.is_zero(), "{s:?} has no body hash");
        assert!(!s.signature_hash.is_zero(), "{s:?} has no signature hash");
        assert!(!s.attr_hash.is_zero(), "{s:?} has no attribute hash");
    }
    assert!(symbol(&unit, "Service").body_token_count > 0);
    assert_eq!(
        symbol(&unit, "Service.run").body_token_count,
        6,
        "the statement semicolon is normalized away"
    );
}

#[test]
fn reformat_and_comments_do_not_change_hashes() {
    let a = hashed(
        "src/a.ts",
        "export function total(items: number[]): number {\n  return items.reduce((sum, i) => sum + i, 0);\n}\n",
    );
    let b = hashed(
        "src/a.ts",
        "// a comment that does not matter\n\nexport function total( items : number[] ) : number {\n  /* block */\n  return items.reduce(\n    (sum, i) => sum + i,\n    0,\n  );\n}\n",
    );
    assert_eq!(symbol(&a, "total").body_hash, symbol(&b, "total").body_hash);
    assert_eq!(
        symbol(&a, "total").signature_hash,
        symbol(&b, "total").signature_hash
    );
}

#[test]
fn crlf_and_lf_hash_the_same() {
    let lf = hashed("src/a.ts", "export const a = 1;\nexport const b = 2;\n");
    let crlf = hashed("src/a.ts", "export const a = 1;\r\nexport const b = 2;\r\n");
    for name in ["a", "b"] {
        assert_eq!(symbol(&lf, name).body_hash, symbol(&crlf, name).body_hash);
    }
}

#[test]
fn a_behaviour_change_changes_the_body_hash() {
    let a = hashed(
        "src/a.ts",
        "export function total(items: number[]): number {\n  return items.reduce((sum, i) => sum + i, 0);\n}\n",
    );
    let b = hashed(
        "src/a.ts",
        "export function total(items: number[]): number {\n  return items.length;\n}\n",
    );
    assert_ne!(symbol(&a, "total").body_hash, symbol(&b, "total").body_hash);
    assert_eq!(
        symbol(&a, "total").signature_hash,
        symbol(&b, "total").signature_hash,
        "a body edit is not a signature change"
    );
}

#[test]
fn a_rename_keeps_the_signature_hash_and_changes_the_body_hash() {
    let a = hashed(
        "src/a.ts",
        "export function total(items: number[]): number {\n  return items.length;\n}\n",
    );
    let b = hashed(
        "src/a.ts",
        "export function count(items: number[]): number {\n  return items.length;\n}\n",
    );
    assert_eq!(
        symbol(&a, "total").signature_hash,
        symbol(&b, "count").signature_hash
    );
    assert_eq!(
        symbol(&a, "total").body_hash,
        symbol(&b, "count").body_hash,
        "a name lives outside the body, so a pure rename keeps the body hash: this is what lets \\
         the matcher pair a renamed symbol by body"
    );
}

#[test]
fn a_parameter_type_change_is_a_signature_change() {
    let a = hashed(
        "src/a.ts",
        "export function run(input: string): void {\n  input.trim();\n}\n",
    );
    let b = hashed(
        "src/a.ts",
        "export function run(input: string | null): void {\n  input?.trim();\n}\n",
    );
    assert_ne!(
        symbol(&a, "run").signature_hash,
        symbol(&b, "run").signature_hash
    );
}

#[test]
fn container_hash_ignores_member_bodies_and_tracks_members() {
    let base = "export class Service {\n  run(): number {\n    return 1;\n  }\n}\n";
    let body_edit = "export class Service {\n  run(): number {\n    return 2;\n  }\n}\n";
    let member_added =
        "export class Service {\n  run(): number {\n    return 1;\n  }\n\n  stop(): void {}\n}\n";
    let member_renamed = "export class Service {\n  execute(): number {\n    return 1;\n  }\n}\n";
    let original = hashed("src/a.ts", base);
    assert!(is_container(SymbolKind::Class));
    assert_eq!(
        symbol(&original, "Service").body_hash,
        symbol(&hashed("src/a.ts", body_edit), "Service").body_hash,
        "editing a member body must not change the container hash"
    );
    assert_ne!(
        symbol(&original, "Service").body_hash,
        symbol(&hashed("src/a.ts", member_added), "Service").body_hash,
        "adding a member changes the container hash"
    );
    assert_ne!(
        symbol(&original, "Service").body_hash,
        symbol(&hashed("src/a.ts", member_renamed), "Service").body_hash,
        "renaming a member changes the container hash"
    );
}

#[test]
fn decorator_and_export_changes_are_attribute_changes() {
    let plain = hashed("src/a.ts", "export class Service {\n  run(): void {}\n}\n");
    let decorated = hashed(
        "src/a.ts",
        "@Injectable()\nexport class Service {\n  run(): void {}\n}\n",
    );
    assert_ne!(
        symbol(&plain, "Service").attr_hash,
        symbol(&decorated, "Service").attr_hash
    );
    assert_eq!(
        symbol(&plain, "Service").body_hash,
        symbol(&decorated, "Service").body_hash
    );
    assert_eq!(
        symbol(&plain, "Service").signature_hash,
        symbol(&decorated, "Service").signature_hash
    );
    let not_exported = hashed("src/a.ts", "class Service {\n  run(): void {}\n}\n");
    assert_ne!(
        symbol(&plain, "Service").attr_hash,
        symbol(&not_exported, "Service").attr_hash,
        "exported is an attribute"
    );
}

#[test]
fn shingles_and_token_counts_describe_the_body() {
    let unit = hashed(
        "src/a.ts",
        "export class Service {\n  run(): number {\n    const total = 1 + 2 + 3;\n    return total;\n  }\n}\n",
    );
    let run = symbol(&unit, "Service.run");
    assert!(run.body_token_count >= 10, "{}", run.body_token_count);
    assert!(!run.body_shingles.is_empty());
    let same = hashed(
        "src/a.ts",
        "export class Service {\n  run(): number {\n    const total = 1 + 2 + 3;\n    return total;\n  }\n}\n",
    );
    assert_eq!(
        run.body_shingles,
        symbol(&same, "Service.run").body_shingles
    );
    assert_eq!(run.body_shingles.jaccard(&run.body_shingles), 1.0);
}

#[test]
fn hashes_are_stable_across_runs_and_independent_of_paths() {
    let source = "export function total(items: number[]): number {\n  return items.length;\n}\n";
    let first = hashed("src/a.ts", source);
    let second = hashed("src/b.ts", source);
    assert_eq!(
        symbol(&first, "total").body_hash,
        symbol(&second, "total").body_hash,
        "a hash is about code, the path is part of the id"
    );
    let bytes = source.as_bytes();
    let text = Source::new(bytes);
    let mut unit = analyze_with("src/a.ts", bytes, &AnalyzerConfig::default());
    assign_hashes(&mut unit.symbols, &text);
    let third = unit
        .symbols
        .iter()
        .find(|s| s.name == "total")
        .unwrap()
        .body_hash;
    assert_eq!(third, symbol(&first, "total").body_hash);
}

#[test]
fn unparsable_source_still_hashes_without_panicking() {
    let mut unit = analyze(
        "src/broken.ts",
        "export function broken( {\n  return ;\n}\n",
    );
    let text = Source::new(b"export function broken( {\n  return ;\n}\n");
    let report = assign_hashes(&mut unit.symbols, &text);
    assert_eq!(report.hashed, unit.symbols.len());
    assert!(unit
        .symbols
        .iter()
        .all(|s| s.body_hash != Hash128::ZERO || is_container(s.kind)));
}

#[test]
fn empty_bodies_hash_to_a_stable_non_zero_value() {
    let unit = hashed(
        "src/a.ts",
        "export interface Shape {\n  width: number;\n}\n",
    );
    let shape = symbol(&unit, "Shape.width");
    assert!(!shape.body_hash.is_zero());
    assert_eq!(
        shape.body_token_count, 3,
        "an interface member with no body range hashes its own declaration text"
    );
    assert_eq!(
        shape.body_hash,
        hashed(
            "src/a.ts",
            "export interface Shape {\n  width: number;\n}\n"
        )
        .symbols
        .iter()
        .find(|s| s.name == "width")
        .unwrap()
        .body_hash
    );
}
