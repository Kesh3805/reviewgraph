//! Position-free fact keys (TSA-006). See `docs/languages/typescript.md` for the grammar.
//!
//! Keys never contain string-literal contents: callee chains drop literals, conditions are
//! summarized by `h8` (the first 8 hex characters of a BLAKE3 hash over the normalized token
//! stream), so reformatting, comments and quote style never change a key.

use analysis_ir::hashing::TokenHasher;
use tree_sitter::Node;

use crate::kinds::{field, kind};
use crate::text::Source;
use crate::tokens::tokens_of;

/// Domain separator of the fact hashes.
pub const HASH_DOMAIN_FACT: &str = "rg.fact.v1";

/// First 8 hex characters of the normalized-token hash of `text`.
pub fn h8_text(text: &str) -> String {
    let tokens = tokens_of(text);
    let mut hasher = TokenHasher::with_domain(HASH_DOMAIN_FACT);
    hasher.extend(tokens.iter());
    let hex = hasher.finish().to_string();
    hex.chars().take(8).collect()
}

/// [`h8_text`] of a node's source text.
pub fn h8(node: Node<'_>, src: &Source<'_>) -> String {
    h8_text(&src.text(node))
}

/// The segments of a callee or receiver chain: identifiers, `this` and `super` are kept, calls
/// inside the chain are written `name()`, subscripts `[]`, and literals are dropped.
pub fn chain_segments(node: Node<'_>, src: &Source<'_>, depth: usize) -> Vec<String> {
    if depth > 32 {
        return Vec::new();
    }
    match node.kind() {
        kind::IDENTIFIER
        | kind::THIS
        | kind::SUPER
        | kind::PROPERTY_IDENTIFIER
        | kind::PRIVATE_PROPERTY_IDENTIFIER
        | kind::TYPE_IDENTIFIER => vec![src.text(node).into_owned()],
        kind::MEMBER_EXPRESSION => {
            let mut parts = node
                .child_by_field_name(field::OBJECT)
                .map(|object| chain_segments(object, src, depth + 1))
                .unwrap_or_default();
            if let Some(property) = node.child_by_field_name(field::PROPERTY) {
                parts.push(src.text(property).into_owned());
            }
            parts
        }
        kind::CALL_EXPRESSION => {
            let mut parts = node
                .child_by_field_name(field::FUNCTION)
                .map(|function| chain_segments(function, src, depth + 1))
                .unwrap_or_default();
            if let Some(last) = parts.last_mut() {
                last.push_str("()");
            }
            parts
        }
        kind::SUBSCRIPT_EXPRESSION => {
            let mut parts = node
                .child_by_field_name(field::OBJECT)
                .map(|object| chain_segments(object, src, depth + 1))
                .unwrap_or_default();
            parts.push("[]".to_owned());
            parts
        }
        kind::NON_NULL_EXPRESSION
        | kind::PARENTHESIZED_EXPRESSION
        | kind::AS_EXPRESSION
        | kind::SATISFIES_EXPRESSION
        | kind::AWAIT_EXPRESSION => node
            .named_child(0)
            .map(|inner| chain_segments(inner, src, depth + 1))
            .unwrap_or_default(),
        kind::NESTED_IDENTIFIER | kind::NESTED_TYPE_IDENTIFIER | kind::GENERIC_TYPE => {
            let mut parts = Vec::new();
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if child.kind() != kind::TYPE_ARGUMENTS {
                    parts.extend(chain_segments(child, src, depth + 1));
                }
            }
            parts
        }
        _ => Vec::new(),
    }
}

/// A callee split into its receiver segments and its name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Callee {
    pub receiver: Vec<String>,
    pub name: String,
}

impl Callee {
    /// `receiver.name`, or `name` without a receiver.
    pub fn text(&self) -> String {
        if self.receiver.is_empty() {
            self.name.clone()
        } else {
            format!("{}.{}", self.receiver.join("."), self.name)
        }
    }

    /// The last receiver segment without a trailing `()`: `repo` for `this.repo.save`.
    pub fn receiver_name(&self) -> Option<&str> {
        self.receiver
            .last()
            .map(|segment| segment.trim_end_matches("()"))
    }

    /// Whether any receiver segment is the call `name()`.
    pub fn chain_has_call(&self, name: &str) -> bool {
        self.receiver
            .iter()
            .any(|segment| segment.strip_suffix("()") == Some(name))
    }
}

/// The callee of a call or `new` expression, if it has a name.
pub fn callee_of(function: Node<'_>, src: &Source<'_>) -> Option<Callee> {
    let mut parts = chain_segments(function, src, 0);
    let name = parts.pop()?;
    if name.is_empty() || name == "[]" {
        return None;
    }
    Some(Callee {
        receiver: parts,
        name: name.trim_end_matches("()").to_owned(),
    })
}

/// Number of arguments of a call (`arguments` node or tagged template).
pub fn argc(arguments: Option<Node<'_>>) -> usize {
    let Some(arguments) = arguments else {
        return 0;
    };
    if arguments.kind() == kind::TEMPLATE_STRING {
        return 1;
    }
    let mut cursor = arguments.walk();
    let count = arguments
        .named_children(&mut cursor)
        .filter(|child| child.kind() != kind::COMMENT)
        .count();
    count
}

/// `call:{receiver}.{name}/{argc}`.
pub fn call_key(callee: &Callee, argc: usize) -> String {
    format!("call:{}/{argc}", callee.text())
}

/// `new:{Name}/{argc}`.
pub fn new_key(callee: &Callee, argc: usize) -> String {
    format!("new:{}/{argc}", callee.name)
}

/// Whether a decorator's last-segment name is a guard under the default pattern
/// `^(UseGuards|Roles?|Permissions?|Public|Auth\w*|Authorize\w*|Skip\w*Auth\w*)$`.
pub fn is_default_guard_name(name: &str) -> bool {
    let word = |s: &str| s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !word(name) || name.is_empty() {
        return false;
    }
    matches!(
        name,
        "UseGuards" | "Role" | "Roles" | "Permission" | "Permissions" | "Public"
    ) || name.starts_with("Auth")
        || (name.starts_with("Skip") && name["Skip".len()..].contains("Auth"))
}

/// Whether a decorator name produces a `GuardDecorator` fact under `configured` (or the default
/// pattern when `None`).
pub fn is_guard_name(name: &str, configured: Option<&[String]>) -> bool {
    match configured {
        Some(names) => names.iter().any(|candidate| candidate == name),
        None => is_default_guard_name(name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn h8_ignores_whitespace_comments_and_quotes() {
        assert_eq!(
            h8_text("a === null && b"),
            h8_text("a===null /* x */ &&\n  b")
        );
        assert_eq!(h8_text("x === 'a'"), h8_text("x === \"a\""));
        assert_ne!(h8_text("a > b"), h8_text("a >= b"));
        assert_eq!(h8_text("a").len(), 8);
    }

    #[test]
    fn default_guard_pattern() {
        for name in [
            "UseGuards",
            "Roles",
            "Role",
            "Permissions",
            "Public",
            "Auth",
            "AuthGuarded",
            "Authorize",
            "AuthorizeAdmin",
            "SkipAuth",
            "SkipJwtAuthCheck",
        ] {
            assert!(is_default_guard_name(name), "{name}");
        }
        for name in ["Get", "Injectable", "Skip", "Rolex", "Publicly", ""] {
            assert!(!is_default_guard_name(name), "{name}");
        }
    }
}
