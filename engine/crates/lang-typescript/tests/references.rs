//! TSA-005: reference extraction with receiver hints.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use analysis_ir::reference::BindingRef;
use analysis_ir::{AttrValue, IrReference, LocalId, ParsedUnit, ReceiverHint, RefKind};
use common::{analyze, analyze_fixture, fixture};

fn of<'a>(unit: &'a ParsedUnit, kind: RefKind, name: &str) -> Vec<&'a IrReference> {
    unit.references
        .iter()
        .filter(|r| r.kind == kind && r.name == name)
        .collect()
}

fn one<'a>(unit: &'a ParsedUnit, kind: RefKind, name: &str) -> &'a IrReference {
    let found = of(unit, kind, name);
    assert_eq!(
        found.len(),
        1,
        "{kind:?} {name}: {:?}",
        unit.references
            .iter()
            .map(|r| format!("{:?} {} {:?}", r.kind, r.name, r.receiver))
            .collect::<Vec<_>>()
    );
    found[0]
}

fn owner(unit: &ParsedUnit, reference: &IrReference) -> String {
    unit.symbols[reference.from.0 as usize]
        .qualified_name
        .join(".")
}

fn flag(reference: &IrReference, key: &str) -> bool {
    reference.attrs.get(key) == Some(&AttrValue::Bool(true))
}

fn binding(import: u32, binding: u32) -> BindingRef {
    BindingRef { import, binding }
}

fn this_field(field: &str, declared: Option<&str>) -> ReceiverHint {
    ReceiverHint::ThisField {
        field: field.to_owned(),
        declared_type: declared.map(str::to_owned),
    }
}

fn identifier(name: &str, declared: Option<&str>) -> ReceiverHint {
    ReceiverHint::Identifier {
        name: name.to_owned(),
        declared_type: declared.map(str::to_owned),
    }
}

#[test]
fn bare_call_imported_binding() {
    let unit = analyze(
        "src/a.ts",
        "import { helper } from './util';\nexport function run() { return helper(1, 2); }\n",
    );
    let call = one(&unit, RefKind::Call, "helper");
    assert_eq!(call.receiver, ReceiverHint::None);
    assert_eq!(call.import_binding, Some(binding(0, 0)));
    assert_eq!(call.arg_count, 2);
    assert_eq!(owner(&unit, call), "run");
    assert!(
        of(&unit, RefKind::ValueRead, "helper").is_empty(),
        "a callee is not a value read"
    );
}

#[test]
fn this_method_call() {
    let unit = analyze("src/a.ts", "class C {\n  a() { this.b(); }\n  b() {}\n}\n");
    let call = one(&unit, RefKind::Call, "b");
    assert_eq!(call.receiver, ReceiverHint::This);
    assert_eq!(owner(&unit, call), "C.a");
}

#[test]
fn super_method_and_super_ctor() {
    let unit = analyze(
        "src/a.ts",
        "class A { m() {} }\nclass B extends A {\n  constructor() { super(); }\n  m() { super.m(); }\n}\n",
    );
    let ctor = one(&unit, RefKind::Call, "super");
    assert_eq!(ctor.receiver, ReceiverHint::Super);
    assert!(flag(ctor, "super_ctor"));
    let method = one(&unit, RefKind::Call, "m");
    assert_eq!(method.receiver, ReceiverHint::Super);
    assert_eq!(owner(&unit, method), "B.m");
}

#[test]
fn this_field_call_typed_from_ctor_param_property() {
    let unit = analyze(
        "src/a.ts",
        "class S {\n  constructor(private readonly repo: Repo) {}\n  run() { this.repo.save(1); }\n}\n",
    );
    let call = one(&unit, RefKind::Call, "save");
    assert_eq!(call.receiver, this_field("repo", Some("Repo")));
}

