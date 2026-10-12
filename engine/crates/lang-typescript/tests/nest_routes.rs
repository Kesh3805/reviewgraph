#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use analysis_ir::{AttrValue, DiagCode, FrameworkFactKind, IrFrameworkFact};
use common::{analyze, battr, facts_of, iattr, sattr, strs};

const HEAD: &str = "import { Controller, Get, Post, Put, Patch, Delete, All, Options, Head, Sse, \
                    Version, HttpCode, Redirect, Header, VERSION_NEUTRAL } from '@nestjs/common';\n";

fn routes(body: &str) -> Vec<IrFrameworkFact> {
    facts_of(
        "src/users/users.controller.ts",
        &format!("{HEAD}{body}"),
        FrameworkFactKind::HttpRoute,
    )
}

fn controllers(body: &str) -> Vec<IrFrameworkFact> {
    facts_of(
        "src/users/users.controller.ts",
        &format!("{HEAD}{body}"),
        FrameworkFactKind::Controller,
    )
}

fn paths(facts: &[IrFrameworkFact]) -> Vec<String> {
    facts.iter().filter_map(|f| sattr(f, "path")).collect()
}

#[test]
fn controller_prefix_forms_string_array_object() {
    let facts = controllers(
        "@Controller('users') class A {}\n\
         @Controller(['a', 'b']) class B {}\n\
         @Controller({ path: 'obj', version: '2', host: 'h.example' }) class C {}\n",
    );
    assert_eq!(facts.len(), 3);
    assert_eq!(strs(&facts[0], "prefixes"), vec!["users"]);
    assert_eq!(strs(&facts[1], "prefixes"), vec!["a", "b"]);
    assert_eq!(strs(&facts[2], "prefixes"), vec!["obj"]);
    assert_eq!(sattr(&facts[2], "version").as_deref(), Some("2"));
    assert_eq!(sattr(&facts[2], "host").as_deref(), Some("h.example"));
    assert_eq!(sattr(&facts[0], "symbol").as_deref(), Some("A"));
}

#[test]
fn empty_controller_prefix() {
    let facts = routes("@Controller() class A { @Get() list() {} }\n");
    assert_eq!(paths(&facts), vec!["/"]);
}

#[test]
fn all_http_method_decorators() {
    let facts = routes(
        "@Controller('x') class A {\n\
           @Get() a() {}\n @Post() b() {}\n @Put() c() {}\n @Patch() d() {}\n\
           @Delete() e() {}\n @All() f() {}\n @Options() g() {}\n @Head() h() {}\n\
         }\n",
    );
    let methods: Vec<String> = facts.iter().filter_map(|f| sattr(f, "method")).collect();
    assert_eq!(
        methods,
        vec!["GET", "POST", "PUT", "PATCH", "DELETE", "ALL", "OPTIONS", "HEAD"]
    );
}

#[test]
fn route_path_array_creates_multiple_facts() {
    let facts = routes("@Controller('x') class A { @Get(['a', 'b']) m() {} }\n");
    assert_eq!(paths(&facts), vec!["/x/a", "/x/b"]);
}

#[test]
fn path_normalization_slashes_and_trailing() {
    let facts = routes(
        "@Controller('/users/') class A {\n @Get('/:id/') one() {}\n @Get('//a//b/') two() {}\n @Get('/') root() {}\n}\n",
    );
    assert_eq!(paths(&facts), vec!["/users/:id", "/users/a/b", "/users"]);
}

#[test]
fn param_regex_stripped() {
    let facts = routes(r"@Controller('x') class A { @Get(':id(\\d+)') m() {} }");
    assert_eq!(paths(&facts), vec!["/x/:id"]);
}

#[test]
fn handler_version_overrides_controller_version() {
    let facts = routes(
        "@Controller({ path: 'v', version: '1' }) class A {\n\
           @Get('a') plain() {}\n\
           @Version('2') @Get('b') bumped() {}\n\
         }\n",
    );
    assert_eq!(sattr(&facts[0], "version").as_deref(), Some("1"));
    assert_eq!(sattr(&facts[1], "version").as_deref(), Some("2"));
}

#[test]
fn version_neutral_symbol() {
    let facts = controllers("@Controller({ path: 'x', version: VERSION_NEUTRAL }) class A {}\n");
    assert_eq!(sattr(&facts[0], "version").as_deref(), Some("NEUTRAL"));
}

#[test]
fn const_path_resolved_same_file() {
    let facts = routes("const BASE = 'items';\n@Controller(BASE) class A { @Get('x') m() {} }\n");
    assert_eq!(paths(&facts), vec!["/items/x"]);
    assert_eq!(battr(&facts[0], "dynamic"), Some(false));
}

#[test]
fn dynamic_path_flagged_low_confidence() {
    let facts = routes("@Controller('x') class A { @Get(buildPath()) m() {} }\n");
    assert_eq!(facts.len(), 1);
    assert_eq!(battr(&facts[0], "dynamic"), Some(true));
    assert!(facts[0].confidence <= 0.5);
    assert!(sattr(&facts[0], "path").unwrap().contains("buildPath"));
}

