//! Fail-safe publication decision (DOM-010): the single place a run state becomes a provider
//! review event.
//!
//! ReviewGraph's MVP publishes a **COMMENT** review plus a check run whose conclusion is
//! `neutral` (findings exist, or coverage was degraded) or `success` (a complete review with no
//! findings). It never approves, never requests changes and never merges: those events are not
//! representable here, so they are absent rather than guarded (invariants INV-011 "never merge"
//! and INV-012 "failure is never approval", gap analysis §P; master plan §4 principle 10, see
//! `docs/planning/MASTER_IMPLEMENTATION_PLAN.md`).
//!
//! This is a port of the legacy `decision_for` choke point (legacy `core.rs:316-349`), whose tests
//! (`only_approved_state_can_approve`, `quota_failure_posts_nothing`) are carried over as the
//! `inv_012_*` tests below.
//!
//! | Legacy | New |
//! |---|---|
//! | `Approved` to `APPROVE` | removed |
//! | `ChangesRequested` to `REQUEST_CHANGES` | removed in the MVP (configurable blocking needs a new ADR) |
//! | `Commented` to `COMMENT` | `Publishing` to [`ReviewEvent::Comment`] |
//! | `Failed{..}` / `Cancelled` / `Skipped` / in-flight to `None` | every non-`Publishing` state to `None` |
//!
//! # Caller contract
//!
//! - The publisher reads the run state **inside** the publish transaction with
//!   `SELECT ... FOR UPDATE`, calls [`publication_decision`], and posts only on `Some`. A
//!   concurrent `SUPERSEDED` commit therefore yields `None` (SUP-003, R12).
//! - `None` is the fail-safe output: post nothing, no review, no comment, and no check-run
//!   conclusion of success.
//! - A failed run's check run, if one exists, is finalized by the GH tasks as `neutral` with an
//!   error summary, never `success` (GH-009).
//! - [`PublicationInput`] has no field for model confidence or model "approval" text, so
//!   self-reported completeness cannot reach the decision (INV-013).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::review::ReviewState;

/// The ONLY provider review events ReviewGraph can express. There is deliberately no approve,
/// request-changes or merge variant (legacy `core.rs:316-317`, `github.rs:5-6`). Deserializing
/// anything other than `"COMMENT"` fails, in Rust and in the generated TypeScript type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReviewEvent {
    Comment,
}

impl ReviewEvent {
    /// The value sent as the provider's review `event` parameter.
    pub const fn as_provider_event(self) -> &'static str {
        match self {
            ReviewEvent::Comment => "COMMENT",
        }
    }
}

/// Check-run conclusion. Never `failure` in the MVP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CheckConclusion {
    Success,
    Neutral,
}

/// Whether every applicable reviewer ran to completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Coverage {
    Complete,
    Degraded,
}

/// Facts the decision may use. Deliberately no confidence or model-text fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PublicationInput {
    pub publishable_findings: u32,
    pub coverage: Coverage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PublicationDecision {
    pub event: ReviewEvent,
    pub check_conclusion: CheckConclusion,
}

