//! `@Entity`, column and relation facts (NEST-005).

use std::collections::BTreeMap;

use analysis_ir::{AttrValue, FrameworkFactKind, IrDecorator, IrExpr, IrSymbol};
use review_core::symbol::SymbolKind;

use super::{snake_case, ADAPTER, PACKAGE};
use crate::frameworks::util::{find_key, last_segment, name_of, s};
use crate::frameworks::FrameworkCtx;

const ENTITY_DECORATORS: [(&str, &str); 3] = [
    ("Entity", "entity"),
    ("ViewEntity", "view"),
    ("ChildEntity", "child"),
];

/// `(decorator, kind, primary, generated)`.
const COLUMN_DECORATORS: [(&str, &str, bool, bool); 8] = [
    ("Column", "column", false, false),
    ("PrimaryColumn", "primary", true, false),
    ("PrimaryGeneratedColumn", "primary", true, true),
    ("CreateDateColumn", "created_at", false, false),
    ("UpdateDateColumn", "updated_at", false, false),
    ("DeleteDateColumn", "deleted_at", false, false),
    ("VersionColumn", "version", false, false),
    ("Index", "index", false, false),
];

const RELATION_DECORATORS: [(&str, &str); 4] = [
    ("OneToOne", "one_to_one"),
    ("OneToMany", "one_to_many"),
    ("ManyToOne", "many_to_one"),
    ("ManyToMany", "many_to_many"),
];

fn origin(ctx: &FrameworkCtx<'_>, d: &IrDecorator, wanted: &str) -> Option<f32> {
    ctx.name_origin(&d.name, &[PACKAGE], wanted)
}

fn options_of(args: &[IrExpr]) -> &[(String, IrExpr)] {
    args.iter()
        .find_map(|a| match a {
            IrExpr::Object(pairs) => Some(pairs.as_slice()),
            _ => None,
        })
        .unwrap_or(&[])
}

fn bool_opt(options: &[(String, IrExpr)], key: &str) -> bool {
    matches!(find_key(options, key), Some(IrExpr::Bool(true)))
}

pub fn collect(ctx: &mut FrameworkCtx<'_>) {
    let symbols = ctx.symbols;
    for class in symbols.iter().filter(|c| c.kind == SymbolKind::Class) {
        entity_fact(ctx, class);
    }
    for property in symbols
        .iter()
        .filter(|p| matches!(p.kind, SymbolKind::Property | SymbolKind::Field))
    {
        let Some(class) = property.parent.and_then(|p| symbols.get(p.0 as usize)) else {
            continue;
        };
        column_facts(ctx, class, property);
        relation_facts(ctx, class, property);
    }
}

fn entity_fact(ctx: &mut FrameworkCtx<'_>, class: &IrSymbol) {
    let Some((decorator, entity_kind, conf)) = class.decorators.iter().find_map(|d| {
        ENTITY_DECORATORS
            .iter()
            .find_map(|(wanted, kind)| origin(ctx, d, wanted).map(|c| (d, *kind, c)))
    }) else {
        return;
    };
    let mut name: Option<String> = None;
    let mut schema: Option<String> = None;
    let args = &decorator.args;
    if entity_kind != "child" {
        match args.first() {
            Some(IrExpr::Object(pairs)) => {
                name = find_key(pairs, "name").and_then(|v| ctx.const_str(v));
                schema = find_key(pairs, "schema").and_then(|v| ctx.const_str(v));
            }
            Some(first) => {
                name = ctx.const_str(first);
                if let Some(IrExpr::Object(pairs)) = args.get(1) {
                    schema = find_key(pairs, "schema").and_then(|v| ctx.const_str(v));
                }
            }
            None => {}
        }
    }
    let extends = class
        .heritage
        .extends
        .first()
        .map(|e| e.split(['<', '(']).next().unwrap_or("").trim().to_owned())
        .filter(|e| !e.is_empty());
    let inferred = name.is_none();
    let table = match (&name, entity_kind, &extends) {
        (Some(n), _, _) => n.clone(),
        (None, "child", Some(parent)) => snake_case(parent),
        _ => snake_case(&class.name),
    };
    let mut attrs = BTreeMap::new();
    attrs.insert(
        "symbol".to_owned(),
        AttrValue::Str(ctx.qname(class.local_id)),
    );
    attrs.insert("entity".to_owned(), AttrValue::Str(class.name.clone()));
    attrs.insert("table".to_owned(), AttrValue::Str(table));
    if let Some(schema) = schema {
        attrs.insert("schema".to_owned(), AttrValue::Str(schema));
    }
    attrs.insert("table_inferred".to_owned(), AttrValue::Bool(inferred));
    attrs.insert("kind".to_owned(), s(entity_kind));
    if let Some(extends) = extends {
        attrs.insert("extends".to_owned(), AttrValue::Str(extends));
    }
    ctx.emit(
        ADAPTER,
        FrameworkFactKind::OrmEntity,
        Some(class.local_id),
        attrs,
        decorator.range,
        if inferred { 0.7f32.min(conf) } else { conf },
    );
}