#[test]
fn this_field_call_typed_from_field_declaration() {
    let unit = analyze(
        "src/a.ts",
        "class S {\n  private repo: Repository<User>;\n  run() { this.repo.save(1); }\n}\n",
    );
    let call = one(&unit, RefKind::Call, "save");
    assert_eq!(call.receiver, this_field("repo", Some("Repository<User>")));
    assert_eq!(
        call.attrs.get("declared_head"),
        Some(&AttrValue::Str("Repository".to_owned()))
    );
    assert_eq!(
        call.attrs.get("declared_args"),
        Some(&AttrValue::List(vec![AttrValue::Str("User".to_owned())]))
    );
}

#[test]
fn this_field_initialized_with_new() {
    let unit = analyze(
        "src/a.ts",
        "class S {\n  private cache = new Map<string, number>();\n  run() { this.cache.get('k'); }\n}\n",
    );
    let call = one(&unit, RefKind::Call, "get");
    assert_eq!(
        call.receiver,
        this_field("cache", Some("Map<string, number>"))
    );
}

#[test]
fn param_typed_identifier_receiver() {
    let unit = analyze("src/a.ts", "function f(repo: Repo) { repo.save(); }\n");
    let call = one(&unit, RefKind::Call, "save");
    assert_eq!(call.receiver, identifier("repo", Some("Repo")));
}

#[test]
fn local_const_new_typed_receiver() {
    let unit = analyze(
        "src/a.ts",
        "function f() { const repo = new Repo(); repo.save(); const typed: Other = make(); typed.go(); }\n",
    );
    assert_eq!(
        one(&unit, RefKind::Call, "save").receiver,
        identifier("repo", Some("Repo"))
    );
    assert_eq!(
        one(&unit, RefKind::Call, "go").receiver,
        identifier("typed", Some("Other"))
    );
    assert_eq!(
        one(&unit, RefKind::New, "Repo").receiver,
        ReceiverHint::None
    );
}

#[test]
fn shadowed_local_overrides_outer_type() {
    let unit = analyze(
        "src/a.ts",
        "function f(repo: A) {\n  { const repo = new B(); repo.go(); }\n  { const repo = make(); repo.untyped(); }\n  repo.run();\n}\n",
    );
    assert_eq!(
        one(&unit, RefKind::Call, "go").receiver,
        identifier("repo", Some("B"))
    );
    assert_eq!(
        one(&unit, RefKind::Call, "untyped").receiver,
        identifier("repo", None)
    );
    assert_eq!(
        one(&unit, RefKind::Call, "run").receiver,
        identifier("repo", Some("A"))
    );
}

#[test]
fn imported_namespace_member_call() {
    let unit = analyze(
        "src/a.ts",
        "import * as fs from 'fs';\nexport function f(p: string) { return fs.readFileSync(p); }\n",
    );
    let call = one(&unit, RefKind::Call, "readFileSync");
    assert_eq!(
        call.receiver,
        ReceiverHint::ImportedNamespace {
            binding: binding(0, 0)
        }
    );
    assert_eq!(call.import_binding, Some(binding(0, 0)));
}

#[test]
fn default_import_member_call_is_imported_namespace_hint() {
    let unit = analyze(
        "src/a.ts",
        "import Api from './api';\nexport function f() { return Api.get(); }\n",
    );
    let call = one(&unit, RefKind::Call, "get");
    assert_eq!(
        call.receiver,
        ReceiverHint::ImportedNamespace {
            binding: binding(0, 0)
        }
    );
}

#[test]
fn capitalized_static_candidate() {
    let unit = analyze("src/a.ts", "export function f() { return Foo.create(); }\n");
    let call = one(&unit, RefKind::Call, "create");
    assert_eq!(call.receiver, identifier("Foo", None));
    assert!(flag(call, "static_candidate"));
}

#[test]
fn deep_chain_segments() {
    let unit = analyze("src/a.ts", "class C {\n  run() { this.a.b.c(); }\n}\n");
    let call = one(&unit, RefKind::Call, "c");
    assert_eq!(
        call.receiver,
        ReceiverHint::Chain {
            root: Box::new(ReceiverHint::This),
            segments: vec!["a".to_owned(), "b".to_owned()],
        }
    );
}

