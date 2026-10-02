//! The finding lifecycle and its authoritative transition table (PRD §150, target-arch §4.3).
//!
//! Persisted transitions are compare-and-set in SQL (`UPDATE ... WHERE id=$1 AND state=$expected`)
//! done in VER/PIPE; this module only validates the edge.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Lifecycle state of a candidate finding. Wire form is SCREAMING_SNAKE_CASE.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FindingState {
    Generated,
    EvidenceCollected,
    Verified,
    Deduplicated,
    // A legal resting state at the end of a run: findings in the internal band (0.55-0.70), or
    // in 0.70-0.85 below medium severity, are shown in the UI but not published. (Plain comment:
    // a doc comment here would split the exported schema enum into a oneOf.)
    Prioritized,
    Published,
    SuppressedLowConfidence,
    SuppressedDuplicate,
    SuppressedPreexisting,
    SuppressedNotActionable,
    SuppressedPolicy,
    Invalidated,
}

use FindingState as S;

/// Every legal edge. Everything else is rejected.
///
/// | From | Allowed to |
/// |---|---|
/// | `Generated` | `EvidenceCollected`, `SuppressedNotActionable`, `SuppressedPreexisting`, `SuppressedPolicy`, `Invalidated` |
/// | `EvidenceCollected` | `Verified`, `SuppressedLowConfidence`, `SuppressedPreexisting`, `SuppressedNotActionable`, `SuppressedPolicy`, `Invalidated` |
/// | `Verified` | `Deduplicated`, `SuppressedDuplicate`, `SuppressedPolicy`, `Invalidated` |
/// | `Deduplicated` | `Prioritized`, `SuppressedPolicy`, `Invalidated` |
/// | `Prioritized` | `Published`, `SuppressedPolicy`, `Invalidated` |
/// | `Published`, every `Suppressed*`, `Invalidated` | none (terminal) |
pub const ALLOWED: &[(FindingState, FindingState)] = &[
    (S::Generated, S::EvidenceCollected),
    (S::Generated, S::SuppressedNotActionable),
    (S::Generated, S::SuppressedPreexisting),
    (S::Generated, S::SuppressedPolicy),
    (S::Generated, S::Invalidated),
    (S::EvidenceCollected, S::Verified),
    (S::EvidenceCollected, S::SuppressedLowConfidence),
    (S::EvidenceCollected, S::SuppressedPreexisting),
    (S::EvidenceCollected, S::SuppressedNotActionable),
    (S::EvidenceCollected, S::SuppressedPolicy),
    (S::EvidenceCollected, S::Invalidated),
    (S::Verified, S::Deduplicated),
    (S::Verified, S::SuppressedDuplicate),
    (S::Verified, S::SuppressedPolicy),
    (S::Verified, S::Invalidated),
    (S::Deduplicated, S::Prioritized),
    (S::Deduplicated, S::SuppressedPolicy),
    (S::Deduplicated, S::Invalidated),
    (S::Prioritized, S::Published),
    (S::Prioritized, S::SuppressedPolicy),
    (S::Prioritized, S::Invalidated),
];

impl FindingState {
    pub const ALL: [FindingState; 12] = [
        S::Generated,
        S::EvidenceCollected,
        S::Verified,
        S::Deduplicated,
        S::Prioritized,
        S::Published,
        S::SuppressedLowConfidence,
        S::SuppressedDuplicate,
        S::SuppressedPreexisting,
        S::SuppressedNotActionable,
        S::SuppressedPolicy,
        S::Invalidated,
    ];

