//! Review state machine persistence: compare-and-set transitions (PIPE-007).
//!
//! DOM-008 decides which edges are legal; this module applies them under concurrency.
//! Supersession, cancellation, a retried job and a normal stage advance can all race on one run
//! row, so every transition is `UPDATE ... WHERE state = $from`. Exactly one writer wins any
//! `(run, from)` edge, and a stage that loses must not publish or enqueue anything.
//!
//! Lock order when combined with publish or supersession (SUP-003) is `pull_requests` then
//! `review_runs`; this store never locks the pull request row itself.

use async_trait::async_trait;
use review_core::ids::{OrganizationId, ReviewRunId};
use review_core::review::{ReviewState, RunFailure, TransitionInput};
use review_core::CoreError;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use super::{parse_state, StoreError};

const MAX_DETAIL_CHARS: usize = 2000;

/// Outcome of a transition request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionResult {
    /// This call moved the run and wrote the audit row.
    Applied,
    /// The run is already at or past the target (a retried job re-applying an advance). Done.
    AlreadyAtOrAfter(ReviewState),
    /// Someone else won (superseded, cancelled, failed, or an unrelated state). The caller maps
    /// this to `ErrorClass::Conflict` and aborts with no side effects.
    Lost { observed: ReviewState },
}

/// Position on the forward path (`Received` .. `Completed`); `None` for off-path states.
const fn forward_rank(s: ReviewState) -> Option<u8> {
    match s {
        ReviewState::Received => Some(0),
        ReviewState::Indexing => Some(1),
        ReviewState::Analyzing => Some(2),
        ReviewState::Reviewing => Some(3),
        ReviewState::Verifying => Some(4),
        ReviewState::Publishing => Some(5),
        ReviewState::Completed => Some(6),
        _ => None,
    }
}

/// Interprets a lost CAS: `observed` at or after `to` on the forward path (or exactly `to`) is
/// treated as done; anything else (including every terminal state other than `to`) is lost.
pub fn interpret_conflict(to: ReviewState, observed: ReviewState) -> TransitionResult {
    if observed == to {
        return TransitionResult::AlreadyAtOrAfter(observed);
    }
    match (forward_rank(to), forward_rank(observed)) {
        (Some(t), Some(o)) if o >= t => TransitionResult::AlreadyAtOrAfter(observed),
        _ => TransitionResult::Lost { observed },
    }
}

/// The in-memory pre-check (mirrors `ReviewRun::apply_transition`): a legal edge whose input
/// matches the target. An illegal request never reaches SQL.
pub fn validate_request(
    from: ReviewState,
    to: ReviewState,
    input: &TransitionInput,
    run: ReviewRunId,
) -> Result<(), CoreError> {
    let invalid = || CoreError::InvalidTransition {
        entity: "ReviewRun",
        from: from.as_str(),
        to: to.as_str(),
    };
    if !from.can_transition_to(to) {
        return Err(invalid());
    }
    match (to, input) {
        (t, TransitionInput::Fail(_)) if t.is_failed() => Ok(()),
        (ReviewState::Superseded, TransitionInput::Supersede { by }) if *by != run => Ok(()),
        (ReviewState::Cancelled, TransitionInput::Cancel) => Ok(()),
        (t, TransitionInput::Advance)
            if !t.is_failed() && t != ReviewState::Superseded && t != ReviewState::Cancelled =>
        {
            Ok(())
        }
        _ => Err(invalid()),
    }
}

fn cleaned_detail(failure: &RunFailure) -> String {
    let scrubbed = model_gateway::redact::scrub_text(&failure.detail);
    scrubbed.chars().take(MAX_DETAIL_CHARS).collect()
}

/// The persisted-transition port (faked in orchestrator tests).
#[async_trait]
pub trait ReviewStateStore: Send + Sync + std::fmt::Debug {
    /// Applies `from -> to` as a compare-and-set in its own tenant transaction.
    async fn transition(
        &self,
        org: OrganizationId,
        run: ReviewRunId,
        from: ReviewState,
        to: ReviewState,
        input: TransitionInput,
        job: Option<Uuid>,
    ) -> Result<TransitionResult, StoreError>;

    /// A plain read for the cancel watcher.
    async fn current_state(
        &self,
        org: OrganizationId,
        run: ReviewRunId,
    ) -> Result<ReviewState, StoreError>;
}

/// PostgreSQL implementation.
#[derive(Debug, Clone)]
pub struct PgReviewStateStore {
    pool: PgPool,
}

