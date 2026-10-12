//! `stage_outputs` persistence (PIPE-005): every stage stores its output keyed by
//! `(review_run_id, stage, input_hash)`; a retried job resumes from the stored outputs.
//!
//! Outputs are immutable. `put` is `INSERT ... ON CONFLICT DO NOTHING`: when two workers compute
//! the same stage both execute, one write wins, and both continue with the stored output. A second
//! write with a different output hash is logged as `nondeterministic_stage` (a bug signal for
//! PIPE-009) and the stored output wins.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use review_core::ids::{OrganizationId, ReviewRunId};
use serde_json::Value;
use sqlx::PgPool;

use super::blob::BlobStore;
use super::input_hash::{canonical_json, hash_hex};
use super::{metrics, tenant_tx, StoreError};

/// Outputs larger than this are compressed and spilled to the blob store.
pub const SPILL_THRESHOLD_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageStatus {
    Succeeded,
    /// A deterministic failure, stored so it is not retried indefinitely.
    FailedPermanent,
}

impl StageStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::FailedPermanent => "failed_permanent",
        }
    }

    fn parse(s: &str) -> Result<Self, StoreError> {
        match s {
            "succeeded" => Ok(Self::Succeeded),
            "failed_permanent" => Ok(Self::FailedPermanent),
            other => Err(StoreError::Db(format!("unknown stage status {other}"))),
        }
    }
}

/// What a stage hands to [`StageStore::put`].
#[derive(Debug, Clone)]
pub struct NewStageOutput {
    pub stage: String,
    pub input_hash: String,
    pub status: StageStatus,
    pub schema_version: i32,
    pub duration_ms: i32,
    pub output: Value,
}

/// A stored output (`output_hash` is the blake3 of the canonical JSON of `output`).
#[derive(Debug, Clone, PartialEq)]
pub struct StoredStageOutput {
    pub stage: String,
    pub input_hash: String,
    pub output_hash: String,
    pub status: StageStatus,
    pub schema_version: i32,
    pub duration_ms: i32,
    pub output: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PutOutcome {
    /// This call wrote the row.
    Stored(StoredStageOutput),
    /// A row already existed; it wins. `hash_matches` is false for a nondeterministic stage.
    Existing {
        stored: StoredStageOutput,
        hash_matches: bool,
    },
}

impl PutOutcome {
    /// The output both writers continue with.
    pub fn into_stored(self) -> StoredStageOutput {
        match self {
            Self::Stored(s) | Self::Existing { stored: s, .. } => s,
        }
    }
}

#[async_trait]
pub trait StageStore: Send + Sync + std::fmt::Debug {
    async fn get(
        &self,
        org: OrganizationId,
        run: ReviewRunId,
        stage: &str,
        input_hash: &str,
    ) -> Result<Option<StoredStageOutput>, StoreError>;

    async fn put(
        &self,
        org: OrganizationId,
        run: ReviewRunId,
        output: NewStageOutput,
    ) -> Result<PutOutcome, StoreError>;

    /// Every output of the run, ordered by `(stage, input_hash)`.
    async fn list_for_run(
        &self,
        org: OrganizationId,
        run: ReviewRunId,
    ) -> Result<Vec<StoredStageOutput>, StoreError>;
}

fn blob_key(org: &OrganizationId, run: &ReviewRunId, stage: &str, input_hash: &str) -> String {
    format!(
        "org/{}/runs/{}/{stage}/{input_hash}.json.zst",
        org.as_uuid(),
        run.as_uuid()
    )
}

/// PostgreSQL stage store with blob spill.
#[derive(Debug, Clone)]
pub struct PgStageStore {
    pool: PgPool,
    blobs: Arc<dyn BlobStore>,
}

type StageRow = (
    String,
    String,
    Option<String>,
    Option<String>,
    String,
    i32,
    i32,
);

const SELECT_COLUMNS: &str = "stage, output_hash, output::text, blob_key, status, schema_version,
       duration_ms";

impl PgStageStore {
    pub fn new(pool: PgPool, blobs: Arc<dyn BlobStore>) -> Self {
        Self { pool, blobs }
    }

