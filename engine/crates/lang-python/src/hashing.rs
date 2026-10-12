//! Filling `IrSymbol::{signature_hash, body_hash, attr_hash, body_shingles, body_token_count}`.
//!
//! Tokens come from the syntax tree leaves, so comments, whitespace and quote style never change a
//! hash. A container (module, class) folds each member into one placeholder token, so editing a
//! member's body does not change the container's hash while adding, removing or renaming a member
//! does. The pass is pure and idempotent.

use analysis_ir::hashing::{hash_tokens, shingles, HashKind, Token, TokenClass};
use analysis_ir::{IrExpr, IrSymbol, Modifiers, Visibility};
use review_core::symbol::{Hash128, SymbolKind};
use tree_sitter::Node;

use crate::text::Source;

/// Width of the body shingle n-grams.
pub const SHINGLE_N: usize = 3;

/// Deepest node the token walk descends into.
const MAX_TOKEN_DEPTH: usize = 400;

/// A child symbol's byte span, replaced by one placeholder token in its container's body.
#[derive(Debug, Clone)]
pub struct Fold {
    pub start: usize,
    pub end: usize,
    pub token: Token,
}

/// The placeholder standing in for a child symbol.
pub fn child_placeholder(kind: &str, name: &str) -> Token {
    Token::new(TokenClass::Placeholder, format!("<child:{kind}:{name}>"))
}

/// Whether a kind's body hash folds its members into placeholders.
pub const fn is_container(kind: SymbolKind) -> bool {
    matches!(kind, SymbolKind::Module | SymbolKind::Class)
}

/// Normalized tokens of one subtree, with `folds` replaced by placeholders.
pub fn node_tokens(node: Node<'_>, src: &Source<'_>, folds: &[Fold]) -> Vec<Token> {
    let mut out = Vec::new();
    push_tokens(node, src, folds, &mut out, 0);
    out
}

fn push_tokens(
    node: Node<'_>,
    src: &Source<'_>,
    folds: &[Fold],
    out: &mut Vec<Token>,
    depth: usize,
) {
    if depth > MAX_TOKEN_DEPTH {
        return;
    }
    if let Some(fold) = folds
        .iter()
        .find(|f| f.start == node.start_byte() && f.end == node.end_byte())
    {
        out.push(fold.token.clone());
        return;
    }
    match node.kind() {
        "comment" | "line_continuation" | "string_start" | "string_end" => return,
        "string_content" => {
            out.push(Token::string(src.text(node).into_owned()));
            return;
        }
        _ => {}
    }
    if node.child_count() == 0 {
        if node.start_byte() == node.end_byte() {
            return;
        }
        let text = src.text(node);
        let token = match node.kind() {
            "integer" | "float" => Token::new(
                TokenClass::Num,
                text.chars()
                    .filter(|c| *c != '_')
                    .collect::<String>()
                    .to_lowercase(),
            ),
            kind if kind == "identifier"
                || text.chars().all(|c| c.is_alphanumeric() || c == '_') =>
            {
                Token::ident(text.into_owned())
            }
            _ => Token::punct(text.into_owned()),
        };
        out.push(token);
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        push_tokens(child, src, folds, out, depth + 1);
    }
}

/// Splits display text (a type annotation) into tokens.
pub fn tokens_of(text: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut word = String::new();
    for c in text.chars() {
        if c.is_alphanumeric() || c == '_' {
            word.push(c);
            continue;
        }
        if !word.is_empty() {
            out.push(Token::ident(std::mem::take(&mut word)));
        }
        if !c.is_whitespace() {
            out.push(Token::punct(c.to_string()));
        }
    }
    if !word.is_empty() {
        out.push(Token::ident(word));
    }
    out
}

/// Sets the hash fields of one symbol from its body tokens.
pub fn assign(symbol: &mut IrSymbol, body: &[Token]) {
    symbol.body_hash = hash_tokens(HashKind::Body, body);
    symbol.body_shingles = shingles(body, SHINGLE_N);
    symbol.body_token_count = u32::try_from(body.len()).unwrap_or(u32::MAX);
    symbol.signature_hash = hash_tokens(HashKind::Signature, &signature_tokens(symbol));
    symbol.attr_hash = hash_tokens(HashKind::Attributes, &attribute_tokens(symbol));
}