    /// The wire name, also the `findings_suppressed_total{reason}` label.
    pub const fn as_str(self) -> &'static str {
        match self {
            S::Generated => "GENERATED",
            S::EvidenceCollected => "EVIDENCE_COLLECTED",
            S::Verified => "VERIFIED",
            S::Deduplicated => "DEDUPLICATED",
            S::Prioritized => "PRIORITIZED",
            S::Published => "PUBLISHED",
            S::SuppressedLowConfidence => "SUPPRESSED_LOW_CONFIDENCE",
            S::SuppressedDuplicate => "SUPPRESSED_DUPLICATE",
            S::SuppressedPreexisting => "SUPPRESSED_PREEXISTING",
            S::SuppressedNotActionable => "SUPPRESSED_NOT_ACTIONABLE",
            S::SuppressedPolicy => "SUPPRESSED_POLICY",
            S::Invalidated => "INVALIDATED",
        }
    }

    pub const fn is_suppressed(self) -> bool {
        matches!(
            self,
            S::SuppressedLowConfidence
                | S::SuppressedDuplicate
                | S::SuppressedPreexisting
                | S::SuppressedNotActionable
                | S::SuppressedPolicy
        )
    }

    /// `Published`, every suppressed state and `Invalidated` have no outgoing edges.
    pub const fn is_terminal(self) -> bool {
        matches!(self, S::Published | S::Invalidated) || self.is_suppressed()
    }

    pub fn can_transition_to(self, to: FindingState) -> bool {
        ALLOWED.iter().any(|&(f, t)| f == self && t == to)
    }
}

impl fmt::Display for FindingState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Independently written expectation: for each state, the exact set of legal targets.
    fn expected_targets(from: FindingState) -> &'static [FindingState] {
        use FindingState::*;
        match from {
            Generated => &[
                EvidenceCollected,
                SuppressedNotActionable,
                SuppressedPreexisting,
                SuppressedPolicy,
                Invalidated,
            ],
            EvidenceCollected => &[
                Verified,
                SuppressedLowConfidence,
                SuppressedPreexisting,
                SuppressedNotActionable,
                SuppressedPolicy,
                Invalidated,
            ],
            Verified => &[
                Deduplicated,
                SuppressedDuplicate,
                SuppressedPolicy,
                Invalidated,
            ],
            Deduplicated => &[Prioritized, SuppressedPolicy, Invalidated],
            Prioritized => &[Published, SuppressedPolicy, Invalidated],
            Published
            | SuppressedLowConfidence
            | SuppressedDuplicate
            | SuppressedPreexisting
            | SuppressedNotActionable
            | SuppressedPolicy
            | Invalidated => &[],
        }
    }

    #[test]
    fn finding_state_transition_table_exhaustive() {
        assert_eq!(FindingState::ALL.len(), 12);
        let mut checked = 0;
        for from in FindingState::ALL {
            for to in FindingState::ALL {
                let expected = expected_targets(from).contains(&to);
                assert_eq!(from.can_transition_to(to), expected, "{from} -> {to}");
                checked += 1;
            }
        }
        assert_eq!(checked, 144);
        let edges: usize = FindingState::ALL
            .iter()
            .map(|s| expected_targets(*s).len())
            .sum();
        assert_eq!(
            ALLOWED.len(),
            edges,
            "ALLOWED has an edge the expected matrix lacks"
        );
    }

    #[test]
    fn terminal_states_have_no_outgoing_edges() {
        for s in FindingState::ALL {
            let outgoing = FindingState::ALL
                .iter()
                .filter(|t| s.can_transition_to(**t))
                .count();
            assert_eq!(s.is_terminal(), outgoing == 0, "{s}");
        }
        assert!(FindingState::Published.is_terminal());
        assert!(FindingState::Invalidated.is_terminal());
        assert!(!FindingState::Prioritized.is_terminal());
    }

    #[test]
    fn every_non_terminal_can_be_invalidated() {
        for s in FindingState::ALL {
            if !s.is_terminal() {
                assert!(s.can_transition_to(FindingState::Invalidated), "{s}");
            }
        }
    }

    #[test]
    fn published_reachable_only_via_prioritized() {
        let into_published: Vec<_> = ALLOWED
            .iter()
            .filter(|(_, t)| *t == FindingState::Published)
            .map(|(f, _)| *f)
            .collect();
        assert_eq!(into_published, vec![FindingState::Prioritized]);
    }

    #[test]
    fn suppressed_predicate_matches_wire_names() {
        for s in FindingState::ALL {
            assert_eq!(
                s.is_suppressed(),
                s.as_str().starts_with("SUPPRESSED_"),
                "{s}"
            );
            assert_eq!(
                serde_json::to_string(&s).unwrap(),
                format!("\"{}\"", s.as_str())
            );
        }
        assert!(serde_json::from_str::<FindingState>("\"published\"").is_err());
    }
}
