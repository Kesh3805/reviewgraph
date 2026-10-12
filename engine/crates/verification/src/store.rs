//! Finding lifecycle repository contract (VER-001).
//!
//! All state changes are compare-and-set on `(state, state_version)`. The legality of an edge is
//! decided here, from the `review-core` transition table, and never sent to SQL when illegal. The
//! PG implementation lives in `pipeline::store` and writes the audit row in the same transaction.

use std::future::Future;

use review_core::finding::{FindingState, Suppression};
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
    /// Required exactly when `to` is a `SUPPRESSED_*` state or `INVALIDATED` (the table CHECK
    /// couples the two), and its reason must match `to`. `REJECTED_INVALID` carries only
    /// `reason_code`.
    pub suppression: Option<Suppression>,
}

impl Transition {
    /// Rejects an edge that is not in the transition table, before any I/O.
    pub fn validate(&self) -> Result<(), TransitionError> {
        let illegal = || TransitionError::IllegalTransition {
            from: self.from,
            to: self.to,
        };
        if !self.from.can_transition_to(self.to) {
            return Err(illegal());
        }
        let needs_suppression = self.to.is_suppressed() || self.to == FindingState::Invalidated;
        match &self.suppression {
            Some(s) if needs_suppression && s.reason.target_state() == self.to => Ok(()),
            None if !needs_suppression => Ok(()),
            _ => Err(illegal()),
        }
    }
}

/// A reviewer candidate to persist in state `GENERATED`. `idempotency_key` is
/// `blake3(reviewer_run_id || ordinal || raw_output_hash)`.
#[derive(Debug, Clone, PartialEq)]
pub struct NewCandidate {
    pub id: Uuid,
    pub review_run_id: Uuid,
    pub reviewer_run_id: Uuid,
    pub reviewer: String,
    pub category: String,
    pub title: String,
    pub description: String,
    pub changed_path: String,
    pub changed_side: String,
    pub changed_start_line: i32,
    pub changed_end_line: i32,
    pub severity_candidate: String,
    /// Recorded for calibration only; never a publication input (ADR-011).
    pub confidence_candidate: Option<f32>,
    pub affected_symbols: Vec<String>,
    pub evidence: serde_json::Value,
    pub fingerprint: String,
    pub idempotency_key: String,
    pub raw_output_hash: String,
}

/// One verification attempt of a candidate. Unique per `(candidate_id, verification_version)`
/// (PRD §76 `verification:{candidateFindingId}:{verificationVersion}`).
#[derive(Debug, Clone, PartialEq)]
pub struct NewVerification {
    pub id: Uuid,
    pub candidate_id: Uuid,
    pub verification_version: String,
    pub base_snapshot_id: Uuid,
    pub head_snapshot_id: Uuid,
    pub depth: String,
    pub stage_outcomes: serde_json::Value,
    pub signals: serde_json::Value,
    pub confidence: Option<f64>,
    pub eligibility: Option<String>,
    pub placement: Option<String>,
    pub cache_key: String,
    pub from_cache: bool,
    pub duration_ms: i32,
}

/// One evidence item attached to a candidate, optionally to a verification.
#[derive(Debug, Clone, PartialEq)]
pub struct NewEvidence {
    pub candidate_id: Uuid,
    pub verification_id: Option<Uuid>,
    pub kind: String,
    pub source: String,
    pub strength: String,
    pub polarity: String,
    pub path: Option<String>,
    pub side: Option<String>,
    pub start_line: Option<i32>,
    pub end_line: Option<i32>,
    pub symbol_keys: Vec<String>,
    pub payload: serde_json::Value,
}

/// Persistence operations for the finding lifecycle.
pub trait FindingStore: Send + Sync {
    /// Inserts a `GENERATED` candidate. `Ok(false)` when it already exists (idempotent retry).
    fn insert_candidate(
        &self,
        c: NewCandidate,
    ) -> impl Future<Output = Result<bool, TransitionError>> + Send;

    /// Records a verification. `Ok(false)` when this version already exists for the candidate.
    fn record_verification(
        &self,
        v: NewVerification,
    ) -> impl Future<Output = Result<bool, TransitionError>> + Send;

    /// Attaches evidence rows in one transaction.
    fn add_evidence(
        &self,
        items: Vec<NewEvidence>,
    ) -> impl Future<Output = Result<(), TransitionError>> + Send;

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
    use review_core::finding::SuppressionReason;

    fn t(from: FindingState, to: FindingState) -> Transition {
        Transition {
            candidate_id: Uuid::nil(),
            from,
            expected_version: 0,
            to,
            reason_code: None,
            stage: "test".into(),
            detail: None,
            suppression: None,
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

    fn sup(reason: SuppressionReason) -> Option<Suppression> {
        Suppression::new(reason, "x", None).ok()
    }

    #[test]
    fn suppression_required_exactly_for_suppressed_targets() {
        let mut bad = t(
            FindingState::EvidenceCollected,
            FindingState::SuppressedPolicy,
        );
        assert!(bad.validate().is_err(), "missing suppression");
        bad.suppression = sup(SuppressionReason::Preexisting);
        assert!(bad.validate().is_err(), "reason does not match target");
        bad.suppression = sup(SuppressionReason::Policy { rule: "r".into() });
        assert!(bad.validate().is_ok());
        let mut rejected = t(FindingState::Generated, FindingState::RejectedInvalid);
        assert!(
            rejected.validate().is_ok(),
            "rejected carries only a reason code"
        );
        rejected.suppression = sup(SuppressionReason::Preexisting);
        assert!(rejected.validate().is_err());
    }

    #[test]
    fn invalidated_candidate_cannot_be_verified() {
        assert!(t(FindingState::Invalidated, FindingState::Verified)
            .validate()
            .is_err());
    }
}
