//! SID-002: the frozen TypeScript qualified-name rules.
//!
//! Each test names one row of the rule table in `docs/languages/typescript.md`. The pure
//! `naming::qualify` cases are table-driven; the analyzer cases run against the fixture files
//! under `fixtures/repositories/ts-basic/src/naming/`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use analysis_ir::{AnalyzerConfig, AnonymousFnPolicy, DiagCode, ParsedUnit};
use common::{analyze_fixture, analyze_with, fixture, try_analyze_fixture};
use lang_typescript::naming::{qualify, Construct, MemberName, NameDecision, MAX_SEGMENT_BYTES};
use review_core::symbol::SymbolKind;

fn named(decision: NameDecision) -> Vec<String> {
    match decision {
        NameDecision::Named(qn) => qn,
        other => panic!("expected a name, got {other:?}"),
    }
}

fn qn_of(parent: &[&str], segment: &str) -> Vec<String> {
    let owner: Vec<String> = parent.iter().map(|s| (*s).to_owned()).collect();
    named(qualify(
        Some(&owner),
        Construct::Member(MemberName::Identifier(segment.to_owned())),
    ))
}

/// `kind qualified.name[~ordinal]`, in symbol order.
fn list(unit: &ParsedUnit) -> Vec<String> {
    unit.symbols
        .iter()
        .skip(1)
        .map(|s| {
            let mut out = format!("{} {}", s.kind.as_id_str(), s.qualified_name.join("."));
            if s.ordinal > 0 {
                out.push_str(&format!("~{}", s.ordinal));
            }
            out
        })
        .collect()
}

fn has(unit: &ParsedUnit, entry: &str) -> bool {
    list(unit).iter().any(|e| e == entry)
}

fn naming_fixture(file: &str) -> ParsedUnit {
    try_analyze_fixture("ts-basic", &format!("src/naming/{file}"))
        .unwrap_or_else(|e| panic!("fixture src/naming/{file} must exist: {e}"))
}

#[test]
fn nested_class_in_namespace() {
    let unit = naming_fixture("nested.ts");
    assert!(has(&unit, "namespace Billing"));
    assert!(
        has(&unit, "class Billing.Payment"),
        "a class inside a namespace is a child: {}",
        list(&unit).join(", ")
    );
    assert!(has(&unit, "method Billing.Payment.capture"));
    assert!(
        has(&unit, "function inBody"),
        "the function itself is a symbol"
    );
    assert!(
        !list(&unit).iter().any(|e| e.contains("NotASymbol")),
        "classes inside a function body are not symbols: {}",
        list(&unit).join(", ")
    );
    // The same shape through the pure rule: parent name plus one segment.
    assert_eq!(
        named(qualify(
            Some(&vec!["Billing".to_owned()]),
            Construct::Declared("Payment".to_owned())
        )),
        vec!["Billing", "Payment"]
    );
}

#[test]
fn namespace_dotted_expands_to_nested() {
    let unit = naming_fixture("namespaces.ts");
    for entry in [
        "namespace Outer",
        "namespace Outer.Inner",
        "namespace Outer.Inner.Deepest",
        "constant Outer.Inner.Deepest.value",
    ] {
        assert!(
            has(&unit, entry),
            "{entry} missing: {}",
            list(&unit).join(", ")
        );
    }
}

#[test]
fn declare_module_string_and_global() {
    let unit = naming_fixture("namespaces.ts");
    assert!(
        has(&unit, "namespace external-package"),
        "`declare module 'pkg'` is one namespace named after the literal: {}",
        list(&unit).join(", ")
    );
    assert!(has(&unit, "function external-package.ambient"));
    assert!(has(&unit, "namespace global"));
    assert!(has(&unit, "interface global.Window"));
    assert!(has(&unit, "property global.Window.appName"));
}

#[test]
fn object_literal_methods_depth_two() {
    let unit = naming_fixture("objects.ts");
    for entry in [
        "constant routes",
        "method routes.list",
        "function routes.create",
        "method routes.nested.remove",
    ] {
        assert!(
            has(&unit, entry),
            "{entry} missing: {}",
            list(&unit).join(", ")
        );
    }
    assert!(
        !has(&unit, "method routes.nested.deeper.tooDeep"),
        "members three levels below the constant are not emitted: {}",
        list(&unit).join(", ")
    );
}