impl PgReviewStateStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// The CAS inside the caller's transaction (tenant already set), so a transition can be
    /// combined with other writes, such as enqueuing `review-publish` on the hand-off.
    pub async fn transition_in(
        conn: &mut PgConnection,
        org: OrganizationId,
        run: ReviewRunId,
        from: ReviewState,
        to: ReviewState,
        input: TransitionInput,
        job: Option<Uuid>,
    ) -> Result<TransitionResult, StoreError> {
        validate_request(from, to, &input, run)?;
        let (failure_class, failure_detail, superseded_by) = match &input {
            TransitionInput::Fail(f) => (
                Some(f.class.as_str()),
                Some(cleaned_detail(f)),
                None::<Uuid>,
            ),
            TransitionInput::Supersede { by } => (None, None, Some(*by.as_uuid())),
            TransitionInput::Advance | TransitionInput::Cancel => (None, None, None),
        };
        let applied: Option<Uuid> = sqlx::query_scalar(
            "UPDATE review_runs SET state = $4, failure_class = $5, failure_detail = $6,
                    superseded_by = $7,
                    superseded_at = CASE WHEN $4 = 'SUPERSEDED' THEN now() ELSE superseded_at END,
                    completed_at = CASE WHEN $8 THEN now() ELSE completed_at END
             WHERE id = $1 AND organization_id = $2 AND state = $3
             RETURNING id",
        )
        .bind(run.as_uuid())
        .bind(org.as_uuid())
        .bind(from.as_str())
        .bind(to.as_str())
        .bind(failure_class)
        .bind(&failure_detail)
        .bind(superseded_by)
        .bind(to.is_terminal())
        .fetch_optional(&mut *conn)
        .await?;
        if applied.is_some() {
            let detail = failure_class
                .map(|c| serde_json::json!({ "class": c, "detail": failure_detail }).to_string());
            sqlx::query(
                "INSERT INTO review_run_transitions
                   (review_run_id, organization_id, from_state, to_state, job_id, detail)
                 VALUES ($1, $2, $3, $4, $5, $6::jsonb)",
            )
            .bind(run.as_uuid())
            .bind(org.as_uuid())
            .bind(from.as_str())
            .bind(to.as_str())
            .bind(job)
            .bind(detail)
            .execute(&mut *conn)
            .await?;
            tracing::debug!(
                from = from.as_str(),
                to = to.as_str(),
                "review_state_transition"
            );
            return Ok(TransitionResult::Applied);
        }
        let observed = Self::read_state(conn, org, run).await?;
        let result = interpret_conflict(to, observed);
        if matches!(result, TransitionResult::Lost { .. }) {
            tracing::info!(
                from = from.as_str(),
                observed = observed.as_str(),
                "review_state_cas_lost"
            );
        }
        Ok(result)
    }

    async fn read_state(
        conn: &mut PgConnection,
        org: OrganizationId,
        run: ReviewRunId,
    ) -> Result<ReviewState, StoreError> {
        let state: Option<String> = sqlx::query_scalar(
            "SELECT state FROM review_runs WHERE id = $1 AND organization_id = $2",
        )
        .bind(run.as_uuid())
        .bind(org.as_uuid())
        .fetch_optional(conn)
        .await?;
        parse_state(&state.ok_or(StoreError::NotFound)?)
    }

    /// Fails the run from `from`, choosing the matching `FAILED_*` target (INDEXING to
    /// FAILED_INDEXING, ANALYZING to FAILED_ANALYSIS, REVIEWING or VERIFYING to FAILED_REVIEW,
    /// PUBLISHING to FAILED_PUBLISH).
    pub async fn fail_run(
        &self,
        org: OrganizationId,
        run: ReviewRunId,
        from: ReviewState,
        failure: RunFailure,
        job: Option<Uuid>,
    ) -> Result<TransitionResult, StoreError> {
        let to = from.failure_state().ok_or(CoreError::InvalidTransition {
            entity: "ReviewRun",
            from: from.as_str(),
            to: "FAILED_*",
        })?;
        self.transition(org, run, from, to, TransitionInput::Fail(failure), job)
            .await
    }

    /// Supersedes the run (`by` is the newer run).
    pub async fn supersede(
        &self,
        org: OrganizationId,
        run: ReviewRunId,
        from: ReviewState,
        by: ReviewRunId,
        job: Option<Uuid>,
    ) -> Result<TransitionResult, StoreError> {
        self.transition(
            org,
            run,
            from,
            ReviewState::Superseded,
            TransitionInput::Supersede { by },
            job,
        )
        .await
    }

    /// Cancels the run.
    pub async fn cancel(
        &self,
        org: OrganizationId,
        run: ReviewRunId,
        from: ReviewState,
        job: Option<Uuid>,
    ) -> Result<TransitionResult, StoreError> {
        self.transition(
            org,
            run,
            from,
            ReviewState::Cancelled,
            TransitionInput::Cancel,
            job,
        )
        .await
    }
}

