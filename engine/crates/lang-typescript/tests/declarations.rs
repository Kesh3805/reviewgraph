#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use analysis_ir::{
    AnalyzerConfig, AnonymousFnPolicy, AttrValue, ConstValue, IrExpr, IrSymbol, Modifiers,
    ParseStatus, ParsedUnit, Visibility,
};
use common::{analyze, analyze_fixture, analyze_with};
use review_core::symbol::SymbolKind;

fn kind_str(k: SymbolKind) -> &'static str {
    k.as_id_str()
}

/// `kind qualified.name[~ordinal]`, in symbol order.
fn list(unit: &ParsedUnit) -> Vec<String> {
    unit.symbols
        .iter()
        .skip(1)
        .map(|s| {
            let mut out = format!("{} {}", kind_str(s.kind), s.qualified_name.join("."));
            if s.ordinal > 0 {
                out.push_str(&format!("~{}", s.ordinal));
            }
            out
        })
        .collect()
}

fn find<'a>(unit: &'a ParsedUnit, qn: &str, kind: SymbolKind) -> &'a IrSymbol {
    unit.symbols
        .iter()
        .find(|s| s.qualified_name.join(".") == qn && s.kind == kind)
        .unwrap_or_else(|| panic!("no {kind:?} `{qn}` in {:#?}", list(unit)))
}

fn has(unit: &ParsedUnit, entry: &str) -> bool {
    list(unit).iter().any(|e| e == entry)
}

#[test]
fn module_symbol_first() {
    let unit = analyze("src/auth/auth.service.ts", "export const a = 1;\n");
    let module = &unit.symbols[0];
    assert_eq!(module.kind, SymbolKind::Module);
    assert_eq!(module.name, "auth.service");
    assert_eq!(module.qualified_name, vec!["__module__"]);
    assert_eq!(module.local_id.0, 0);
    assert!(module.parent.is_none());
    assert_eq!(unit.symbols[1].parent, Some(module.local_id));
}

#[test]
fn class_with_heritage_and_decorators() {
    let unit = analyze(
        "a.ts",
        "@Injectable({ scope: 1 })\nexport class AuthService<T> extends Base<T> implements IAuth, Other {}\n",
    );
    let class = find(&unit, "AuthService", SymbolKind::Class);
    assert!(class.modifiers.contains(Modifiers::EXPORTED));
    assert_eq!(class.heritage.extends, vec!["Base<T>"]);
    assert_eq!(class.heritage.implements, vec!["IAuth", "Other"]);
    assert_eq!(class.type_params, vec!["T"]);
    assert_eq!(class.decorators.len(), 1);
    assert_eq!(class.decorators[0].name, "Injectable");
    assert_eq!(
        class.decorators[0].args,
        vec![IrExpr::Object(vec![(
            "scope".to_owned(),
            IrExpr::Num("1".to_owned())
        )])]
    );
    // The class range includes the decorator line.
    assert_eq!(class.range.start.line, 1);
}

#[test]
fn abstract_class_and_abstract_method() {
    let unit = analyze_fixture("ts-basic", "src/shapes.ts");
    let base = find(&unit, "Base", SymbolKind::Class);
    assert!(base.modifiers.contains(Modifiers::ABSTRACT));
    let area = find(&unit, "Base.area", SymbolKind::Method);
    assert!(area.modifiers.contains(Modifiers::ABSTRACT));
    let name = find(&unit, "Base.name", SymbolKind::Property);
    assert!(name.modifiers.contains(Modifiers::ABSTRACT));
    assert!(name.modifiers.contains(Modifiers::READONLY));
}

#[test]
fn interface_members_and_extends() {
    let unit = analyze(
        "a.ts",
        "interface I extends J, K<string> { a?: string; m(x: number): void; readonly r: number }\n",
    );
    let iface = find(&unit, "I", SymbolKind::Interface);
    assert_eq!(iface.heritage.extends, vec!["J", "K<string>"]);
    let a = find(&unit, "I.a", SymbolKind::Property);
    assert!(a.modifiers.contains(Modifiers::OPTIONAL));
    assert_eq!(a.declared_type.as_deref(), Some("string"));
    assert!(has(&unit, "method I.m"));
    assert!(find(&unit, "I.r", SymbolKind::Property)
        .modifiers
        .contains(Modifiers::READONLY));
}