#[test]
fn object_literal_depth_three_not_emitted() {
    let unit = naming_fixture("objects.ts");
    let entries = list(&unit);
    let deep: Vec<&str> = entries
        .iter()
        .filter(|e| e.contains("tooDeep"))
        .map(String::as_str)
        .collect();
    assert!(deep.is_empty(), "unexpected {deep:?}");
    assert_eq!(
        named(qualify(
            Some(&vec!["o".to_owned(), "c".to_owned()]),
            Construct::Member(MemberName::Identifier("d".to_owned()))
        )),
        vec!["o", "c", "d"],
        "the depth cap lives in the visitor, not in the name rule"
    );
}

#[test]
fn const_arrow_and_function_expression_names() {
    let unit = naming_fixture("objects.ts");
    assert!(
        has(&unit, "function routes.create"),
        "a pair with a function value is a function symbol"
    );
    assert!(
        has(&unit, "method routes.list"),
        "a shorthand method is a method"
    );
    assert!(has(&unit, "function routes.quoted-key"));
    let anon = naming_fixture("defaults-anon.ts");
    assert!(
        has(&anon, "function alsoAnonymous"),
        "a const-bound function expression takes the declarator name"
    );
    assert_eq!(
        named(qualify(None, Construct::Declared("handler".to_owned()))),
        vec!["handler"]
    );
}

#[test]
fn default_export_named_class() {
    let unit = naming_fixture("defaults-named.ts");
    assert!(
        has(&unit, "class OrdersService"),
        "a named default export keeps its own name: {}",
        list(&unit).join(", ")
    );
    assert!(has(&unit, "method OrdersService.total"));
}

#[test]
fn default_export_anonymous_class_function_arrow() {
    let unit = naming_fixture("defaults-anon.ts");
    assert!(
        has(&unit, "class default"),
        "an anonymous default export is named `default`: {}",
        list(&unit).join(", ")
    );
    assert!(
        has(&unit, "method default.handle"),
        "members of the default export live under `default`: {}",
        list(&unit).join(", ")
    );
    let object_default = analyze_with(
        "src/naming/default-object.ts",
        b"export default { load() {}, unload: () => undefined };\n",
        &AnalyzerConfig::default(),
    );
    assert!(has(&object_default, "constant default"));
    assert!(has(&object_default, "method default.load"));
    assert!(has(&object_default, "function default.unload"));
    assert_eq!(
        named(qualify(None, Construct::DefaultExport)),
        vec!["default"]
    );
}

#[test]
fn default_export_identifier_no_symbol() {
    let unit = naming_fixture("defaults-ident.ts");
    assert_eq!(
        list(&unit),
        vec!["constant existing"],
        "`export default <identifier>` exports a binding, it adds no symbol: {}",
        list(&unit).join(", ")
    );
    // The export itself is recorded by TSA-004 as `IrExport::DefaultExpr`; TSA-004 has not landed
    // yet, so only the naming rule is asserted here: no symbol is added for the identifier.
    assert_eq!(
        unit.exports.len(),
        0,
        "no export is extracted yet (TSA-004)"
    );
}

#[test]
fn anonymous_emit_policy_names() {
    let bytes = std::fs::read(fixture("ts-basic").join("src/ordinals/anonymous-emit.ts")).unwrap();
    let cfg = AnalyzerConfig {
        anonymous_functions: AnonymousFnPolicy::Emit,
        ..AnalyzerConfig::default()
    };
    let emitted = analyze_with("src/ordinals/anonymous-emit.ts", &bytes, &cfg);
    assert!(has(&emitted, "function withCallbacks.<anonymous>~1"));
    assert!(has(&emitted, "function withCallbacks.<anonymous>~2"));

    let attribute = analyze_with(
        "src/ordinals/anonymous-emit.ts",
        &bytes,
        &AnalyzerConfig::default(),
    );
    assert!(
        !list(&attribute).iter().any(|e| e.contains("<anonymous>")),
        "the default policy attributes anonymous functions to the enclosing symbol: {}",
        list(&attribute).join(", ")
    );
    assert_eq!(
        named(qualify(None, Construct::Anonymous)),
        vec!["<anonymous>"]
    );
}

