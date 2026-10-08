//! Filling `IrSymbol::{signature_hash, body_hash, attr_hash, body_shingles, body_token_count}`
//! (the field-filling half of TSA-007, consumed by SID-004 and SID-005).
//!
//! The pass is pure and idempotent: the same bytes and symbols in, the same hashes out. It is a
//! separate function rather than a visitor call so the indexer and the tests apply it explicitly
//! and a normalization change has exactly one call site.

use std::collections::BTreeMap;

use analysis_ir::hashing::{hash_tokens, shingles, HashKind, Token, TokenClass};
use analysis_ir::{IrExpr, IrSymbol, LocalId, Modifiers, Visibility};
use review_core::location::{Position, SourceRange};
use review_core::symbol::{Hash128, SymbolKind};

use crate::text::Source;
use crate::tokens::{child_placeholder, spanned_tokens, SpannedToken};

/// Width of the body shingle n-grams (TSA-007).
pub const SHINGLE_N: usize = 3;

/// Byte span of one child symbol inside the parent's slice.
#[derive(Debug, Clone)]
struct ChildSpan {
    start: usize,
    end: usize,
    placeholder: Token,
}

/// What one pass did, for metrics and diagnostics.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct HashReport {
    /// Symbols that received a hash.
    pub hashed: usize,
    /// Symbols whose body hash came from an empty token stream.
    pub empty_bodies: usize,
    /// Symbols flagged `has_errors`, whose hashes cover only the tokens that parsed.
    pub partial: usize,
    /// Container placeholders emitted.
    pub placeholders: usize,
}

/// Fills the hash fields of every symbol in place. `symbols` must be the whole unit in
/// `local_id` order, because a container's hash needs its children's ranges.
pub fn assign_hashes(symbols: &mut [IrSymbol], source: &Source<'_>) -> HashReport {
    let mut report = HashReport::default();
    let index = LineIndex::new(source.bytes);
    let children = child_spans(symbols, &index, source.bom);
    let spans: Vec<Option<(usize, usize)>> = symbols
        .iter()
        .map(|symbol| index.span(body_span_of(symbol), source.bom))
        .collect();

    let mut bodies: Vec<Vec<Token>> = Vec::with_capacity(symbols.len());
    let mut signatures: Vec<Vec<Token>> = Vec::with_capacity(symbols.len());
    let mut attributes: Vec<Vec<Token>> = Vec::with_capacity(symbols.len());
    for (position, symbol) in symbols.iter().enumerate() {
        let local_id = LocalId(position as u32);
        let (start, end) = spans[position].unwrap_or((0, 0));
        let slice = text_slice(source, start, end);
        let mut stream: Vec<SpannedToken> = spanned_tokens(&slice);
        if is_container(symbol.kind) {
            if let Some(children) = children.get(&local_id) {
                stream = fold_children(stream, children, start, &mut report);
            }
        }
        bodies.push(stream.into_iter().map(|spanned| spanned.token).collect());
        signatures.push(signature_tokens(symbol));
        attributes.push(attribute_tokens(symbol));
    }

    for (position, symbol) in symbols.iter_mut().enumerate() {
        let body = &bodies[position];
        symbol.body_hash = hash_tokens(HashKind::Body, body);
        symbol.body_shingles = shingles(body, SHINGLE_N);
        symbol.body_token_count = u32::try_from(body.len()).unwrap_or(u32::MAX);
        if body.is_empty() {
            report.empty_bodies += 1;
        }
        symbol.signature_hash = hash_tokens(HashKind::Signature, &signatures[position]);
        symbol.attr_hash = hash_tokens(HashKind::Attributes, &attributes[position]);
        if symbol.has_errors {
            report.partial += 1;
        }
        report.hashed += 1;
    }
    report
}

/// Whether a kind's body hash folds its members into placeholders.
pub const fn is_container(kind: SymbolKind) -> bool {
    matches!(
        kind,
        SymbolKind::Module
            | SymbolKind::Class
            | SymbolKind::Interface
            | SymbolKind::Enum
            | SymbolKind::Namespace
    )
}

/// The range whose tokens form one symbol's body hash: its `body_range` when the analyzer set one
/// (the body of a callable, the body of a container, the initializer of a constant), and the whole
/// declaration otherwise (an ambient signature, a bare interface member).
fn body_span_of(symbol: &IrSymbol) -> SourceRange {
    symbol.body_range.unwrap_or(symbol.range)
}

/// The hash a symbol gets for an empty body: stable and non-zero, so "no body" is distinguishable
/// from the unset [`Hash128::ZERO`].
pub fn empty_body_hash() -> Hash128 {
    hash_tokens(HashKind::Body, &[])
}

