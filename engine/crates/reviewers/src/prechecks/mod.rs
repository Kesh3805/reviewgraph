//! Deterministic pre-checks that run before any model call (principle 2: deterministic before
//! probabilistic). They are pure functions over a [`crate::ReviewContext`] and produce
//! [`DeterministicFinding`]s that enter the candidate pool with `reviewer = "deterministic:*"`
//! and feed the matching specialist reviewer as evidence.
//!
//! - [`tests_gap`]: changed public symbol without mapped tests (REV-T-002).
//! - [`boundaries`]: forbidden layer dependencies and new import cycles (REV-A-002).
//! - [`io_in_loop`]: I/O and queries inside loops (REV-P-002).

pub mod boundaries;
pub mod io_in_loop;
pub mod tests_gap;

use serde::{Deserialize, Serialize};

use crate::context::{ChangedSymbol, NeighborEdge};
use crate::routing::NON_BEHAVIOURAL;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeverityHint {
    Low,
    Medium,
}

/// Evidence of an absence in the graph (for example "no inbound TESTS edge").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphAbsence {
    pub edge_kind: String,
    pub min_confidence: f64,
    pub searched_depth: u32,
}

/// One deterministic finding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeterministicFinding {
    /// `tests.public_symbol_untested`, `architecture.forbidden_dependency`,
    /// `architecture.new_cycle`, `performance.io_in_loop`, `performance.fan_out_query`.
    pub rule_id: String,
    /// `deterministic:tests`, `deterministic:architecture`, `deterministic:performance`.
    pub reviewer: String,
    pub symbol_key: String,
    pub path: String,
    /// Head-side line range the finding is anchored to.
    pub anchor_range: [u32; 2],
    pub severity_hint: SeverityHint,
    pub message: String,
    #[serde(default)]
    pub absence: Option<GraphAbsence>,
    /// Symbol keys or module paths that make up the evidence (cycle members, call path, ...).
    #[serde(default)]
    pub members: Vec<String>,
}

/// Result of a pre-check. A check that cannot run reports why; it never passes silently
/// (INV-014).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum PrecheckOutcome {
    Executed {
        findings: Vec<DeterministicFinding>,
        /// The evaluation budget was exceeded and the finding set is partial.
        truncated: bool,
    },
    NotExecuted {
        reason: String,
    },
}

impl PrecheckOutcome {
    pub fn findings(&self) -> &[DeterministicFinding] {
        match self {
            Self::Executed { findings, .. } => findings,
            Self::NotExecuted { .. } => &[],
        }
    }
}

/// True when the symbol changed behaviour: it carries at least one change class that is not
/// purely cosmetic.
pub(crate) fn behaviourally_changed(s: &ChangedSymbol) -> bool {
    s.change_classes
        .iter()
        .any(|c| !NON_BEHAVIOURAL.contains(&c.as_str()))
}

pub(crate) fn is_test_path(path: &str) -> bool {
    path.contains(".spec.")
        || path.contains(".test.")
        || path.contains("/__tests__/")
        || path.starts_with("test/")
        || path.contains("/test/")
}

/// Edges of `kind` pointing at `target`.
pub(crate) fn inbound<'a>(
    edges: &'a [NeighborEdge],
    target: &'a str,
    kind: &'a str,
) -> impl Iterator<Item = &'a NeighborEdge> + 'a {
    edges
        .iter()
        .filter(move |e| e.to == target && e.kind == kind)
}
