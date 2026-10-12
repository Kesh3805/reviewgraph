//! Repository, `EntityManager`, `DataSource`, QueryBuilder and raw-SQL access facts (NEST-005).
//!
//! Raw SQL text is never stored: only the verb class and table names.

use std::collections::BTreeMap;

use analysis_ir::{AttrValue, FrameworkFactKind};
use tree_sitter::Node;

use super::ADAPTER;
use crate::frameworks::nestjs::{injected_properties, InjectedMap};
use crate::frameworks::util::{
    arguments_of, class_ident, node_string, nodes_of_kinds, receiver_types, s,
};
use crate::frameworks::FrameworkCtx;
use crate::kinds::{field, kind};
use crate::visit::db_heuristics::{
    generic_arg, is_db_name, is_db_type, sql_access, type_head, DbAccess,
};
use crate::visit::expr::to_ir_expr;
use crate::visit::fact_keys::{callee_of, Callee};
use crate::visit::facts::is_transaction;

const WRITES: [&str; 13] = [
    "save",
    "insert",
    "update",
    "upsert",
    "delete",
    "remove",
    "softDelete",
    "softRemove",
    "restore",
    "increment",
    "decrement",
    "clear",
    "recover",
];

const READS: [&str; 17] = [
    "find",
    "findOne",
    "findOneBy",
    "findBy",
    "findAndCount",
    "findAndCountBy",
    "findOneOrFail",
    "findOneByOrFail",
    "count",
    "countBy",
    "exist",
    "exists",
    "existsBy",
    "sum",
    "average",
    "minimum",
    "maximum",
];

const BUILDER_READS: [&str; 8] = [
    "getOne",
    "getMany",
    "getRawOne",
    "getRawMany",
    "getCount",
    "getManyAndCount",
    "getOneOrFail",
    "stream",
];

const BUILDER_WRITE_STEPS: [&str; 5] = ["insert", "update", "delete", "softDelete", "restore"];

struct Access {
    op: &'static str,
    via: &'static str,
    entity: Option<String>,
    receiver_type: Option<String>,
    table_hint: Option<String>,
    confidence: f32,
}

