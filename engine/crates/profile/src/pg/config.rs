//! `repository_configs` (POL-002): content-addressed per `(repository_id, config_hash)`.

use repository::store::RepoScope;
use sqlx::{PgPool, Row};

use super::{begin_scoped, PgStoreError};
use crate::config::sync::SyncedConfig;
use crate::config::ConfigStatus;

#[derive(Debug, Clone)]
pub struct PgConfigStore {
    pool: PgPool,
}

/// A stored config row.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredConfig {
    pub config_hash: String,
    pub status: String,
    pub normalized: serde_json::Value,
    pub raw_blob_sha: Option<String>,
    pub validation: serde_json::Value,
}

fn status_str(status: ConfigStatus) -> &'static str {
    match status {
        ConfigStatus::Missing => "missing",
        ConfigStatus::Valid => "valid",
        ConfigStatus::Invalid => "invalid",
    }
}

impl PgConfigStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Stores the config; returns `false` when the same hash was already stored (a no-op).
    pub async fn save(
        &self,
        scope: &RepoScope,
        synced: &SyncedConfig,
    ) -> Result<bool, PgStoreError> {
        let validation = serde_json::to_value(&synced.loaded.issues)?;
        let mut tx = begin_scoped(&self.pool, scope).await?;
        let inserted = sqlx::query(
            "INSERT INTO repository_configs
               (organization_id, repository_id, config_hash, status, normalized, raw_blob_sha,
                validation)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT (repository_id, config_hash) DO NOTHING",
        )
        .bind(scope.organization_id.as_uuid())
        .bind(scope.repository_id.as_uuid())
        .bind(synced.loaded.config_hash.to_string())
        .bind(status_str(synced.loaded.status))
        .bind(&synced.loaded.normalized)
        .bind(synced.raw_blob_sha.as_deref())
        .bind(validation)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        tx.commit().await?;
        Ok(inserted == 1)
    }

    /// Records where a snapshot's config came from (`snapshots.config_source_path`).
    pub async fn bind_snapshot(
        &self,
        scope: &RepoScope,
        snapshot_id: uuid::Uuid,
        synced: &SyncedConfig,
    ) -> Result<(), PgStoreError> {
        let mut tx = begin_scoped(&self.pool, scope).await?;
        sqlx::query(
            "UPDATE snapshots SET config_source_path = $1
              WHERE id = $2 AND organization_id = $3",
        )
        .bind(synced.source_path.as_deref())
        .bind(snapshot_id)
        .bind(scope.organization_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn get(
        &self,
        scope: &RepoScope,
        config_hash: &str,
    ) -> Result<Option<StoredConfig>, PgStoreError> {
        let mut tx = begin_scoped(&self.pool, scope).await?;
        let row = sqlx::query(
            "SELECT config_hash, status, normalized, raw_blob_sha, validation
               FROM repository_configs
              WHERE organization_id = $1 AND repository_id = $2 AND config_hash = $3",
        )
        .bind(scope.organization_id.as_uuid())
        .bind(scope.repository_id.as_uuid())
        .bind(config_hash)
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        let Some(row) = row else {
            return Ok(None);
        };
        Ok(Some(StoredConfig {
            config_hash: row.try_get("config_hash")?,
            status: row.try_get("status")?,
            normalized: row.try_get("normalized")?,
            raw_blob_sha: row.try_get("raw_blob_sha")?,
            validation: row.try_get("validation")?,
        }))
    }
}