#[test]
fn builder_chain_call_result() {
    let unit = analyze(
        "src/a.ts",
        "function f(qb: QB) { qb.where('a').andWhere('b'); }\n",
    );
    let outer = one(&unit, RefKind::Call, "andWhere");
    assert_eq!(
        outer.receiver,
        ReceiverHint::CallResult {
            callee: "qb.where".to_owned()
        }
    );
    let inner = one(&unit, RefKind::Call, "where");
    assert_eq!(inner.receiver, identifier("qb", Some("QB")));
}

#[test]
fn optional_chain_flagged() {
    let unit = analyze("src/a.ts", "function f(a?: A) { a?.b(); a.c(); }\n");
    assert!(flag(one(&unit, RefKind::Call, "b"), "optional"));
    assert!(!flag(one(&unit, RefKind::Call, "c"), "optional"));
}

#[test]
fn non_null_and_as_unwrapped() {
    let unit = analyze(
        "src/a.ts",
        "class S {\n  private repo: Repo;\n  run(x: unknown) {\n    (this.repo as Repo)!.save();\n    (x as Other).go();\n  }\n}\n",
    );
    assert_eq!(
        one(&unit, RefKind::Call, "save").receiver,
        this_field("repo", Some("Repo"))
    );
    assert_eq!(
        one(&unit, RefKind::Call, "go").receiver,
        identifier("x", Some("unknown"))
    );
}

#[test]
fn new_expression_classified() {
    let unit = analyze(
        "src/a.ts",
        "import * as ns from './ns';\nexport function f() { new ns.Thing(1); new Foo(); }\n",
    );
    let thing = one(&unit, RefKind::New, "Thing");
    assert_eq!(
        thing.receiver,
        ReceiverHint::ImportedNamespace {
            binding: binding(0, 0)
        }
    );
    assert_eq!(thing.arg_count, 1);
    let foo = one(&unit, RefKind::New, "Foo");
    assert_eq!(foo.receiver, ReceiverHint::None);
    assert_eq!(foo.arg_count, 0);
}

#[test]
fn type_refs_in_annotations_generics_returns() {
    let unit = analyze(
        "src/a.ts",
        "export function f(a: Foo, b: Array<Bar>): Promise<Baz> { return null as unknown as Promise<Baz>; }\n",
    );
    one(&unit, RefKind::TypeRef, "Foo");
    one(&unit, RefKind::TypeRef, "Array");
    let bar = one(&unit, RefKind::TypeRef, "Bar");
    assert_eq!(
        bar.attrs.get("type_arg_of"),
        Some(&AttrValue::Str("Array".to_owned()))
    );
    let promises = of(&unit, RefKind::TypeRef, "Promise");
    assert_eq!(promises.len(), 2, "return type and the `as` type");
    assert!(!of(&unit, RefKind::TypeRef, "Baz").is_empty());
}

#[test]
fn nested_type_identifier() {
    let unit = analyze(
        "src/a.ts",
        "export const user: Api.Models.User | null = null;\nexport let other: Api.Thing;\n",
    );
    let user = one(&unit, RefKind::TypeRef, "User");
    assert_eq!(
        user.receiver,
        ReceiverHint::Chain {
            root: Box::new(identifier("Api", None)),
            segments: vec!["Models".to_owned()],
        }
    );
    assert_eq!(
        one(&unit, RefKind::TypeRef, "Thing").receiver,
        identifier("Api", None)
    );
}

#[test]
fn extends_identifier_and_extends_call_mixin() {
    let unit = analyze(
        "src/a.ts",
        "class A extends Base {}\nclass B extends AuthGuard('jwt') {}\nclass C extends mixin(Base) {}\n",
    );
    let base = of(&unit, RefKind::Extends, "Base");
    assert_eq!(base.len(), 1);
    assert!(!flag(base[0], "extends_call"));
    let guard = one(&unit, RefKind::Extends, "AuthGuard");
    assert!(flag(guard, "extends_call"));
    assert_eq!(guard.arg_count, 1);
    assert!(flag(one(&unit, RefKind::Extends, "mixin"), "extends_call"));
}

