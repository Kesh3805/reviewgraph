//! The review-run lifecycle and its authoritative transition table (PRD §108, target-arch §4.1).
//!
//! `FAILED_*` states are terminal: a manual retry creates a *new* run with `retry_of` set, which
//! keeps an audit trail and keeps the compare-and-set simple. Every active state has an edge to
//! `SUPERSEDED`, so supersession can win against any stage.
//!
//! Metric label values for `review_runs_total{state}` and
//! `review_state_transitions_total{from,to}` are [`ReviewState::as_str`]; the span attribute is
//! `review.state`.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Run state. Wire form is SCREAMING_SNAKE_CASE.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReviewState {
    Received,
    Indexing,
    Analyzing,
    Reviewing,
    Verifying,
    Publishing,
    Completed,
    FailedIndexing,
    FailedAnalysis,
    FailedReview,
    FailedPublish,
    Superseded,
    Cancelled,
}

use ReviewState as S;

/// Every legal edge.
///
/// | From | To |
/// |---|---|
/// | `Received` | `Indexing`, `Superseded`, `Cancelled` |
/// | `Indexing` | `Analyzing`, `FailedIndexing`, `Superseded`, `Cancelled` |
/// | `Analyzing` | `Reviewing`, `Publishing` (no applicable reviewers: a summary-only publication, recorded as such, never a silent skip), `FailedAnalysis`, `Superseded`, `Cancelled` |
/// | `Reviewing` | `Verifying`, `FailedReview`, `Superseded`, `Cancelled` |
/// | `Verifying` | `Publishing`, `FailedReview` (verification is part of review), `Superseded`, `Cancelled` |
/// | `Publishing` | `Completed`, `FailedPublish`, `Superseded`, `Cancelled` |
/// | `Completed`, `Failed*`, `Superseded`, `Cancelled` | none (terminal) |
pub const ALLOWED: &[(ReviewState, ReviewState)] = &[
    (S::Received, S::Indexing),
    (S::Received, S::Superseded),
    (S::Received, S::Cancelled),
    (S::Indexing, S::Analyzing),
    (S::Indexing, S::FailedIndexing),
    (S::Indexing, S::Superseded),
    (S::Indexing, S::Cancelled),
    (S::Analyzing, S::Reviewing),
    (S::Analyzing, S::Publishing),
    (S::Analyzing, S::FailedAnalysis),
    (S::Analyzing, S::Superseded),
    (S::Analyzing, S::Cancelled),
    (S::Reviewing, S::Verifying),
    (S::Reviewing, S::FailedReview),
    (S::Reviewing, S::Superseded),
    (S::Reviewing, S::Cancelled),
    (S::Verifying, S::Publishing),
    (S::Verifying, S::FailedReview),
    (S::Verifying, S::Superseded),
    (S::Verifying, S::Cancelled),
    (S::Publishing, S::Completed),
    (S::Publishing, S::FailedPublish),
    (S::Publishing, S::Superseded),
    (S::Publishing, S::Cancelled),
];

impl ReviewState {
    pub const ALL: [ReviewState; 13] = [
        S::Received,
        S::Indexing,
        S::Analyzing,
        S::Reviewing,
        S::Verifying,
        S::Publishing,
        S::Completed,
        S::FailedIndexing,
        S::FailedAnalysis,
        S::FailedReview,
        S::FailedPublish,
        S::Superseded,
        S::Cancelled,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            S::Received => "RECEIVED",
            S::Indexing => "INDEXING",
            S::Analyzing => "ANALYZING",
            S::Reviewing => "REVIEWING",
            S::Verifying => "VERIFYING",
            S::Publishing => "PUBLISHING",
            S::Completed => "COMPLETED",
            S::FailedIndexing => "FAILED_INDEXING",
            S::FailedAnalysis => "FAILED_ANALYSIS",
            S::FailedReview => "FAILED_REVIEW",
            S::FailedPublish => "FAILED_PUBLISH",
            S::Superseded => "SUPERSEDED",
            S::Cancelled => "CANCELLED",
        }
    }

    pub const fn is_failed(self) -> bool {
        matches!(
            self,
            S::FailedIndexing | S::FailedAnalysis | S::FailedReview | S::FailedPublish
        )
    }

    /// `Completed`, every failed state, `Superseded` and `Cancelled`.
    pub const fn is_terminal(self) -> bool {
        matches!(self, S::Completed | S::Superseded | S::Cancelled) || self.is_failed()
    }

    /// A stage is in progress (not terminal).
    pub const fn is_active(self) -> bool {
        !self.is_terminal()
    }

    /// The failure state a stage enters when it fails, if it has one.
    pub const fn failure_state(self) -> Option<ReviewState> {
        match self {
            S::Indexing => Some(S::FailedIndexing),
            S::Analyzing => Some(S::FailedAnalysis),
            S::Reviewing | S::Verifying => Some(S::FailedReview),
            S::Publishing => Some(S::FailedPublish),
            _ => None,
        }
    }

    pub fn can_transition_to(self, to: ReviewState) -> bool {
        ALLOWED.iter().any(|&(f, t)| f == self && t == to)
    }
}

