//! PostgreSQL adapter of [`ProfileStore`] (PROF-001) plus the advisory-locked compute guard
//! (PROF-006).
//!
//! `repository_profiles` holds the whole profile as jsonb; `profile_conventions` repeats each
//! convention as a row so the API can filter them. Both are written in one transaction and the
//! profile insert is `ON CONFLICT DO NOTHING` on the natural key, so concurrent writers converge.

use repository::store::RepoScope;
use review_core::ids::SnapshotId;
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

use super::{begin_scoped, PgStoreError};
use crate::model::{ProfileKey, RepositoryProfile};
use crate::store::{validate_for_save, ProfileStore, ProfileStoreError, SavedProfile};

#[derive(Debug, Clone)]
pub struct PgProfileStore {
    pool: PgPool,
}

fn backend(e: impl std::fmt::Display) -> ProfileStoreError {
    ProfileStoreError::Backend(e.to_string())
}

/// Deterministic row id of a key, so the conventions can be written in the same statement batch.
pub fn profile_row_id(key: &ProfileKey) -> Uuid {
    let name = format!(
        "{}/{}/{}/{}",
        key.repository_id, key.snapshot_id, key.profile_version.0, key.config_hash
    );
    Uuid::new_v5(&Uuid::NAMESPACE_OID, name.as_bytes())
}

fn version_i32(key: &ProfileKey) -> i32 {
    i32::try_from(key.profile_version.0).unwrap_or(i32::MAX)
}

async fn insert(
    tx: &mut Transaction<'_, Postgres>,
    scope: &RepoScope,
    profile: &RepositoryProfile,
    json: serde_json::Value,
) -> Result<bool, sqlx::Error> {
    let key = profile.key();
    let id = profile_row_id(&key);
    let inserted = sqlx::query(
        "INSERT INTO repository_profiles
           (id, organization_id, repository_id, snapshot_id, profile_version, config_hash,
            profile, computed_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         ON CONFLICT (repository_id, snapshot_id, profile_version, config_hash) DO NOTHING",
    )
    .bind(id)
    .bind(scope.organization_id.as_uuid())
    .bind(scope.repository_id.as_uuid())
    .bind(profile.snapshot_id.as_uuid())
    .bind(version_i32(&key))
    .bind(key.config_hash.to_string())
    .bind(json)
    .bind(profile.computed_at)
    .execute(&mut **tx)
    .await?
    .rows_affected()
        == 1;
    if !inserted {
        return Ok(false);
    }
    for c in &profile.conventions {
        sqlx::query(
            "INSERT INTO profile_conventions
               (profile_id, organization_id, convention_id, rule, scope, samples, violations,
                consistency, confidence, enforceable, exceptions, source)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
             ON CONFLICT (profile_id, convention_id) DO NOTHING",
        )
        .bind(id)
        .bind(scope.organization_id.as_uuid())
        .bind(&c.id)
        .bind(&c.rule)
        .bind(&c.scope)
        .bind(i32::try_from(c.samples).unwrap_or(i32::MAX))
        .bind(i32::try_from(c.violations).unwrap_or(i32::MAX))
        .bind(c.consistency)
        .bind(c.confidence)
        .bind(c.enforceable)
        .bind(serde_json::to_value(&c.exceptions).unwrap_or(serde_json::Value::Null))
        .bind(c.source.as_str())
        .execute(&mut **tx)
        .await?;
    }
    Ok(true)
}

fn decode(row: &sqlx::postgres::PgRow) -> Result<RepositoryProfile, ProfileStoreError> {
    let json: serde_json::Value = row.try_get("profile").map_err(backend)?;
    serde_json::from_value(json).map_err(backend)
}

impl PgProfileStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    async fn fetch(
        tx: &mut Transaction<'_, Postgres>,
        scope: &RepoScope,
        key: &ProfileKey,
    ) -> Result<Option<sqlx::postgres::PgRow>, sqlx::Error> {
        sqlx::query(
            "SELECT profile FROM repository_profiles
              WHERE organization_id = $1 AND repository_id = $2 AND snapshot_id = $3
                AND profile_version = $4 AND config_hash = $5",
        )
        .bind(scope.organization_id.as_uuid())
        .bind(scope.repository_id.as_uuid())
        .bind(key.snapshot_id.as_uuid())
        .bind(version_i32(key))
        .bind(key.config_hash.to_string())
        .fetch_optional(&mut **tx)
        .await
    }

    /// Returns the stored profile for `key`, computing and inserting it under
    /// `pg_advisory_xact_lock(hashtext('profile:'||repository_id||':'||snapshot_id))` when it is
    /// missing. Concurrent callers serialize on the lock; losers read the winner's row, so
    /// `compute` runs once per key (PROF-006). The `bool` is true when this call computed it.
    pub async fn get_or_compute<F>(
        &self,
        scope: &RepoScope,
        key: &ProfileKey,
        compute: F,
    ) -> Result<(RepositoryProfile, bool), PgStoreError>
    where
        F: FnOnce() -> RepositoryProfile + Send,
    {
        let mut tx = begin_scoped(&self.pool, scope).await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
            .bind(format!("profile:{}:{}", key.repository_id, key.snapshot_id))
            .execute(&mut *tx)
            .await?;
        if let Some(row) = Self::fetch(&mut tx, scope, key).await? {
            let json: serde_json::Value = row.try_get("profile")?;
            tx.commit().await?;
            return Ok((serde_json::from_value(json)?, false));
        }
        let profile = compute();
        if profile.key() != *key || profile.repository_id != scope.repository_id {
            return Err(PgStoreError::Corrupt(
                "computed profile does not match the requested key".to_owned(),
            ));
        }
        let json = serde_json::to_value(&profile)?;
        insert(&mut tx, scope, &profile, json).await?;
        tx.commit().await?;
        Ok((profile, true))
    }
}

