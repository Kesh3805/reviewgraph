#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use analysis_ir::{ImportKind, Imported, IrExport};
use common::analyze;

fn named(name: &str) -> Imported {
    Imported::Named(name.to_owned())
}

#[test]
fn esm_default_named_namespace() {
    let unit = analyze(
        "a.ts",
        "import d, { a, b as c } from './x';\nimport * as ns from './y';\n",
    );
    assert_eq!(unit.imports.len(), 2);
    let first = &unit.imports[0];
    assert_eq!(first.specifier, "./x");
    assert_eq!(first.kind, ImportKind::Esm);
    assert_eq!(first.bindings.len(), 3);
    assert_eq!(first.bindings[0].local, "d");
    assert_eq!(first.bindings[0].imported, Imported::Default);
    assert_eq!(first.bindings[1].imported, named("a"));
    assert_eq!(first.bindings[2].local, "c");
    assert_eq!(first.bindings[2].imported, named("b"));
    assert_eq!(unit.imports[1].bindings[0].imported, Imported::Namespace);
    assert_eq!(unit.imports[1].bindings[0].local, "ns");
}

#[test]
fn side_effect_import() {
    let unit = analyze("a.ts", "import './polyfill';\n");
    assert_eq!(unit.imports[0].kind, ImportKind::SideEffect);
    assert!(unit.imports[0].bindings.is_empty());
}

#[test]
fn import_type_whole_statement_and_inline() {
    let unit = analyze(
        "a.ts",
        "import type { A } from './a';\nimport { type B, C } from './b';\n",
    );
    assert!(unit.imports[0].type_only);
    assert!(unit.imports[0].bindings[0].type_only);
    assert!(!unit.imports[1].type_only);
    assert!(unit.imports[1].bindings[0].type_only);
    assert!(!unit.imports[1].bindings[1].type_only);
}

#[test]
fn import_equals_require() {
    let unit = analyze("a.ts", "import fs = require('fs');\n");
    assert_eq!(unit.imports[0].kind, ImportKind::ImportEquals);
    assert_eq!(unit.imports[0].bindings[0].imported, Imported::CjsModule);
}

#[test]
fn dynamic_import_literal_and_nonliteral() {
    let unit = analyze(
        "a.ts",
        "async function f(n: string) { await import('./lazy'); await import(n); }\n",
    );
    let dynamic: Vec<_> = unit
        .imports
        .iter()
        .filter(|i| i.kind == ImportKind::Dynamic)
        .collect();
    assert_eq!(dynamic.len(), 1);
    assert_eq!(dynamic[0].specifier, "./lazy");
    assert!(unit
        .diagnostics
        .iter()
        .any(|d| d.code == analysis_ir::DiagCode::DynamicImportNonLiteral));
}

#[test]
fn require_plain_destructured_member() {
    let unit = analyze(
        "a.js",
        "const a = require('a');\nconst { b, c: d } = require('b');\nconst e = require('c').prop;\n",
    );
    assert_eq!(unit.imports.len(), 3);
    assert!(unit
        .imports
        .iter()
        .all(|i| i.kind == ImportKind::CjsRequire));
    assert_eq!(unit.imports[0].bindings[0].imported, Imported::CjsModule);
    assert_eq!(unit.imports[1].bindings.len(), 2);
    assert_eq!(unit.imports[1].bindings[1].local, "d");
    assert_eq!(unit.imports[1].bindings[1].imported, named("c"));
    assert_eq!(unit.imports[2].bindings[0].imported, named("prop"));
}

#[test]
fn require_shadowed_is_not_import() {
    let unit = analyze(
        "a.js",
        "function f(require) { const x = require('nope'); }\n",
    );
    assert!(unit.imports.is_empty());
}

#[test]
fn bare_require_side_effect() {
    let unit = analyze("a.js", "require('./setup');\n");
    assert_eq!(unit.imports[0].kind, ImportKind::CjsRequire);
    assert!(unit.imports[0].bindings.is_empty());
}