fn column_facts(ctx: &mut FrameworkCtx<'_>, class: &IrSymbol, property: &IrSymbol) {
    for decorator in &property.decorators {
        let Some((kind, primary, generated, conf)) =
            COLUMN_DECORATORS
                .iter()
                .find_map(|(wanted, kind, primary, generated)| {
                    origin(ctx, decorator, wanted).map(|c| (*kind, *primary, *generated, c))
                })
        else {
            continue;
        };
        let options = options_of(&decorator.args);
        let column_type = match decorator.args.first() {
            Some(IrExpr::Str(t)) if kind != "index" => Some(t.clone()),
            _ => find_key(options, "type").and_then(|t| match t {
                IrExpr::Str(v) => Some(v.clone()),
                other => last_segment(other),
            }),
        };
        let mut attrs = BTreeMap::new();
        attrs.insert(
            "symbol".to_owned(),
            AttrValue::Str(ctx.qname(property.local_id)),
        );
        attrs.insert("entity".to_owned(), AttrValue::Str(class.name.clone()));
        attrs.insert("property".to_owned(), AttrValue::Str(property.name.clone()));
        let column = find_key(options, "name")
            .and_then(|v| ctx.const_str(v))
            .unwrap_or_else(|| property.name.clone());
        attrs.insert("column".to_owned(), AttrValue::Str(column));
        if let Some(t) = column_type {
            attrs.insert("column_type".to_owned(), AttrValue::Str(t));
        }
        attrs.insert(
            "primary".to_owned(),
            AttrValue::Bool(primary || bool_opt(options, "primary")),
        );
        attrs.insert(
            "generated".to_owned(),
            AttrValue::Bool(generated || bool_opt(options, "generated")),
        );
        attrs.insert(
            "nullable".to_owned(),
            AttrValue::Bool(bool_opt(options, "nullable")),
        );
        attrs.insert(
            "unique".to_owned(),
            AttrValue::Bool(bool_opt(options, "unique")),
        );
        if let Some(IrExpr::Num(n)) = find_key(options, "length") {
            if let Ok(v) = n.parse::<i64>() {
                attrs.insert("length".to_owned(), AttrValue::Int(v));
            }
        }
        attrs.insert("kind".to_owned(), s(kind));
        ctx.emit(
            ADAPTER,
            FrameworkFactKind::OrmColumn,
            Some(property.local_id),
            attrs,
            decorator.range,
            conf,
        );
    }
}

fn relation_facts(ctx: &mut FrameworkCtx<'_>, class: &IrSymbol, property: &IrSymbol) {
    let join_column = property
        .decorators
        .iter()
        .find(|d| origin(ctx, d, "JoinColumn").is_some());
    let join_table = property
        .decorators
        .iter()
        .find(|d| origin(ctx, d, "JoinTable").is_some());
    for decorator in &property.decorators {
        let Some((relation, conf)) = RELATION_DECORATORS
            .iter()
            .find_map(|(wanted, relation)| origin(ctx, decorator, wanted).map(|c| (*relation, c)))
        else {
            continue;
        };
        let mut attrs = BTreeMap::new();
        attrs.insert(
            "symbol".to_owned(),
            AttrValue::Str(ctx.qname(property.local_id)),
        );
        attrs.insert("entity".to_owned(), AttrValue::Str(class.name.clone()));
        attrs.insert("property".to_owned(), AttrValue::Str(property.name.clone()));
        attrs.insert("relation".to_owned(), s(relation));
        let target = match decorator.args.first() {
            Some(IrExpr::Arrow { returns }) => name_of(returns),
            Some(IrExpr::Str(t)) => t.clone(),
            _ => String::new(),
        };
        if !target.is_empty() {
            attrs.insert("target".to_owned(), AttrValue::Str(target));
        }
        if let Some(IrExpr::Arrow { returns }) = decorator.args.get(1) {
            if let IrExpr::Member(parts) = returns.as_ref() {
                if let Some(inverse) = parts.last() {
                    attrs.insert(
                        "inverse_property".to_owned(),
                        AttrValue::Str(inverse.clone()),
                    );
                }
            }
        }
        let options = options_of(&decorator.args);
        match find_key(options, "cascade") {
            Some(IrExpr::Bool(b)) => {
                attrs.insert("cascade".to_owned(), AttrValue::Bool(*b));
            }
            Some(IrExpr::Array(items)) => {
                attrs.insert(
                    "cascade".to_owned(),
                    AttrValue::List(
                        items
                            .iter()
                            .filter_map(|i| match i {
                                IrExpr::Str(v) => Some(AttrValue::Str(v.clone())),
                                _ => None,
                            })
                            .collect(),
                    ),
                );
            }
            _ => {}
        }
        attrs.insert(
            "eager".to_owned(),
            AttrValue::Bool(bool_opt(options, "eager")),
        );
        attrs.insert(
            "nullable".to_owned(),
            AttrValue::Bool(bool_opt(options, "nullable")),
        );
        if let Some(join) = join_column {
            attrs.insert("join_column".to_owned(), AttrValue::Bool(true));
            if let Some(name) = options_of(&join.args)
                .iter()
                .find(|(k, _)| k == "name")
                .and_then(|(_, v)| ctx.const_str(v))
            {
                attrs.insert("join_column_name".to_owned(), AttrValue::Str(name));
            }
        }
        if let Some(join) = join_table {
            attrs.insert("join_table".to_owned(), AttrValue::Bool(true));
            if let Some(name) = options_of(&join.args)
                .iter()
                .find(|(k, _)| k == "name")
                .and_then(|(_, v)| ctx.const_str(v))
            {
                attrs.insert("join_table_name".to_owned(), AttrValue::Str(name));
            }
        }
        ctx.emit(
            ADAPTER,
            FrameworkFactKind::OrmRelation,
            Some(property.local_id),
            attrs,
            decorator.range,
            conf,
        );
    }
}