    async fn hydrate(
        &self,
        row: StageRow,
        input_hash: &str,
    ) -> Result<Option<StoredStageOutput>, StoreError> {
        let (stage, output_hash, inline, blob, status, schema_version, duration_ms) = row;
        let output: Value = match (inline, blob) {
            (Some(text), _) => serde_json::from_str(&text)?,
            (None, Some(key)) => {
                let Some(compressed) = self.blobs.get(&key).await? else {
                    // A missing blob is a cache miss: the stage is recomputed.
                    tracing::warn!(stage = %stage, "stage_output_blob_missing");
                    return Ok(None);
                };
                let bytes = zstd::decode_all(&compressed[..])
                    .map_err(|e| StoreError::Blob(format!("decompress: {e}")))?;
                serde_json::from_slice(&bytes)?
            }
            (None, None) => return Err(StoreError::Db("stage output has no payload".into())),
        };
        Ok(Some(StoredStageOutput {
            stage,
            input_hash: input_hash.to_owned(),
            output_hash,
            status: StageStatus::parse(&status)?,
            schema_version,
            duration_ms,
            output,
        }))
    }
}

#[async_trait]
impl StageStore for PgStageStore {
    async fn get(
        &self,
        org: OrganizationId,
        run: ReviewRunId,
        stage: &str,
        input_hash: &str,
    ) -> Result<Option<StoredStageOutput>, StoreError> {
        let mut tx = tenant_tx(&self.pool, org.as_uuid()).await?;
        let row: Option<StageRow> = sqlx::query_as(&format!(
            "SELECT {SELECT_COLUMNS} FROM stage_outputs
             WHERE review_run_id = $1 AND organization_id = $2 AND stage = $3 AND input_hash = $4"
        ))
        .bind(run.as_uuid())
        .bind(org.as_uuid())
        .bind(stage)
        .bind(input_hash)
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        let stored = match row {
            Some(r) => self.hydrate(r, input_hash).await?,
            None => None,
        };
        if stored.is_some() {
            metrics::record_hit(stage);
        } else {
            metrics::record_miss(stage);
        }
        Ok(stored)
    }

    async fn put(
        &self,
        org: OrganizationId,
        run: ReviewRunId,
        n: NewStageOutput,
    ) -> Result<PutOutcome, StoreError> {
        let bytes = canonical_json(&n.output)?;
        let output_hash = hash_hex(&bytes);
        let (inline, key) = if bytes.len() > SPILL_THRESHOLD_BYTES {
            let key = blob_key(&org, &run, &n.stage, &n.input_hash);
            let compressed = zstd::encode_all(&bytes[..], 3)
                .map_err(|e| StoreError::Blob(format!("compress: {e}")))?;
            self.blobs.put(&key, compressed).await?;
            (None, Some(key))
        } else {
            let text = String::from_utf8(bytes)
                .map_err(|e| StoreError::Serde(format!("output is not UTF-8: {e}")))?;
            (Some(text), None)
        };
        let mut tx = tenant_tx(&self.pool, org.as_uuid()).await?;
        let inserted: Option<String> = sqlx::query_scalar(
            "INSERT INTO stage_outputs
               (review_run_id, organization_id, stage, input_hash, output_hash, output, blob_key,
                status, schema_version, duration_ms)
             VALUES ($1, $2, $3, $4, $5, $6::jsonb, $7, $8, $9, $10)
             ON CONFLICT (review_run_id, stage, input_hash) DO NOTHING
             RETURNING input_hash",
        )
        .bind(run.as_uuid())
        .bind(org.as_uuid())
        .bind(&n.stage)
        .bind(&n.input_hash)
        .bind(&output_hash)
        .bind(&inline)
        .bind(&key)
        .bind(n.status.as_str())
        .bind(n.schema_version)
        .bind(n.duration_ms)
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        let stored_now = StoredStageOutput {
            stage: n.stage.clone(),
            input_hash: n.input_hash.clone(),
            output_hash: output_hash.clone(),
            status: n.status,
            schema_version: n.schema_version,
            duration_ms: n.duration_ms,
            output: n.output,
        };
        if inserted.is_some() {
            return Ok(PutOutcome::Stored(stored_now));
        }
        // Lost the race (or a repeat write): the stored row wins.
        let Some(existing) = self.get(org, run, &n.stage, &n.input_hash).await? else {
            return Ok(PutOutcome::Stored(stored_now));
        };
        let hash_matches = existing.output_hash == output_hash;
        if !hash_matches {
            metrics::record_conflict(&n.stage);
            tracing::error!(stage = %n.stage, "nondeterministic_stage");
        }
        Ok(PutOutcome::Existing {
            stored: existing,
            hash_matches,
        })
    }

