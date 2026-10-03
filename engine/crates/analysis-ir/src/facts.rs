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
