#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use analysis_ir::{AttrValue, FrameworkFactKind, IrExpr, IrFrameworkFact};
use common::{analyze, battr, facts_of, iattr, lattr, sattr, strs};

const HEAD: &str = "import { Injectable, Controller, Get, Body, Query, UseGuards, UseInterceptors, \
                    UsePipes, UseFilters, SetMetadata, CanActivate, NestInterceptor, PipeTransform, \
                    ExceptionFilter, Catch, Module } from '@nestjs/common';\n\
                    import { AuthGuard } from '@nestjs/passport';\n\
                    import { Reflector, APP_GUARD, NestFactory } from '@nestjs/core';\n";

fn run(kind: FrameworkFactKind, body: &str) -> Vec<IrFrameworkFact> {
    facts_of("src/auth/auth.ts", &format!("{HEAD}{body}"), kind)
}

fn entries(fact: &IrFrameworkFact) -> Vec<std::collections::BTreeMap<String, AttrValue>> {
    lattr(fact, "middleware")
        .into_iter()
        .filter_map(|v| match v {
            AttrValue::Map(m) => Some(m),
            _ => None,
        })
        .collect()
}

fn text(m: &std::collections::BTreeMap<String, AttrValue>, key: &str) -> Option<String> {
    match m.get(key) {
        Some(AttrValue::Str(s)) => Some(s.clone()),
        _ => None,
    }
}

#[test]
fn guard_class_by_implements_can_activate() {
    let facts = run(
        FrameworkFactKind::Middleware,
        "@Injectable() export class G implements CanActivate { canActivate(c: any) { return true; } }\n",
    );
    assert_eq!(facts.len(), 1);
    assert_eq!(sattr(&facts[0], "role").as_deref(), Some("guard"));
    assert!(facts[0].confidence >= 0.95 - 1e-6);
    assert!(iattr(&facts[0], "entry_symbol").is_some());
    assert_eq!(strs(&facts[0], "implements"), vec!["CanActivate"]);
}

#[test]
fn guard_by_authguard_extends_call() {
    let facts = run(
        FrameworkFactKind::Middleware,
        "@Injectable() export class J extends AuthGuard('jwt') {}\n",
    );
    assert_eq!(sattr(&facts[0], "role").as_deref(), Some("guard"));
    assert!(facts[0].confidence >= 0.9 - 1e-6);
}

#[test]
fn interceptor_pipe_filter_roles() {
    let facts = run(
        FrameworkFactKind::Middleware,
        "@Injectable() class I implements NestInterceptor { intercept() {} }\n\
         @Injectable() class P implements PipeTransform { transform(v: any) { return v; } }\n\
         @Catch() class F implements ExceptionFilter { catch() {} }\n",
    );
    let roles: Vec<String> = facts.iter().filter_map(|f| sattr(f, "role")).collect();
    assert_eq!(roles, vec!["interceptor", "pipe", "filter"]);
}

#[test]
fn name_only_guard_has_low_confidence() {
    let facts = run(FrameworkFactKind::Middleware, "class SomeGuard {}\n");
    assert_eq!(facts.len(), 1);
    assert!(facts[0].confidence <= 0.6 + 1e-6);
}

#[test]
fn use_guards_on_method_and_class_scopes() {
    let facts = run(
        FrameworkFactKind::MiddlewareBinding,
        "@UseGuards(A) @Controller('x') class C { @UseGuards(B) @Get() m() {} }\n",
    );
    assert_eq!(facts.len(), 2);
    assert_eq!(sattr(&facts[0], "scope").as_deref(), Some("class"));
    assert_eq!(sattr(&facts[0], "target").as_deref(), Some("C"));
    assert_eq!(sattr(&facts[1], "scope").as_deref(), Some("method"));
    assert_eq!(sattr(&facts[1], "target").as_deref(), Some("C.m"));
    assert_eq!(sattr(&facts[1], "role").as_deref(), Some("guard"));
}

