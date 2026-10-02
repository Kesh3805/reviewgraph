//! `ReviewRun`: one review of one pull-request head (PRD §108). Maps onto `review_runs`.

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::state::ReviewState;
use crate::error::{CoreError, ErrorClass};
use crate::ids::{CommitSha, OrganizationId, PullRequestId, RepositoryId, ReviewRunId};
use crate::provenance::Provenance;
use crate::reviewer_type::ReviewerType;

const MAX_FAILURE_DETAIL_CHARS: usize = 2000;

/// What started the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReviewTrigger {
    Webhook,
    Manual,
    Reconciler,
    Cli,
}

/// Why a run failed. `detail` is shown in the UI, so it holds **no source, prompts or tokens**,
/// and is capped at 2000 characters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunFailure {
    pub class: ErrorClass,
    pub detail: String,
}

impl RunFailure {
    pub fn new(class: ErrorClass, detail: impl Into<String>) -> Result<Self, CoreError> {
        Self {
            class,
            detail: detail.into(),
        }
        .validated()
    }

    fn validated(self) -> Result<Self, CoreError> {
        let n = self.detail.chars().count();
        if n > MAX_FAILURE_DETAIL_CHARS {
            return Err(CoreError::OutOfRange {
                field: "run_failure.detail",
                value: format!("{n} characters (at most {MAX_FAILURE_DETAIL_CHARS})"),
            });
        }
        Ok(self)
    }
}

impl<'de> Deserialize<'de> for RunFailure {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            class: ErrorClass,
            detail: String,
        }
        let r = Raw::deserialize(deserializer)?;
        Self {
            class: r.class,
            detail: r.detail,
        }
        .validated()
        .map_err(serde::de::Error::custom)
    }
}

/// What a caller supplies when moving a run along an edge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransitionInput {
    /// Any normal forward edge, including `COMPLETED`.
    Advance,
    /// Required for the `FAILED_*` states.
    Fail(RunFailure),
    /// Required for `SUPERSEDED`; `by` must be a different run.
    Supersede { by: ReviewRunId },
    /// Required for `CANCELLED` (closed PR, manual cancel, shutdown abandonment).
    Cancel,
}

/// The creation-time facts of a run; [`ReviewRun::new`] starts it in `RECEIVED`.
#[derive(Debug, Clone)]
pub struct ReviewRunSpec {
    pub id: ReviewRunId,
    pub organization_id: OrganizationId,
    pub repository_id: RepositoryId,
    pub pull_request_id: PullRequestId,
    pub base_sha: CommitSha,
    pub head_sha: CommitSha,
    pub merge_base_sha: Option<CommitSha>,
    pub trigger: ReviewTrigger,
    /// Set when this run is a manual retry of a failed run.
    pub retry_of: Option<ReviewRunId>,
    /// W3C `traceparent` of the request that started the run.
    pub trace_parent: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReviewRun {
    pub id: ReviewRunId,
    pub organization_id: OrganizationId,
    pub repository_id: RepositoryId,
    pub pull_request_id: PullRequestId,
    pub base_sha: CommitSha,
    pub head_sha: CommitSha,
    pub merge_base_sha: Option<CommitSha>,
    pub state: ReviewState,
    pub trigger: ReviewTrigger,
    /// Present exactly when the state is `SUPERSEDED`.
    pub superseded_by: Option<ReviewRunId>,
    pub retry_of: Option<ReviewRunId>,
    /// Present exactly when the state is a `FAILED_*` state.
    pub failure: Option<RunFailure>,
    /// Reviewers that did not run (PRD §109). Kept sorted and deduplicated.
    pub degraded_reviewers: Vec<ReviewerType>,
    pub provenance: Option<Provenance>,
    /// A W3C traceparent, validated so arbitrary header text cannot be stored.
    pub trace_parent: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Set when the run reaches a terminal state.
    pub completed_at: Option<DateTime<Utc>>,
}

/// `^[0-9a-f]{2}-[0-9a-f]{32}-[0-9a-f]{16}-[0-9a-f]{2}$`
fn is_valid_trace_parent(s: &str) -> bool {
    let parts: Vec<&str> = s.split('-').collect();
    let lower_hex = |p: &str, n: usize| {
        p.len() == n && p.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    };
    matches!(parts.as_slice(), [a, b, c, d] if lower_hex(a, 2) && lower_hex(b, 32) && lower_hex(c, 16) && lower_hex(d, 2))
}

impl ReviewRun {
    pub fn new(spec: ReviewRunSpec, at: DateTime<Utc>) -> Result<Self, CoreError> {
        Self {
            id: spec.id,
            organization_id: spec.organization_id,
            repository_id: spec.repository_id,
            pull_request_id: spec.pull_request_id,
            base_sha: spec.base_sha,
            head_sha: spec.head_sha,
            merge_base_sha: spec.merge_base_sha,
            state: ReviewState::Received,
            trigger: spec.trigger,
            superseded_by: None,
            retry_of: spec.retry_of,
            failure: None,
            degraded_reviewers: Vec::new(),
            provenance: None,
            trace_parent: spec.trace_parent,
            created_at: at,
            updated_at: at,
            completed_at: None,
        }
        .validated()
    }