    async fn list_for_run(
        &self,
        org: OrganizationId,
        run: ReviewRunId,
    ) -> Result<Vec<StoredStageOutput>, StoreError> {
        let mut tx = tenant_tx(&self.pool, org.as_uuid()).await?;
        let rows: Vec<(StageRow, String)> = sqlx::query(&format!(
            "SELECT {SELECT_COLUMNS}, input_hash FROM stage_outputs
             WHERE review_run_id = $1 AND organization_id = $2
             ORDER BY stage, input_hash"
        ))
        .bind(run.as_uuid())
        .bind(org.as_uuid())
        .try_map(|r: sqlx::postgres::PgRow| {
            use sqlx::Row;
            Ok((
                (
                    r.try_get(0)?,
                    r.try_get(1)?,
                    r.try_get(2)?,
                    r.try_get(3)?,
                    r.try_get(4)?,
                    r.try_get(5)?,
                    r.try_get(6)?,
                ),
                r.try_get(7)?,
            ))
        })
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        let mut out = Vec::with_capacity(rows.len());
        for (row, input_hash) in rows {
            if let Some(s) = self.hydrate(row, &input_hash).await? {
                out.push(s);
            }
        }
        Ok(out)
    }
}

/// In-memory stage store: the orchestrator's tests and local runs use it instead of Postgres.
#[derive(Debug, Default)]
pub struct MemoryStageStore {
    rows: Mutex<BTreeMap<(String, String, String), StoredStageOutput>>,
}

impl MemoryStageStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.rows.lock().map(|r| r.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[async_trait]
impl StageStore for MemoryStageStore {
    async fn get(
        &self,
        _org: OrganizationId,
        run: ReviewRunId,
        stage: &str,
        input_hash: &str,
    ) -> Result<Option<StoredStageOutput>, StoreError> {
        let found = self
            .rows
            .lock()
            .map_err(|_| StoreError::Db("stage store lock poisoned".into()))?
            .get(&(run.to_string(), stage.to_owned(), input_hash.to_owned()))
            .cloned();
        if found.is_some() {
            metrics::record_hit(stage);
        } else {
            metrics::record_miss(stage);
        }
        Ok(found)
    }

    async fn put(
        &self,
        _org: OrganizationId,
        run: ReviewRunId,
        n: NewStageOutput,
    ) -> Result<PutOutcome, StoreError> {
        let bytes = canonical_json(&n.output)?;
        let output_hash = hash_hex(&bytes);
        let stored = StoredStageOutput {
            stage: n.stage.clone(),
            input_hash: n.input_hash.clone(),
            output_hash: output_hash.clone(),
            status: n.status,
            schema_version: n.schema_version,
            duration_ms: n.duration_ms,
            output: n.output,
        };
        let mut rows = self
            .rows
            .lock()
            .map_err(|_| StoreError::Db("stage store lock poisoned".into()))?;
        let key = (run.to_string(), n.stage.clone(), n.input_hash);
        if let Some(existing) = rows.get(&key) {
            let hash_matches = existing.output_hash == output_hash;
            if !hash_matches {
                metrics::record_conflict(&n.stage);
            }
            return Ok(PutOutcome::Existing {
                stored: existing.clone(),
                hash_matches,
            });
        }
        rows.insert(key, stored.clone());
        Ok(PutOutcome::Stored(stored))
    }

    async fn list_for_run(
        &self,
        _org: OrganizationId,
        run: ReviewRunId,
    ) -> Result<Vec<StoredStageOutput>, StoreError> {
        let run = run.to_string();
        Ok(self
            .rows
            .lock()
            .map_err(|_| StoreError::Db("stage store lock poisoned".into()))?
            .iter()
            .filter(|((r, _, _), _)| *r == run)
            .map(|(_, v)| v.clone())
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn out(stage: &str, input: &str, v: Value) -> NewStageOutput {
        NewStageOutput {
            stage: stage.into(),
            input_hash: input.into(),
            status: StageStatus::Succeeded,
            schema_version: 1,
            duration_ms: 3,
            output: v,
        }
    }

    #[tokio::test]
    async fn memory_put_get_roundtrip_and_conflict() {
        let s = MemoryStageStore::new();
        let (org, run) = (OrganizationId::new(), ReviewRunId::new());
        let h = "a".repeat(64);
        let first = s
            .put(org, run, out("diff", &h, serde_json::json!({"a": 1})))
            .await
            .unwrap();
        assert!(matches!(first, PutOutcome::Stored(_)));
        let got = s.get(org, run, "diff", &h).await.unwrap().unwrap();
        assert_eq!(got.output, serde_json::json!({"a": 1}));
        let again = s
            .put(org, run, out("diff", &h, serde_json::json!({"a": 2})))
            .await
            .unwrap();
        match again {
            PutOutcome::Existing {
                stored,
                hash_matches,
            } => {
                assert!(!hash_matches);
                assert_eq!(stored.output, serde_json::json!({"a": 1}));
            }
            PutOutcome::Stored(_) => panic!("second put must not overwrite"),
        }
    }
}
