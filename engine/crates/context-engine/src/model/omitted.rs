//! Items left out of a package, with the reason.

use review_core::ids::SymbolKey;
use serde::{Deserialize, Serialize};

use super::item::{ItemId, ItemKind};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum OmitReason {
    BudgetTokens,
    BudgetCount { category: String },
    BelowThreshold { score: f32, threshold: f32 },
    DistanceRule { distance: u8 },
    Duplicate { of: ItemId },
    Generated,
    Binary,
    SecretBearing,
    SemanticFillNotNeeded,
    LowConfidencePath { min_confidence: f32 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OmittedItem {
    pub node: Option<SymbolKey>,
    pub title: String,
    pub kind: ItemKind,
    pub score: f32,
    pub reason: OmitReason,
}

impl OmittedItem {
    /// Omitted order: score descending, then title.
    pub fn list_order(a: &Self, b: &Self) -> std::cmp::Ordering {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.title.cmp(&b.title))
    }
}