#[test]
fn implements_multiple() {
    let unit = analyze(
        "src/a.ts",
        "class A implements I, J<K> {}\ninterface X extends Y {}\n",
    );
    one(&unit, RefKind::Implements, "I");
    one(&unit, RefKind::Implements, "J");
    let k = one(&unit, RefKind::TypeRef, "K");
    assert_eq!(
        k.attrs.get("type_arg_of"),
        Some(&AttrValue::Str("J".to_owned()))
    );
    one(&unit, RefKind::Extends, "Y");
}

#[test]
fn decorator_refs_from_decorated_symbol() {
    let unit = analyze(
        "src/a.ts",
        "@Injectable()\nexport class S {\n  @Get('x')\n  run(@Body() dto: D) {}\n}\n",
    );
    let injectable = one(&unit, RefKind::Decorator, "Injectable");
    assert_eq!(owner(&unit, injectable), "S");
    assert_eq!(injectable.arg_count, 0);
    let get = one(&unit, RefKind::Decorator, "Get");
    assert_eq!(owner(&unit, get), "S.run");
    assert_eq!(get.arg_count, 1);
    assert_eq!(
        owner(&unit, one(&unit, RefKind::Decorator, "Body")),
        "S.run"
    );
}

#[test]
fn jsx_component_refs_only_capitalized() {
    let unit = analyze(
        "src/a.tsx",
        "import { Button } from './b';\nimport * as UI from './ui';\nexport function V() {\n  return <div><Button a={1} /><UI.Panel>t</UI.Panel></div>;\n}\n",
    );
    let button = one(&unit, RefKind::JsxElement, "Button");
    assert_eq!(button.import_binding, Some(binding(0, 0)));
    let panel = one(&unit, RefKind::JsxElement, "Panel");
    assert_eq!(
        panel.receiver,
        ReceiverHint::ImportedNamespace {
            binding: binding(1, 0)
        }
    );
    assert!(of(&unit, RefKind::JsxElement, "div").is_empty());
    assert_eq!(
        unit.references
            .iter()
            .filter(|r| r.kind == RefKind::JsxElement)
            .count(),
        2,
        "closing tags are not references"
    );
}

#[test]
fn imported_value_reads_in_arrays_and_args() {
    let unit = analyze(
        "src/a.ts",
        "import { Mod } from './m';\nimport { Svc, User } from './x';\n@Module({ providers: [Svc], imports: [Mod.forFeature([User])] })\nexport class M {}\n",
    );
    one(&unit, RefKind::ValueRead, "Svc");
    one(&unit, RefKind::ValueRead, "User");
    assert!(
        of(&unit, RefKind::ValueRead, "Mod").is_empty(),
        "a receiver root is covered by the call's hint"
    );
    let for_feature = one(&unit, RefKind::Call, "forFeature");
    assert_eq!(for_feature.import_binding, Some(binding(0, 0)));
    assert!(flag(for_feature, "static_candidate"));
}

#[test]
fn callback_refs_attributed_to_enclosing_with_flag() {
    let unit = analyze(
        "src/a.ts",
        "import { helper } from './h';\nexport function f(items: number[]) {\n  items.forEach((x) => { helper(x); });\n}\n",
    );
    let helper = one(&unit, RefKind::Call, "helper");
    assert_eq!(owner(&unit, helper), "f");
    assert!(flag(helper, "in_callback"));
    assert!(!flag(one(&unit, RefKind::Call, "forEach"), "in_callback"));
}

#[test]
fn module_level_calls_from_module_symbol() {
    let unit = analyze("src/a.ts", "setup();\nfunction setup() {}\n");
    let call = one(&unit, RefKind::Call, "setup");
    assert_eq!(call.from, LocalId(0));
}

#[test]
fn in_test_block_flag() {
    let unit = analyze(
        "src/a.ts",
        "import { svc } from './s';\ndescribe('x', () => {\n  it('y', () => { svc.run(); });\n});\nsvc.outside();\n",
    );
    assert!(!one(&unit, RefKind::Call, "describe").in_test_block);
    assert!(one(&unit, RefKind::Call, "it").in_test_block);
    assert!(one(&unit, RefKind::Call, "run").in_test_block);
    assert!(!one(&unit, RefKind::Call, "outside").in_test_block);
}