#[test]
fn interface_method_overloads_fold() {
    let unit = analyze_fixture("ts-basic", "src/shapes.ts");
    let scale = find(&unit, "Shape.scale", SymbolKind::Method);
    assert_eq!(scale.overload_signatures.len(), 1);
    assert_eq!(
        scale.overload_signatures[0],
        "scale(x: number, y: number): Shape"
    );
    assert_eq!(
        list(&unit)
            .iter()
            .filter(|e| e.contains("Shape.scale"))
            .count(),
        1
    );
}

#[test]
fn type_alias_body_range() {
    let unit = analyze("a.ts", "export type Point = { x: number };\n");
    let alias = find(&unit, "Point", SymbolKind::TypeAlias);
    let body = alias.body_range.unwrap();
    assert_eq!((body.start.line, body.start.column), (1, 20));
    assert!(alias.modifiers.contains(Modifiers::EXPORTED));
}

#[test]
fn enum_members_with_const_values() {
    let unit = analyze_fixture("ts-basic", "src/enums.ts");
    assert_eq!(
        find(&unit, "Color.Red", SymbolKind::EnumMember).const_value,
        Some(ConstValue::Str("red".to_owned()))
    );
    assert_eq!(
        find(&unit, "Color.Blue", SymbolKind::EnumMember).const_value,
        Some(ConstValue::Num("3".to_owned()))
    );
    assert_eq!(
        find(&unit, "Implicit.A", SymbolKind::EnumMember).const_value,
        None
    );
    assert!(has(&unit, "enum_member Implicit.B"));
}

#[test]
fn const_enum_modifier() {
    let unit = analyze_fixture("ts-basic", "src/enums.ts");
    assert!(find(&unit, "Flags", SymbolKind::Enum)
        .modifiers
        .contains(Modifiers::CONST_ENUM));
    assert!(!find(&unit, "Color", SymbolKind::Enum)
        .modifiers
        .contains(Modifiers::CONST_ENUM));
}

#[test]
fn function_overloads_fold_into_implementation() {
    let unit = analyze_fixture("ts-basic", "src/util/strings.ts");
    let pad = find(&unit, "pad", SymbolKind::Function);
    assert_eq!(pad.overload_signatures.len(), 2);
    assert_eq!(
        pad.overload_signatures[0],
        "pad(s: string, n: number): string"
    );
    assert_eq!(
        list(&unit).iter().filter(|e| e.ends_with(" pad")).count(),
        1
    );
    assert_eq!(pad.ordinal, 0);
    // Class method overloads fold as well.
    let unit = analyze_fixture("ts-basic", "src/overloads.ts");
    let parse = find(&unit, "Parser.parse", SymbolKind::Method);
    assert_eq!(parse.overload_signatures.len(), 2);
}

#[test]
fn ambient_function_signature_is_symbol() {
    let unit = analyze_fixture("ts-basic", "src/ambient.d.ts");
    let ambient = find(&unit, "ambient", SymbolKind::Function);
    assert!(ambient.modifiers.contains(Modifiers::DECLARE));
    assert_eq!(ambient.overload_signatures.len(), 1);
    assert!(has(&unit, "constant VERSION"));
}

#[test]
fn method_kinds_getter_setter_constructor_static() {
    let unit = analyze_fixture("ts-basic", "src/shapes.ts");
    assert!(has(&unit, "constructor Circle.constructor"));
    assert!(has(&unit, "get Circle.diameter"));
    assert!(has(&unit, "set Circle.diameter"));
    assert!(find(&unit, "Circle.unit", SymbolKind::Method)
        .modifiers
        .contains(Modifiers::STATIC));
    assert!(find(&unit, "Circle.PI", SymbolKind::Property)
        .modifiers
        .contains(Modifiers::STATIC));
}