impl fmt::Display for ReviewState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeSet, VecDeque};

    /// Independently written expectation.
    fn expected_targets(from: ReviewState) -> &'static [ReviewState] {
        use ReviewState::*;
        match from {
            Received => &[Indexing, Superseded, Cancelled],
            Indexing => &[Analyzing, FailedIndexing, Superseded, Cancelled],
            Analyzing => &[Reviewing, Publishing, FailedAnalysis, Superseded, Cancelled],
            Reviewing => &[Verifying, FailedReview, Superseded, Cancelled],
            Verifying => &[Publishing, FailedReview, Superseded, Cancelled],
            Publishing => &[Completed, FailedPublish, Superseded, Cancelled],
            Completed | FailedIndexing | FailedAnalysis | FailedReview | FailedPublish
            | Superseded | Cancelled => &[],
        }
    }

    #[test]
    fn review_state_transition_table_exhaustive() {
        assert_eq!(ReviewState::ALL.len(), 13);
        let mut checked = 0;
        for from in ReviewState::ALL {
            for to in ReviewState::ALL {
                assert_eq!(
                    from.can_transition_to(to),
                    expected_targets(from).contains(&to),
                    "{from} -> {to}"
                );
                checked += 1;
            }
        }
        assert_eq!(checked, 169);
        let edges: usize = ReviewState::ALL
            .iter()
            .map(|s| expected_targets(*s).len())
            .sum();
        assert_eq!(ALLOWED.len(), edges);
    }

    #[test]
    fn terminal_states_have_no_outgoing_edges() {
        for s in ReviewState::ALL {
            let out = ReviewState::ALL
                .iter()
                .filter(|t| s.can_transition_to(**t))
                .count();
            assert_eq!(s.is_terminal(), out == 0, "{s}");
            assert_eq!(s.is_active(), !s.is_terminal());
        }
    }

    #[test]
    fn every_active_state_can_be_superseded_and_cancelled() {
        for s in ReviewState::ALL {
            if s.is_active() {
                assert!(s.can_transition_to(ReviewState::Superseded), "{s}");
                assert!(s.can_transition_to(ReviewState::Cancelled), "{s}");
            }
        }
    }

    #[test]
    fn every_state_reachable_from_received() {
        let mut seen = BTreeSet::from([ReviewState::Received]);
        let mut queue = VecDeque::from([ReviewState::Received]);
        while let Some(s) = queue.pop_front() {
            for t in ReviewState::ALL {
                if s.can_transition_to(t) && seen.insert(t) {
                    queue.push_back(t);
                }
            }
        }
        assert_eq!(seen.len(), ReviewState::ALL.len());
    }

    #[test]
    fn completed_reachable_only_via_publishing() {
        let into: Vec<_> = ALLOWED
            .iter()
            .filter(|(_, t)| *t == ReviewState::Completed)
            .map(|(f, _)| *f)
            .collect();
        assert_eq!(into, vec![ReviewState::Publishing]);
    }

    #[test]
    fn each_active_stage_maps_to_its_failure_state() {
        let expected = [
            (ReviewState::Indexing, Some(ReviewState::FailedIndexing)),
            (ReviewState::Analyzing, Some(ReviewState::FailedAnalysis)),
            (ReviewState::Reviewing, Some(ReviewState::FailedReview)),
            (ReviewState::Verifying, Some(ReviewState::FailedReview)),
            (ReviewState::Publishing, Some(ReviewState::FailedPublish)),
            (ReviewState::Received, None),
        ];
        for (stage, failed) in expected {
            assert_eq!(stage.failure_state(), failed, "{stage}");
            if let Some(f) = failed {
                assert!(stage.can_transition_to(f), "{stage}");
                assert!(f.is_failed());
            }
        }
        for s in ReviewState::ALL.iter().filter(|s| s.is_terminal()) {
            assert_eq!(s.failure_state(), None);
        }
    }

    #[test]
    fn review_state_wire_names_snapshot() {
        let names: Vec<String> = ReviewState::ALL
            .iter()
            .map(|s| {
                serde_json::to_string(s)
                    .unwrap()
                    .trim_matches('"')
                    .to_owned()
            })
            .collect();
        for (s, n) in ReviewState::ALL.iter().zip(&names) {
            assert_eq!(s.as_str(), n);
        }
        insta::assert_yaml_snapshot!(names);
        assert!(serde_json::from_str::<ReviewState>("\"completed\"").is_err());
    }
}