#[test]
fn references_are_in_source_order() {
    let unit = analyze("src/a.ts", "function f(a: A) { a.x(); b.y().z(); }\n");
    let starts: Vec<_> = unit.references.iter().map(|r| r.range.start).collect();
    let mut sorted = starts.clone();
    sorted.sort();
    assert_eq!(starts, sorted);
}

#[test]
fn ts_basic_reference_fixtures() {
    let calls = analyze_fixture("ts-basic", "src/refs/calls.ts");
    let ctor = one(&calls, RefKind::Call, "super");
    assert_eq!(owner(&calls, ctor), "Derived.constructor");
    assert_eq!(
        one(&calls, RefKind::Call, "shout").import_binding,
        Some(binding(0, 0))
    );
    assert_eq!(one(&calls, RefKind::Call, "setup").from, LocalId(0));

    let chains = analyze_fixture("ts-basic", "src/refs/chains.ts");
    assert_eq!(
        one(&chains, RefKind::Call, "save").receiver,
        this_field("repo", Some("Repository<User>"))
    );
    assert_eq!(
        one(&chains, RefKind::Call, "get").receiver,
        this_field("cache", Some("Map<string, number>"))
    );
    assert!(matches!(
        one(&chains, RefKind::Call, "join").receiver,
        ReceiverHint::ImportedNamespace { .. }
    ));
    assert!(matches!(
        one(&chains, RefKind::Call, "load").receiver,
        ReceiverHint::ImportedNamespace { .. }
    ));
    assert!(!flag(
        one(&chains, RefKind::Call, "trim"),
        "static_candidate"
    ));

    let types = analyze_fixture("ts-basic", "src/refs/types.ts");
    one(&types, RefKind::Extends, "Base1");
    one(&types, RefKind::Extends, "Base2");
    one(&types, RefKind::TypeRef, "Item");

    let heritage = analyze_fixture("ts-basic", "src/refs/heritage.ts");
    assert!(flag(
        one(&heritage, RefKind::Extends, "AuthGuard"),
        "extends_call"
    ));
    one(&heritage, RefKind::Implements, "CanActivate");

    let jsx = analyze_fixture("ts-basic", "src/refs/jsx.tsx");
    one(&jsx, RefKind::JsxElement, "Button");
    one(&jsx, RefKind::JsxElement, "Panel");

    let ns = analyze_fixture("ts-basic", "src/refs/namespace-import.ts");
    assert!(matches!(
        one(&ns, RefKind::Call, "readFileSync").receiver,
        ReceiverHint::ImportedNamespace { .. }
    ));
    assert!(matches!(
        one(&ns, RefKind::Call, "get").receiver,
        ReceiverHint::ImportedNamespace { .. }
    ));
}

fn nest_sources() -> Vec<String> {
    let root = fixture("nest-app");
    let mut out = Vec::new();
    let mut stack = vec![root.join("src")];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "ts") {
                out.push(
                    path.strip_prefix(&root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }
    out.sort();
    out
}

#[test]
fn nest_api_member_calls_all_typed() {
    let mut checked = 0;
    for rel in nest_sources() {
        let unit = analyze_fixture("nest-app", &rel);
        for reference in &unit.references {
            if let ReceiverHint::ThisField {
                field,
                declared_type,
            } = &reference.receiver
            {
                assert!(
                    declared_type.is_some(),
                    "{rel}: this.{field}.{}() has no declared type",
                    reference.name
                );
                checked += 1;
            }
        }
    }
    assert!(checked >= 3, "only {checked} this-field calls found");
}

#[test]
fn every_fixture_unit_validates_with_references() {
    for rel in nest_sources() {
        let unit = analyze_fixture("nest-app", &rel);
        analysis_ir::validate(&unit).unwrap();
    }
}