#[async_trait]
impl ReviewStateStore for PgReviewStateStore {
    async fn transition(
        &self,
        org: OrganizationId,
        run: ReviewRunId,
        from: ReviewState,
        to: ReviewState,
        input: TransitionInput,
        job: Option<Uuid>,
    ) -> Result<TransitionResult, StoreError> {
        // Illegal requests are rejected before a transaction is opened.
        validate_request(from, to, &input, run)?;
        let mut tx = super::tenant_tx(&self.pool, org.as_uuid()).await?;
        let result = Self::transition_in(&mut tx, org, run, from, to, input, job).await?;
        tx.commit().await?;
        Ok(result)
    }

    async fn current_state(
        &self,
        org: OrganizationId,
        run: ReviewRunId,
    ) -> Result<ReviewState, StoreError> {
        let mut tx = super::tenant_tx(&self.pool, org.as_uuid()).await?;
        let state = Self::read_state(&mut tx, org, run).await?;
        tx.commit().await?;
        Ok(state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use review_core::ErrorClass;
    use ReviewState as S;

    #[test]
    fn illegal_edge_rejected_before_sql() {
        let run = ReviewRunId::new();
        assert!(
            validate_request(S::Received, S::Reviewing, &TransitionInput::Advance, run).is_err()
        );
        assert!(
            validate_request(S::Completed, S::Indexing, &TransitionInput::Advance, run).is_err()
        );
    }

    #[test]
    fn input_must_match_target() {
        let run = ReviewRunId::new();
        let fail = TransitionInput::Fail(RunFailure::new(ErrorClass::Permanent, "x").unwrap());
        assert!(validate_request(S::Indexing, S::FailedIndexing, &fail, run).is_ok());
        assert!(validate_request(
            S::Indexing,
            S::FailedIndexing,
            &TransitionInput::Advance,
            run
        )
        .is_err());
        assert!(validate_request(S::Indexing, S::Analyzing, &fail, run).is_err());
        let by = ReviewRunId::new();
        assert!(validate_request(
            S::Indexing,
            S::Superseded,
            &TransitionInput::Supersede { by },
            run
        )
        .is_ok());
        assert!(validate_request(
            S::Indexing,
            S::Superseded,
            &TransitionInput::Supersede { by: run },
            run
        )
        .is_err());
        assert!(validate_request(S::Indexing, S::Cancelled, &TransitionInput::Cancel, run).is_ok());
    }

    #[test]
    fn duplicate_advance_is_already_at_or_after() {
        assert_eq!(
            interpret_conflict(S::Analyzing, S::Reviewing),
            TransitionResult::AlreadyAtOrAfter(S::Reviewing)
        );
        assert_eq!(
            interpret_conflict(S::Analyzing, S::Analyzing),
            TransitionResult::AlreadyAtOrAfter(S::Analyzing)
        );
        assert_eq!(
            interpret_conflict(S::Publishing, S::Completed),
            TransitionResult::AlreadyAtOrAfter(S::Completed)
        );
    }

    #[test]
    fn late_advance_after_supersede_is_lost() {
        for off_path in [
            S::Superseded,
            S::Cancelled,
            S::FailedIndexing,
            S::FailedAnalysis,
            S::FailedReview,
            S::FailedPublish,
        ] {
            assert_eq!(
                interpret_conflict(S::Analyzing, off_path),
                TransitionResult::Lost { observed: off_path }
            );
        }
        // An earlier forward state than the target is also not "done".
        assert_eq!(
            interpret_conflict(S::Reviewing, S::Indexing),
            TransitionResult::Lost {
                observed: S::Indexing
            }
        );
    }

    #[test]
    fn terminal_observed_state_is_lost_unless_equal_to_target() {
        assert_eq!(
            interpret_conflict(S::Superseded, S::Superseded),
            TransitionResult::AlreadyAtOrAfter(S::Superseded)
        );
        assert_eq!(
            interpret_conflict(S::Superseded, S::Cancelled),
            TransitionResult::Lost {
                observed: S::Cancelled
            }
        );
    }

    #[test]
    fn failure_detail_truncated_and_redacted() {
        let secret = format!("token ghp_{}", "a".repeat(36));
        let long = format!("{secret}{}", "x".repeat(5000));
        let f = RunFailure {
            class: ErrorClass::Permanent,
            detail: long,
        };
        let cleaned = cleaned_detail(&f);
        assert!(cleaned.chars().count() <= MAX_DETAIL_CHARS);
        assert!(!cleaned.contains("ghp_aaaa"));
    }
}
