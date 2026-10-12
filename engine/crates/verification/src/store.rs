//! Finding lifecycle repository contract (VER-001).
//!
//! All state changes are compare-and-set on `(state, state_version)`. The legality of an edge is
//! decided here, from the `review-core` transition table, and never sent to SQL when illegal. The
//! PG implementation lives in `pipeline::store` and writes the audit row in the same transaction.

use std::future::Future;

use review_core::finding::FindingState;
use uuid::Uuid;

/// Why a lifecycle write did not happen.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TransitionError {
    #[error("illegal finding transition {from} -> {to}")]
    IllegalTransition {
        from: FindingState,
        to: FindingState,
    },
    /// Another worker moved the candidate first (a retried job, or bulk invalidation). The loser
    /// re-reads and decides again.
    #[error("finding state changed concurrently")]
    Conflict,
    #[error("finding not found")]
    NotFound,
    #[error("store: {0}")]
    Store(String),
}

/// A requested CAS edge with its audit context.
#[derive(Debug, Clone, PartialEq)]
pub struct Transition {
    pub candidate_id: Uuid,
    pub from: FindingState,
    pub expected_version: i32,
    pub to: FindingState,
    pub reason_code: Option<String>,
    /// Pipeline stage label recorded in `finding_state_transitions.stage`.
    pub stage: String,
    pub detail: Option<serde_json::Value>,
}

impl Transition {
    /// Rejects an edge that is not in the transition table, before any I/O.
    pub fn validate(&self) -> Result<(), TransitionError> {
        if self.from.can_transition_to(self.to) {
            Ok(())
        } else {
            Err(TransitionError::IllegalTransition {
                from: self.from,
                to: self.to,
            })
        }
    }
}

/// Persistence operations for the finding lifecycle.
pub trait FindingStore: Send + Sync {
    /// CAS transition plus audit row in one transaction. Returns the new `state_version`.
    fn transition(
        &self,
        t: Transition,
    ) -> impl Future<Output = Result<i32, TransitionError>> + Send;

    /// Moves every non-terminal candidate of the run to `INVALIDATED` with reason
    /// `HEAD_SUPERSEDED`, auditing each. Returns how many were invalidated.
    fn invalidate_run(
        &self,
        review_run_id: Uuid,
    ) -> impl Future<Output = Result<u64, TransitionError>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(from: FindingState, to: FindingState) -> Transition {
        Transition {
            candidate_id: Uuid::nil(),
            from,
            expected_version: 0,
            to,
            reason_code: None,
            stage: "test".into(),
            detail: None,
        }
    }

    #[test]
    fn illegal_transition_rejected() {
        let e = t(FindingState::Generated, FindingState::Published)
            .validate()
            .unwrap_err();
        assert!(matches!(e, TransitionError::IllegalTransition { .. }));
    }

    #[test]
    fn legal_transition_accepted() {
        assert!(t(FindingState::Generated, FindingState::EvidenceCollected)
            .validate()
            .is_ok());
    }

    #[test]
    fn invalidated_candidate_cannot_be_verified() {
        assert!(t(FindingState::Invalidated, FindingState::Verified)
            .validate()
            .is_err());
    }
}
