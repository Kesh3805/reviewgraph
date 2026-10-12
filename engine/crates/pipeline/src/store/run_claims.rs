//! One run per head, and manual retries (PIPE-005).
//!
//! `get_or_create_run` is the engine-side twin of the control plane's run creation (SUP-001 owns
//! the webhook path): it inserts against the `review_runs_first_run_per_head` index and returns
//! the existing run for a duplicate event. `retry_run` creates a new run with `retry_of` set from
//! a `FAILED_*` run and copies the stage outputs that are reusable across a retry.

use review_core::ids::{OrganizationId, ReviewRunId};
use review_core::review::{ReviewState, ReviewTrigger};
use sqlx::PgPool;
use uuid::Uuid;

use super::{is_unique_violation, parse_state, tenant_tx, StoreError};

/// Stages whose stored output stays valid for a retry of a failed run (they depend only on the
/// repository content, which a retry does not change).
pub const REUSABLE_ACROSS_RETRY: &[&str] = &["checkout", "index"];

/// What the engine needs to know about a run row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRow {
    pub id: ReviewRunId,
    pub state: ReviewState,
    pub retry_of: Option<ReviewRunId>,
    pub head_sha: String,
    pub base_sha: String,
}

/// The facts of a run to create.
#[derive(Debug, Clone)]
pub struct NewRun {
    pub id: ReviewRunId,
    pub organization_id: OrganizationId,
    pub repository_id: Uuid,
    pub pull_request_id: Uuid,
    pub base_sha: String,
    pub head_sha: String,
    pub merge_base_sha: Option<String>,
    pub trigger: ReviewTrigger,
    pub trace_parent: Option<String>,
}

const fn trigger_str(t: ReviewTrigger) -> &'static str {
    match t {
        ReviewTrigger::Webhook => "webhook",
        ReviewTrigger::Manual => "manual",
        ReviewTrigger::Reconciler => "reconciler",
        ReviewTrigger::Cli => "cli",
    }
}

type RunTuple = (Uuid, String, Option<Uuid>, String, String);

fn row_from(t: RunTuple) -> Result<RunRow, StoreError> {
    Ok(RunRow {
        id: ReviewRunId::from_uuid(t.0),
        state: parse_state(&t.1)?,
        retry_of: t.2.map(ReviewRunId::from_uuid),
        head_sha: t.3,
        base_sha: t.4,
    })
}

#[derive(Debug, Clone)]
pub struct RunClaims {
    pool: PgPool,
}

impl RunClaims {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Returns `(run, created)`. A replay of the same `(pull request, head)` returns the existing
    /// first run. A different head while another run is still active is a `Conflict` (supersede
    /// the old run first, which is the control plane's job).
    pub async fn get_or_create_run(&self, new: NewRun) -> Result<(RunRow, bool), StoreError> {
        let org = *new.organization_id.as_uuid();
        let mut tx = tenant_tx(&self.pool, &org).await?;
        let inserted = sqlx::query_scalar::<_, Uuid>(
            "INSERT INTO review_runs
               (id, organization_id, repository_id, pull_request_id, base_sha, head_sha,
                merge_base_sha, state, trigger, trace_parent)
             VALUES ($1, $2, $3, $4, $5, $6, $7, 'RECEIVED', $8, $9)
             ON CONFLICT (pull_request_id, head_sha) WHERE retry_of IS NULL DO NOTHING
             RETURNING id",
        )
        .bind(new.id.as_uuid())
        .bind(org)
        .bind(new.repository_id)
        .bind(new.pull_request_id)
        .bind(&new.base_sha)
        .bind(&new.head_sha)
        .bind(&new.merge_base_sha)
        .bind(trigger_str(new.trigger))
        .bind(&new.trace_parent)
        .fetch_optional(&mut *tx)
        .await;
        let created = match inserted {
            Ok(id) => id.is_some(),
            Err(e) if is_unique_violation(&e) => {
                return Err(StoreError::Conflict(
                    "another run is active for this pull request".into(),
                ))
            }
            Err(e) => return Err(e.into()),
        };
        let row: RunTuple = sqlx::query_as(
            "SELECT id, state, retry_of, head_sha, base_sha FROM review_runs
             WHERE organization_id = $1 AND pull_request_id = $2 AND head_sha = $3
               AND retry_of IS NULL",
        )
        .bind(org)
        .bind(new.pull_request_id)
        .bind(&new.head_sha)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok((row_from(row)?, created))
    }

