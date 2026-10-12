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

fn form(entry: &AttrValue) -> String {
    match field(entry, "form") {
        Some(AttrValue::Str(s)) => s.clone(),
        other => panic!("entry without form: {other:?}"),
    }
}

fn name(entry: &AttrValue) -> String {
    match field(entry, "name") {
        Some(AttrValue::Str(s)) => s.clone(),
        other => panic!("entry without name: {other:?}"),
    }
}

#[test]
fn for_root_for_feature_register_args_preserved() {
    let facts = modules(
        "import { Module } from '@nestjs/common';\n\
         import { TypeOrmModule } from '@nestjs/typeorm';\n\
         import { BullModule } from '@nestjs/bullmq';\n\
         @Module({ imports: [TypeOrmModule.forRoot({ type: 'postgres' }), \
         TypeOrmModule.forFeature([User]), BullModule.registerQueue({ name: 'mail' })] })\n\
         class A {}\n",
    );
    let imports = list(&facts[0], "imports");
    let forms: Vec<String> = imports.iter().map(form).collect();
    assert_eq!(forms, vec!["for_root", "for_feature", "register_queue"]);
    assert_eq!(name(&imports[0]), "TypeOrmModule");
    for entry in imports {
        match field(entry, "args") {
            Some(AttrValue::List(args)) => assert_eq!(args.len(), 1),
            other => panic!("args not preserved: {other:?}"),
        }
    }
}

#[test]
fn provider_object_use_class_use_existing_use_factory() {
    let facts = modules(
        "import { Module } from '@nestjs/common';\n\
         @Module({ providers: [\n\
           { provide: 'A', useClass: AImpl },\n\
           { provide: B, useExisting: AImpl },\n\
           { provide: C, useFactory: () => 1, inject: [D, E] },\n\
         ] })\n\
         class M {}\n",
    );
    let providers = list(&facts[0], "providers");
    assert_eq!(providers.len(), 3);
    assert_eq!(
        field(&providers[0], "use_class"),
        Some(&AttrValue::Str("AImpl".to_owned()))
    );
    assert_eq!(
        field(&providers[1], "use_existing"),
        Some(&AttrValue::Str("AImpl".to_owned()))
    );
    assert!(field(&providers[2], "use_factory").is_some());
    match field(&providers[2], "inject") {
        Some(AttrValue::List(names)) => assert_eq!(names.len(), 2),
        other => panic!("inject missing: {other:?}"),
    }
}

#[test]
fn provide_token_string_and_symbol_constants() {
    let facts = modules(
        "import { Module } from '@nestjs/common';\n\
         @Module({ providers: [\n\
           { provide: 'STR_TOKEN', useValue: 1 },\n\
           { provide: APP_GUARD, useClass: Guard },\n\
         ] })\n\
         class M {}\n",
    );
    let providers = list(&facts[0], "providers");
    assert_eq!(
        field(&providers[0], "provide"),
        Some(&AttrValue::Expr(analysis_ir::IrExpr::Str(
            "STR_TOKEN".to_owned()
        )))
    );
    assert_eq!(
        field(&providers[1], "provide"),
        Some(&AttrValue::Expr(analysis_ir::IrExpr::Ident(
            "APP_GUARD".to_owned()
        )))
    );
}

#[test]
fn global_decorator_sets_flag() {
    let plain = modules("import { Module } from '@nestjs/common';\n@Module({})\nclass A {}\n");
    assert_eq!(plain[0].attrs.get("global"), Some(&AttrValue::Bool(false)));
    let global = modules(
        "import { Module, Global } from '@nestjs/common';\n@Global()\n@Module({})\nclass A {}\n",
    );
    assert_eq!(global[0].attrs.get("global"), Some(&AttrValue::Bool(true)));
}

#[test]
fn dynamic_module_static_for_root() {
    let facts = modules(
        "import { Module } from '@nestjs/common';\n\
         @Module({})\n\
         export class ConfigModule {\n\
           static forRoot(opts: Options) {\n\
             return { module: ConfigModule, providers: [ConfigService], exports: [ConfigService] };\n\
           }\n\
         }\n",
    );
    assert_eq!(facts.len(), 2);
    assert_eq!(facts[0].attrs.get("dynamic"), Some(&AttrValue::Bool(true)));
    assert_eq!(
        facts[1].attrs.get("dynamic_method"),
        Some(&AttrValue::Str("forRoot".to_owned()))
    );
    assert_eq!(list(&facts[1], "providers").len(), 1);
    assert_eq!(list(&facts[1], "exports").len(), 1);
}

#[test]
fn non_literal_metadata_diagnostic() {
    let unit = analyze(
        "src/app.module.ts",
        "import { Module } from '@nestjs/common';\nconst META = { providers: [] };\n@Module(META)\nclass A {}\n",
    );
    let fact = unit
        .framework
        .iter()
        .find(|f| f.kind == FrameworkFactKind::ModuleDeclaration)
        .unwrap();
    assert!(fact.attrs.contains_key("metadata_ref"));
    assert!(list(fact, "providers").is_empty());
    assert!(unit
        .diagnostics
        .iter()
        .any(|d| d.code == analysis_ir::DiagCode::UnsupportedConstruct));
}

#[test]
fn forward_ref_entries() {
    let facts = modules(
        "import { Module, forwardRef } from '@nestjs/common';\n\
         @Module({ imports: [forwardRef(() => OtherModule)] })\nclass A {}\n",
    );
    let entry = &list(&facts[0], "imports")[0];
    assert_eq!(form(entry), "forward_ref");
    assert_eq!(name(entry), "OtherModule");
}

#[test]
fn detect_false_without_nest_signals() {
    let facts = modules("@Module({ imports: [A] })\nclass X {}\n");
    assert!(facts.is_empty());
}

#[test]
fn detect_true_from_framework_signals() {
    let mut cfg = AnalyzerConfig::default();
    cfg.frameworks.frameworks.insert(
        "nestjs".to_owned(),
        analysis_ir::FrameworkPresence {
            scope_dirs: vec![String::new()],
            major: Some(10),
            confidence: 0.9,
        },
    );
    let unit = analyze_with(
        "src/app.module.ts",
        b"@Module({ imports: [A] })\nclass X {}\n",
        &cfg,
    );
    assert_eq!(unit.framework.len(), 1);
}
