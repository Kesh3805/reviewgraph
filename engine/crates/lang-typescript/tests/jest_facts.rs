#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use analysis_ir::{AttrValue, FrameworkFactKind, IrFrameworkFact};
use common::{battr, facts_of, iattr, lattr, sattr, strs};

const SPEC: &str = "src/users/users.service.spec.ts";

fn cases(source: &str) -> Vec<IrFrameworkFact> {
    facts_of(SPEC, source, FrameworkFactKind::TestCase)
}

fn suites(source: &str) -> Vec<IrFrameworkFact> {
    facts_of(SPEC, source, FrameworkFactKind::TestSuite)
}

#[test]
fn nested_describe_suite_path() {
    let source =
        "describe('Users', () => { describe('create', () => { it('works', () => {}); }); });\n";
    let s = suites(source);
    assert_eq!(s.len(), 2);
    assert_eq!(sattr(&s[0], "suite_path").as_deref(), Some(""));
    assert_eq!(sattr(&s[1], "suite_path").as_deref(), Some("Users"));
    let c = cases(source);
    assert_eq!(strs(&c[0], "suite_path_parts"), vec!["Users", "create"]);
    assert_eq!(
        sattr(&c[0], "suite_path").as_deref(),
        Some("Users \u{203a} create")
    );
}

#[test]
fn it_and_test_cases() {
    let c = cases("describe('S', () => { it('a', () => {}); test('b', () => {}); });\n");
    let names: Vec<String> = c.iter().filter_map(|f| sattr(f, "name")).collect();
    assert_eq!(names, vec!["a", "b"]);
    assert_eq!(iattr(&c[0], "index"), Some(0));
    assert_eq!(iattr(&c[1], "index"), Some(1));
    assert_eq!(battr(&c[0], "is_test_file"), Some(true));
    assert_eq!(lattr(&c[0], "range").len(), 4);
}

#[test]
fn modifiers_skip_only_todo() {
    let c =
        cases("it.skip('a', () => {}); it.only('b', () => {}); it.todo('c'); it('d', () => {});\n");
    let m: Vec<String> = c.iter().filter_map(|f| sattr(f, "modifier")).collect();
    assert_eq!(m, vec!["skip", "only", "todo", "none"]);
    let s = suites("describe.skip('x', () => {});\n");
    assert_eq!(sattr(&s[0], "modifier").as_deref(), Some("skip"));
}

#[test]
fn x_and_f_prefixed_aliases() {
    let c = cases("xit('a', () => {}); fit('b', () => {}); xtest('c', () => {});\n");
    let m: Vec<String> = c.iter().filter_map(|f| sattr(f, "modifier")).collect();
    assert_eq!(m, vec!["skip", "only", "skip"]);
    let s = suites("xdescribe('a', () => {}); fdescribe('b', () => {});\n");
    let m: Vec<String> = s.iter().filter_map(|f| sattr(f, "modifier")).collect();
    assert_eq!(m, vec!["skip", "only"]);
}

#[test]
fn each_tables_single_fact() {
    let source = "describe.each([1, 2])('case %s', (n) => { it.each([[1, 2], [3, 4]])('adds %i', () => {}); });\n";
    let s = suites(source);
    assert_eq!(s.len(), 1);
    assert_eq!(sattr(&s[0], "name").as_deref(), Some("case %s"));
    assert_eq!(battr(&s[0], "each"), Some(true));
    assert_eq!(sattr(&s[0], "modifier").as_deref(), Some("each"));
    let c = cases(source);
    assert_eq!(c.len(), 1);
    assert_eq!(battr(&c[0], "each"), Some(true));
}

#[test]
fn template_name_placeholder() {
    let c = cases("const n = 1;\nit(`handles ${n} items`, () => {});\n");
    assert_eq!(
        sattr(&c[0], "name").as_deref(),
        Some("handles ${\u{2026}} items")
    );
    assert_eq!(battr(&c[0], "name_dynamic"), Some(true));
    assert!(c[0].confidence <= 0.5);
    let plain = cases("it(`plain name`, () => {});\n");
    assert_eq!(sattr(&plain[0], "name").as_deref(), Some("plain name"));
    assert_eq!(battr(&plain[0], "name_dynamic"), Some(false));
}