#[test]
fn sse_route_is_get_with_flag() {
    let facts = routes("@Controller('x') class A { @Sse('events') m() {} }\n");
    assert_eq!(sattr(&facts[0], "method").as_deref(), Some("GET"));
    assert_eq!(battr(&facts[0], "sse"), Some(true));
}

#[test]
fn http_code_and_redirect_attrs() {
    let facts = routes(
        "@Controller('x') class A {\n\
           @Get('a') @HttpCode(204) @Redirect('https://secret.example/landing', 301) \
           @Header('X-Foo', 'bar') m() {}\n\
         }\n",
    );
    assert_eq!(iattr(&facts[0], "status_code"), Some(204));
    assert_eq!(battr(&facts[0], "redirect"), Some(true));
    assert_eq!(iattr(&facts[0], "redirect_status"), Some(301));
    assert_eq!(strs(&facts[0], "header_names"), vec!["X-Foo"]);
    let dump = format!("{facts:?}");
    assert!(!dump.contains("secret.example"));
    assert!(!dump.contains("bar"));
}

#[test]
fn route_in_non_controller_ignored() {
    let source = format!("{HEAD}class A {{ @Get('x') m() {{}} }}\n");
    let unit = analyze("src/a.ts", &source);
    assert!(unit
        .framework
        .iter()
        .all(|f| f.kind != FrameworkFactKind::HttpRoute));
    assert!(unit
        .diagnostics
        .iter()
        .any(|d| d.code == DiagCode::UnsupportedConstruct));
}

#[test]
fn abstract_base_controller_routes_diagnostic() {
    let source = format!("{HEAD}abstract class Base {{ @Get('z') m() {{}} }}\n");
    let unit = analyze("src/base.ts", &source);
    assert!(unit
        .framework
        .iter()
        .all(|f| f.kind != FrameworkFactKind::HttpRoute));
    assert!(!unit.diagnostics.is_empty());
}

fn global_config(path: &str, source: &str) -> Vec<IrFrameworkFact> {
    facts_of(path, source, FrameworkFactKind::HttpGlobalConfig)
}

const MAIN: &str = "import { NestFactory } from '@nestjs/core';\n\
    import { VersioningType } from '@nestjs/common';\n\
    async function bootstrap() {\n\
      const app = await NestFactory.create(AppModule);\n\
      app.setGlobalPrefix('api', { exclude: ['health'] });\n\
      app.enableVersioning({ type: VersioningType.URI, defaultVersion: '1' });\n\
      await app.listen(3000);\n\
    }\n";

#[test]
fn set_global_prefix_with_exclude() {
    let facts = global_config("src/main.ts", MAIN);
    let prefix = facts
        .iter()
        .find(|f| sattr(f, "kind").as_deref() == Some("global_prefix"))
        .unwrap();
    assert_eq!(sattr(prefix, "prefix").as_deref(), Some("api"));
    assert_eq!(strs(prefix, "exclude"), vec!["health"]);
    assert_eq!(sattr(prefix, "symbol").as_deref(), Some("bootstrap"));
}

#[test]
fn enable_versioning_uri_default_version() {
    let facts = global_config("src/main.ts", MAIN);
    let versioning = facts
        .iter()
        .find(|f| sattr(f, "kind").as_deref() == Some("versioning"))
        .unwrap();
    assert_eq!(sattr(versioning, "type").as_deref(), Some("URI"));
    assert_eq!(sattr(versioning, "default_version").as_deref(), Some("1"));
    assert!(!versioning.attrs.contains_key("prefix"));
}

#[test]
fn global_config_in_non_main_file() {
    let facts = global_config("src/bootstrap/app.factory.ts", MAIN);
    assert_eq!(facts.len(), 2);
}

#[test]
fn unrelated_set_global_prefix_ignored() {
    let facts = global_config(
        "src/other.ts",
        "import { Controller } from '@nestjs/common';\nfunction f(thing: Router) { thing.setGlobalPrefix('x'); }\n",
    );
    assert!(facts.is_empty());
}

#[test]
fn alias_imported_decorators() {
    let facts = facts_of(
        "src/a.controller.ts",
        "import { Controller as C, Get as G } from '@nestjs/common';\n@C('x') class A { @G('y') m() {} }\n",
        FrameworkFactKind::HttpRoute,
    );
    assert_eq!(paths(&facts), vec!["/x/y"]);
    assert!(facts[0].confidence > 0.9);
}

#[test]
fn route_symbol_names_the_handler() {
    let facts = routes("@Controller('x') class A { @Get('y') handle() {} }\n");
    assert_eq!(sattr(&facts[0], "symbol").as_deref(), Some("A.handle"));
    assert_eq!(sattr(&facts[0], "controller").as_deref(), Some("A"));
    assert!(matches!(
        facts[0].attrs.get("header_names"),
        Some(AttrValue::List(items)) if items.is_empty()
    ));
}