#[test]
fn use_guards_instance_and_call_forms() {
    let facts = run(
        FrameworkFactKind::MiddlewareBinding,
        "@UseGuards(new JwtGuard(), AuthGuard('jwt'), RolesGuard) @Controller('x') class C {}\n",
    );
    let list = entries(&facts[0]);
    let forms: Vec<String> = list.iter().filter_map(|m| text(m, "form")).collect();
    assert_eq!(forms, vec!["instance", "call", "class"]);
    assert_eq!(text(&list[0], "name").as_deref(), Some("JwtGuard"));
    assert_eq!(text(&list[1], "name").as_deref(), Some("AuthGuard"));
    assert_eq!(
        list[1].get("args"),
        Some(&AttrValue::List(vec![AttrValue::Expr(IrExpr::Str(
            "jwt".to_owned()
        ))]))
    );
}

#[test]
fn guard_order_preserved() {
    let facts = run(
        FrameworkFactKind::MiddlewareBinding,
        "@UseGuards(A, B, C) @Controller('x') class K {}\n",
    );
    let names: Vec<String> = entries(&facts[0])
        .iter()
        .filter_map(|m| text(m, "name"))
        .collect();
    assert_eq!(names, vec!["A", "B", "C"]);
    let orders: Vec<AttrValue> = entries(&facts[0])
        .iter()
        .filter_map(|m| m.get("order").cloned())
        .collect();
    assert_eq!(
        orders,
        vec![AttrValue::Int(0), AttrValue::Int(1), AttrValue::Int(2)]
    );
}

#[test]
fn param_level_pipe_binding() {
    let facts = run(
        FrameworkFactKind::MiddlewareBinding,
        "@Controller('x') class C { @Get() m(@Body(ValidationPipe) dto: Dto, @Query('id', ParseIntPipe) id: number) {} }\n",
    );
    assert_eq!(facts.len(), 2);
    for fact in &facts {
        assert_eq!(sattr(fact, "scope").as_deref(), Some("param"));
        assert_eq!(sattr(fact, "role").as_deref(), Some("pipe"));
    }
    assert_eq!(sattr(&facts[1], "param_name").as_deref(), Some("id"));
}

#[test]
fn app_guard_provider_global() {
    let facts = run(
        FrameworkFactKind::GlobalProvider,
        "@Module({ providers: [{ provide: APP_GUARD, useClass: JwtGuard }] }) class AppModule {}\n",
    );
    assert_eq!(facts.len(), 1);
    assert_eq!(sattr(&facts[0], "role").as_deref(), Some("guard"));
    assert_eq!(sattr(&facts[0], "via").as_deref(), Some("module_provider"));
    assert_eq!(sattr(&facts[0], "module").as_deref(), Some("AppModule"));
    assert_eq!(
        text(&entries(&facts[0])[0], "name").as_deref(),
        Some("JwtGuard")
    );
}

#[test]
fn use_global_pipes_in_main() {
    let facts = run(
        FrameworkFactKind::GlobalProvider,
        "async function bootstrap() { const app = await NestFactory.create(AppModule); app.useGlobalPipes(new ValidationPipe()); }\n",
    );
    assert_eq!(facts.len(), 1);
    assert_eq!(sattr(&facts[0], "via").as_deref(), Some("main"));
    assert_eq!(sattr(&facts[0], "role").as_deref(), Some("pipe"));
    assert_eq!(
        text(&entries(&facts[0])[0], "form").as_deref(),
        Some("instance")
    );
}

#[test]
fn set_metadata_decorator_definition() {
    let facts = run(
        FrameworkFactKind::MetadataDecoratorDefinition,
        "export const Roles = (...roles: string[]) => SetMetadata('roles', roles);\n",
    );
    assert_eq!(facts.len(), 1);
    assert_eq!(sattr(&facts[0], "name").as_deref(), Some("Roles"));
    assert_eq!(sattr(&facts[0], "key").as_deref(), Some("roles"));
    assert_eq!(sattr(&facts[0], "kind").as_deref(), Some("set_metadata"));
}

