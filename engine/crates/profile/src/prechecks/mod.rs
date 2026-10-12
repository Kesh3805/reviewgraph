//! Deterministic pre-checks that run before any model (principle 2):
//!
//! * [`boundaries`] and [`cycles`] — REV-A-002, forbidden layer dependencies and new import
//!   cycles.
//! * [`tests_gap`] — REV-T-002, a changed public symbol with zero `TESTS` edges.
//!
//! A hit is a *candidate*, never a verdict: it enters the candidate pool with reviewer
//! `deterministic:*` and goes through verification like any other.

pub mod boundaries;
pub mod cycles;
pub mod tests_gap;

use review_core::evidence::Evidence;
use serde::{Deserialize, Serialize};

/// Whether a pre-check ran. `NotExecuted` is never a silent pass (INV-014): its reason is shown
/// in the summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PrecheckStatus {
    Executed,
    NotExecuted { reason: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeverityHint {
    Low,
    Medium,
    High,
}

/// Structured evidence specific to pre-checks, kept next to the generic evidence items.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PrecheckEvidence {
    /// The graph has no edge of `edge_kind` with at least `min_confidence` within
    /// `searched_depth` hops.
    GraphAbsence {
        edge_kind: String,
        min_confidence: f32,
        searched_depth: u32,
    },
    /// A dependency edge.
    Edge {
        kind: String,
        from: String,
        to: String,
        path: String,
        line: u32,
    },
    /// The members of a new cycle, sorted.
    Cycle { members: Vec<String> },
}

/// One deterministic hit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeterministicFinding {
    /// `architecture.forbidden_dependency`, `architecture.new_cycle`,
    /// `tests.public_symbol_untested`.
    pub rule_id: String,
    pub symbol_key: Option<String>,
    pub path: String,
    pub line: u32,
    pub message: String,
    pub severity_hint: SeverityHint,
    pub details: Vec<PrecheckEvidence>,
    pub evidence: Vec<Evidence>,
    /// For boundary hits: `(from layer, to layer)`.
    pub layers: Option<(String, String)>,
    /// The explicit rule id or convention id the hit is based on.
    pub basis_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrecheckOutcome {
    pub status: PrecheckStatus,
    pub findings: Vec<DeterministicFinding>,
    /// A budget cut the search short; the summary says so.
    pub truncated: bool,
}

impl PrecheckOutcome {
    pub fn not_executed(reason: impl Into<String>) -> Self {
        Self {
            status: PrecheckStatus::NotExecuted {
                reason: reason.into(),
            },
            findings: Vec::new(),
            truncated: false,
        }
    }

    pub fn executed(findings: Vec<DeterministicFinding>, truncated: bool) -> Self {
        Self {
            status: PrecheckStatus::Executed,
            findings,
            truncated,
        }
    }
}
