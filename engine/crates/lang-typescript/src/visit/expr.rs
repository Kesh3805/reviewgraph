//! Conversion of expression nodes into the bounded [`IrExpr`] tree.

use analysis_ir::IrExpr;
use tree_sitter::Node;

use crate::kinds::{field, kind};
use crate::text::Source;

/// Hard recursion guard, independent of the IR limits (which are applied afterwards).
const MAX_CONVERT_DEPTH: usize = 12;

/// Collapses whitespace runs to single spaces and trims.
pub fn collapse_ws(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut pending_space = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            pending_space = !out.is_empty();
        } else {
            if pending_space {
                out.push(' ');
                pending_space = false;
            }
            out.push(ch);
        }
    }
    out
}

/// Truncates at a char boundary to at most `max_chars` characters.
pub fn truncate_chars(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

/// The content of a `string` node (between the quotes), escapes preserved.
pub fn string_content(node: Node<'_>, src: &Source<'_>) -> String {
    let mut out = String::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind::STRING_FRAGMENT || child.kind() == "escape_sequence" {
            out.push_str(&src.text(child));
        }
    }
    out
}

/// Dotted identifier chain of a member expression (`a.b.c`), or `None` when any part is not a
/// plain identifier / `this` / `super`.
pub fn member_chain(node: Node<'_>, src: &Source<'_>) -> Option<Vec<String>> {
    match node.kind() {
        kind::IDENTIFIER | kind::THIS | kind::SUPER | kind::PROPERTY_IDENTIFIER => {
            Some(vec![src.text(node).into_owned()])
        }
        kind::MEMBER_EXPRESSION => {
            let mut parts = member_chain(node.child_by_field_name(field::OBJECT)?, src)?;
            let property = node.child_by_field_name(field::PROPERTY)?;
            parts.push(src.text(property).into_owned());
            Some(parts)
        }
        kind::NESTED_IDENTIFIER | kind::NESTED_TYPE_IDENTIFIER => {
            let mut parts = Vec::new();
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                parts.extend(member_chain(child, src)?);
            }
            Some(parts)
        }
        kind::TYPE_IDENTIFIER => Some(vec![src.text(node).into_owned()]),
        kind::NON_NULL_EXPRESSION | kind::PARENTHESIZED_EXPRESSION => {
            member_chain(node.named_child(0)?, src)
        }
        _ => None,
    }
}

fn is_const_assertion(node: Node<'_>, src: &Source<'_>) -> bool {
    node.kind() == kind::AS_EXPRESSION
        && node
            .named_child(1)
            .is_some_and(|t| src.text(t).trim() == "const")
}

/// Converts `node` into a bounded expression tree.
pub fn to_ir_expr(node: Node<'_>, src: &Source<'_>) -> IrExpr {
    convert(node, src, 0).bounded()
}

fn convert(node: Node<'_>, src: &Source<'_>, depth: usize) -> IrExpr {
    if depth > MAX_CONVERT_DEPTH {
        return IrExpr::truncated();
    }
    match node.kind() {
        kind::STRING => IrExpr::Str(string_content(node, src)),
        kind::NUMBER => IrExpr::Num(src.text(node).replace('_', "").to_lowercase()),
        kind::TRUE => IrExpr::Bool(true),
        kind::FALSE => IrExpr::Bool(false),
        kind::NULL => IrExpr::Null,
        kind::UNDEFINED => IrExpr::Ident("undefined".to_owned()),
        kind::IDENTIFIER | kind::SHORTHAND_PROPERTY_IDENTIFIER | kind::THIS => {
            IrExpr::Ident(src.text(node).into_owned())
        }
        kind::MEMBER_EXPRESSION => match member_chain(node, src) {
            Some(parts) => IrExpr::Member(parts),
            None => other(node, src),
        },
        kind::PARENTHESIZED_EXPRESSION | kind::NON_NULL_EXPRESSION | kind::SATISFIES_EXPRESSION => {
            node.named_child(0)
                .map(|inner| convert(inner, src, depth + 1))
                .unwrap_or_else(|| other(node, src))
        }
        kind::AS_EXPRESSION => {
            // `x as const` and `x as T` are transparent: the value is what matters.
            let _ = is_const_assertion(node, src);
            node.named_child(0)
                .map(|inner| convert(inner, src, depth + 1))
                .unwrap_or_else(|| other(node, src))
        }
        kind::ARRAY => {
            let mut cursor = node.walk();
            IrExpr::Array(
                node.named_children(&mut cursor)
                    .filter(|c| c.kind() != kind::COMMENT)
                    .map(|c| convert(c, src, depth + 1))
                    .collect(),
            )
        }
        kind::OBJECT => {
            let mut pairs = Vec::new();
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                match child.kind() {
                    kind::PAIR => {
                        let key = child
                            .child_by_field_name(field::KEY)
                            .map(|k| key_text(k, src))
                            .unwrap_or_default();
                        let value = child
                            .child_by_field_name(field::VALUE)
                            .map(|v| convert(v, src, depth + 1))
                            .unwrap_or_else(IrExpr::truncated);
                        pairs.push((key, value));
                    }
                    kind::SHORTHAND_PROPERTY_IDENTIFIER => {
                        let name = src.text(child).into_owned();
                        pairs.push((name.clone(), IrExpr::Ident(name)));
                    }
                    kind::SPREAD_ELEMENT => {
                        let value = child
                            .named_child(0)
                            .map(|v| convert(v, src, depth + 1))
                            .unwrap_or_else(IrExpr::truncated);
                        pairs.push(("...".to_owned(), value));
                    }
                    kind::METHOD_DEFINITION => {
                        let name = child
                            .child_by_field_name(field::NAME)
                            .map(|k| key_text(k, src))
                            .unwrap_or_default();
                        pairs.push((name, IrExpr::Other("<method>".to_owned())));
                    }
                    _ => {}
                }
            }
            IrExpr::Object(pairs)
        }
        kind::CALL_EXPRESSION => {
            let callee = node
                .child_by_field_name(field::FUNCTION)
                .and_then(|f| member_chain(f, src));
            let Some(callee) = callee else {
                return other(node, src);
            };
            let mut args = Vec::new();
            if let Some(arguments) = node.child_by_field_name(field::ARGUMENTS) {
                let mut cursor = arguments.walk();
                for arg in arguments.named_children(&mut cursor) {
                    if arg.kind() != kind::COMMENT {
                        args.push(convert(arg, src, depth + 1));
                    }
                }
            }
            IrExpr::Call { callee, args }
        }
        kind::ARROW_FUNCTION => match node.child_by_field_name(field::BODY) {
            Some(body) if body.kind() != kind::STATEMENT_BLOCK => IrExpr::Arrow {
                returns: Box::new(convert(body, src, depth + 1)),
            },
            _ => other(node, src),
        },
        kind::TEMPLATE_STRING => {
            let mut has_subst = false;
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if child.kind() == kind::TEMPLATE_SUBSTITUTION {
                    has_subst = true;
                }
            }
            let raw = src.text(node).trim_matches('`').to_owned();
            IrExpr::Template { raw, has_subst }
        }
        _ => other(node, src),
    }
}

fn other(node: Node<'_>, src: &Source<'_>) -> IrExpr {
    IrExpr::Other(truncate_chars(&collapse_ws(&src.text(node)), 120))
}

/// Text of an object key node (`a`, `'a-b'`, `42`).
pub fn key_text(node: Node<'_>, src: &Source<'_>) -> String {
    match node.kind() {
        kind::STRING => string_content(node, src),
        _ => src.text(node).into_owned(),
    }
}