/// The hash a symbol gets for an empty body: stable and non-zero.
pub fn empty_body_hash() -> Hash128 {
    hash_tokens(HashKind::Body, &[])
}

fn modifier_named(name: &str) -> Option<Modifiers> {
    Modifiers::NAMES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, modifier)| *modifier)
}

fn visibility_name(visibility: Visibility) -> &'static str {
    match visibility {
        Visibility::Public => "public",
        Visibility::Protected => "protected",
        Visibility::Private => "private",
        Visibility::EcmaPrivate => "ecma-private",
    }
}

/// Kind, signature-relevant modifiers, visibility, parameters, return type, declared type and
/// heritage. The symbol's own name is excluded, so a pure rename keeps its signature hash.
fn signature_tokens(symbol: &IrSymbol) -> Vec<Token> {
    let mut out = vec![Token::ident(symbol.kind.as_id_str())];
    for name in ["async", "static", "abstract", "readonly", "optional"] {
        if let Some(modifier) = modifier_named(name) {
            if symbol.modifiers.contains(modifier) {
                out.push(Token::ident(name));
            }
        }
    }
    out.push(Token::ident(visibility_name(symbol.visibility)));
    out.push(Token::punct("("));
    for param in &symbol.params {
        out.push(Token::ident(&param.name));
        if param.optional {
            out.push(Token::punct("?"));
        }
        if param.rest {
            out.push(Token::punct("*"));
        }
        if let Some(type_text) = &param.type_text {
            out.push(Token::punct(":"));
            out.extend(tokens_of(type_text));
        }
        out.push(Token::punct(","));
    }
    out.push(Token::punct(")"));
    if let Some(return_type) = &symbol.return_type {
        out.push(Token::punct("->"));
        out.extend(tokens_of(return_type));
    }
    if let Some(declared_type) = &symbol.declared_type {
        out.push(Token::ident("declared"));
        out.extend(tokens_of(declared_type));
    }
    if !symbol.heritage.extends.is_empty() {
        out.push(Token::ident("extends"));
        for entry in &symbol.heritage.extends {
            out.extend(tokens_of(entry));
        }
    }
    if !symbol.heritage.implements.is_empty() {
        out.push(Token::ident("implements"));
        for entry in &symbol.heritage.implements {
            out.extend(tokens_of(entry));
        }
    }
    out
}

/// Decorators in order, then the export flag and the visibility.
fn attribute_tokens(symbol: &IrSymbol) -> Vec<Token> {
    let mut out = Vec::new();
    for decorator in &symbol.decorators {
        out.push(Token::punct("@"));
        out.extend(tokens_of(&decorator.name));
        for arg in &decorator.args {
            out.extend(expr_tokens(arg));
        }
    }
    if symbol.modifiers.contains(Modifiers::EXPORTED) {
        out.push(Token::ident("exported"));
    }
    out.push(Token::ident(visibility_name(symbol.visibility)));
    out
}

fn expr_tokens(expr: &IrExpr) -> Vec<Token> {
    match expr {
        IrExpr::Str(text) => vec![Token::string(text)],
        IrExpr::Num(text) | IrExpr::Other(text) | IrExpr::Ident(text) => vec![Token::ident(text)],
        IrExpr::Bool(value) => vec![Token::ident(value.to_string())],
        IrExpr::Null => vec![Token::ident("null")],
        IrExpr::Member(parts) => parts.iter().map(Token::ident).collect(),
        IrExpr::Array(items) => items.iter().flat_map(expr_tokens).collect(),
        IrExpr::Object(entries) => entries
            .iter()
            .flat_map(|(key, value)| {
                let mut acc = vec![Token::ident(key)];
                acc.extend(expr_tokens(value));
                acc
            })
            .collect(),
        IrExpr::Call { callee, args } => {
            let mut out: Vec<Token> = callee.iter().map(Token::ident).collect();
            for arg in args {
                out.extend(expr_tokens(arg));
            }
            out
        }
        IrExpr::Arrow { returns } => expr_tokens(returns),
        IrExpr::Template { raw, .. } => vec![Token::new(TokenClass::Template, raw)],
    }
}
