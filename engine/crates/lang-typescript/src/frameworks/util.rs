//! Small helpers shared by the framework adapters.

use std::collections::BTreeMap;

use analysis_ir::{AttrValue, IrExpr, IrSymbol};
use review_core::symbol::SymbolKind;
use tree_sitter::Node;

use crate::kinds::{field, kind};

/// Nodes visited by [`nodes_of_kinds`] before it gives up on a pathological tree.
const MAX_SCAN_NODES: usize = 400_000;

pub fn s(text: &str) -> AttrValue {
    AttrValue::Str(text.to_owned())
}

pub fn str_list<I: IntoIterator<Item = String>>(items: I) -> AttrValue {
    AttrValue::List(items.into_iter().map(AttrValue::Str).collect())
}

/// Every node below (and including) `root` whose kind is in `kinds`, in source order. Iterative,
/// so no input can overflow the stack.
pub fn nodes_of_kinds<'t>(root: Node<'t>, kinds: &[&str]) -> Vec<Node<'t>> {
    let mut out = Vec::new();
    let mut cursor = root.walk();
    let mut visited = 0usize;
    'outer: loop {
        let node = cursor.node();
        visited += 1;
        if visited > MAX_SCAN_NODES {
            break;
        }
        if kinds.contains(&node.kind()) {
            out.push(node);
        }
        if cursor.goto_first_child() {
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                break 'outer;
            }
        }
    }
    out
}

/// The non-comment arguments of a call expression.
pub fn arguments_of(call: Node<'_>) -> Vec<Node<'_>> {
    let Some(arguments) = call.child_by_field_name(field::ARGUMENTS) else {
        return Vec::new();
    };
    let mut cursor = arguments.walk();
    let args: Vec<Node<'_>> = arguments
        .named_children(&mut cursor)
        .filter(|c| c.kind() != kind::COMMENT)
        .collect();
    args
}

/// Short display text of an expression, at most 60 characters.
pub fn expr_text(expr: &IrExpr) -> String {
    let text = match expr {
        IrExpr::Ident(n) => n.clone(),
        IrExpr::Member(parts) => parts.join("."),
        IrExpr::Str(v) => format!("'{v}'"),
        IrExpr::Num(n) => n.clone(),
        IrExpr::Template { raw, .. } => format!("`{raw}`"),
        IrExpr::Call { callee, .. } => format!("{}(...)", callee.join(".")),
        IrExpr::Other(t) => t.clone(),
        other => format!("{other:?}"),
    };
    text.chars().take(60).collect()
}

/// Identifier text of an identifier or member expression, empty for anything else.
pub fn name_of(expr: &IrExpr) -> String {
    match expr {
        IrExpr::Ident(n) => n.clone(),
        IrExpr::Member(parts) => parts.join("."),
        _ => String::new(),
    }
}

/// The key/value pairs of an object literal.
pub fn pairs_of(expr: &IrExpr) -> Option<&[(String, IrExpr)]> {
    match expr {
        IrExpr::Object(pairs) => Some(pairs),
        _ => None,
    }
}

pub fn find_key<'a>(pairs: &'a [(String, IrExpr)], key: &str) -> Option<&'a IrExpr> {
    pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

/// Last path segment of an identifier or member expression (`VersioningType.URI` gives `URI`).
pub fn last_segment(expr: &IrExpr) -> Option<String> {
    match expr {
        IrExpr::Ident(n) => Some(n.clone()),
        IrExpr::Member(parts) => parts.last().cloned(),
        _ => None,
    }
}

/// A coarse description of an expression for `*_kind` attributes; never the value.
pub fn expr_kind(expr: &IrExpr) -> &'static str {
    match expr {
        IrExpr::Str(_) | IrExpr::Template { .. } => "string",
        IrExpr::Num(_) => "number",
        IrExpr::Bool(_) => "bool",
        IrExpr::Object(_) => "object",
        IrExpr::Array(_) => "array",
        IrExpr::Ident(_) | IrExpr::Member(_) => "ident",
        IrExpr::Call { .. } => "call",
        _ => "other",
    }
}

/// Top-level generic arguments of a declared type: `Repository<User>` gives `["User"]`.
pub fn type_args(declared: &str) -> Vec<String> {
    let (Some(start), Some(end)) = (declared.find('<'), declared.rfind('>')) else {
        return Vec::new();
    };
    if end <= start {
        return Vec::new();
    }
    let inner = &declared[start + 1..end];
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut current = String::new();
    for c in inner.chars() {
        match c {
            '<' | '(' | '[' | '{' => {
                depth += 1;
                current.push(c);
            }
            '>' | ')' | ']' | '}' => {
                depth -= 1;
                current.push(c);
            }
            ',' if depth == 0 => {
                out.push(current.trim().to_owned());
                current.clear();
            }
            _ => current.push(c),
        }
    }
    if !current.trim().is_empty() {
        out.push(current.trim().to_owned());
    }
    out
}

/// Declared types of fields, properties, parameters and variables by name.
pub fn receiver_types(symbols: &[IrSymbol]) -> BTreeMap<String, String> {
    let mut out: BTreeMap<String, String> = BTreeMap::new();
    for symbol in symbols {
        if matches!(
            symbol.kind,
            SymbolKind::Property | SymbolKind::Field | SymbolKind::Variable | SymbolKind::Constant
        ) {
            if let Some(declared) = &symbol.declared_type {
                out.entry(symbol.name.clone())
                    .or_insert_with(|| declared.clone());
            }
        }
        for param in &symbol.params {
            if let Some(declared) = &param.type_text {
                out.entry(param.name.clone())
                    .or_insert_with(|| declared.clone());
            }
        }
    }
    out
}

/// A string literal or no-substitution template as text.
pub fn node_string(node: Node<'_>, src: &crate::text::Source<'_>) -> Option<String> {
    match node.kind() {
        kind::STRING => Some(crate::visit::expr::string_content(node, src)),
        kind::TEMPLATE_STRING => {
            let text = src.text(node);
            Some(text.trim_matches('`').to_owned())
        }
        _ => None,
    }
}

/// Capitalized identifier text, the usual shape of a class reference.
pub fn class_ident(expr: &IrExpr) -> Option<String> {
    match expr {
        IrExpr::Ident(n) if n.chars().next().is_some_and(char::is_uppercase) => Some(n.clone()),
        _ => None,
    }
}
