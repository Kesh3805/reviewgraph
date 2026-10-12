//! Progress and outcome of index runs in `index_runs` (IDX-002).
//!
//! One row per snapshot being indexed. Every statement runs in a transaction with the tenant
//! context set, so RLS scopes it to the caller's organization. Progress is advisory: callers
//! log failures and carry on.

use chrono::{DateTime, Utc};
use repository::store::RepoScope;
use review_core::ids::SnapshotId;
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

use crate::StoreError;

/// One row of `index_runs`.
#[derive(Debug, Clone, PartialEq)]
pub struct IndexRunRow {
    pub id: Uuid,
    pub snapshot_id: Uuid,
    pub kind: String,
    pub phase: String,
    pub files_total: Option<i32>,
    pub files_done: i32,
    pub started_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub stats: serde_json::Value,
    pub error: Option<String>,
}

/// Reads and writes `index_runs`.
#[derive(Debug, Clone)]
pub struct PgIndexRuns {
    pool: PgPool,
}

fn db(error: sqlx::Error) -> StoreError {
    StoreError::backend(error)
}

fn clamp_i32(value: u64) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

fn row_of(row: &PgRow) -> Result<IndexRunRow, StoreError> {
    Ok(IndexRunRow {
        id: row.try_get("id").map_err(db)?,
        snapshot_id: row.try_get("snapshot_id").map_err(db)?,
        kind: row.try_get("kind").map_err(db)?,
        phase: row.try_get("phase").map_err(db)?,
        files_total: row.try_get("files_total").map_err(db)?,
        files_done: row.try_get("files_done").map_err(db)?,
        started_at: row.try_get("started_at").map_err(db)?,
        updated_at: row.try_get("updated_at").map_err(db)?,
        finished_at: row.try_get("finished_at").map_err(db)?,
        stats: row.try_get("stats").map_err(db)?,
        error: row.try_get("error").map_err(db)?,
    })
}

impl PgIndexRuns {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    async fn begin(&self, scope: &RepoScope) -> Result<Transaction<'static, Postgres>, StoreError> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        sqlx::query("SELECT set_config('app.organization_id', $1, true)")
            .bind(scope.organization_id.as_uuid().to_string())
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        Ok(tx)
    }

    /// Opens (or re-opens) the run row of `snapshot` in phase `walking` and returns its id.
    pub async fn start(
        &self,
        scope: &RepoScope,
        snapshot: SnapshotId,
        kind: &str,
        job_id: Option<Uuid>,
    ) -> Result<Uuid, StoreError> {
        let mut tx = self.begin(scope).await?;
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO index_runs (id, organization_id, repository_id, snapshot_id, job_id, \
                                     kind, phase)
             VALUES ($1, $2, $3, $4, $5, $6, 'walking')
             ON CONFLICT (snapshot_id)
               DO UPDATE SET phase = 'walking', files_done = 0, updated_at = now()
             RETURNING id",
        )
        .bind(Uuid::now_v7())
        .bind(scope.organization_id.as_uuid())
        .bind(scope.repository_id.as_uuid())
        .bind(snapshot.as_uuid())
        .bind(job_id)
        .bind(kind)
        .fetch_one(&mut *tx)
        .await
        .map_err(db)?;
        tx.commit().await.map_err(db)?;
        Ok(id)
    }

    /// Records the current phase, and the file counters when given.
    pub async fn update(
        &self,
        scope: &RepoScope,
        run: Uuid,
        phase: &str,
        files: Option<(u64, u64)>,
    ) -> Result<(), StoreError> {
        let mut tx = self.begin(scope).await?;
        sqlx::query(
            "UPDATE index_runs
                SET phase = $2,
                    files_done = COALESCE($3, files_done),
                    files_total = COALESCE($4, files_total),
                    updated_at = now()
              WHERE id = $1",
        )
        .bind(run)
        .bind(phase)
        .bind(files.map(|(done, _)| clamp_i32(done)))
        .bind(files.map(|(_, total)| clamp_i32(total)))
        .execute(&mut *tx)
        .await
        .map_err(db)?;
        tx.commit().await.map_err(db)?;
        Ok(())
    }

    /// Closes the run with its final phase, statistics and (redacted) error.
    pub async fn finish(
        &self,
        scope: &RepoScope,
        run: Uuid,
        phase: &str,
        stats: &serde_json::Value,
        error: Option<&str>,
    ) -> Result<(), StoreError> {
        let mut tx = self.begin(scope).await?;
        sqlx::query(
            "UPDATE index_runs
                SET phase = $2, stats = $3, error = $4, finished_at = now(), updated_at = now()
              WHERE id = $1",
        )
        .bind(run)
        .bind(phase)
        .bind(stats)
        .bind(error)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
        tx.commit().await.map_err(db)?;
        Ok(())
    }

    /// The newest run of the scope's repository, if any (backs the index-status endpoint).
    pub async fn latest(&self, scope: &RepoScope) -> Result<Option<IndexRunRow>, StoreError> {
        let mut tx = self.begin(scope).await?;
        let row = sqlx::query(
            "SELECT id, snapshot_id, kind, phase, files_total, files_done, started_at, \
                    updated_at, finished_at, stats, error
               FROM index_runs
              WHERE repository_id = $1
              ORDER BY started_at DESC, id DESC
              LIMIT 1",
        )
        .bind(scope.repository_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(db)?;
        tx.commit().await.map_err(db)?;
        row.as_ref().map(row_of).transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_clamp_to_i32() {
        assert_eq!(clamp_i32(5), 5);
        assert_eq!(clamp_i32(u64::MAX), i32::MAX);
    }
}