#[test]
fn export_declarations_local() {
    let unit = analyze(
        "a.ts",
        "export class A {}\nexport function f() {}\nexport const x = 1;\nclass Hidden {}\n",
    );
    let names: Vec<_> = unit
        .exports
        .iter()
        .filter_map(|e| match e {
            IrExport::Local { exported_as, .. } => Some(exported_as.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(names, ["A", "f", "x"]);
}

#[test]
fn export_clause_local_and_of_imported_binding() {
    let unit = analyze(
        "a.ts",
        "import { Dep } from './dep';\nclass A {}\nexport { A as B, Dep };\n",
    );
    assert!(matches!(
        &unit.exports[0],
        IrExport::Local { exported_as, .. } if exported_as == "B"
    ));
    match &unit.exports[1] {
        IrExport::Reexport {
            specifier,
            imported,
            exported_as,
            ..
        } => {
            assert_eq!(specifier, "./dep");
            assert_eq!(*imported, named("Dep"));
            assert_eq!(exported_as, "Dep");
        }
        other => panic!("expected reexport, got {other:?}"),
    }
}

#[test]
fn export_from_named_and_default() {
    let unit = analyze(
        "a.ts",
        "export { a, default as D } from './m';\nexport type { T } from './t';\n",
    );
    assert!(matches!(
        &unit.exports[0],
        IrExport::Reexport { imported, .. } if *imported == named("a")
    ));
    assert!(matches!(
        &unit.exports[1],
        IrExport::Reexport { imported: Imported::Default, exported_as, .. } if exported_as == "D"
    ));
    assert!(matches!(
        &unit.exports[2],
        IrExport::Reexport {
            type_only: true,
            ..
        }
    ));
}

#[test]
fn export_star_and_star_as() {
    let unit = analyze("a.ts", "export * from './a';\nexport * as ns from './b';\n");
    assert!(matches!(
        &unit.exports[0],
        IrExport::StarReexport {
            as_namespace: None,
            ..
        }
    ));
    assert!(matches!(
        &unit.exports[1],
        IrExport::StarReexport { as_namespace: Some(n), .. } if n == "ns"
    ));
}

#[test]
fn export_default_expr_vs_decl() {
    let decl = analyze("a.ts", "export default class Foo {}\n");
    assert!(matches!(
        &decl.exports[0],
        IrExport::Local { exported_as, .. } if exported_as == "default"
    ));
    let expr = analyze("b.ts", "export default 42;\n");
    assert!(matches!(&expr.exports[0], IrExport::DefaultExpr { .. }));
}

#[test]
fn export_equals() {
    let unit = analyze("a.ts", "class A {}\nexport = A;\n");
    assert!(matches!(
        &unit.exports[0],
        IrExport::ExportAssignment { symbol: Some(_) }
    ));
}

#[test]
fn module_exports_identifier_and_object() {
    let ident = analyze("a.js", "function f() {}\nmodule.exports = f;\n");
    assert!(matches!(
        &ident.exports[0],
        IrExport::CjsModuleExports {
            symbol: Some(_),
            ..
        }
    ));
    let object = analyze("b.js", "function f() {}\nmodule.exports = { f, g: 1 };\n");
    assert!(matches!(
        &object.exports[0],
        IrExport::CjsModuleExports { .. }
    ));
    assert!(matches!(
        &object.exports[1],
        IrExport::CjsExportsProperty { name, symbol: Some(_) } if name == "f"
    ));
    assert!(matches!(
        &object.exports[2],
        IrExport::CjsExportsProperty { name, symbol: None } if name == "g"
    ));
}

#[test]
fn exports_property_assignments() {
    let unit = analyze("a.js", "exports.a = 1;\nmodule.exports.b = 2;\n");
    assert!(matches!(
        &unit.exports[0],
        IrExport::CjsExportsProperty { name, .. } if name == "a"
    ));
    assert!(matches!(
        &unit.exports[1],
        IrExport::CjsExportsProperty { name, .. } if name == "b"
    ));
}