#[test]
fn ecma_private_member() {
    let unit = analyze_fixture("ts-basic", "src/shapes.ts");
    let radius = find(&unit, "Circle.#radius", SymbolKind::Property);
    assert_eq!(radius.visibility, Visibility::EcmaPrivate);
    assert_eq!(radius.name, "#radius");
}

#[test]
fn class_property_with_arrow_is_method() {
    let unit = analyze_fixture("ts-basic", "src/services/user.service.ts");
    let handler = find(&unit, "UserService.handler", SymbolKind::Method);
    assert_eq!(
        handler.attrs.get("binding"),
        Some(&AttrValue::Str("arrow_property".to_owned()))
    );
    assert!(handler.modifiers.contains(Modifiers::ASYNC));
    assert_eq!(handler.params[0].name, "id");
    assert!(has(&unit, "property UserService.cache"));
}

#[test]
fn constructor_parameter_properties_become_properties() {
    let unit = analyze_fixture("ts-basic", "src/shapes.ts");
    let r = find(&unit, "Circle.r", SymbolKind::Property);
    assert_eq!(r.visibility, Visibility::Private);
    assert!(r.modifiers.contains(Modifiers::READONLY));
    assert_eq!(r.declared_type.as_deref(), Some("number"));
    assert_eq!(
        r.attrs.get("from_constructor_param"),
        Some(&AttrValue::Bool(true))
    );
    let label = find(&unit, "Circle.label", SymbolKind::Property);
    assert!(label.modifiers.contains(Modifiers::OPTIONAL));
    assert_eq!(label.visibility, Visibility::Public);
    // The constructor still lists them as parameters with `property` set.
    let ctor = find(&unit, "Circle.constructor", SymbolKind::Constructor);
    assert!(ctor.params.iter().all(|p| p.property.is_some()));
    // Parameters that are not property parameters do not become symbols.
    let plain = analyze("a.ts", "class A { constructor(x: number) {} }\n");
    assert!(!has(&plain, "property A.x"));
}

#[test]
fn module_const_let_var_kinds() {
    let unit = analyze_fixture("ts-basic", "src/util/strings.ts");
    assert!(has(&unit, "constant MAX_LENGTH"));
    assert!(has(&unit, "variable counter"));
    assert!(has(&unit, "variable legacy"));
    assert_eq!(
        find(&unit, "MAX_LENGTH", SymbolKind::Constant).const_value,
        Some(ConstValue::Num("255".to_owned()))
    );
    assert!(find(&unit, "MAX_LENGTH", SymbolKind::Constant)
        .modifiers
        .contains(Modifiers::EXPORTED));
    assert!(!find(&unit, "legacy", SymbolKind::Variable)
        .modifiers
        .contains(Modifiers::EXPORTED));
}

#[test]
fn destructured_module_consts_one_symbol_each() {
    let unit = analyze_fixture("ts-basic", "src/util/strings.ts");
    for name in ["alpha", "gamma"] {
        let s = find(&unit, name, SymbolKind::Constant);
        assert_eq!(
            s.attrs.get("destructured"),
            Some(&AttrValue::Bool(true)),
            "{name}"
        );
    }
    assert!(!has(&unit, "constant beta"));
}

#[test]
fn const_arrow_and_function_expression_are_functions() {
    let unit = analyze_fixture("ts-basic", "src/util/strings.ts");
    let shout = find(&unit, "shout", SymbolKind::Function);
    assert_eq!(
        shout.attrs.get("binding"),
        Some(&AttrValue::Str("const_arrow".to_owned()))
    );
    assert_eq!(shout.return_type.as_deref(), Some("string"));
    assert_eq!(shout.params[0].type_text.as_deref(), Some("string"));
    assert!(shout.modifiers.contains(Modifiers::EXPORTED));
    let whisper = find(&unit, "whisper", SymbolKind::Function);
    assert_eq!(
        whisper.attrs.get("binding"),
        Some(&AttrValue::Str("const_function_expr".to_owned()))
    );
    assert_eq!(shout.signature.as_deref(), Some("shout(s: string): string"));
}

