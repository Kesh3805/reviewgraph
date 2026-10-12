//! Configuration reads: `process.env` and Nest `ConfigService` (NEST-007).
//!
//! Only names are recorded. Defaults, values and keys built from other text are dropped.

use std::collections::{BTreeMap, BTreeSet};

use analysis_ir::{AttrValue, FrameworkFactKind, LocalId};
use tree_sitter::Node;

use super::util::{arguments_of, node_string, nodes_of_kinds, receiver_types, s};
use super::FrameworkCtx;
use crate::kinds::{field, kind};
use crate::visit::db_heuristics::type_head;
use crate::visit::expr::{key_text, member_chain, string_content};
use crate::visit::fact_keys::callee_of;

pub const ADAPTER: &str = "config";

/// Receiver names that are assumed to be a `ConfigService` when no type is known.
const CONFIG_NAMES: [&str; 3] = ["configService", "config", "cfg"];

fn contains(bytes: &[u8], needle: &[u8]) -> bool {
    bytes.windows(needle.len()).any(|w| w == needle)
}

/// The file mentions `process.env` or a config service.
pub fn detect(ctx: &FrameworkCtx<'_>) -> bool {
    let needles: [&[u8]; 3] = [b"process.env", b"ConfigService", b"configService"];
    needles.iter().any(|n| contains(ctx.src.bytes, n))
}

/// `^[A-Za-z0-9_.:-]{1,128}$`
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '-'))
}

/// `^[A-Z][A-Z0-9_]*$`
fn is_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_uppercase())
        && chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

fn is_process_env(node: Node<'_>, ctx: &FrameworkCtx<'_>) -> bool {
    node.kind() == kind::MEMBER_EXPRESSION
        && member_chain(node, ctx.src)
            .is_some_and(|c| c.len() == 2 && c[0] == "process" && c[1] == "env")
}

struct Read {
    source: &'static str,
    name: String,
    has_default: bool,
    or_throw: bool,
    type_arg: Option<String>,
    confidence: f32,
}

fn emit(ctx: &mut FrameworkCtx<'_>, node: Node<'_>, read: &Read) {
    if !valid_name(&read.name) {
        return;
    }
    let range = ctx.src.range(node);
    let owner = ctx.owner(range.start);
    let mut attrs = BTreeMap::new();
    attrs.insert("symbol".to_owned(), AttrValue::Str(ctx.qname(owner)));
    attrs.insert("source".to_owned(), s(read.source));
    attrs.insert("name".to_owned(), AttrValue::Str(read.name.clone()));
    attrs.insert(
        "is_env_name".to_owned(),
        AttrValue::Bool(read.source == "process_env" || is_env_name(&read.name)),
    );
    attrs.insert("has_default".to_owned(), AttrValue::Bool(read.has_default));
    attrs.insert("or_throw".to_owned(), AttrValue::Bool(read.or_throw));
    if let Some(t) = &read.type_arg {
        attrs.insert("declared_type_arg".to_owned(), AttrValue::Str(t.clone()));
    }
    ctx.emit(
        ADAPTER,
        FrameworkFactKind::ConfigRead,
        Some(owner),
        attrs,
        range,
        read.confidence,
    );
}

/// `process.env.X || 'd'` and `process.env.X ?? 'd'`.
fn has_fallback(node: Node<'_>, ctx: &FrameworkCtx<'_>) -> bool {
    let Some(parent) = node.parent() else {
        return false;
    };
    if parent.kind() != kind::BINARY_EXPRESSION {
        return false;
    }
    let left_is_node = parent
        .child_by_field_name(field::LEFT)
        .is_some_and(|l| l.id() == node.id());
    let operator = parent
        .child_by_field_name(field::OPERATOR)
        .map(|o| ctx.src.text(o).into_owned())
        .unwrap_or_default();
    left_is_node && (operator == "||" || operator == "??")
}

fn is_assignment_target(node: Node<'_>) -> bool {
    node.parent().is_some_and(|p| {
        p.kind() == kind::ASSIGNMENT_EXPRESSION
            && p.child_by_field_name(field::LEFT)
                .is_some_and(|l| l.id() == node.id())
    })
}

pub fn run(ctx: &mut FrameworkCtx<'_>) {
    process_env(ctx);
    config_service(ctx);
}

