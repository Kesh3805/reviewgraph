//! The per-attempt `model_calls` ledger (GW-008).
//!
//! Recording never blocks a model call: [`ChannelLedger`] does a `try_send` into a bounded
//! channel and counts drops (`model_calls_dropped_total`). The PostgreSQL writer (feature `pg`)
//! drains the channel in batches of 100 rows or every 500 ms.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use chrono::NaiveDate;
use review_core::ids::{OrganizationId, RepositoryId, ReviewRunId, ReviewerRunId};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::types::{ServedFrom, Usage};

/// One provider attempt (or one response-cache hit).
#[derive(Debug, Clone)]
pub struct LedgerRecord {
    pub id: Uuid,
    pub organization_id: OrganizationId,
    pub repository_id: RepositoryId,
    pub review_run_id: Option<ReviewRunId>,
    pub reviewer_run_id: Option<ReviewerRunId>,
    pub task: String,
    pub tier: String,
    pub provider: String,
    pub model: String,
    pub attempt: i16,
    pub request_hash: String,
    pub served_from: ServedFrom,
    /// `ok` or a `GatewayError::class()`.
    pub outcome: String,
    pub usage: Usage,
    pub cost_usd_micros: Option<u64>,
    pub latency_ms: u32,
    pub prices_as_of: Option<NaiveDate>,
}

pub trait LedgerSink: Send + Sync {
    /// Must not block or fail the call.
    fn record(&self, rec: LedgerRecord);
}

/// Discards records (default).
#[derive(Debug, Default, Clone, Copy)]
pub struct NoLedger;

impl LedgerSink for NoLedger {
    fn record(&self, _rec: LedgerRecord) {}
}

/// Keeps records in memory (tests, EVAL runs that read the totals back).
#[derive(Debug, Default)]
pub struct MemoryLedger {
    records: Mutex<Vec<LedgerRecord>>,
}

impl MemoryLedger {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn records(&self) -> Vec<LedgerRecord> {
        self.records.lock().map(|r| r.clone()).unwrap_or_default()
    }

    /// Sum of known costs, in micro-USD.
    pub fn total_cost_micros(&self) -> u64 {
        self.records()
            .iter()
            .filter_map(|r| r.cost_usd_micros)
            .sum()
    }
}

impl LedgerSink for MemoryLedger {
    fn record(&self, rec: LedgerRecord) {
        if let Ok(mut r) = self.records.lock() {
            r.push(rec);
        }
    }
}

/// Bounded-channel sink; pair it with a writer task that owns the receiver.
#[derive(Debug)]
pub struct ChannelLedger {
    tx: mpsc::Sender<LedgerRecord>,
    dropped: AtomicU64,
}

impl ChannelLedger {
    pub fn channel(capacity: usize) -> (Self, mpsc::Receiver<LedgerRecord>) {
        let (tx, rx) = mpsc::channel(capacity.max(1));
        (
            Self {
                tx,
                dropped: AtomicU64::new(0),
            },
            rx,
        )
    }

    /// `model_calls_dropped_total`.
    pub fn dropped_total(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

impl LedgerSink for ChannelLedger {
    fn record(&self, rec: LedgerRecord) {
        if self.tx.try_send(rec).is_err() {
            let n = self.dropped.fetch_add(1, Ordering::Relaxed) + 1;
            tracing::warn!(
                dropped_total = n,
                "model_calls ledger channel full or closed"
            );
        }
    }
}

#[cfg(feature = "pg")]
pub mod pg {
    //! PostgreSQL batch writer for [`ChannelLedger`].

    use std::time::Duration;

    use sqlx::PgPool;
    use tokio::sync::mpsc;
    use tokio::task::JoinHandle;

    use super::LedgerRecord;

    pub const BATCH_ROWS: usize = 100;
    pub const FLUSH_EVERY: Duration = Duration::from_millis(500);
    const FLUSH_RETRIES: u32 = 3;

    /// Writes one batch in a single transaction. `app.organization_id` is set per row so the
    /// RLS `WITH CHECK` policy accepts rows of different tenants.
    pub async fn write_batch(pool: &PgPool, batch: &[LedgerRecord]) -> Result<(), sqlx::Error> {
        let mut tx = pool.begin().await?;
        for r in batch {
            sqlx::query("SELECT set_config('app.organization_id', $1, true)")
                .bind(r.organization_id.to_string())
                .execute(&mut *tx)
                .await?;
            sqlx::query(
                "INSERT INTO model_calls (id, organization_id, repository_id, review_run_id, \
                 reviewer_run_id, task, tier, provider, model, attempt, request_hash, served_from, \
                 outcome, input_uncached, cache_write, cache_read, output_tokens, cost_usd_micros, \
                 latency_ms, prices_as_of) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20)",
            )
            .bind(r.id)
            .bind(r.organization_id.into_uuid())
            .bind(r.repository_id.into_uuid())
            .bind(r.review_run_id.map(|i| i.into_uuid()))
            .bind(r.reviewer_run_id.map(|i| i.into_uuid()))
            .bind(&r.task)
            .bind(&r.tier)
            .bind(&r.provider)
            .bind(&r.model)
            .bind(r.attempt)
            .bind(&r.request_hash)
            .bind(r.served_from.as_str())
            .bind(&r.outcome)
            .bind(i32::try_from(r.usage.input_uncached).unwrap_or(i32::MAX))
            .bind(i32::try_from(r.usage.cache_write).unwrap_or(i32::MAX))
            .bind(i32::try_from(r.usage.cache_read).unwrap_or(i32::MAX))
            .bind(i32::try_from(r.usage.output).unwrap_or(i32::MAX))
            .bind(r.cost_usd_micros.and_then(|c| i64::try_from(c).ok()))
            .bind(i32::try_from(r.latency_ms).unwrap_or(i32::MAX))
            .bind(r.prices_as_of)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await
    }

    async fn flush(pool: &PgPool, batch: &mut Vec<LedgerRecord>) {
        if batch.is_empty() {
            return;
        }
        for attempt in 1..=FLUSH_RETRIES {
            match write_batch(pool, batch).await {
                Ok(()) => break,
                Err(e) if attempt == FLUSH_RETRIES => {
                    tracing::error!(error = %e, rows = batch.len(), "dropping ledger batch");
                }
                Err(e) => {
                    tracing::warn!(error = %e, attempt, "ledger flush failed; retrying");
                    tokio::time::sleep(Duration::from_millis(100 * u64::from(attempt))).await;
                }
            }
        }
        batch.clear();
    }

    /// Spawns the single batch-writer task for this process. It drains the channel and flushes
    /// everything left when the sender side is dropped.
    pub fn spawn_writer(pool: PgPool, mut rx: mpsc::Receiver<LedgerRecord>) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut batch: Vec<LedgerRecord> = Vec::with_capacity(BATCH_ROWS);
            let mut tick = tokio::time::interval(FLUSH_EVERY);
            loop {
                tokio::select! {
                    maybe = rx.recv() => match maybe {
                        Some(rec) => {
                            batch.push(rec);
                            if batch.len() >= BATCH_ROWS {
                                flush(&pool, &mut batch).await;
                            }
                        }
                        None => {
                            flush(&pool, &mut batch).await;
                            break;
                        }
                    },
                    _ = tick.tick() => flush(&pool, &mut batch).await,
                }
            }
        })
    }
}