#[test]
fn public_decorator_route_metadata() {
    let facts = run(
        FrameworkFactKind::RouteMetadata,
        "export const IS_PUBLIC_KEY = 'isPublic';\n\
         export const Public = () => SetMetadata(IS_PUBLIC_KEY, true);\n\
         @Controller('x') class C { @Public() @Get() m() {} }\n",
    );
    assert_eq!(facts.len(), 1);
    assert_eq!(sattr(&facts[0], "key").as_deref(), Some("isPublic"));
    assert_eq!(battr(&facts[0], "convention_based"), Some(false));
    assert_eq!(sattr(&facts[0], "symbol").as_deref(), Some("C.m"));
}

#[test]
fn convention_based_public_from_another_file() {
    let facts = run(
        FrameworkFactKind::RouteMetadata,
        "import { Public } from './public.decorator';\n@Controller('x') class C { @Public() @Get() m() {} }\n",
    );
    assert_eq!(facts.len(), 1);
    assert_eq!(battr(&facts[0], "convention_based"), Some(true));
    assert!(facts[0].confidence <= 0.6 + 1e-6);
}

#[test]
fn roles_metadata_with_values() {
    let facts = run(
        FrameworkFactKind::RouteMetadata,
        "export const Roles = (...r: string[]) => SetMetadata('roles', r);\n\
         @Controller('x') class C { @Roles('admin', 'user') @Get() m() {} }\n",
    );
    assert_eq!(facts.len(), 1);
    assert_eq!(sattr(&facts[0], "key").as_deref(), Some("roles"));
    match facts[0].attrs.get("value") {
        Some(AttrValue::Expr(IrExpr::Array(items))) => assert_eq!(items.len(), 2),
        other => panic!("unexpected value {other:?}"),
    }
}

#[test]
fn reflector_keys_collected() {
    let facts = run(
        FrameworkFactKind::Middleware,
        "export const IS_PUBLIC_KEY = 'isPublic';\n\
         @Injectable() export class G implements CanActivate {\n\
           constructor(private reflector: Reflector) {}\n\
           canActivate(c: any) { return this.reflector.getAllAndOverride<boolean>(IS_PUBLIC_KEY, [c.getHandler()]); }\n\
         }\n",
    );
    assert_eq!(
        facts[0].attrs.get("uses_reflector"),
        Some(&AttrValue::Bool(true))
    );
    assert_eq!(strs(&facts[0], "reads_metadata_keys"), vec!["isPublic"]);
}

#[test]
fn unparsed_binding_kept_as_unknown() {
    let facts = run(
        FrameworkFactKind::MiddlewareBinding,
        "@UseGuards(flag ? A : B) @Controller('x') class C {}\n",
    );
    assert_eq!(facts.len(), 1);
    assert!(facts[0].confidence <= 0.4 + 1e-6);
    assert_eq!(
        text(&entries(&facts[0])[0], "form").as_deref(),
        Some("dynamic_expr")
    );
}

#[test]
fn alias_and_namespace_imports() {
    let facts = facts_of(
        "src/a.ts",
        "import * as Nest from '@nestjs/common';\nimport { UseGuards as UG } from '@nestjs/common';\n\
         @Nest.UseGuards(A) class C {}\n@UG(B) class D {}\n",
        FrameworkFactKind::MiddlewareBinding,
    );
    assert_eq!(facts.len(), 2);
}

#[test]
fn non_nest_use_guards_name_ignored() {
    let unit = analyze(
        "src/a.ts",
        "import { Injectable } from '@nestjs/common';\nimport { UseGuards } from 'other-lib';\n@UseGuards(G) class C {}\n",
    );
    assert!(unit
        .framework
        .iter()
        .all(|f| f.kind != FrameworkFactKind::MiddlewareBinding));
}
