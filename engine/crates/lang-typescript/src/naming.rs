//! Qualified-name rules for TypeScript (SID-002). Pure functions: a name depends only on the
//! declaring construct and its ancestors, never on siblings, position or traversal order.
//!
//! The rules are a frozen contract: changing one needs an `ANALYZER_VERSION` major bump.

use analysis_ir::{DiagCode, QualifiedName};
use unicode_normalization::UnicodeNormalization;

/// Longest accepted segment, in bytes.
pub const MAX_SEGMENT_BYTES: usize = 256;

/// How a member's name is written in source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemberName {
    /// `foo`
    Identifier(String),
    /// `#foo`, kept with the `#` so it cannot collide with a public `foo`.
    Private(String),
    /// `'a-b'` (content only)
    StringKey(String),
    /// `42`
    Numeric(String),
    /// `[Symbol.iterator]`: the identifier after `Symbol.`.
    WellKnownSymbol(String),
    /// `[expr]`
    Computed,
}

/// What is being named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Construct {
    /// A declared name (class, interface, enum, type alias, function, namespace, variable).
    Declared(String),
    /// A class, interface, object or enum member.
    Member(MemberName),
    /// `export default <anonymous class/function/arrow/object>`.
    DefaultExport,
    /// An anonymous function under `AnonymousFnPolicy::Emit`.
    Anonymous,
    /// An overload signature: folded into its implementation.
    OverloadSignature,
}

/// Outcome of naming a construct.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameDecision {
    Named(QualifiedName),
    /// No symbol; the diagnostic code explains why.
    Skip(DiagCode),
    /// No segment of its own: merged into another symbol.
    Fold,
}

fn normalize(segment: &str) -> String {
    segment.nfc().collect()
}

/// Names `construct` below `parent` (the qualified name of the enclosing symbol, without the
/// module segment; `None` at module level).
pub fn qualify(parent: Option<&QualifiedName>, construct: Construct) -> NameDecision {
    let segment = match construct {
        Construct::OverloadSignature => return NameDecision::Fold,
        Construct::Declared(name) => name,
        Construct::DefaultExport => "default".to_owned(),
        Construct::Anonymous => "<anonymous>".to_owned(),
        Construct::Member(member) => match member {
            MemberName::Identifier(name)
            | MemberName::StringKey(name)
            | MemberName::Numeric(name) => name,
            MemberName::Private(name) => {
                if name.starts_with('#') {
                    name
                } else {
                    format!("#{name}")
                }
            }
            MemberName::WellKnownSymbol(name) => format!("@@{name}"),
            MemberName::Computed => return NameDecision::Skip(DiagCode::ComputedMemberName),
        },
    };
    let segment = normalize(&segment);
    if segment.is_empty() || segment.len() > MAX_SEGMENT_BYTES {
        return NameDecision::Skip(DiagCode::UnsupportedConstruct);
    }
    let mut qn = parent.cloned().unwrap_or_default();
    qn.push(segment);
    NameDecision::Named(qn)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(decision: NameDecision) -> Vec<String> {
        match decision {
            NameDecision::Named(qn) => qn,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn declared_and_member_names() {
        let a = named(qualify(None, Construct::Declared("A".to_owned())));
        assert_eq!(a, vec!["A"]);
        let m = named(qualify(
            Some(&a),
            Construct::Member(MemberName::Identifier("m".to_owned())),
        ));
        assert_eq!(m, vec!["A", "m"]);
        let p = named(qualify(
            Some(&a),
            Construct::Member(MemberName::Private("x".to_owned())),
        ));
        assert_eq!(p, vec!["A", "#x"]);
        let s = named(qualify(
            Some(&a),
            Construct::Member(MemberName::WellKnownSymbol("iterator".to_owned())),
        ));
        assert_eq!(s, vec!["A", "@@iterator"]);
    }

    #[test]
    fn skip_fold_and_limits() {
        assert_eq!(
            qualify(None, Construct::Member(MemberName::Computed)),
            NameDecision::Skip(DiagCode::ComputedMemberName)
        );
        assert_eq!(
            qualify(None, Construct::OverloadSignature),
            NameDecision::Fold
        );
        assert_eq!(
            qualify(None, Construct::Declared("x".repeat(257))),
            NameDecision::Skip(DiagCode::UnsupportedConstruct)
        );
        assert_eq!(
            named(qualify(None, Construct::DefaultExport)),
            vec!["default"]
        );
        assert_eq!(
            named(qualify(None, Construct::Anonymous)),
            vec!["<anonymous>"]
        );
    }

    #[test]
    fn unicode_is_nfc_normalized() {
        // "é" as e + combining acute versus the precomposed character.
        let decomposed = "e\u{301}";
        let a = named(qualify(None, Construct::Declared(decomposed.to_owned())));
        let b = named(qualify(None, Construct::Declared("\u{e9}".to_owned())));
        assert_eq!(a, b);
    }
}