#[test]
fn getter_setter_same_name_distinct_kinds() {
    let unit = naming_fixture("accessors.ts");
    assert!(has(&unit, "get Account.balance"));
    assert!(
        has(&unit, "set Account.balance"),
        "an accessor pair shares the name and is distinguished by kind: {}",
        list(&unit).join(", ")
    );
    assert!(
        unit.symbols
            .iter()
            .any(|s| s.kind == SymbolKind::Getter && s.name == "balance")
            && unit
                .symbols
                .iter()
                .any(|s| s.kind == SymbolKind::Setter && s.name == "balance")
    );
}

#[test]
fn static_and_instance_same_name() {
    let unit = naming_fixture("statics.ts");
    assert!(
        has(&unit, "method Registry.create"),
        "the instance member keeps the plain name: {}",
        list(&unit).join(", ")
    );
    assert!(
        has(&unit, "method Registry.create~1"),
        "the static twin gets the ordinal (SID-003): {}",
        list(&unit).join(", ")
    );
    let owner = vec!["Registry".to_owned()];
    assert_eq!(
        named(qualify(
            Some(&owner),
            Construct::Member(MemberName::Identifier("create".to_owned()))
        )),
        vec!["Registry", "create"],
        "the name itself does not carry static-ness"
    );
}

#[test]
fn private_hash_member_distinct_from_public() {
    let unit = naming_fixture("accessors.ts");
    assert!(
        has(&unit, "property Account.#secret"),
        "a private member keeps its `#`: {}",
        list(&unit).join(", ")
    );
    let owner = vec!["Account".to_owned()];
    assert_eq!(
        named(qualify(
            Some(&owner),
            Construct::Member(MemberName::Private("x".to_owned()))
        )),
        vec!["Account", "#x"]
    );
    assert_eq!(
        named(qualify(
            Some(&owner),
            Construct::Member(MemberName::Private("#x".to_owned()))
        )),
        vec!["Account", "#x"],
        "an already hashed name is not double-prefixed"
    );
}

#[test]
fn quoted_and_numeric_keys() {
    let owner = vec!["o".to_owned()];
    assert_eq!(
        named(qualify(
            Some(&owner),
            Construct::Member(MemberName::StringKey("a-b".to_owned()))
        )),
        vec!["o", "a-b"]
    );
    assert_eq!(
        named(qualify(
            Some(&owner),
            Construct::Member(MemberName::Numeric("42".to_owned()))
        )),
        vec!["o", "42"]
    );
    // A data property with no function value is not a symbol, so the fixture only has the
    // container: the name rules above are what a future data-property rule would use.
    let unit = naming_fixture("quoted-keys.ts");
    assert_eq!(list(&unit), vec!["constant lookup"]);
}

#[test]
fn well_known_symbol_member() {
    let unit = naming_fixture("symbols.ts");
    assert!(has(&unit, "method iterable.@@iterator"));
    assert!(has(&unit, "method iterable.@@asyncIterator"));
    let owner = vec!["o".to_owned()];
    assert_eq!(
        named(qualify(
            Some(&owner),
            Construct::Member(MemberName::WellKnownSymbol("iterator".to_owned()))
        )),
        vec!["o", "@@iterator"]
    );
}

#[test]
fn computed_member_skipped_with_diagnostic() {
    let unit = naming_fixture("symbols.ts");
    assert!(
        unit.diagnostics
            .iter()
            .any(|d| d.code == DiagCode::ComputedMemberName),
        "a computed key is reported, never named: {:?}",
        unit.diagnostics.iter().map(|d| d.code).collect::<Vec<_>>()
    );
    assert_eq!(
        qualify(None, Construct::Member(MemberName::Computed)),
        NameDecision::Skip(DiagCode::ComputedMemberName)
    );
    assert!(
        !list(&unit).iter().any(|e| e.contains("computed.dynamic")),
        "the computed key itself is skipped: {}",
        list(&unit).join(", ")
    );
}

