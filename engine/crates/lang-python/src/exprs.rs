//! Bounded literal expressions and small node helpers shared by the extraction passes.

use analysis_ir::symbol::{MAX_EXPR_DEPTH, MAX_EXPR_OTHER_CHARS};
use analysis_ir::IrExpr;
use tree_sitter::Node;

use crate::text::Source;

/// Named children that are not comments.
pub fn children_of(node: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .filter(|c| c.kind() != "comment")
        .collect()
}

/// Collapses runs of whitespace into single spaces.
pub fn collapse_ws(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The dotted path of an identifier or attribute chain (`a.b.c`), or `None` for anything else.
pub fn dotted(node: Node<'_>, src: &Source<'_>) -> Option<Vec<String>> {
    match node.kind() {
        "identifier" => Some(vec![src.text(node).into_owned()]),
        "attribute" => {
            let object = node.child_by_field_name("object")?;
            let attribute = node.child_by_field_name("attribute")?;
            let mut parts = dotted(object, src)?;
            parts.push(src.text(attribute).into_owned());
            Some(parts)
        }
        _ => None,
    }
}

/// The literal value of a plain string (no interpolation), or `None`.
pub fn string_value(node: Node<'_>, src: &Source<'_>) -> Option<String> {
    if node.kind() != "string" {
        return None;
    }
    let mut out = String::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "string_content" => out.push_str(&src.text(child)),
            "interpolation" => return None,
            _ => {}
        }
    }
    Some(out)
}

/// The positional arguments of a call as expressions, with keyword arguments gathered into one
/// trailing `Object`.
pub fn call_args(call: Node<'_>, src: &Source<'_>, depth: usize) -> Vec<IrExpr> {
    let Some(arguments) = call.child_by_field_name("arguments") else {
        return Vec::new();
    };
    if arguments.kind() != "argument_list" {
        return Vec::new();
    }
    let mut positional = Vec::new();
    let mut keywords = Vec::new();
    for arg in children_of(arguments) {
        if arg.kind() == "keyword_argument" {
            let name = arg
                .child_by_field_name("name")
                .map(|n| src.text(n).into_owned())
                .unwrap_or_default();
            if let Some(value) = arg.child_by_field_name("value") {
                keywords.push((name, expr(value, src, depth + 1)));
            }
        } else {
            positional.push(expr(arg, src, depth + 1));
        }
    }
    if !keywords.is_empty() {
        positional.push(IrExpr::Object(keywords));
    }
    positional
}

/// Converts a literal-ish expression into the bounded IR tree.
pub fn expr(node: Node<'_>, src: &Source<'_>, depth: usize) -> IrExpr {
    if depth > MAX_EXPR_DEPTH {
        return IrExpr::truncated();
    }
    let out = match node.kind() {
        "string" => match string_value(node, src) {
            Some(value) => IrExpr::Str(value),
            None => IrExpr::Template {
                raw: src.text(node).into_owned(),
                has_subst: true,
            },
        },
        "integer" | "float" => IrExpr::Num(src.text(node).into_owned()),
        "true" => IrExpr::Bool(true),
        "false" => IrExpr::Bool(false),
        "none" => IrExpr::Null,
        "identifier" => IrExpr::Ident(src.text(node).into_owned()),
        "attribute" => match dotted(node, src) {
            Some(parts) => IrExpr::Member(parts),
            None => other(node, src),
        },
        "list" | "tuple" | "set" => IrExpr::Array(
            children_of(node)
                .into_iter()
                .map(|c| expr(c, src, depth + 1))
                .collect(),
        ),
        "dictionary" => {
            let mut pairs = Vec::new();
            for pair in children_of(node) {
                if pair.kind() != "pair" {
                    continue;
                }
                let (Some(key), Some(value)) = (
                    pair.child_by_field_name("key"),
                    pair.child_by_field_name("value"),
                ) else {
                    continue;
                };
                let key_text = string_value(key, src).unwrap_or_else(|| src.text(key).into_owned());
                pairs.push((key_text, expr(value, src, depth + 1)));
            }
            IrExpr::Object(pairs)
        }
        "call" => {
            let callee = node
                .child_by_field_name("function")
                .and_then(|f| dotted(f, src))
                .unwrap_or_else(|| vec!["?".to_owned()]);
            IrExpr::Call {
                callee,
                args: call_args(node, src, depth),
            }
        }
        "lambda" => match node.child_by_field_name("body") {
            Some(body) => IrExpr::Arrow {
                returns: Box::new(expr(body, src, depth + 1)),
            },
            None => other(node, src),
        },
        "parenthesized_expression" => match children_of(node).first() {
            Some(inner) => expr(*inner, src, depth + 1),
            None => other(node, src),
        },
        _ => other(node, src),
    };
    out.bounded()
}

fn other(node: Node<'_>, src: &Source<'_>) -> IrExpr {
    IrExpr::Other(
        collapse_ws(&src.text(node))
            .chars()
            .take(MAX_EXPR_OTHER_CHARS)
            .collect(),
    )
}