fn process_env(ctx: &mut FrameworkCtx<'_>) {
    // `const env = process.env;` aliases, by owning symbol.
    let mut aliases: BTreeSet<(LocalId, String)> = BTreeSet::new();
    let declarators = nodes_of_kinds(ctx.root, &[kind::VARIABLE_DECLARATOR]);
    for declarator in &declarators {
        let (Some(name), Some(value)) = (
            declarator.child_by_field_name(field::NAME),
            declarator.child_by_field_name(field::VALUE),
        ) else {
            continue;
        };
        if name.kind() == kind::IDENTIFIER && is_process_env(value, ctx) {
            let owner = ctx.owner(ctx.src.range(*declarator).start);
            aliases.insert((owner, ctx.src.text(name).into_owned()));
        }
    }

    for node in nodes_of_kinds(
        ctx.root,
        &[kind::MEMBER_EXPRESSION, kind::SUBSCRIPT_EXPRESSION],
    ) {
        let Some(object) = node.child_by_field_name(field::OBJECT) else {
            continue;
        };
        let direct = is_process_env(object, ctx);
        let aliased = object.kind() == kind::IDENTIFIER
            && aliases.contains(&(
                ctx.owner(ctx.src.range(node).start),
                ctx.src.text(object).into_owned(),
            ));
        if !(direct || aliased) || is_assignment_target(node) {
            continue;
        }
        let name = if node.kind() == kind::MEMBER_EXPRESSION {
            node.child_by_field_name(field::PROPERTY)
                .map(|p| ctx.src.text(p).into_owned())
        } else {
            node.child_by_field_name(field::INDEX)
                .filter(|i| i.kind() == kind::STRING)
                .map(|i| string_content(i, ctx.src))
        };
        let Some(name) = name else { continue };
        let read = Read {
            source: "process_env",
            name,
            has_default: has_fallback(node, ctx),
            or_throw: false,
            type_arg: None,
            confidence: 0.95,
        };
        emit(ctx, node, &read);
    }

    for declarator in declarators {
        let (Some(name), Some(value)) = (
            declarator.child_by_field_name(field::NAME),
            declarator.child_by_field_name(field::VALUE),
        ) else {
            continue;
        };
        if name.kind() != kind::OBJECT_PATTERN || !is_process_env(value, ctx) {
            continue;
        }
        let mut cursor = name.walk();
        let entries: Vec<Node<'_>> = name.named_children(&mut cursor).collect();
        for entry in entries {
            let (key, has_default) = match entry.kind() {
                kind::SHORTHAND_PROPERTY_IDENTIFIER_PATTERN => {
                    (Some(ctx.src.text(entry).into_owned()), false)
                }
                kind::PAIR_PATTERN => (
                    entry
                        .child_by_field_name(field::KEY)
                        .map(|k| key_text(k, ctx.src)),
                    false,
                ),
                kind::OBJECT_ASSIGNMENT_PATTERN => (
                    entry
                        .child_by_field_name(field::LEFT)
                        .map(|k| ctx.src.text(k).into_owned()),
                    true,
                ),
                _ => (None, false),
            };
            let Some(key) = key else { continue };
            let read = Read {
                source: "process_env",
                name: key,
                has_default,
                or_throw: false,
                type_arg: None,
                confidence: 0.95,
            };
            emit(ctx, entry, &read);
        }
    }
}

fn config_service(ctx: &mut FrameworkCtx<'_>) {
    let types = receiver_types(ctx.symbols);
    for call in nodes_of_kinds(ctx.root, &[kind::CALL_EXPRESSION]) {
        let Some(function) = call.child_by_field_name(field::FUNCTION) else {
            continue;
        };
        let Some(callee) = callee_of(function, ctx.src) else {
            continue;
        };
        if callee.name != "get" && callee.name != "getOrThrow" {
            continue;
        }
        let Some(receiver) = callee.receiver_name() else {
            continue;
        };
        if callee.receiver.len() > 2 {
            continue;
        }
        let typed = types
            .get(receiver)
            .is_some_and(|t| type_head(t) == "ConfigService");
        let named = CONFIG_NAMES.contains(&receiver);
        if !(typed || named) {
            continue;
        }
        let args = arguments_of(call);
        let Some(key) = args.first().and_then(|a| node_string(*a, ctx.src)) else {
            continue;
        };
        let type_arg = call
            .child_by_field_name(field::TYPE_ARGUMENTS)
            .map(|t| {
                ctx.src
                    .text(t)
                    .trim_matches(|c| c == '<' || c == '>')
                    .trim()
                    .to_owned()
            })
            .filter(|t| !t.is_empty());
        let namespaced = key.contains('.');
        let read = Read {
            source: if namespaced {
                "config_service_namespaced"
            } else {
                "config_service"
            },
            name: key,
            has_default: args.len() > 1,
            or_throw: callee.name == "getOrThrow",
            type_arg,
            confidence: if typed { 0.9 } else { 0.7 },
        };
        emit(ctx, call, &read);
    }
}