/// Byte spans of every symbol's children, keyed by the parent's `LocalId`.
fn child_spans(
    symbols: &[IrSymbol],
    index: &LineIndex,
    bom: u32,
) -> BTreeMap<LocalId, Vec<ChildSpan>> {
    let mut out: BTreeMap<LocalId, Vec<(SourceRange, String, SymbolKind)>> = BTreeMap::new();
    for symbol in symbols {
        let Some(parent) = symbol.parent else {
            continue;
        };
        out.entry(parent).or_default().push((
            symbol.range,
            // The placeholder carries the child's simple name, not its qualified name: renaming a
            // class must not change its members' placeholder text, or the container hash would
            // change on every class rename and the matcher could never pair it.
            symbol.name.clone(),
            symbol.kind,
        ));
    }
    out.into_iter()
        .map(|(parent, children)| {
            let mut spans: Vec<ChildSpan> = children
                .into_iter()
                .filter_map(|(range, name, kind)| {
                    index.span(range, bom).map(|(start, end)| ChildSpan {
                        start,
                        end,
                        placeholder: child_placeholder(kind.as_id_str(), &name),
                    })
                })
                .collect();
            spans.sort_by_key(|span| span.start);
            (parent, spans)
        })
        .collect()
}

/// Replaces the tokens of every child with one placeholder token, so editing a member's body does
/// not change the container's hash while adding, removing or renaming a member does. Token offsets
/// are relative to the parent's slice, so `base` translates them to file offsets.
fn fold_children(
    stream: Vec<SpannedToken>,
    children: &[ChildSpan],
    base: usize,
    report: &mut HashReport,
) -> Vec<SpannedToken> {
    let mut out: Vec<SpannedToken> = Vec::with_capacity(stream.len());
    let mut index = 0usize;
    let mut folded_until = 0usize;
    for spanned in stream {
        let absolute = spanned.start + base;
        while index < children.len() && children[index].end <= absolute {
            index += 1;
        }
        if absolute < folded_until {
            // Inside a child that was already folded: every remaining token of that child is
            // dropped, which is what makes a member's body invisible to its container.
            continue;
        }
        if index < children.len() && absolute >= children[index].start {
            out.push(SpannedToken {
                start: spanned.start,
                end: spanned.end,
                token: children[index].placeholder.clone(),
            });
            folded_until = children[index].end;
            index += 1;
            report.placeholders += 1;
            continue;
        }
        out.push(spanned);
    }
    out
}

fn text_slice(source: &Source<'_>, start: usize, end: usize) -> String {
    let end = end.min(source.bytes.len());
    let start = start.min(end);
    String::from_utf8_lossy(&source.bytes[start..end]).into_owned()
}

/// The signature token stream (TSA-007): kind, signature-relevant modifiers, visibility, type
/// parameters, parameters without their default expressions, the return type, the declared type,
/// heritage, then the overload signatures. The symbol's own name is excluded, so a pure rename
/// keeps its signature hash (SID-005 compares names separately).
fn signature_tokens(symbol: &IrSymbol) -> Vec<Token> {
    let mut out = vec![Token::ident(symbol.kind.as_id_str())];
    for name in [
        "async",
        "static",
        "abstract",
        "readonly",
        "optional",
        "generator",
    ] {
        if let Some(modifier) = modifier_named(name) {
            if symbol.modifiers.contains(modifier) {
                out.push(Token::ident(name));
            }
        }
    }
    out.push(Token::ident(visibility_name(symbol.visibility)));
    for param in &symbol.type_params {
        out.push(Token::punct("<"));
        out.extend(crate::tokens::tokens_of(param));
        out.push(Token::punct(">"));
    }
    out.push(Token::punct("("));
    for param in &symbol.params {
        out.push(Token::ident(&param.name));
        if param.optional {
            out.push(Token::punct("?"));
        }
        if param.rest {
            out.push(Token::punct("..."));
        }
        if let Some(type_text) = &param.type_text {
            out.push(Token::punct(":"));
            out.extend(crate::tokens::tokens_of(type_text));
        }
        if param.property.is_some() {
            out.push(Token::ident("param-property"));
        }
        out.push(Token::punct(","));
    }
    out.push(Token::punct(")"));
    if let Some(return_type) = &symbol.return_type {
        out.push(Token::punct(":"));
        out.extend(crate::tokens::tokens_of(return_type));
    }
    if let Some(declared_type) = &symbol.declared_type {
        out.push(Token::ident("declared"));
        out.extend(crate::tokens::tokens_of(declared_type));
    }
    if !symbol.heritage.extends.is_empty() {
        out.push(Token::ident("extends"));
        for entry in &symbol.heritage.extends {
            out.extend(crate::tokens::tokens_of(entry));
        }
    }
    if !symbol.heritage.implements.is_empty() {
        out.push(Token::ident("implements"));
        for entry in &symbol.heritage.implements {
            out.extend(crate::tokens::tokens_of(entry));
        }
    }
    for overload in &symbol.overload_signatures {
        out.push(Token::ident("overload"));
        out.extend(crate::tokens::tokens_of(overload));
    }
    out
}

