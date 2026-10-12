//! `repository_profiles` / `profile_conventions` (PROF-001) and the compute guard (PROF-006).

use std::future::Future;

use repository::store::RepoScope;
use sqlx::{PgPool, Postgres, Row, Transaction};

use super::{begin_scoped, PgStoreError};
use crate::model::{ProfileKey, RepositoryProfile};

#[derive(Debug, Clone)]
pub struct PgProfileStore {
    pool: PgPool,
}

/// Inserts the profile and its conventions inside `tx`; `false` when the key already existed.
async fn insert_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    scope: &RepoScope,
    profile: &RepositoryProfile,
) -> Result<bool, PgStoreError> {
    let json = serde_json::to_value(profile)?;
    let row = sqlx::query(
        "INSERT INTO repository_profiles
           (organization_id, repository_id, snapshot_id, profile_version, config_hash,
            profile, computed_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT (repository_id, snapshot_id, profile_version, config_hash) DO NOTHING
         RETURNING id",
    )
    .bind(scope.organization_id.as_uuid())
    .bind(scope.repository_id.as_uuid())
    .bind(profile.snapshot_id.as_uuid())
    .bind(profile.profile_version as i32)
    .bind(profile.config_hash.to_string())
    .bind(json)
    .bind(profile.computed_at)
    .fetch_optional(&mut **tx)
    .await?;
    let Some(row) = row else {
        return Ok(false);
    };
    let profile_id: uuid::Uuid = row.try_get("id")?;
    for c in &profile.conventions {
        sqlx::query(
            "INSERT INTO profile_conventions
               (profile_id, convention_id, rule, scope, samples, violations, consistency,
                confidence, exceptions, source)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
             ON CONFLICT DO NOTHING",
        )
        .bind(profile_id)
        .bind(&c.id)
        .bind(&c.rule)
        .bind(&c.scope)
        .bind(c.samples as i32)
        .bind(c.violations as i32)
        .bind(c.consistency)
        .bind(c.confidence)
        .bind(serde_json::to_value(&c.exceptions)?)
        .bind(c.source.as_str())
        .execute(&mut **tx)
        .await?;
    }
    Ok(true)
}

async fn select_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    scope: &RepoScope,
    key: &ProfileKey,
) -> Result<Option<RepositoryProfile>, PgStoreError> {
    let row = sqlx::query(
        "SELECT profile FROM repository_profiles
          WHERE organization_id = $1 AND repository_id = $2 AND snapshot_id = $3
            AND profile_version = $4 AND config_hash = $5",
    )
    .bind(scope.organization_id.as_uuid())
    .bind(key.repository_id.as_uuid())
    .bind(key.snapshot_id.as_uuid())
    .bind(key.profile_version as i32)
    .bind(key.config_hash.to_string())
    .fetch_optional(&mut **tx)
    .await?;
    match row {
        None => Ok(None),
        Some(r) => {
            let v: serde_json::Value = r.try_get("profile")?;
            Ok(Some(serde_json::from_value(v)?))
        }
    }
}

impl PgProfileStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Stores the profile and its conventions; returns `false` when the key already existed.
    pub async fn put(
        &self,
        scope: &RepoScope,
        profile: &RepositoryProfile,
    ) -> Result<bool, PgStoreError> {
        let mut tx = begin_scoped(&self.pool, scope).await?;
        let inserted = insert_in_tx(&mut tx, scope, profile).await?;
        tx.commit().await?;
        Ok(inserted)
    }

    pub async fn get(
        &self,
        scope: &RepoScope,
        key: &ProfileKey,
    ) -> Result<Option<RepositoryProfile>, PgStoreError> {
        let mut tx = begin_scoped(&self.pool, scope).await?;
        let found = select_in_tx(&mut tx, scope, key).await?;
        tx.commit().await?;
        Ok(found)
    }

    /// Returns the stored profile for `key`, computing and inserting it when absent (PROF-006).
    ///
    /// The computation is serialized per `(repository, snapshot)` by a transaction-scoped
    /// advisory lock, so concurrent workers never duplicate the work: the losers block on the
    /// lock, then read the row the winner inserted. The boolean is `true` when this call ran
    /// `compute`. A compute failure rolls back and leaves the previous profile untouched.
    pub async fn get_or_compute<F, Fut>(
        &self,
        scope: &RepoScope,
        key: &ProfileKey,
        compute: F,
    ) -> Result<(RepositoryProfile, bool), PgStoreError>
    where
        F: FnOnce() -> Fut + Send,
        Fut: Future<Output = Result<RepositoryProfile, String>> + Send,
    {
        let mut tx = begin_scoped(&self.pool, scope).await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
            .bind(format!(
                "profile:{}:{}",
                key.repository_id.as_uuid(),
                key.snapshot_id.as_uuid()
            ))
            .execute(&mut *tx)
            .await?;
        if let Some(existing) = select_in_tx(&mut tx, scope, key).await? {
            tx.commit().await?;
            return Ok((existing, false));
        }
        let computed = compute().await.map_err(PgStoreError::Compute)?;
        if computed.key() != *key {
            return Err(PgStoreError::Corrupt(
                "computed profile does not match the requested key".to_owned(),
            ));
        }
        insert_in_tx(&mut tx, scope, &computed).await?;
        tx.commit().await?;
        Ok((computed, true))
    }
}