    /// Checks the cross-field invariants (also run on deserialize) and normalizes
    /// `degraded_reviewers`.
    pub fn validated(mut self) -> Result<Self, CoreError> {
        if let Some(tp) = &self.trace_parent {
            if !is_valid_trace_parent(tp) {
                return Err(CoreError::InvalidId {
                    kind: "TraceParent",
                    reason: "expected a W3C traceparent (00-<32 hex>-<16 hex>-<2 hex>)".to_owned(),
                });
            }
        }
        if self.retry_of == Some(self.id) {
            return Err(CoreError::InvalidId {
                kind: "ReviewRun",
                reason: "a run cannot be a retry of itself".to_owned(),
            });
        }
        let consistent = self.failure.is_some() == self.state.is_failed()
            && self.superseded_by.is_some() == (self.state == ReviewState::Superseded)
            && self.completed_at.is_some() == self.state.is_terminal()
            && self.superseded_by != Some(self.id);
        if !consistent {
            return Err(CoreError::InvalidTransition {
                entity: "ReviewRun",
                from: self.state.as_str(),
                to: self.state.as_str(),
            });
        }
        self.degraded_reviewers.sort();
        self.degraded_reviewers.dedup();
        Ok(self)
    }

    /// Records a reviewer that did not run. Idempotent.
    pub fn record_degraded(&mut self, reviewer: ReviewerType) {
        if let Err(pos) = self.degraded_reviewers.binary_search(&reviewer) {
            self.degraded_reviewers.insert(pos, reviewer);
        }
    }

    /// Validates and applies a transition: the edge must be in the table, and the input must
    /// match the target (`FAILED_*` needs `Fail`, `SUPERSEDED` needs `Supersede` by another run,
    /// `CANCELLED` needs `Cancel`, anything else needs `Advance`). Sets `updated_at`, and
    /// `completed_at` when the target is terminal. A transition to the current state is rejected.
    pub fn apply_transition(
        &mut self,
        to: ReviewState,
        input: TransitionInput,
        at: DateTime<Utc>,
    ) -> Result<(), CoreError> {
        let invalid = || CoreError::InvalidTransition {
            entity: "ReviewRun",
            from: self.state.as_str(),
            to: to.as_str(),
        };
        if !self.state.can_transition_to(to) {
            return Err(invalid());
        }
        let mut failure = None;
        let mut superseded_by = None;
        match (to, input) {
            (t, TransitionInput::Fail(f)) if t.is_failed() => failure = Some(f.validated()?),
            (ReviewState::Superseded, TransitionInput::Supersede { by }) if by != self.id => {
                superseded_by = Some(by);
            }
            (ReviewState::Cancelled, TransitionInput::Cancel) => {}
            (t, TransitionInput::Advance)
                if !t.is_failed()
                    && t != ReviewState::Superseded
                    && t != ReviewState::Cancelled => {}
            _ => return Err(invalid()),
        }
        self.state = to;
        self.updated_at = at;
        if failure.is_some() {
            self.failure = failure;
        }
        if superseded_by.is_some() {
            self.superseded_by = superseded_by;
        }
        if to.is_terminal() {
            self.completed_at = Some(at);
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for ReviewRun {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            id: ReviewRunId,
            organization_id: OrganizationId,
            repository_id: RepositoryId,
            pull_request_id: PullRequestId,
            base_sha: CommitSha,
            head_sha: CommitSha,
            merge_base_sha: Option<CommitSha>,
            state: ReviewState,
            trigger: ReviewTrigger,
            superseded_by: Option<ReviewRunId>,
            retry_of: Option<ReviewRunId>,
            failure: Option<RunFailure>,
            degraded_reviewers: Vec<ReviewerType>,
            provenance: Option<Provenance>,
            trace_parent: Option<String>,
            created_at: DateTime<Utc>,
            updated_at: DateTime<Utc>,
            completed_at: Option<DateTime<Utc>>,
        }
        let r = Raw::deserialize(deserializer)?;
        Self {
            id: r.id,
            organization_id: r.organization_id,
            repository_id: r.repository_id,
            pull_request_id: r.pull_request_id,
            base_sha: r.base_sha,
            head_sha: r.head_sha,
            merge_base_sha: r.merge_base_sha,
            state: r.state,
            trigger: r.trigger,
            superseded_by: r.superseded_by,
            retry_of: r.retry_of,
            failure: r.failure,
            degraded_reviewers: r.degraded_reviewers,
            provenance: r.provenance,
            trace_parent: r.trace_parent,
            created_at: r.created_at,
            updated_at: r.updated_at,
            completed_at: r.completed_at,
        }
        .validated()
        .map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};
    use proptest::prelude::*;

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
    }