#[test]
fn constructor_parameter_property_name() {
    let unit = try_analyze_fixture("ts-basic", "src/ordinals/param-property-collision.ts")
        .unwrap_or_else(|e| panic!("fixture must exist: {e}"));
    assert!(has(&unit, "constructor Repository.constructor"));
    assert!(
        has(&unit, "property Repository.id"),
        "a parameter property takes the parameter name: {}",
        list(&unit).join(", ")
    );
    assert!(has(&unit, "property Repository.name~1"));
    let symbol = unit
        .symbols
        .iter()
        .find(|s| s.qualified_name.last().is_some_and(|n| n == "id"))
        .unwrap();
    assert!(symbol.attrs.contains_key("from_constructor_param"));
}

#[test]
fn overloads_fold_no_segment() {
    let unit = analyze_fixture("ts-basic", "src/overloads.ts");
    let names: Vec<&String> = unit
        .symbols
        .iter()
        .filter(|s| !s.overload_signatures.is_empty())
        .map(|s| s.qualified_name.last().unwrap())
        .collect();
    for name in names {
        assert!(
            !name.contains('(') && !name.contains('~'),
            "an overload signature contributes no segment: {name}"
        );
    }
    assert_eq!(
        qualify(None, Construct::OverloadSignature),
        NameDecision::Fold,
        "signatures fold into their implementation"
    );
}

#[test]
fn names_independent_of_sibling_order() {
    let a = analyze_with(
        "src/order.ts",
        b"export class Alpha { run(): void {} }\nexport class Beta { run(): void {} }\n",
        &AnalyzerConfig::default(),
    );
    let b = analyze_with(
        "src/order.ts",
        b"export class Beta { run(): void {} }\nexport class Alpha { run(): void {} }\n",
        &AnalyzerConfig::default(),
    );
    let names = |unit: &ParsedUnit| -> Vec<String> {
        unit.symbols
            .iter()
            .map(|s| format!("{}.{}", s.qualified_name.join("."), s.kind.as_id_str()))
            .collect()
    };
    let mut first = names(&a);
    let mut second = names(&b);
    first.sort();
    second.sort();
    assert_eq!(first, second, "reordering declarations changes no name");
}

#[test]
fn overlong_segment_skipped() {
    let long = "x".repeat(MAX_SEGMENT_BYTES + 1);
    assert_eq!(
        qualify(None, Construct::Declared(long)),
        NameDecision::Skip(DiagCode::UnsupportedConstruct)
    );
    assert_eq!(
        named(qualify(
            None,
            Construct::Declared("x".repeat(MAX_SEGMENT_BYTES))
        )),
        vec!["x".repeat(MAX_SEGMENT_BYTES)],
        "a segment of exactly the limit is accepted"
    );
}

#[test]
fn unicode_names_nfc() {
    let decomposed = "Cafe\u{301}";
    let precomposed = "Caf\u{e9}";
    assert_eq!(
        named(qualify(None, Construct::Declared(decomposed.to_owned()))),
        named(qualify(None, Construct::Declared(precomposed.to_owned())))
    );
    let unit = analyze_with(
        "src/üñí.ts",
        "export const Caf\u{e9} = 1;\n".as_bytes(),
        &AnalyzerConfig::default(),
    );
    assert_eq!(unit.symbols[1].qualified_name, vec!["Caf\u{e9}".to_owned()]);
}

#[test]
fn qualified_names_chain_below_the_parent() {
    assert_eq!(qn_of(&["A"], "m"), vec!["A", "m"]);
    assert_eq!(qn_of(&["A", "m"], "n"), vec!["A", "m", "n"]);
    assert_eq!(
        named(qualify(
            None,
            Construct::Member(MemberName::Identifier("top".to_owned()))
        )),
        vec!["top"],
        "a module-level member has no parent segment"
    );
    assert_eq!(
        named(qualify(None, Construct::Declared("A".to_owned()))),
        vec!["A"],
        "the module symbol is not part of the parent chain, so top-level names never repeat it"
    );
    assert_eq!(
        named(qualify(
            None,
            Construct::Member(MemberName::Identifier("__module__".to_owned()))
        )),
        vec!["__module__"],
        "the module symbol is one reserved segment"
    );
}
