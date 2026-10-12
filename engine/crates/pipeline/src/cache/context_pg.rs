//! PostgreSQL context cache over `stage_outputs` (CTX-009).
//!
//! Rows are `(review_run_id, stage = 'context', input_hash)`; the cluster and reviewer are part
//! of the input hash. Every statement runs in a transaction that sets `app.organization_id`
//! (row-level security) and repeats the tenant predicate. Lookups for reuse by a superseding run
//! are bounded to the same repository.

use std::sync::Arc;

use async_trait::async_trait;
use context_engine::cache::CtxKey;
use context_engine::hash::package_hash;
use context_engine::model::ContextPackage;
use repository::store::RepoScope;
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

use super::{AsyncContextCache, BlobStore, CacheError, HitScope};

/// The `stage` value of context rows.
pub const STAGE: &str = "context";
/// Packages above this serialized size go to the blob store; the row keeps a reference.
pub const INLINE_LIMIT_BYTES: usize = 1024 * 1024;
const ZSTD_LEVEL: i32 = 3;

fn backend(e: impl std::fmt::Display) -> CacheError {
    CacheError::Backend(e.to_string())
}

#[derive(Clone)]
pub struct PgContextCache {
    pool: PgPool,
    scope: RepoScope,
    blobs: Option<Arc<dyn BlobStore>>,
}

impl std::fmt::Debug for PgContextCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PgContextCache")
            .field("scope", &self.scope)
            .field("blobs", &self.blobs.is_some())
            .finish_non_exhaustive()
    }
}

impl PgContextCache {
    pub fn new(pool: PgPool, scope: RepoScope, blobs: Option<Arc<dyn BlobStore>>) -> Self {
        Self { pool, scope, blobs }
    }

    async fn begin(&self) -> Result<Transaction<'_, Postgres>, CacheError> {
        let mut tx = self.pool.begin().await.map_err(backend)?;
        sqlx::query("SELECT set_config('app.organization_id', $1, true)")
            .bind(self.scope.organization_id.as_uuid().to_string())
            .execute(&mut *tx)
            .await
            .map_err(backend)?;
        Ok(tx)
    }

    fn run_id(key: &CtxKey) -> Result<Uuid, CacheError> {
        Uuid::parse_str(&key.review_run_id)
            .map_err(|_| CacheError::Corrupt("review run id is not a uuid".to_owned()))
    }

    /// Decodes a stored row into a verified package; `None` when the blob is gone or the
    /// package no longer matches its hash (treated as a miss and recomputed).
    async fn decode(
        &self,
        output: Option<serde_json::Value>,
        blob_key: Option<String>,
        output_hash: &str,
    ) -> Result<Option<ContextPackage>, CacheError> {
        let package: ContextPackage = match (output, blob_key) {
            (Some(value), _) => serde_json::from_value(value).map_err(backend)?,
            (None, Some(blob_key)) => {
                let Some(store) = &self.blobs else {
                    return Ok(None);
                };
                let Some(bytes) = store.get(&blob_key).await? else {
                    return Ok(None);
                };
                let raw = zstd::decode_all(bytes.as_slice()).map_err(backend)?;
                serde_json::from_slice(&raw).map_err(backend)?
            }
            (None, None) => return Ok(None),
        };
        if package.package_hash != output_hash || package_hash(&package) != output_hash {
            return Ok(None);
        }
        Ok(Some(package))
    }
}

#[async_trait]
impl AsyncContextCache for PgContextCache {
    async fn lookup(&self, key: &CtxKey) -> Result<Option<(ContextPackage, HitScope)>, CacheError> {
        let run = Self::run_id(key)?;
        let org = self.scope.organization_id.as_uuid();
        let repo = self.scope.repository_id.as_uuid();
        let mut tx = self.begin().await?;
        let same_run = sqlx::query(
            "SELECT output, blob_key, output_hash FROM stage_outputs
             WHERE organization_id = $1 AND review_run_id = $2 AND stage = $3 AND input_hash = $4",
        )
        .bind(org)
        .bind(run)
        .bind(STAGE)
        .bind(&key.input_hash)
        .fetch_optional(&mut *tx)
        .await
        .map_err(backend)?;
        let (row, hit) = match same_run {
            Some(row) => (Some(row), HitScope::Run),
            None => {
                let other = sqlx::query(
                    "SELECT output, blob_key, output_hash FROM stage_outputs
                     WHERE organization_id = $1 AND repository_id = $2 AND stage = $3
                       AND input_hash = $4
                     ORDER BY created_at LIMIT 1",
                )
                .bind(org)
                .bind(repo)
                .bind(STAGE)
                .bind(&key.input_hash)
                .fetch_optional(&mut *tx)
                .await
                .map_err(backend)?;
                (other, HitScope::Repo)
            }
        };
        tx.commit().await.map_err(backend)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let output: Option<serde_json::Value> = row.try_get("output").map_err(backend)?;
        let blob_key: Option<String> = row.try_get("blob_key").map_err(backend)?;
        let output_hash: String = row.try_get("output_hash").map_err(backend)?;
        let Some(package) = self.decode(output, blob_key, &output_hash).await? else {
            return Ok(None);
        };
        if hit == HitScope::Repo {
            // Copy into this run's row so the run resumes from its own output.
            self.put(key, &package).await?;
        }
        Ok(Some((package, hit)))
    }

    async fn put(&self, key: &CtxKey, package: &ContextPackage) -> Result<(), CacheError> {
        if package.package_hash.len() != 64 {
            return Err(CacheError::Corrupt("package is not sealed".to_owned()));
        }
        let run = Self::run_id(key)?;
        let org = self.scope.organization_id.as_uuid();
        let repo = self.scope.repository_id.as_uuid();
        let bytes = serde_json::to_vec(package).map_err(backend)?;
        let (output, blob_key) = if bytes.len() > INLINE_LIMIT_BYTES {
            let Some(store) = &self.blobs else {
                // Too large for a row and nowhere to spill: skip caching, never fail the stage.
                return Ok(());
            };
            let blob_key = format!("org/{org}/runs/{run}/{STAGE}/{}.json.zst", key.input_hash);
            let packed = zstd::encode_all(bytes.as_slice(), ZSTD_LEVEL).map_err(backend)?;
            store.put(&blob_key, packed).await?;
            (None, Some(blob_key))
        } else {
            (
                Some(serde_json::from_slice::<serde_json::Value>(&bytes).map_err(backend)?),
                None,
            )
        };
        let mut tx = self.begin().await?;
        sqlx::query(
            "INSERT INTO stage_outputs
               (review_run_id, organization_id, repository_id, stage, input_hash, output_hash,
                output, blob_key, status, schema_version)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'succeeded', $9)
             ON CONFLICT (review_run_id, stage, input_hash) DO NOTHING",
        )
        .bind(run)
        .bind(org)
        .bind(repo)
        .bind(STAGE)
        .bind(&key.input_hash)
        .bind(&package.package_hash)
        .bind(output)
        .bind(blob_key)
        .bind(i32::from(package.schema_version))
        .execute(&mut *tx)
        .await
        .map_err(backend)?;
        tx.commit().await.map_err(backend)?;
        Ok(())
    }
}