    fn spec() -> ReviewRunSpec {
        ReviewRunSpec {
            id: ReviewRunId::new(),
            organization_id: OrganizationId::new(),
            repository_id: RepositoryId::new(),
            pull_request_id: PullRequestId::new(),
            base_sha: "a".repeat(40).parse().unwrap(),
            head_sha: "b".repeat(40).parse().unwrap(),
            merge_base_sha: None,
            trigger: ReviewTrigger::Webhook,
            retry_of: None,
            trace_parent: None,
        }
    }

    fn run() -> ReviewRun {
        ReviewRun::new(spec(), t0()).unwrap()
    }

    fn fail() -> TransitionInput {
        TransitionInput::Fail(RunFailure::new(ErrorClass::Transient, "model timed out").unwrap())
    }

    /// The input a caller must pass to reach `to`.
    fn input_for(to: ReviewState) -> TransitionInput {
        match to {
            ReviewState::Superseded => TransitionInput::Supersede {
                by: ReviewRunId::new(),
            },
            ReviewState::Cancelled => TransitionInput::Cancel,
            t if t.is_failed() => fail(),
            _ => TransitionInput::Advance,
        }
    }

    #[test]
    fn failed_requires_failure_input() {
        for (from, failed) in [
            (ReviewState::Indexing, ReviewState::FailedIndexing),
            (ReviewState::Analyzing, ReviewState::FailedAnalysis),
            (ReviewState::Reviewing, ReviewState::FailedReview),
            (ReviewState::Publishing, ReviewState::FailedPublish),
        ] {
            for bad in [
                TransitionInput::Advance,
                TransitionInput::Cancel,
                TransitionInput::Supersede {
                    by: ReviewRunId::new(),
                },
            ] {
                let mut r = run();
                r.state = from;
                assert!(r.apply_transition(failed, bad, t0()).is_err());
                assert_eq!(r.state, from);
            }
            let mut r = run();
            r.state = from;
            r.apply_transition(failed, fail(), t0()).unwrap();
            assert_eq!(
                r.failure.as_ref().map(|f| f.class),
                Some(ErrorClass::Transient)
            );
        }
        // And `Fail` is not accepted for non-failed targets.
        let mut r = run();
        assert!(r
            .apply_transition(ReviewState::Indexing, fail(), t0())
            .is_err());
    }

    #[test]
    fn superseded_requires_other_run_id() {
        let mut r = run();
        let own = r.id;
        assert!(r
            .apply_transition(
                ReviewState::Superseded,
                TransitionInput::Supersede { by: own },
                t0()
            )
            .is_err());
        assert!(r
            .apply_transition(ReviewState::Superseded, TransitionInput::Advance, t0())
            .is_err());
        assert!(r
            .apply_transition(ReviewState::Superseded, TransitionInput::Cancel, t0())
            .is_err());
        let other = ReviewRunId::new();
        r.apply_transition(
            ReviewState::Superseded,
            TransitionInput::Supersede { by: other },
            t0(),
        )
        .unwrap();
        assert_eq!(r.superseded_by, Some(other));
        assert!(r.state.is_terminal());
    }

    #[test]
    fn cancel_input_only_for_cancelled() {
        let mut r = run();
        assert!(r
            .apply_transition(ReviewState::Indexing, TransitionInput::Cancel, t0())
            .is_err());
        assert!(r
            .apply_transition(ReviewState::Cancelled, TransitionInput::Advance, t0())
            .is_err());
        r.apply_transition(ReviewState::Cancelled, TransitionInput::Cancel, t0())
            .unwrap();
        assert_eq!(r.state, ReviewState::Cancelled);
    }