    /// Creates a new run (`RECEIVED`, trigger `manual`) that retries `failed_run`. Valid only for
    /// a `FAILED_*` run; at most one run per pull request may be active.
    pub async fn retry_run(
        &self,
        org: OrganizationId,
        failed_run: ReviewRunId,
        new_id: ReviewRunId,
    ) -> Result<RunRow, StoreError> {
        let mut tx = tenant_tx(&self.pool, org.as_uuid()).await?;
        let inserted = sqlx::query_as::<_, RunTuple>(
            "INSERT INTO review_runs
               (id, organization_id, repository_id, pull_request_id, base_sha, head_sha,
                merge_base_sha, state, trigger, retry_of, trace_parent)
             SELECT $3, organization_id, repository_id, pull_request_id, base_sha, head_sha,
                    merge_base_sha, 'RECEIVED', 'manual', id, trace_parent
             FROM review_runs
             WHERE id = $2 AND organization_id = $1 AND state LIKE 'FAILED\\_%'
             RETURNING id, state, retry_of, head_sha, base_sha",
        )
        .bind(org.as_uuid())
        .bind(failed_run.as_uuid())
        .bind(new_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await;
        let row = match inserted {
            Ok(Some(r)) => r,
            Ok(None) => {
                let exists: Option<String> = sqlx::query_scalar(
                    "SELECT state FROM review_runs WHERE id = $1 AND organization_id = $2",
                )
                .bind(failed_run.as_uuid())
                .bind(org.as_uuid())
                .fetch_optional(&mut *tx)
                .await?;
                return Err(match exists {
                    None => StoreError::NotFound,
                    Some(_) => StoreError::Conflict("only a FAILED_* run can be retried".into()),
                });
            }
            Err(e) if is_unique_violation(&e) => {
                return Err(StoreError::Conflict(
                    "another run is active for this pull request".into(),
                ))
            }
            Err(e) => return Err(e.into()),
        };
        tx.commit().await?;
        row_from(row)
    }

    /// Copies the succeeded outputs of `stages` from one run to another (their `input_hash`
    /// does not include the run id, so they are valid for the retry). Returns the rows copied.
    pub async fn copy_reusable_outputs(
        &self,
        org: OrganizationId,
        from: ReviewRunId,
        to: ReviewRunId,
        stages: &[&str],
    ) -> Result<u64, StoreError> {
        let mut tx = tenant_tx(&self.pool, org.as_uuid()).await?;
        let stages: Vec<&str> = stages.to_vec();
        let done = sqlx::query(
            "INSERT INTO stage_outputs
               (review_run_id, organization_id, stage, input_hash, output_hash, output, blob_key,
                status, schema_version, duration_ms)
             SELECT $3, organization_id, stage, input_hash, output_hash, output, blob_key,
                    status, schema_version, duration_ms
             FROM stage_outputs
             WHERE review_run_id = $2 AND organization_id = $1 AND stage = ANY($4)
               AND status = 'succeeded'
             ON CONFLICT DO NOTHING",
        )
        .bind(org.as_uuid())
        .bind(from.as_uuid())
        .bind(to.as_uuid())
        .bind(&stages)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(done.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triggers_map_to_the_check_values() {
        assert_eq!(trigger_str(ReviewTrigger::Webhook), "webhook");
        assert_eq!(trigger_str(ReviewTrigger::Manual), "manual");
        assert_eq!(trigger_str(ReviewTrigger::Reconciler), "reconciler");
        assert_eq!(trigger_str(ReviewTrigger::Cli), "cli");
    }
}
