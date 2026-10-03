//! PostgreSQL adapter of `RepositoryFactsStore` (INIT-013).
//!
//! Every statement runs inside a transaction that first sets `app.organization_id`
//! (`set_config(..., true)`), so row-level security applies, and carries an explicit
//! `organization_id` predicate as defence in depth. Parameters are always bound, never
//! interpolated. Facts are never logged: only ids and hashes.

use std::time::Duration;

use chrono::{DateTime, Utc};
use repository::facts::{RepositoryFacts, REPOSITORY_FACTS_SCHEMA};
use repository::store::{
    parse_detected_at, validate_for_save, FactsId, FactsStoreError, RepoScope,
    RepositoryFactsStore, SavedFacts, StoredFacts,
};
use review_core::ids::CommitSha;
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

const MAX_ATTEMPTS: u32 = 3;

#[derive(Debug, Clone)]
pub struct PgRepositoryFactsStore {
    pool: PgPool,
}

impl PgRepositoryFactsStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    async fn begin(&self, scope: &RepoScope) -> Result<Transaction<'_, Postgres>, sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT set_config('app.organization_id', $1, true)")
            .bind(scope.organization_id.as_uuid().to_string())
            .execute(&mut *tx)
            .await?;
        Ok(tx)
    }
}

/// Connection resets, serialization failures and deadlocks are worth retrying.
fn is_transient(error: &sqlx::Error) -> bool {
    match error {
        sqlx::Error::Io(_) | sqlx::Error::PoolTimedOut | sqlx::Error::PoolClosed => true,
        sqlx::Error::Database(db) => db
            .code()
            .is_some_and(|c| c == "40001" || c == "40P01" || c.starts_with("08")),
        _ => false,
    }
}

fn backend(error: sqlx::Error) -> FactsStoreError {
    FactsStoreError::Backend(error.to_string())
}

fn jitter(attempt: u32) -> Duration {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    Duration::from_millis(20 * u64::from(attempt) + u64::from(nanos % 25))
}

async fn retrying<T, F, Fut>(mut op: F) -> Result<T, FactsStoreError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, sqlx::Error>>,
{
    let mut attempt = 0;
    loop {
        attempt += 1;
        match op().await {
            Ok(value) => return Ok(value),
            Err(e) if attempt < MAX_ATTEMPTS && is_transient(&e) => {
                tokio::time::sleep(jitter(attempt)).await;
            }
            Err(e) => return Err(backend(e)),
        }
    }
}

fn stored_from_row(row: &sqlx::postgres::PgRow) -> Result<StoredFacts, FactsStoreError> {
    let version: i32 = row.try_get("facts_schema_version").map_err(backend)?;
    let version = u32::try_from(version).unwrap_or(u32::MAX);
    if version > REPOSITORY_FACTS_SCHEMA {
        return Err(FactsStoreError::SchemaUnsupported { version });
    }
    let json: serde_json::Value = row.try_get("facts").map_err(backend)?;
    let facts = RepositoryFacts::upgrade_from(version, &json.to_string())
        .map_err(|e| FactsStoreError::Backend(e.to_string()))?;
    let id: Uuid = row.try_get("id").map_err(backend)?;
    let created_at: DateTime<Utc> = row.try_get("created_at").map_err(backend)?;
    Ok(StoredFacts {
        id: FactsId(id),
        facts,
        created_at,
    })
}

impl RepositoryFactsStore for PgRepositoryFactsStore {
    async fn save(
        &self,
        scope: &RepoScope,
        facts: &RepositoryFacts,
    ) -> Result<SavedFacts, FactsStoreError> {
        let span = tracing::info_span!(
            "init.persist_facts",
            organization_id = %scope.organization_id,
            repository_id = %scope.repository_id,
            commit_sha = tracing::field::Empty,
            facts.created = tracing::field::Empty,
            facts.bytes = tracing::field::Empty
        );
        let _guard = span.enter();

        let (commit, bytes) = validate_for_save(facts)?;
        let detected_at = parse_detected_at(facts)?;
        span.record("commit_sha", commit.as_str());
        span.record("facts.bytes", bytes.len());
        let json: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|e| FactsStoreError::Backend(e.to_string()))?;
        let frameworks: Vec<String> = {
            let mut ids: Vec<String> = facts.frameworks.iter().map(|f| f.id.clone()).collect();
            ids.sort();
            ids.dedup();
            ids
        };
        let primary = facts.primary_language.map(|l| l.as_str().to_owned());
        let default_branch = facts.git.as_ref().and_then(|g| g.default_branch.clone());
        let warnings_count = i32::try_from(facts.warnings.len()).unwrap_or(i32::MAX);
        let version = i32::try_from(facts.schema_version).unwrap_or(i32::MAX);
        let org = *scope.organization_id.as_uuid();
        let repo = *scope.repository_id.as_uuid();