    #[test]
    fn completed_at_set_on_terminal() {
        let mut r = run();
        let mut at = t0();
        for next in [
            ReviewState::Indexing,
            ReviewState::Analyzing,
            ReviewState::Reviewing,
            ReviewState::Verifying,
            ReviewState::Publishing,
        ] {
            at += Duration::seconds(1);
            r.apply_transition(next, TransitionInput::Advance, at)
                .unwrap();
            assert_eq!(r.updated_at, at);
            assert_eq!(r.completed_at, None, "{next}");
        }
        at += Duration::seconds(1);
        r.apply_transition(ReviewState::Completed, TransitionInput::Advance, at)
            .unwrap();
        assert_eq!(r.completed_at, Some(at));
        assert_eq!(r.updated_at, at);
        assert!(r.failure.is_none() && r.superseded_by.is_none());
        // Terminal: nothing moves out.
        assert!(r
            .apply_transition(ReviewState::Publishing, TransitionInput::Advance, at)
            .is_err());
    }

    #[test]
    fn same_state_transition_is_rejected() {
        let mut r = run();
        assert!(r
            .apply_transition(ReviewState::Received, TransitionInput::Advance, t0())
            .is_err());
    }

    #[test]
    fn analyzing_can_skip_to_publishing_for_summary_only_runs() {
        let mut r = run();
        r.apply_transition(ReviewState::Indexing, TransitionInput::Advance, t0())
            .unwrap();
        r.apply_transition(ReviewState::Analyzing, TransitionInput::Advance, t0())
            .unwrap();
        r.apply_transition(ReviewState::Publishing, TransitionInput::Advance, t0())
            .unwrap();
    }

    #[test]
    fn degraded_reviewers_sorted_dedup() {
        let mut r = run();
        for rv in [
            ReviewerType::Security,
            ReviewerType::Correctness,
            ReviewerType::Security,
            ReviewerType::Test,
            ReviewerType::Correctness,
        ] {
            r.record_degraded(rv);
        }
        assert_eq!(
            r.degraded_reviewers,
            vec![
                ReviewerType::Correctness,
                ReviewerType::Security,
                ReviewerType::Test
            ]
        );
        // Deserialization normalizes too.
        let mut json = serde_json::to_value(&r).unwrap();
        json["degraded_reviewers"] = serde_json::json!(["test", "correctness", "test"]);
        let back: ReviewRun = serde_json::from_value(json).unwrap();
        assert_eq!(
            back.degraded_reviewers,
            vec![ReviewerType::Correctness, ReviewerType::Test]
        );
    }

    #[test]
    fn trace_parent_validated() {
        let good = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";
        let mut s = spec();
        s.trace_parent = Some(good.to_owned());
        assert!(ReviewRun::new(s, t0()).is_ok());
        for bad in [
            "",
            "garbage",
            "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331",
            "00-0AF7651916CD43DD8448EB211C80319C-b7ad6b7169203331-01",
            "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01\r\nX-Evil: 1",
            "0-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01",
            "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01-extra",
        ] {
            let mut s = spec();
            s.trace_parent = Some(bad.to_owned());
            assert!(ReviewRun::new(s, t0()).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn failure_detail_is_capped() {
        assert!(RunFailure::new(ErrorClass::Internal, "x".repeat(2000)).is_ok());
        assert!(RunFailure::new(ErrorClass::Internal, "x".repeat(2001)).is_err());
    }

    #[test]
    fn run_roundtrips_and_deserialize_validates_consistency() {
        let mut r = run();
        r.apply_transition(ReviewState::Indexing, TransitionInput::Advance, t0())
            .unwrap();
        r.apply_transition(ReviewState::FailedIndexing, fail(), t0())
            .unwrap();
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(serde_json::from_str::<ReviewRun>(&json).unwrap(), r);
        // A failed state without a failure is inconsistent.
        let mut v: serde_json::Value = serde_json::from_str(&json).unwrap();
        v["failure"] = serde_json::Value::Null;
        assert!(serde_json::from_value::<ReviewRun>(v).is_err());
        // A retry of itself is rejected.
        let mut s = spec();
        s.retry_of = Some(s.id);
        assert!(ReviewRun::new(s, t0()).is_err());
    }

    proptest! {
        #[test]
        fn random_walk_respects_table(targets in prop::collection::vec(0usize..13, 0..40)) {
            let mut r = run();
            for idx in targets {
                let to = ReviewState::ALL[idx];
                let from = r.state;
                let result = r.apply_transition(to, input_for(to), t0());
                if from.can_transition_to(to) {
                    prop_assert!(result.is_ok(), "{from} -> {to}");
                    prop_assert_eq!(r.state, to);
                } else {
                    prop_assert!(result.is_err());
                    prop_assert_eq!(r.state, from);
                }
                if from.is_terminal() {
                    prop_assert_eq!(r.state, from, "left terminal state {}", from);
                }
                prop_assert!(r.clone().validated().is_ok());
            }
        }
    }
}