#[test]
fn object_literal_methods_qualified() {
    let unit = analyze_fixture("ts-basic", "src/objects.ts");
    assert!(has(&unit, "constant handlers"));
    assert!(has(&unit, "method handlers.create"));
    assert!(has(&unit, "function handlers.update"));
    assert!(has(&unit, "method handlers.nested.remove"));
    assert!(has(&unit, "function handlers.quoted-key"));
    // Depth three is not emitted.
    assert!(!list(&unit).iter().any(|e| e.contains("tooDeep")));
}

#[test]
fn computed_object_key_skipped_with_diagnostic() {
    let unit = analyze("a.ts", "export const o = { [k]() {}, ok() {} };\n");
    assert!(has(&unit, "method o.ok"));
    assert!(!list(&unit).iter().any(|e| e.contains("[k]")));
    assert!(unit
        .diagnostics
        .iter()
        .any(|d| d.code == analysis_ir::DiagCode::ComputedMemberName));
}

#[test]
fn nested_namespace_a_b_c() {
    let unit = analyze_fixture("ts-basic", "src/namespaces.ts");
    assert!(has(&unit, "namespace A"));
    assert!(has(&unit, "namespace A.B"));
    assert!(has(&unit, "namespace A.B.C"));
    assert!(has(&unit, "constant A.B.C.value"));
    assert!(has(&unit, "class A.B.C.Inner"));
    assert_eq!(
        find(&unit, "A.B.C", SymbolKind::Namespace).parent.unwrap(),
        find(&unit, "A.B", SymbolKind::Namespace).local_id
    );
}

#[test]
fn declare_module_string_namespace() {
    let unit = analyze_fixture("ts-basic", "src/namespaces.ts");
    let pkg = find(&unit, "pkg", SymbolKind::Namespace);
    assert!(pkg.modifiers.contains(Modifiers::AMBIENT));
    assert!(has(&unit, "function pkg.f"));
    assert!(find(&unit, "pkg.f", SymbolKind::Function)
        .modifiers
        .contains(Modifiers::AMBIENT));
}

#[test]
fn declare_global_namespace() {
    let unit = analyze_fixture("ts-basic", "src/namespaces.ts");
    assert!(find(&unit, "global", SymbolKind::Namespace)
        .modifiers
        .contains(Modifiers::DECLARE));
    assert!(has(&unit, "interface global.Window"));
    assert!(has(&unit, "property global.Window.appName"));
}

#[test]
fn export_default_variants() {
    let named = analyze_fixture("ts-basic", "src/defaults/named-class.ts");
    assert!(find(&named, "Greeter", SymbolKind::Class)
        .modifiers
        .contains(Modifiers::DEFAULT_EXPORT));
    let anon_class = analyze_fixture("ts-basic", "src/defaults/anon-class.ts");
    assert!(has(&anon_class, "class default"));
    assert!(has(&anon_class, "method default.run"));
    let anon_fn = analyze_fixture("ts-basic", "src/defaults/anon-fn.ts");
    assert!(has(&anon_fn, "function default"));
    let object = analyze_fixture("ts-basic", "src/defaults/object.ts");
    assert!(has(&object, "constant default"));
    assert!(has(&object, "method default.start"));
    assert!(has(&object, "function default.stop"));
    let ident = analyze_fixture("ts-basic", "src/defaults/ident.ts");
    assert!(!list(&ident).iter().any(|e| e.contains("default")));
    assert!(has(&ident, "constant value"));
}

#[test]
fn getter_setter_and_member_name_forms() {
    let unit = analyze_fixture("ts-basic", "src/accessors.ts");
    assert!(has(&unit, "get Temperature.celsius"));
    assert!(has(&unit, "set Temperature.celsius"));
    assert!(has(&unit, "get Temperature.zero"));
    assert!(has(&unit, "method Temperature.@@iterator"));
    assert!(has(&unit, "method Temperature.string-key"));
}

#[test]
fn anonymous_callbacks_not_symbols_by_default() {
    let src =
        "export function run() {\n  [1].map((x) => x);\n  setTimeout(function () {}, 1);\n}\n";
    let unit = analyze("a.ts", src);
    assert_eq!(list(&unit), vec!["function run"]);
}