impl ProfileStore for PgProfileStore {
    async fn save(
        &self,
        scope: &RepoScope,
        profile: &RepositoryProfile,
    ) -> Result<SavedProfile, ProfileStoreError> {
        let bytes = validate_for_save(scope, profile)?;
        let json: serde_json::Value = serde_json::from_slice(&bytes).map_err(backend)?;
        let mut tx = begin_scoped(&self.pool, scope).await.map_err(backend)?;
        let created = insert(&mut tx, scope, profile, json)
            .await
            .map_err(backend)?;
        tx.commit().await.map_err(backend)?;
        Ok(SavedProfile { created })
    }

    async fn get(
        &self,
        scope: &RepoScope,
        key: &ProfileKey,
    ) -> Result<Option<RepositoryProfile>, ProfileStoreError> {
        let mut tx = begin_scoped(&self.pool, scope).await.map_err(backend)?;
        let row = Self::fetch(&mut tx, scope, key).await.map_err(backend)?;
        tx.commit().await.map_err(backend)?;
        row.as_ref().map(decode).transpose()
    }

    async fn latest_for_snapshot(
        &self,
        scope: &RepoScope,
        snapshot_id: SnapshotId,
    ) -> Result<Option<RepositoryProfile>, ProfileStoreError> {
        let mut tx = begin_scoped(&self.pool, scope).await.map_err(backend)?;
        let row = sqlx::query(
            "SELECT profile FROM repository_profiles
              WHERE organization_id = $1 AND repository_id = $2 AND snapshot_id = $3
              ORDER BY computed_at DESC, profile_version DESC
              LIMIT 1",
        )
        .bind(scope.organization_id.as_uuid())
        .bind(scope.repository_id.as_uuid())
        .bind(snapshot_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(backend)?;
        tx.commit().await.map_err(backend)?;
        row.as_ref().map(decode).transpose()
    }
}
