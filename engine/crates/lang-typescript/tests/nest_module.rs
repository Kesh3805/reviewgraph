#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::collections::BTreeSet;

use analysis_ir::{AnalyzerConfig, AttrValue, FrameworkFactKind, IrFrameworkFact};
use common::{analyze, analyze_with};

fn modules(source: &str) -> Vec<IrFrameworkFact> {
    analyze("src/app.module.ts", source)
        .framework
        .into_iter()
        .filter(|f| f.kind == FrameworkFactKind::ModuleDeclaration)
        .collect()
}

fn list<'a>(fact: &'a IrFrameworkFact, key: &str) -> &'a [AttrValue] {
    match fact.attrs.get(key) {
        Some(AttrValue::List(items)) => items,
        other => panic!("expected list for {key}, got {other:?}"),
    }
}

fn field<'a>(entry: &'a AttrValue, key: &str) -> Option<&'a AttrValue> {
    match entry {
        AttrValue::Map(m) => m.get(key),
        _ => None,
    }
}

const BASIC: &str = r#"
import { Module, Global } from '@nestjs/common';
import { UsersModule } from './users/users.module';
import { TypeOrmModule } from '@nestjs/typeorm';
@Global()
@Module({
  imports: [UsersModule, TypeOrmModule.forFeature([User])],
  providers: [UsersService, { provide: 'TOKEN', useValue: 'secret-value' }, ...OTHER],
  controllers: [UsersController],
  exports: [UsersService],
})
export class AppModule {}
"#;

#[test]
fn module_with_all_four_arrays() {
    let facts = modules(BASIC);
    assert_eq!(facts.len(), 1);
    let f = &facts[0];
    assert_eq!(list(f, "imports").len(), 2);
    assert_eq!(list(f, "providers").len(), 3);
    assert_eq!(list(f, "controllers").len(), 1);
    assert_eq!(list(f, "exports").len(), 1);
    assert_eq!(f.attrs.get("global"), Some(&AttrValue::Bool(true)));
}

#[test]
fn import_binding_captured_for_each_class_entry() {
    let facts = modules(BASIC);
    let first = &list(&facts[0], "imports")[0];
    assert_eq!(
        field(first, "import_specifier"),
        Some(&AttrValue::Str("./users/users.module".to_owned()))
    );
}

#[test]
fn spread_entries_marked_and_confidence_lowered() {
    let facts = modules(BASIC);
    let spread = &list(&facts[0], "providers")[2];
    assert_eq!(
        field(spread, "form"),
        Some(&AttrValue::Str("spread".to_owned()))
    );
    assert!(facts[0].confidence < 0.9);
}

#[test]
fn use_value_secret_not_stored() {
    let facts = modules(BASIC);
    let provider = &list(&facts[0], "providers")[1];
    assert_eq!(
        field(provider, "use_value"),
        Some(&AttrValue::Str("string".to_owned()))
    );
    assert!(!format!("{facts:?}").contains("secret-value"));
}

#[test]
fn alias_import_of_module_decorator() {
    let facts = modules("import { Module as M } from '@nestjs/common';\n@M({})\nclass A {}\n");
    assert_eq!(facts.len(), 1);
    assert!(facts[0].confidence > 0.9);
}

#[test]
fn unbound_module_decorator_lower_confidence() {
    let facts = modules("import { X } from '@nestjs/common';\n@Module({})\nclass A {}\n");
    assert_eq!(facts.len(), 1);
    assert!(facts[0].confidence < 0.9);
}

#[test]
fn non_nest_decorator_named_module_ignored() {
    let facts = modules("import { Module } from 'other-lib';\n@Module({})\nclass A {}\n");
    assert!(facts.is_empty());
}

#[test]
fn enabled_adapters_filter_disables_nestjs() {
    let cfg = AnalyzerConfig {
        enabled_adapters: Some(BTreeSet::new()),
        ..AnalyzerConfig::default()
    };
    let unit = analyze_with("src/app.module.ts", BASIC.as_bytes(), &cfg);
    assert!(unit.framework.is_empty());
}