        let saved = retrying(|| async {
            let mut tx = self.begin(scope).await?;
            let inserted = sqlx::query(
                "INSERT INTO repository_init_facts
                   (organization_id, repository_id, commit_sha, facts_schema_version, tool_version,
                    facts_hash, fingerprint, primary_language, is_monorepo, frameworks,
                    warnings_count, facts, detected_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
                 ON CONFLICT ON CONSTRAINT repository_init_facts_dedup DO NOTHING
                 RETURNING id",
            )
            .bind(org)
            .bind(repo)
            .bind(commit.as_str())
            .bind(version)
            .bind(&facts.tool_version)
            .bind(&facts.facts_hash)
            .bind(&facts.fingerprint)
            .bind(&primary)
            .bind(facts.workspaces.is_monorepo)
            .bind(&frameworks)
            .bind(warnings_count)
            .bind(&json)
            .bind(detected_at)
            .fetch_optional(&mut *tx)
            .await?;
            let (id, created) = match inserted {
                Some(row) => (row.try_get::<Uuid, _>("id")?, true),
                None => {
                    let row = sqlx::query(
                        "SELECT id FROM repository_init_facts
                         WHERE repository_id = $1 AND organization_id = $2
                           AND commit_sha = $3 AND facts_hash = $4",
                    )
                    .bind(repo)
                    .bind(org)
                    .bind(commit.as_str())
                    .bind(&facts.facts_hash)
                    .fetch_one(&mut *tx)
                    .await?;
                    (row.try_get::<Uuid, _>("id")?, false)
                }
            };
            // The pointer only moves forward: a late-finishing older init cannot overwrite a
            // newer one.
            sqlx::query(
                "UPDATE repositories
                   SET latest_init_facts_id = $1,
                       default_branch = COALESCE($2, default_branch),
                       primary_language = $3,
                       initialized_at = now()
                 WHERE id = $4 AND organization_id = $5
                   AND (latest_init_facts_id IS NULL
                        OR (SELECT detected_at FROM repository_init_facts
                             WHERE id = latest_init_facts_id) <= $6)",
            )
            .bind(id)
            .bind(&default_branch)
            .bind(&primary)
            .bind(repo)
            .bind(org)
            .bind(detected_at)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            Ok::<_, sqlx::Error>(SavedFacts {
                id: FactsId(id),
                created,
            })
        })
        .await?;
        span.record("facts.created", saved.created);
        tracing::debug!(facts_id = %saved.id.0, facts_hash = %facts.facts_hash, "repository facts saved");
        Ok(saved)
    }

    async fn latest(&self, scope: &RepoScope) -> Result<Option<StoredFacts>, FactsStoreError> {
        let org = *scope.organization_id.as_uuid();
        let repo = *scope.repository_id.as_uuid();
        let row = retrying(|| async {
            let mut tx = self.begin(scope).await?;
            let row = sqlx::query(
                "SELECT id, facts, facts_schema_version, created_at
                   FROM repository_init_facts
                  WHERE repository_id = $1 AND organization_id = $2
                  ORDER BY detected_at DESC, created_at DESC
                  LIMIT 1",
            )
            .bind(repo)
            .bind(org)
            .fetch_optional(&mut *tx)
            .await?;
            tx.commit().await?;
            Ok::<_, sqlx::Error>(row)
        })
        .await?;
        row.as_ref().map(stored_from_row).transpose()
    }

    async fn by_commit(
        &self,
        scope: &RepoScope,
        commit_sha: &CommitSha,
    ) -> Result<Option<StoredFacts>, FactsStoreError> {
        let org = *scope.organization_id.as_uuid();
        let repo = *scope.repository_id.as_uuid();
        let row = retrying(|| async {
            let mut tx = self.begin(scope).await?;
            let row = sqlx::query(
                "SELECT id, facts, facts_schema_version, created_at
                   FROM repository_init_facts
                  WHERE repository_id = $1 AND organization_id = $2 AND commit_sha = $3
                  ORDER BY detected_at DESC, created_at DESC
                  LIMIT 1",
            )
            .bind(repo)
            .bind(org)
            .bind(commit_sha.as_str())
            .fetch_optional(&mut *tx)
            .await?;
            tx.commit().await?;
            Ok::<_, sqlx::Error>(row)
        })
        .await?;
        row.as_ref().map(stored_from_row).transpose()
    }
}