/// Port of the legacy `decision_for`. Only a run in `PUBLISHING` can publish; every other state,
/// including in-flight stages, `COMPLETED` (already published, so a replayed job cannot post a
/// second review), every `FAILED_*`, `SUPERSEDED` and `CANCELLED`, returns `None`, which callers
/// MUST treat as "post nothing".
///
/// The match has no wildcard arm: adding a `ReviewState` variant fails to compile until someone
/// decides what it maps to.
pub fn publication_decision(
    state: ReviewState,
    input: &PublicationInput,
) -> Option<PublicationDecision> {
    match state {
        ReviewState::Publishing => Some(PublicationDecision {
            event: ReviewEvent::Comment,
            check_conclusion: match (input.publishable_findings, input.coverage) {
                (0, Coverage::Complete) => CheckConclusion::Success,
                // Findings exist, or coverage was degraded: never a green pass on a partial review.
                _ => CheckConclusion::Neutral,
            },
        }),
        ReviewState::Received
        | ReviewState::Indexing
        | ReviewState::Analyzing
        | ReviewState::Reviewing
        | ReviewState::Verifying
        | ReviewState::Completed
        | ReviewState::FailedIndexing
        | ReviewState::FailedAnalysis
        | ReviewState::FailedReview
        | ReviewState::FailedPublish
        | ReviewState::Superseded
        | ReviewState::Cancelled => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorClass;
    use crate::ids::{CommitSha, OrganizationId, PullRequestId, RepositoryId, ReviewRunId};
    use crate::review::{ReviewRun, ReviewRunSpec, ReviewTrigger, RunFailure, TransitionInput};
    use chrono::{TimeZone, Utc};

    const FINDING_COUNTS: [u32; 3] = [0, 1, 50];
    const COVERAGES: [Coverage; 2] = [Coverage::Complete, Coverage::Degraded];

    fn input(publishable_findings: u32, coverage: Coverage) -> PublicationInput {
        PublicationInput {
            publishable_findings,
            coverage,
        }
    }

    #[test]
    fn inv_011_review_event_cannot_represent_approve_or_merge() {
        // An exhaustive match with no wildcard: this stops compiling if a variant is added.
        fn only_comment(event: ReviewEvent) -> &'static str {
            match event {
                ReviewEvent::Comment => "COMMENT",
            }
        }
        assert_eq!(only_comment(ReviewEvent::Comment), "COMMENT");
        assert_eq!(ReviewEvent::Comment.as_provider_event(), "COMMENT");

        for forbidden in [
            "APPROVE",
            "REQUEST_CHANGES",
            "MERGE",
            "approve",
            "comment",
            "Comment",
        ] {
            let json = format!("\"{forbidden}\"");
            assert!(
                serde_json::from_str::<ReviewEvent>(&json).is_err(),
                "{forbidden} must not deserialize"
            );
        }
        assert_eq!(
            serde_json::from_str::<ReviewEvent>("\"COMMENT\"").unwrap(),
            ReviewEvent::Comment
        );

        let schema = serde_json::to_value(schemars::schema_for!(ReviewEvent)).unwrap();
        assert_eq!(schema["enum"], serde_json::json!(["COMMENT"]));
    }

    #[test]
    fn inv_012_failure_states_publish_nothing() {
        for state in ReviewState::ALL {
            if state == ReviewState::Publishing {
                continue;
            }
            for findings in FINDING_COUNTS {
                for coverage in COVERAGES {
                    assert_eq!(
                        publication_decision(state, &input(findings, coverage)),
                        None,
                        "{state} with {findings} findings and {coverage:?} must publish nothing"
                    );
                }
            }
        }
    }

    #[test]
    fn inv_012_quota_style_failure_posts_nothing() {
        let at = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let sha = |c: char| -> CommitSha { c.to_string().repeat(40).parse().unwrap() };
        let mut run = ReviewRun::new(
            ReviewRunSpec {
                id: ReviewRunId::new(),
                organization_id: OrganizationId::new(),
                repository_id: RepositoryId::new(),
                pull_request_id: PullRequestId::new(),
                base_sha: sha('a'),
                head_sha: sha('b'),
                merge_base_sha: None,
                trigger: ReviewTrigger::Webhook,
                retry_of: None,
                trace_parent: None,
            },
            at,
        )
        .unwrap();
        for next in [
            ReviewState::Indexing,
            ReviewState::Analyzing,
            ReviewState::Reviewing,
        ] {
            run.apply_transition(next, TransitionInput::Advance, at)
                .unwrap();
        }
        let failure = RunFailure::new(ErrorClass::RateLimited, "provider quota exhausted").unwrap();
        run.apply_transition(
            ReviewState::FailedReview,
            TransitionInput::Fail(failure),
            at,
        )
        .unwrap();
        assert_eq!(run.state, ReviewState::FailedReview);
        for findings in FINDING_COUNTS {
            for coverage in COVERAGES {
                assert_eq!(
                    publication_decision(run.state, &input(findings, coverage)),
                    None
                );
            }
        }
    }

    #[test]
    fn inv_012_degraded_run_never_reports_success() {
        let decision =
            publication_decision(ReviewState::Publishing, &input(0, Coverage::Degraded)).unwrap();
        assert_eq!(decision.check_conclusion, CheckConclusion::Neutral);
        assert_eq!(decision.event, ReviewEvent::Comment);
    }

    #[test]
    fn completed_run_cannot_republish() {
        for findings in FINDING_COUNTS {
            for coverage in COVERAGES {
                assert_eq!(
                    publication_decision(ReviewState::Completed, &input(findings, coverage)),
                    None
                );
            }
        }
    }

    #[test]
    fn publishing_with_findings_is_neutral_comment() {
        for findings in [1, 2, 50, u32::MAX] {
            for coverage in COVERAGES {
                let d = publication_decision(ReviewState::Publishing, &input(findings, coverage))
                    .unwrap();
                assert_eq!(d.event, ReviewEvent::Comment);
                assert_eq!(d.check_conclusion, CheckConclusion::Neutral);
            }
        }
    }

    #[test]
    fn publishing_clean_complete_is_success_comment() {
        let d =
            publication_decision(ReviewState::Publishing, &input(0, Coverage::Complete)).unwrap();
        assert_eq!(
            d,
            PublicationDecision {
                event: ReviewEvent::Comment,
                check_conclusion: CheckConclusion::Success,
            }
        );
    }

    #[test]
    fn wire_forms() {
        assert_eq!(
            serde_json::to_string(&ReviewEvent::Comment).unwrap(),
            "\"COMMENT\""
        );
        assert_eq!(
            serde_json::to_string(&CheckConclusion::Success).unwrap(),
            "\"success\""
        );
        assert_eq!(
            serde_json::to_string(&CheckConclusion::Neutral).unwrap(),
            "\"neutral\""
        );
        assert!(serde_json::from_str::<CheckConclusion>("\"failure\"").is_err());
        let bad_input = r#"{"publishable_findings":0,"coverage":"complete","confidence":0.99}"#;
        assert!(
            serde_json::from_str::<PublicationInput>(bad_input).is_err(),
            "model confidence must not be accepted as an input"
        );
    }
}