#[test]
fn duplicate_names_suffixed_deterministically() {
    let c = cases("describe('S', () => { it('dup', () => {}); it('dup', () => {}); it('dup', () => {}); });\n");
    let names: Vec<String> = c.iter().filter_map(|f| sattr(f, "name")).collect();
    assert_eq!(names, vec!["dup", "dup #2", "dup #3"]);
}

#[test]
fn jest_globals_import_origin() {
    let c = cases("import { describe, it } from '@jest/globals';\ndescribe('S', () => { it('a', () => {}); });\n");
    assert_eq!(c.len(), 1);
    let other = cases("import { it } from 'mocha-like';\nit('a', () => {});\n");
    assert!(other.is_empty());
}

#[test]
fn shadowed_describe_ignored() {
    let s =
        suites("function describe(name: string, fn: () => void) {}\ndescribe('x', () => {});\n");
    assert!(s.is_empty());
    let p = suites("function run(describe: any) { describe('x', () => {}); }\n");
    assert!(p.is_empty());
}

#[test]
fn non_test_file_lower_confidence() {
    let c = facts_of(
        "src/util/helpers.ts",
        "it('a', () => {});\n",
        FrameworkFactKind::TestCase,
    );
    assert_eq!(c.len(), 1);
    assert_eq!(battr(&c[0], "is_test_file"), Some(false));
    assert!(c[0].confidence <= 0.7 + 1e-6);
}

fn mocks(source: &str) -> Vec<IrFrameworkFact> {
    facts_of(SPEC, source, FrameworkFactKind::TestMock)
}

#[test]
fn jest_mock_module_specifier() {
    let m = mocks("jest.mock('./users.repository');\n");
    assert_eq!(sattr(&m[0], "kind").as_deref(), Some("module_mock"));
    assert_eq!(
        sattr(&m[0], "target").as_deref(),
        Some("./users.repository")
    );
}

#[test]
fn spy_on_target() {
    let m = mocks("const s = jest.spyOn(service, 'find');\nconst t = jest.fn();\n");
    assert_eq!(sattr(&m[0], "kind").as_deref(), Some("spy"));
    assert_eq!(sattr(&m[0], "target").as_deref(), Some("service.find"));
    assert_eq!(sattr(&m[1], "kind").as_deref(), Some("fn"));
}

#[test]
fn testing_module_override_provider() {
    let m = mocks(
        "import { Test } from '@nestjs/testing';\n\
         const mod = Test.createTestingModule({ providers: [UsersService] })\n\
           .overrideProvider(Repo).useValue({ find: jest.fn() }).compile();\n",
    );
    let kinds: Vec<String> = m.iter().filter_map(|f| sattr(f, "kind")).collect();
    assert!(kinds.contains(&"testing_module".to_owned()));
    let over = m
        .iter()
        .find(|f| sattr(f, "kind").as_deref() == Some("override_provider"))
        .unwrap();
    assert_eq!(sattr(over, "provider").as_deref(), Some("Repo"));
    assert_eq!(sattr(over, "value_kind").as_deref(), Some("object"));
}

#[test]
fn provide_value_mock_kind_only() {
    let m = mocks(
        "import { Test } from '@nestjs/testing';\n\
         Test.createTestingModule({ providers: [{ provide: Repo, useValue: { secret: 'hunter2' } }] });\n",
    );
    let pv = m
        .iter()
        .find(|f| sattr(f, "kind").as_deref() == Some("provide_value"))
        .unwrap();
    assert_eq!(sattr(pv, "provider").as_deref(), Some("Repo"));
    assert_eq!(sattr(pv, "value_kind").as_deref(), Some("object"));
    assert!(!format!("{m:?}").contains("hunter2"));
}

#[test]
fn in_test_block_refs_attributed_to_module() {
    let c = cases("describe('S', () => { it('a', () => { run(); }); });\n");
    assert_eq!(sattr(&c[0], "symbol").as_deref(), Some("__module__"));
    assert_eq!(c[0].symbol, Some(analysis_ir::LocalId(0)));
    assert!(matches!(
        c[0].attrs.get("range"),
        Some(AttrValue::List(items)) if items.len() == 4
    ));
}
