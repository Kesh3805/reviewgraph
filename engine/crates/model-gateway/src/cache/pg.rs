//! PostgreSQL response cache (`model_cache`, migration `20261003000020`).
//!
//! Every statement runs in a transaction that sets `app.organization_id`, so RLS applies on top
//! of the explicit `organization_id` predicate and the organisation-scoped key.

use std::time::Duration;

use async_trait::async_trait;
use review_core::ids::OrganizationId;
use sqlx::{PgPool, Row};

use super::{CacheEntry, CacheError, ResponseCache};
use crate::types::{ModelOutput, Usage};

#[derive(Debug, Clone)]
pub struct PgCache {
    pool: PgPool,
}

fn err(e: impl std::fmt::Display) -> CacheError {
    CacheError(e.to_string())
}

/// Rows deleted per purge statement.
const PURGE_CHUNK: i64 = 5000;

impl PgCache {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ResponseCache for PgCache {
    async fn get(
        &self,
        organization_id: OrganizationId,
        key: &str,
    ) -> Result<Option<CacheEntry>, CacheError> {
        let mut tx = self.pool.begin().await.map_err(err)?;
        sqlx::query("SELECT set_config('app.organization_id', $1, true)")
            .bind(organization_id.to_string())
            .execute(&mut *tx)
            .await
            .map_err(err)?;
        let row = sqlx::query(
            "SELECT request_hash, provider, model, prompt_version, schema_hash, output, usage \
             FROM model_cache WHERE organization_id = $1 AND cache_key = $2 AND expires_at > now()",
        )
        .bind(organization_id.into_uuid())
        .bind(key)
        .fetch_optional(&mut *tx)
        .await
        .map_err(err)?;
        tx.commit().await.map_err(err)?;
        let Some(row) = row else { return Ok(None) };
        let output: ModelOutput =
            serde_json::from_value(row.try_get("output").map_err(err)?).map_err(err)?;
        let usage: Usage =
            serde_json::from_value(row.try_get("usage").map_err(err)?).map_err(err)?;
        Ok(Some(CacheEntry {
            request_hash: row.try_get("request_hash").map_err(err)?,
            provider: row.try_get("provider").map_err(err)?,
            model: row.try_get("model").map_err(err)?,
            prompt_version: row.try_get("prompt_version").map_err(err)?,
            schema_hash: row.try_get("schema_hash").map_err(err)?,
            output,
            usage,
        }))
    }

    async fn put(
        &self,
        organization_id: OrganizationId,
        key: &str,
        entry: CacheEntry,
        ttl: Duration,
    ) -> Result<(), CacheError> {
        let mut tx = self.pool.begin().await.map_err(err)?;
        sqlx::query("SELECT set_config('app.organization_id', $1, true)")
            .bind(organization_id.to_string())
            .execute(&mut *tx)
            .await
            .map_err(err)?;
        // An expired row with the same key is replaced; a live one wins (first writer).
        sqlx::query("DELETE FROM model_cache WHERE organization_id = $1 AND cache_key = $2 AND expires_at <= now()")
            .bind(organization_id.into_uuid())
            .bind(key)
            .execute(&mut *tx)
            .await
            .map_err(err)?;
        sqlx::query(
            "INSERT INTO model_cache (organization_id, cache_key, request_hash, provider, model, \
             prompt_version, schema_hash, output, usage, expires_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, now() + make_interval(secs => $10)) \
             ON CONFLICT (organization_id, cache_key) DO NOTHING",
        )
        .bind(organization_id.into_uuid())
        .bind(key)
        .bind(&entry.request_hash)
        .bind(&entry.provider)
        .bind(&entry.model)
        .bind(&entry.prompt_version)
        .bind(&entry.schema_hash)
        .bind(serde_json::to_value(&entry.output).map_err(err)?)
        .bind(serde_json::to_value(entry.usage).map_err(err)?)
        .bind(ttl.as_secs_f64())
        .execute(&mut *tx)
        .await
        .map_err(err)?;
        tx.commit().await.map_err(err)
    }

    /// Housekeeping (PIPE-010). Runs as a role that sees every tenant (`rg_ops`).
    async fn purge_expired(&self) -> Result<u64, CacheError> {
        let mut total = 0;
        loop {
            let res = sqlx::query(
                "DELETE FROM model_cache WHERE ctid IN \
                 (SELECT ctid FROM model_cache WHERE expires_at < now() LIMIT $1)",
            )
            .bind(PURGE_CHUNK)
            .execute(&self.pool)
            .await
            .map_err(err)?;
            total += res.rows_affected();
            if res.rows_affected() == 0 {
                return Ok(total);
            }
        }
    }
}
