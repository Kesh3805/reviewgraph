#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use analysis_ir::{AttrValue, FrameworkFactKind, IrExpr, IrFrameworkFact};
use common::{facts_of, iattr, sattr, strs};

const HEAD: &str = "import { Injectable, Inject, Optional, forwardRef } from '@nestjs/common';\n\
                    import { InjectRepository, InjectDataSource } from '@nestjs/typeorm';\n\
                    import { InjectQueue } from '@nestjs/bullmq';\n";

fn di(body: &str) -> Vec<IrFrameworkFact> {
    facts_of(
        "src/users/users.service.ts",
        &format!("{HEAD}{body}"),
        FrameworkFactKind::DiInjection,
    )
}

#[test]
fn constructor_class_typed_params() {
    let facts = di(
        "@Injectable() class S { constructor(private readonly users: UsersService, private cfg: ConfigService) {} }\n",
    );
    assert_eq!(facts.len(), 2);
    assert_eq!(sattr(&facts[0], "type").as_deref(), Some("UsersService"));
    assert_eq!(sattr(&facts[0], "token_kind").as_deref(), Some("class"));
    assert_eq!(iattr(&facts[0], "param_index"), Some(0));
    assert_eq!(iattr(&facts[1], "param_index"), Some(1));
    assert!(facts[0].confidence >= 0.9);
    assert_eq!(sattr(&facts[0], "symbol").as_deref(), Some("S"));
}

#[test]
fn parameter_property_links_field() {
    let facts =
        di("@Injectable() class S { constructor(private readonly users: UsersService) {} }\n");
    assert_eq!(sattr(&facts[0], "property").as_deref(), Some("users"));
    assert_eq!(sattr(&facts[0], "param_name").as_deref(), Some("users"));
}

#[test]
fn explicit_inject_string_token() {
    let facts =
        di("@Injectable() class S { constructor(@Inject('CONFIG') private cfg: Config) {} }\n");
    assert_eq!(sattr(&facts[0], "token_kind").as_deref(), Some("string"));
    assert_eq!(
        facts[0].attrs.get("token"),
        Some(&AttrValue::Expr(IrExpr::Str("CONFIG".to_owned())))
    );
    assert!((facts[0].confidence - 0.7).abs() < 1e-6);
}

#[test]
fn explicit_inject_symbol_constant_token() {
    let facts =
        di("@Injectable() class S { constructor(@Inject(APP_CONFIG) private cfg: Config) {} }\n");
    assert_eq!(sattr(&facts[0], "token_kind").as_deref(), Some("symbol"));
}

#[test]
fn inject_overrides_type_annotation() {
    let facts =
        di("@Injectable() class S { constructor(@Inject('X') private foo: SomeClass) {} }\n");
    assert_eq!(sattr(&facts[0], "token_kind").as_deref(), Some("string"));
    assert_eq!(sattr(&facts[0], "type").as_deref(), Some("SomeClass"));
}

#[test]
fn forward_ref_unwraps_arrow() {
    let facts = di(
        "@Injectable() class S { constructor(@Inject(forwardRef(() => Other)) private other: Other) {} }\n",
    );
    assert_eq!(
        sattr(&facts[0], "token_kind").as_deref(),
        Some("forward_ref")
    );
}

#[test]
fn optional_flag() {
    let facts = di("@Injectable() class S { constructor(@Optional() private dep?: Dep) {} }\n");
    assert_eq!(facts[0].attrs.get("optional"), Some(&AttrValue::Bool(true)));
    let plain = di("@Injectable() class S { constructor(private dep: Dep) {} }\n");
    assert_eq!(
        plain[0].attrs.get("optional"),
        Some(&AttrValue::Bool(false))
    );
}

#[test]
fn inject_repository_extracts_entity() {
    let facts = di(
        "@Injectable() class S { constructor(@InjectRepository(User) private repo: Repository<User>) {} }\n",
    );
    assert_eq!(
        sattr(&facts[0], "token_kind").as_deref(),
        Some("typeorm_repository")
    );
    assert_eq!(sattr(&facts[0], "entity").as_deref(), Some("User"));
    assert_eq!(strs(&facts[0], "type_args"), vec!["User"]);
}

#[test]
fn inject_queue_extracts_name() {
    let facts =
        di("@Injectable() class S { constructor(@InjectQueue('mail') private queue: Queue) {} }\n");
    assert_eq!(
        sattr(&facts[0], "token_kind").as_deref(),
        Some("bullmq_queue")
    );
    assert_eq!(sattr(&facts[0], "queue").as_deref(), Some("mail"));
}

#[test]
fn inject_data_source_and_entity_manager() {
    let facts = di(
        "@Injectable() class S { constructor(@InjectDataSource() private ds: DataSource, @InjectEntityManager() private em: EntityManager) {} }\n",
    );
    assert_eq!(
        sattr(&facts[0], "token_kind").as_deref(),
        Some("typeorm_data_source")
    );
    assert_eq!(
        sattr(&facts[1], "token_kind").as_deref(),
        Some("typeorm_entity_manager")
    );
}

#[test]
fn property_injection() {
    let facts = di("@Injectable() class S { @Inject('X') private x: Foo; }\n");
    assert_eq!(facts.len(), 1);
    assert_eq!(iattr(&facts[0], "param_index"), Some(-1));
    assert_eq!(sattr(&facts[0], "property").as_deref(), Some("x"));
}

#[test]
fn non_injectable_class_skipped() {
    let facts = di("class Plain { constructor(private a: A) {} }\n");
    assert!(facts.is_empty());
}

#[test]
fn interface_typed_param_without_token_low_confidence() {
    let facts = di(
        "interface Cfg { a: string }\n@Injectable() class S { constructor(private cfg: Cfg, private n: string) {} }\n",
    );
    assert_eq!(facts.len(), 2);
    for fact in &facts {
        assert_eq!(sattr(fact, "token_kind").as_deref(), Some("unknown"));
        assert!(fact.confidence <= 0.3 + 1e-6);
    }
}

#[test]
fn generic_type_args_recorded() {
    let facts = di("@Injectable() class S { constructor(private repo: Repository<User>) {} }\n");
    assert_eq!(sattr(&facts[0], "type").as_deref(), Some("Repository"));
    assert_eq!(strs(&facts[0], "type_args"), vec!["User"]);
}

#[test]
fn unbound_inject_decorator_lower_confidence() {
    let bound = di("@Injectable() class S { constructor(@Inject(Foo) private f: Foo) {} }\n");
    let unbound = facts_of(
        "src/a.ts",
        "import { Injectable } from '@nestjs/common';\n@Injectable() class S { constructor(@Inject(Foo) private f: Foo) {} }\n",
        FrameworkFactKind::DiInjection,
    );
    assert!(bound[0].confidence >= 0.9);
    assert!(unbound[0].confidence < bound[0].confidence);
}

#[test]
fn secret_like_token_redacted() {
    let facts =
        di("@Injectable() class S { constructor(@Inject('API_TOKEN') private k: string) {} }\n");
    assert_eq!(
        facts[0].attrs.get("token"),
        Some(&AttrValue::Expr(IrExpr::Str("<redacted>".to_owned())))
    );
    assert!(!format!("{facts:?}").contains("API_TOKEN"));
}

#[test]
fn every_constructor_param_of_injectable_gets_one_fact() {
    let facts = di(
        "@Injectable() class A { constructor(private a: X, private b: Y, private c: Z) {} }\n\
         @Injectable() class B { constructor(private d: W) {} }\n",
    );
    assert_eq!(facts.len(), 4);
}