pub fn collect(ctx: &mut FrameworkCtx<'_>) {
    let types = receiver_types(ctx.symbols);
    let injected = injected_properties(ctx);
    let mut found: Vec<(Node<'_>, Callee, Access)> = Vec::new();
    for call in nodes_of_kinds(ctx.root, &[kind::CALL_EXPRESSION]) {
        let Some(function) = call.child_by_field_name(field::FUNCTION) else {
            continue;
        };
        let Some(callee) = callee_of(function, ctx.src) else {
            continue;
        };
        if let Some(access) = classify(ctx, &types, &injected, call, function, &callee) {
            found.push((call, callee, access));
        }
    }
    for (call, callee, access) in found {
        let range = ctx.src.range(call);
        let owner = ctx.owner(range.start);
        let mut attrs = BTreeMap::new();
        attrs.insert("symbol".to_owned(), AttrValue::Str(ctx.qname(owner)));
        if let Some(entity) = &access.entity {
            attrs.insert("entity".to_owned(), AttrValue::Str(entity.clone()));
        }
        attrs.insert("op".to_owned(), s(access.op));
        attrs.insert("method".to_owned(), AttrValue::Str(callee.name.clone()));
        attrs.insert("via".to_owned(), s(access.via));
        if let Some(declared) = &access.receiver_type {
            attrs.insert(
                "receiver_declared_type".to_owned(),
                AttrValue::Str(declared.clone()),
            );
        }
        if let Some(hint) = &access.table_hint {
            attrs.insert("table_hint".to_owned(), AttrValue::Str(hint.clone()));
        }
        attrs.insert(
            "in_transaction".to_owned(),
            AttrValue::Bool(in_transaction(ctx, call)),
        );
        ctx.emit(
            ADAPTER,
            FrameworkFactKind::OrmAccess,
            Some(owner),
            attrs,
            range,
            access.confidence,
        );
    }
}

fn in_transaction(ctx: &FrameworkCtx<'_>, call: Node<'_>) -> bool {
    let mut current = call.parent();
    while let Some(node) = current {
        if node.kind() == kind::CALL_EXPRESSION {
            let callee = node
                .child_by_field_name(field::FUNCTION)
                .and_then(|f| callee_of(f, ctx.src));
            if callee.is_some_and(|c| is_transaction(&c)) {
                return true;
            }
        }
        current = node.parent();
    }
    false
}

/// The entity class named by the first capitalized identifier argument of a builder step such as
/// `createQueryBuilder(User, 'u')`, `.from(User)`, `.into(User)` or `.update(User)`.
fn builder_entity(ctx: &FrameworkCtx<'_>, call: Node<'_>) -> Option<String> {
    let mut current = Some(call);
    for _ in 0..32 {
        let node = current?;
        if node.kind() != kind::CALL_EXPRESSION {
            break;
        }
        let function = node.child_by_field_name(field::FUNCTION)?;
        if function.kind() != kind::MEMBER_EXPRESSION {
            break;
        }
        let property = function.child_by_field_name(field::PROPERTY)?;
        let name = ctx.src.text(property);
        if matches!(&*name, "createQueryBuilder" | "from" | "into" | "update") {
            if let Some(arg) = arguments_of(node).into_iter().next() {
                if let Some(entity) = class_ident(&to_ir_expr(arg, ctx.src)) {
                    return Some(entity);
                }
            }
        }
        current = function.child_by_field_name(field::OBJECT);
    }
    None
}

/// `getRepository(User)` / `getTreeRepository(User)` directly in the receiver chain.
fn get_repository_entity(ctx: &FrameworkCtx<'_>, function: Node<'_>) -> Option<String> {
    let object = function.child_by_field_name(field::OBJECT)?;
    if object.kind() != kind::CALL_EXPRESSION {
        return None;
    }
    let inner = callee_of(object.child_by_field_name(field::FUNCTION)?, ctx.src)?;
    if inner.name != "getRepository" && inner.name != "getTreeRepository" {
        return None;
    }
    let arg = arguments_of(object).into_iter().next()?;
    class_ident(&to_ir_expr(arg, ctx.src))
}

fn classify(
    ctx: &FrameworkCtx<'_>,
    types: &BTreeMap<String, String>,
    injected: &InjectedMap,
    call: Node<'_>,
    function: Node<'_>,
    callee: &Callee,
) -> Option<Access> {
    let method = callee.name.as_str();
    if callee.receiver.is_empty() {
        return None;
    }
    // Query-builder chains: only the terminal call touches the database.
    if callee.chain_has_call("createQueryBuilder") {
        let op = if BUILDER_READS.contains(&method) {
            "read"
        } else if method == "execute"
            && BUILDER_WRITE_STEPS
                .iter()
                .any(|step| callee.chain_has_call(step))
        {
            "write"
        } else {
            return None;
        };
        let index = callee
            .receiver
            .iter()
            .position(|seg| seg == "createQueryBuilder()")?;
        let base = callee
            .receiver
            .get(index.checked_sub(1)?)?
            .trim_end_matches("()");
        let base_type = types.get(base);
        if !(base_type.is_some_and(|t| is_db_type(t)) || is_db_name(base)) {
            return None;
        }
        let from_builder = builder_entity(ctx, call).map(|e| (e, 0.8));
        let from_type = base_type.and_then(|t| generic_arg(t)).map(|e| (e, 0.9));
        let from_injected = injected_entity(ctx, injected, call, callee, base);
        let entity = from_builder.or(from_type).or(from_injected);
        return Some(Access {
            op,
            via: "query_builder",
            confidence: if entity.is_some() { 0.75 } else { 0.4 },
            entity: entity.map(|(e, _)| e),
            receiver_type: base_type.cloned(),
            table_hint: None,
        });
    }

    let receiver_name = callee.receiver_name()?;
    let direct_repo = get_repository_entity(ctx, function);
    let recv_type = types.get(receiver_name);
    let typed = recv_type.is_some_and(|t| is_db_type(t));
    let named = is_db_name(receiver_name);
    if !(typed || named || direct_repo.is_some()) {
        return None;
    }
    let head = recv_type
        .map(|t| type_head(t).to_ascii_lowercase())
        .unwrap_or_default();
    let lower_name = receiver_name.to_ascii_lowercase();
    let is_manager = head.ends_with("entitymanager")
        || head.ends_with("datasource")
        || head.ends_with("queryrunner")
        || lower_name == "em"
        || lower_name.ends_with("manager")
        || lower_name.contains("datasource")
        || lower_name.contains("queryrunner");

    if method == "query" {
        let sql = arguments_of(call)
            .into_iter()
            .next()
            .and_then(|a| node_string(a, ctx.src))?;
        let op = match sql_access(&sql)? {
            DbAccess::Write => "write",
            DbAccess::Read => "read",
        };
        return Some(Access {
            op,
            via: "raw_sql",
            entity: None,
            receiver_type: recv_type.cloned(),
            table_hint: table_hint(&sql),
            confidence: 0.6,
        });
    }
    let op = if WRITES.contains(&method) {
        "write"
    } else if READS.contains(&method) {
        "read"
    } else {
        return None;
    };
    let (via, base_conf) = if is_manager && direct_repo.is_none() {
        ("manager", if typed { 0.85 } else { 0.75 })
    } else {
        ("repository", if typed { 0.9 } else { 0.75 })
    };

    let mut entity: Option<String> = None;
    if let Some(e) = recv_type.and_then(|t| generic_arg(t)) {
        entity = Some(e);
    } else if let Some(e) = injected_entity(ctx, injected, call, callee, receiver_name) {
        entity = Some(e.0);
    } else if let Some(e) = direct_repo {
        entity = Some(e);
    } else if via == "manager" {
        entity = arguments_of(call)
            .into_iter()
            .next()
            .and_then(|a| class_ident(&to_ir_expr(a, ctx.src)));
    }
    Some(Access {
        op,
        via,
        confidence: if entity.is_some() { base_conf } else { 0.4 },
        entity,
        receiver_type: recv_type.cloned(),
        table_hint: None,
    })
}

/// The entity of an injected repository property (`this.users` with `@InjectRepository(User)`).
fn injected_entity(
    ctx: &FrameworkCtx<'_>,
    injected: &InjectedMap,
    call: Node<'_>,
    callee: &Callee,
    property: &str,
) -> Option<(String, f32)> {
    if callee.receiver.first().map(String::as_str) != Some("this") {
        return None;
    }
    let owner = ctx.owner(ctx.src.range(call).start);
    let class = ctx.class_of(owner)?;
    let token = injected.get(&(class.local_id, property.to_owned()))?;
    token
        .entity
        .clone()
        .filter(|e| !e.is_empty())
        .map(|e| (e, 0.9))
}

/// Table names after `FROM`, `JOIN`, `INTO`, `UPDATE` and `TABLE`; the first one.
fn table_hint(sql: &str) -> Option<String> {
    let words: Vec<&str> = sql
        .split(|c: char| c.is_whitespace() || matches!(c, ',' | '(' | ')' | ';'))
        .filter(|w| !w.is_empty())
        .collect();
    for (i, word) in words.iter().enumerate() {
        let is_keyword = ["from", "join", "into", "update", "table"]
            .iter()
            .any(|k| word.eq_ignore_ascii_case(k));
        if !is_keyword {
            continue;
        }
        let Some(next) = words.get(i + 1) else {
            continue;
        };
        let candidate = next.trim_matches(|c| matches!(c, '"' | '`' | '\'' | '[' | ']'));
        if candidate.is_empty()
            || candidate.starts_with('$')
            || ["if", "only", "select"]
                .iter()
                .any(|k| candidate.eq_ignore_ascii_case(k))
        {
            continue;
        }
        return Some(candidate.to_owned());
    }
    None
}
