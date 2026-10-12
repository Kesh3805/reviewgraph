//! `ContextPackage`: the per-reviewer, per-cluster context handed to a model.

use review_core::reviewer_type::ReviewerType;
use serde::{Deserialize, Serialize};

use super::item::ContextItem;
use super::omitted::OmittedItem;

pub const PACKAGE_SCHEMA_VERSION: u16 = 1;
/// At most this many omitted items are listed; the rest only count in `omitted_overflow`.
pub const MAX_OMITTED_LISTED: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextBudget {
    pub max_tokens: u32,
    pub max_items: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RiskSignal {
    pub kind: String,
    pub detail: String,
    pub score: f32,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ChangeSummary {
    pub text: String,
    pub files_changed: u32,
    pub symbols_changed: u32,
}

/// Inputs and versions that determine the package; part of the package hash.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ContextProvenance {
    pub input_hashes: Vec<String>,
    pub engine_version: String,
    pub weights_version: String,
    /// `None` when no semantic space was used.
    pub semantic_space: Option<String>,
    pub structural_degraded: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextPackage {
    pub schema_version: u16,
    pub reviewer: ReviewerType,
    pub cluster_id: String,
    pub items: Vec<ContextItem>,
    pub omitted: Vec<OmittedItem>,
    pub omitted_overflow: u32,
    pub token_estimate: u32,
    pub budget: ContextBudget,
    pub risk_signals: Vec<RiskSignal>,
    pub change_summary: ChangeSummary,
    pub provenance: ContextProvenance,
    /// Hex blake3 over the canonical package (CTX-009); empty until computed.
    pub package_hash: String,
}

impl ContextPackage {
    /// Build a package: sorts items and omitted deterministically, caps the omitted list.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        reviewer: ReviewerType,
        cluster_id: impl Into<String>,
        mut items: Vec<ContextItem>,
        mut omitted: Vec<OmittedItem>,
        budget: ContextBudget,
        risk_signals: Vec<RiskSignal>,
        change_summary: ChangeSummary,
        provenance: ContextProvenance,
    ) -> Self {
        items.sort_by(ContextItem::package_order);
        omitted.sort_by(OmittedItem::list_order);
        let overflow = omitted.len().saturating_sub(MAX_OMITTED_LISTED);
        omitted.truncate(MAX_OMITTED_LISTED);
        let token_estimate = items.iter().map(|i| i.tokens).sum();
        Self {
            schema_version: PACKAGE_SCHEMA_VERSION,
            reviewer,
            cluster_id: cluster_id.into(),
            items,
            omitted,
            omitted_overflow: u32::try_from(overflow).unwrap_or(u32::MAX),
            token_estimate,
            budget,
            risk_signals,
            change_summary,
            provenance,
            package_hash: String::new(),
        }
    }
}
