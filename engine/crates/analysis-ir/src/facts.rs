//! Per-symbol syntax facts used to classify changes (filled by TSA-006).

use std::collections::BTreeMap;

use review_core::location::SourceRange;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::symbol::{AttrValue, LocalId};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
pub enum FactKind {
    Call,
    New,
    Condition,
    Loop,
    Throw,
    TryCatch,
    Await,
    Return,
    DbWriteLike,
    DbReadLike,
    TransactionWrapper,
    GuardDecorator,
    ConfigRead,
    Assignment,
}

impl FactKind {
    pub const ALL: [FactKind; 14] = [
        Self::Call,
        Self::New,
        Self::Condition,
        Self::Loop,
        Self::Throw,
        Self::TryCatch,
        Self::Await,
        Self::Return,
        Self::DbWriteLike,
        Self::DbReadLike,
        Self::TransactionWrapper,
        Self::GuardDecorator,
        Self::ConfigRead,
        Self::Assignment,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Call => "call",
            Self::New => "new",
            Self::Condition => "condition",
            Self::Loop => "loop",
            Self::Throw => "throw",
            Self::TryCatch => "try_catch",
            Self::Await => "await",
            Self::Return => "return",
            Self::DbWriteLike => "db_write_like",
            Self::DbReadLike => "db_read_like",
            Self::TransactionWrapper => "transaction_wrapper",
            Self::GuardDecorator => "guard_decorator",
            Self::ConfigRead => "config_read",
            Self::Assignment => "assignment",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SyntaxFact {
    pub kind: FactKind,
    /// Stable comparison key without positions.
    pub key: String,
    pub range: SourceRange,
    pub detail: BTreeMap<String, AttrValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SymbolFacts {
    pub symbol: LocalId,
    /// Source order.
    pub facts: Vec<SyntaxFact>,
}

/// Multiset difference of two fact lists by `(kind, key)` (TSA-006). The change classifier (CHG)
/// compares the base and head facts of one symbol with it: positions are not part of a key, so a
/// reformat or a moved block yields an empty delta.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FactDelta {
    /// `(kind, key)` pairs present more often in head than in base, once per extra occurrence,
    /// sorted.
    pub added: Vec<(FactKind, String)>,
    /// `(kind, key)` pairs present more often in base than in head, sorted.
    pub removed: Vec<(FactKind, String)>,
}

impl FactDelta {
    /// Whether base and head have the same fact multiset.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty()
    }

    /// Added keys of one kind.
    pub fn added_of(&self, kind: FactKind) -> impl Iterator<Item = &str> {
        self.added
            .iter()
            .filter(move |(k, _)| *k == kind)
            .map(|(_, key)| key.as_str())
    }

    /// Removed keys of one kind.
    pub fn removed_of(&self, kind: FactKind) -> impl Iterator<Item = &str> {
        self.removed
            .iter()
            .filter(move |(k, _)| *k == kind)
            .map(|(_, key)| key.as_str())
    }
}

/// Compares two fact lists as multisets of `(kind, key)`; `detail` and `range` are ignored.
pub fn compare_keys(base: &[SyntaxFact], head: &[SyntaxFact]) -> FactDelta {
    let mut counts: BTreeMap<(FactKind, &str), i64> = BTreeMap::new();
    for fact in base {
        *counts.entry((fact.kind, fact.key.as_str())).or_insert(0) -= 1;
    }
    for fact in head {
        *counts.entry((fact.kind, fact.key.as_str())).or_insert(0) += 1;
    }
    let mut delta = FactDelta::default();
    for ((kind, key), count) in counts {
        let target = if count > 0 {
            &mut delta.added
        } else {
            &mut delta.removed
        };
        for _ in 0..count.unsigned_abs() {
            target.push((kind, key.to_owned()));
        }
    }
    delta
}

#[cfg(test)]
mod tests {
    use super::*;
    use review_core::location::Position;

    fn fact(kind: FactKind, key: &str, line: u32) -> SyntaxFact {
        let at = Position { line, column: 1 };
        SyntaxFact {
            kind,
            key: key.to_owned(),
            range: SourceRange { start: at, end: at },
            detail: BTreeMap::new(),
        }
    }

    #[test]
    fn facts_compare_keys_multiset_delta() {
        let base = vec![
            fact(FactKind::Call, "call:this.repo.save/1", 1),
            fact(FactKind::Call, "call:this.repo.save/1", 2),
            fact(FactKind::Condition, "if:0123abcd", 3),
        ];
        let head = vec![
            fact(FactKind::Call, "call:this.repo.save/1", 9),
            fact(FactKind::Throw, "throw:ForbiddenException", 4),
        ];
        let delta = compare_keys(&base, &head);
        assert_eq!(
            delta.added,
            vec![(FactKind::Throw, "throw:ForbiddenException".to_owned())]
        );
        assert_eq!(
            delta.removed,
            vec![
                (FactKind::Call, "call:this.repo.save/1".to_owned()),
                (FactKind::Condition, "if:0123abcd".to_owned()),
            ]
        );
        assert_eq!(delta.removed_of(FactKind::Condition).count(), 1);
    }

    #[test]
    fn facts_compare_keys_ignores_positions() {
        let base = vec![fact(FactKind::Return, "return:ident", 1)];
        let head = vec![fact(FactKind::Return, "return:ident", 40)];
        assert!(compare_keys(&base, &head).is_empty());
    }
}
