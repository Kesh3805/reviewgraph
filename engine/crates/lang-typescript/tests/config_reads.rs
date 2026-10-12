#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use analysis_ir::{FrameworkFactKind, IrFrameworkFact};
use common::{battr, facts_of, sattr};

fn reads(source: &str) -> Vec<IrFrameworkFact> {
    facts_of(
        "src/config/env.reader.ts",
        source,
        FrameworkFactKind::ConfigRead,
    )
}

fn names(facts: &[IrFrameworkFact]) -> Vec<String> {
    facts.iter().filter_map(|f| sattr(f, "name")).collect()
}

const SERVICE: &str = "import { Injectable } from '@nestjs/common';\n\
    import { ConfigService } from '@nestjs/config';\n\
    @Injectable() class S {\n\
      constructor(private readonly config: ConfigService) {}\n";

#[test]
fn process_env_dot_and_bracket() {
    let r = reads("const a = process.env.A_B;\nconst b = process.env['C_D'];\n");
    assert_eq!(names(&r), vec!["A_B", "C_D"]);
    assert!(r
        .iter()
        .all(|f| sattr(f, "source").as_deref() == Some("process_env")));
    assert!(r.iter().all(|f| battr(f, "is_env_name") == Some(true)));
}

#[test]
fn process_env_destructuring() {
    let r = reads("const { DB_HOST, DB_PORT: port, RETRIES = '3' } = process.env;\n");
    assert_eq!(names(&r), vec!["DB_HOST", "DB_PORT", "RETRIES"]);
    assert_eq!(battr(&r[2], "has_default"), Some(true));
    assert_eq!(battr(&r[0], "has_default"), Some(false));
}

#[test]
fn env_alias_variable() {
    let r = reads("function f() { const env = process.env; return env.API_URL; }\n");
    assert_eq!(names(&r), vec!["API_URL"]);
}

#[test]
fn process_env_fallback_marks_default_without_value() {
    let r = reads("const k = process.env.SECRET_KEY || 'hunter2';\n");
    assert_eq!(battr(&r[0], "has_default"), Some(true));
    assert!(!format!("{r:?}").contains("hunter2"));
}

#[test]
fn config_service_get_with_default() {
    let r = reads(&format!(
        "{SERVICE} f() {{ return this.config.get<string>('API_URL', 'x'); }} }}\n"
    ));
    assert_eq!(r.len(), 1);
    assert_eq!(sattr(&r[0], "source").as_deref(), Some("config_service"));
    assert_eq!(sattr(&r[0], "name").as_deref(), Some("API_URL"));
    assert_eq!(battr(&r[0], "has_default"), Some(true));
    assert_eq!(battr(&r[0], "or_throw"), Some(false));
    assert_eq!(sattr(&r[0], "declared_type_arg").as_deref(), Some("string"));
    assert!((r[0].confidence - 0.9).abs() < 1e-6);
    assert!(!format!("{r:?}").contains("'x'"));
}

#[test]
fn get_or_throw_flag() {
    let r = reads(&format!(
        "{SERVICE} f() {{ return this.config.getOrThrow('JWT_ISSUER'); }} }}\n"
    ));
    assert_eq!(battr(&r[0], "or_throw"), Some(true));
    assert_eq!(battr(&r[0], "has_default"), Some(false));
}

#[test]
fn config_service_typed_field_receiver() {
    let r = reads(
        "import { ConfigService } from '@nestjs/config';\n\
         class S { constructor(private readonly cfgSvc: ConfigService) {} f() { return this.cfgSvc.get('PORT'); } }\n",
    );
    assert_eq!(r.len(), 1);
    assert!((r[0].confidence - 0.9).abs() < 1e-6);
}

#[test]
fn config_service_named_receiver_lower_confidence() {
    let r = reads("function f(configService) { return configService.get('PORT'); }\n");
    assert_eq!(r.len(), 1);
    assert!((r[0].confidence - 0.7).abs() < 1e-6);
}

#[test]
fn namespaced_key_not_env() {
    let r = reads(&format!(
        "{SERVICE} f() {{ return this.config.get('database.host'); }} }}\n"
    ));
    assert_eq!(
        sattr(&r[0], "source").as_deref(),
        Some("config_service_namespaced")
    );
    assert_eq!(battr(&r[0], "is_env_name"), Some(false));
}

#[test]
fn lowercase_key_not_env_name() {
    let r = reads(&format!(
        "{SERVICE} f() {{ return this.config.get('timeout'); }} }}\n"
    ));
    assert_eq!(battr(&r[0], "is_env_name"), Some(false));
    assert_eq!(sattr(&r[0], "source").as_deref(), Some("config_service"));
}

#[test]
fn invalid_key_dropped() {
    let r = reads(&format!(
        "{SERVICE} f(userInput: string) {{ this.config.get('has space'); this.config.get(userInput); }} }}\n"
    ));
    assert!(r.is_empty());
}

#[test]
fn unrelated_get_ignored() {
    let r = reads("const m = new Map();\nconst v = m.get('KEY');\nprocess.env.OK_NAME;\n");
    assert_eq!(names(&r), vec!["OK_NAME"]);
}

#[test]
fn values_never_stored() {
    let r = reads("const t = process.env.TOKEN_X ?? 'abc123';\n");
    let dump = format!("{r:?}");
    assert!(!dump.contains("abc123"));
    assert_eq!(names(&r), vec!["TOKEN_X"]);
}