#[test]
fn anonymous_emit_policy_emits_ordinals() {
    let src =
        "export function run() {\n  [1].map((x) => x);\n  setTimeout(function () {}, 1);\n}\n";
    let cfg = AnalyzerConfig {
        anonymous_functions: AnonymousFnPolicy::Emit,
        ..AnalyzerConfig::default()
    };
    let unit = analyze_with("a.ts", src.as_bytes(), &cfg);
    let entries = list(&unit);
    assert!(
        entries.contains(&"function run.<anonymous>~1".to_owned()),
        "{entries:?}"
    );
    assert!(
        entries.contains(&"function run.<anonymous>~2".to_owned()),
        "{entries:?}"
    );
}

#[test]
fn secret_like_const_value_redacted() {
    let unit = analyze_fixture("ts-basic", "src/util/strings.ts");
    let token = find(&unit, "API_TOKEN", SymbolKind::Constant);
    assert_eq!(token.const_value, None);
    assert_eq!(token.attrs.get("redacted"), Some(&AttrValue::Bool(true)));
    let json = serde_json::to_string(&unit).unwrap();
    assert!(!json.contains("super-secret-token-value"));
}

#[test]
fn decorators_attached_to_correct_member() {
    let src = "class C {\n  @A() @B()\n  both(): void {}\n\n  @C1()\n  // comment between\n  commented(): void {}\n\n  plain(@Inject(T) x: number) {}\n}\n";
    let unit = analyze("a.ts", src);
    let both = find(&unit, "C.both", SymbolKind::Method);
    let names: Vec<&str> = both.decorators.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, vec!["A", "B"]);
    assert_eq!(
        both.range.start.line, 2,
        "range starts at the first decorator"
    );
    let commented = find(&unit, "C.commented", SymbolKind::Method);
    assert_eq!(commented.decorators.len(), 1);
    assert_eq!(commented.decorators[0].name, "C1");
    let plain = find(&unit, "C.plain", SymbolKind::Method);
    assert!(plain.decorators.is_empty());
    assert_eq!(plain.params[0].decorators[0].name, "Inject");
}

#[test]
fn symbols_inside_error_region_flagged() {
    let unit = analyze_fixture("ts-edge", "src/syntax-error-mid-class.ts");
    assert!(matches!(unit.status, ParseStatus::Partial { .. }));
    assert!(has(&unit, "class Before"));
    assert!(has(&unit, "function after"), "{:?}", list(&unit));
    let broken = find(&unit, "Broken", SymbolKind::Class);
    assert!(broken.has_errors);
    let before = find(&unit, "Before", SymbolKind::Class);
    assert!(!before.has_errors);
    assert!(!find(&unit, "after", SymbolKind::Function).has_errors);
}

#[test]
fn validate_passes_for_every_fixture_unit() {
    for (repo, files) in [
        (
            "ts-basic",
            vec![
                "src/shapes.ts",
                "src/services/user.service.ts",
                "src/util/strings.ts",
                "src/enums.ts",
                "src/namespaces.ts",
                "src/objects.ts",
                "src/accessors.ts",
                "src/overloads.ts",
                "src/ambient.d.ts",
            ],
        ),
        (
            "ts-edge",
            vec![
                "src/syntax-error-mid-class.ts",
                "src/component.tsx",
                "src/legacy.jsx",
                "src/module.mjs",
                "src/common.cjs",
                "src/decorators.ts",
                "src/overloads-merging.ts",
            ],
        ),
    ] {
        for file in files {
            // `analyze_fixture` validates.
            let unit = analyze_fixture(repo, file);
            assert!(!unit.symbols.is_empty(), "{file}");
        }
    }
}

#[test]
fn merged_declarations_get_ordinals() {
    let unit = analyze_fixture("ts-edge", "src/overloads-merging.ts");
    assert!(has(&unit, "interface Merged"));
    assert!(has(&unit, "interface Merged~1"));
    assert!(has(&unit, "namespace Merged"));
    assert_eq!(
        find(&unit, "over", SymbolKind::Function)
            .overload_signatures
            .len(),
        2
    );
}