/// The attribute token stream: decorators in order, then the export/declare/override flags and the
/// visibility. `doc_hash` is excluded, so a documentation change is not an attribute change.
fn attribute_tokens(symbol: &IrSymbol) -> Vec<Token> {
    let mut out = Vec::new();
    for decorator in &symbol.decorators {
        out.push(Token::punct("@"));
        out.extend(crate::tokens::tokens_of(&decorator.name));
        for arg in &decorator.args {
            out.extend(expr_tokens(arg));
        }
    }
    for name in ["exported", "default_export", "declare", "override"] {
        if let Some(modifier) = modifier_named(name) {
            if symbol.modifiers.contains(modifier) {
                out.push(Token::ident(name));
            }
        }
    }
    out.push(Token::ident(visibility_name(symbol.visibility)));
    out
}

fn modifier_named(name: &str) -> Option<Modifiers> {
    Modifiers::NAMES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, modifier)| *modifier)
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

fn visibility_name(visibility: Visibility) -> &'static str {
    match visibility {
        Visibility::Public => "public",
        Visibility::Protected => "protected",
        Visibility::Private => "private",
        Visibility::EcmaPrivate => "ecma-private",
    }
}

/// Byte offsets of every line, so a 1-based line plus a 0-based byte column becomes a byte offset.
struct LineIndex {
    starts: Vec<usize>,
}

impl LineIndex {
    fn new(bytes: &[u8]) -> Self {
        let mut starts = vec![0usize];
        for (offset, byte) in bytes.iter().enumerate() {
            if *byte == b'\n' {
                starts.push(offset + 1);
            }
        }
        Self { starts }
    }

    /// The byte span of a source range, honouring the BOM offset of the first line.
    fn span(&self, range: SourceRange, bom: u32) -> Option<(usize, usize)> {
        let start = self.offset(range.start, bom)?;
        let end = self.offset(range.end, bom)?;
        Some((start, end.max(start)))
    }

    fn offset(&self, at: Position, bom: u32) -> Option<usize> {
        let line = at.line.checked_sub(1)? as usize;
        let start = *self.starts.get(line)?;
        Some(start + at.column as usize + if line == 0 { bom as usize } else { 0 })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use review_core::location::RepoPath;

    fn at(line: u32, column: u32) -> Position {
        Position { line, column }
    }

    #[test]
    fn line_index_maps_positions_to_offsets() {
        let index = LineIndex::new(b"ab\ncd\n");
        assert_eq!(index.offset(at(1, 0), 0), Some(0));
        assert_eq!(index.offset(at(1, 2), 0), Some(2));
        assert_eq!(index.offset(at(2, 0), 0), Some(3));
        assert_eq!(index.offset(at(3, 0), 0), Some(6));
        assert_eq!(index.offset(at(4, 0), 0), None);
        assert_eq!(index.offset(at(0, 0), 0), None);
        assert_eq!(
            index.span(
                SourceRange {
                    start: at(2, 1),
                    end: at(2, 2)
                },
                0
            ),
            Some((4, 5))
        );
        assert_eq!(
            index.span(
                SourceRange {
                    start: at(1, 0),
                    end: at(1, 1)
                },
                3
            ),
            Some((3, 4)),
            "the BOM shifts the first line"
        );
    }

    #[test]
    fn containers_and_leaves_are_classified() {
        assert!(is_container(SymbolKind::Class));
        assert!(is_container(SymbolKind::Module));
        assert!(!is_container(SymbolKind::Method));
        assert!(!is_container(SymbolKind::Property));
    }

    #[test]
    fn empty_body_hash_is_stable() {
        assert!(!empty_body_hash().is_zero());
        assert_eq!(empty_body_hash(), empty_body_hash());
        assert_ne!(empty_body_hash(), Hash128::ZERO);
    }

    #[test]
    fn placeholder_names_carry_kind_and_name() {
        let placeholder = child_placeholder("method", "A.m");
        assert_eq!(placeholder.text, "<child:method:A.m>");
        assert_eq!(placeholder.class, TokenClass::Placeholder);
    }

    #[test]
    fn module_paths_are_irrelevant_to_hashes() {
        // The same declarations analyzed under a different path must hash identically: hashes are
        // about code, the path is part of the id.
        let source = Source::new(b"export const a = 1;\n");
        let path = RepoPath::new("src/a.ts").unwrap();
        let _ = path;
        let mut symbols = vec![IrSymbol::new(
            LocalId(0),
            SymbolKind::Constant,
            "a",
            vec!["a".to_owned()],
            SourceRange {
                start: at(1, 0),
                end: at(1, 19),
            },
        )];
        assign_hashes(&mut symbols, &source);
        let first = symbols[0].body_hash;
        assign_hashes(&mut symbols, &source);
        assert_eq!(symbols[0].body_hash, first, "the pass is idempotent");
    }
}
