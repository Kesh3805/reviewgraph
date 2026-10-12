//! PostgreSQL implementation of the finding lifecycle store (VER-001).
//!
//! Every statement runs in a transaction that first sets `app.organization_id`, so the tenant
//! RLS policy applies, and repeats the `organization_id` predicate as defence in depth. State
//! changes are compare-and-set on `(state, state_version)` and write their audit row in the same
//! transaction. Nothing here logs `raw`, `evidence` or `payload`: they may contain code.

use review_core::finding::{FindingState, Suppression, SuppressionReason};
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;
use verification::store::{
    FindingStore, NewCandidate, NewEvidence, NewVerification, Transition, TransitionError,
};

use super::finding_metrics as metrics;

/// Reason code recorded by bulk invalidation on supersession.
pub const HEAD_SUPERSEDED: &str = "HEAD_SUPERSEDED";

/// The lifecycle store for one tenant.
#[derive(Debug, Clone)]
pub struct PgFindingStore {
    pool: PgPool,
    organization_id: Uuid,
}

fn store_err(e: sqlx::Error) -> TransitionError {
    TransitionError::Store(e.to_string())
}

fn json_err(e: serde_json::Error) -> TransitionError {
    TransitionError::Store(e.to_string())
}

impl PgFindingStore {
    pub fn new(pool: PgPool, organization_id: Uuid) -> Self {
        Self {
            pool,
            organization_id,
        }
    }

    async fn begin(&self) -> Result<Transaction<'_, Postgres>, TransitionError> {
        let mut tx = self.pool.begin().await.map_err(store_err)?;
        sqlx::query("SELECT set_config('app.organization_id', $1, true)")
            .bind(self.organization_id.to_string())
            .execute(&mut *tx)
            .await
            .map_err(store_err)?;
        Ok(tx)
    }
}

impl FindingStore for PgFindingStore {
    async fn insert_candidate(&self, c: NewCandidate) -> Result<bool, TransitionError> {
        let mut tx = self.begin().await?;
        let inserted = sqlx::query(
            "INSERT INTO candidate_findings
               (id, organization_id, review_run_id, reviewer_run_id, reviewer, category, title,
                description, changed_path, changed_side, changed_start_line, changed_end_line,
                severity_candidate, confidence_candidate, affected_symbols, evidence, fingerprint,
                state, idempotency_key, raw_output_hash)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17,
                     'GENERATED', $18, $19)
             ON CONFLICT DO NOTHING",
        )
        .bind(c.id)
        .bind(self.organization_id)
        .bind(c.review_run_id)
        .bind(c.reviewer_run_id)
        .bind(&c.reviewer)
        .bind(&c.category)
        .bind(&c.title)
        .bind(&c.description)
        .bind(&c.changed_path)
        .bind(&c.changed_side)
        .bind(c.changed_start_line)
        .bind(c.changed_end_line)
        .bind(&c.severity_candidate)
        .bind(c.confidence_candidate)
        .bind(&c.affected_symbols)
        .bind(&c.evidence)
        .bind(&c.fingerprint)
        .bind(&c.idempotency_key)
        .bind(&c.raw_output_hash)
        .execute(&mut *tx)
        .await
        .map_err(store_err)?
        .rows_affected();
        tx.commit().await.map_err(store_err)?;
        Ok(inserted == 1)
    }

    async fn record_verification(&self, v: NewVerification) -> Result<bool, TransitionError> {
        let mut tx = self.begin().await?;
        let inserted = sqlx::query(
            "INSERT INTO finding_verifications
               (id, organization_id, candidate_finding_id, verification_version, base_snapshot_id,
                head_snapshot_id, depth, stage_outcomes, signals, confidence, eligibility,
                placement, cache_key, from_cache, duration_ms)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10::float8, $11, $12, $13, $14, $15)
             ON CONFLICT (candidate_finding_id, verification_version) DO NOTHING",
        )
        .bind(v.id)
        .bind(self.organization_id)
        .bind(v.candidate_id)
        .bind(&v.verification_version)
        .bind(v.base_snapshot_id)
        .bind(v.head_snapshot_id)
        .bind(&v.depth)
        .bind(&v.stage_outcomes)
        .bind(&v.signals)
        .bind(v.confidence)
        .bind(&v.eligibility)
        .bind(&v.placement)
        .bind(&v.cache_key)
        .bind(v.from_cache)
        .bind(v.duration_ms)
        .execute(&mut *tx)
        .await
        .map_err(store_err)?
        .rows_affected();
        tx.commit().await.map_err(store_err)?;
        Ok(inserted == 1)
    }

    async fn add_evidence(&self, items: Vec<NewEvidence>) -> Result<(), TransitionError> {
        let mut tx = self.begin().await?;
        for e in &items {
            sqlx::query(
                "INSERT INTO finding_evidence
                   (organization_id, candidate_finding_id, verification_id, kind, source,
                    strength, polarity, path, side, start_line, end_line, symbol_keys, payload)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
            )
            .bind(self.organization_id)
            .bind(e.candidate_id)
            .bind(e.verification_id)
            .bind(&e.kind)
            .bind(&e.source)
            .bind(&e.strength)
            .bind(&e.polarity)
            .bind(&e.path)
            .bind(&e.side)
            .bind(e.start_line)
            .bind(e.end_line)
            .bind(&e.symbol_keys)
            .bind(&e.payload)
            .execute(&mut *tx)
            .await
            .map_err(store_err)?;
        }
        tx.commit().await.map_err(store_err)?;
        Ok(())
    }

    async fn transition(&self, t: Transition) -> Result<i32, TransitionError> {
        t.validate()?;
        let suppression = t
            .suppression
            .as_ref()
            .map(serde_json::to_value)
            .transpose()
            .map_err(json_err)?;
        let stage_no = t.suppression.as_ref().and_then(|s| s.stage).map(i16::from);
        let mut tx = self.begin().await?;
        let new_version: Option<i32> = sqlx::query_scalar(
            "UPDATE candidate_findings
                SET state = $1, state_version = state_version + 1, reason_code = $2,
                    suppression = $3, suppressed_at_stage = $4, updated_at = now()
              WHERE id = $5 AND organization_id = $6 AND state = $7 AND state_version = $8
          RETURNING state_version",
        )
        .bind(t.to.as_str())
        .bind(&t.reason_code)
        .bind(&suppression)
        .bind(stage_no)
        .bind(t.candidate_id)
        .bind(self.organization_id)
        .bind(t.from.as_str())
        .bind(t.expected_version)
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_err)?;
        let Some(version) = new_version else {
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM candidate_findings
                                 WHERE id = $1 AND organization_id = $2)",
            )
            .bind(t.candidate_id)
            .bind(self.organization_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(store_err)?;
            tx.rollback().await.map_err(store_err)?;
            if exists {
                metrics::conflict();
                return Err(TransitionError::Conflict);
            }
            return Err(TransitionError::NotFound);
        };
        sqlx::query(
            "INSERT INTO finding_state_transitions
               (organization_id, candidate_finding_id, from_state, to_state, reason_code, stage,
                detail)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(self.organization_id)
        .bind(t.candidate_id)
        .bind(t.from.as_str())
        .bind(t.to.as_str())
        .bind(&t.reason_code)
        .bind(&t.stage)
        .bind(&t.detail)
        .execute(&mut *tx)
        .await
        .map_err(store_err)?;
        tx.commit().await.map_err(store_err)?;
        metrics::transition(t.from.as_str(), t.to.as_str(), 1);
        Ok(version)
    }

    async fn invalidate_run(&self, review_run_id: Uuid) -> Result<u64, TransitionError> {
        let terminal: Vec<&'static str> = FindingState::ALL
            .iter()
            .filter(|s| s.is_terminal())
            .map(|s| s.as_str())
            .collect();
        let suppression = Suppression::new(
            SuppressionReason::Invalidated {
                superseded_by: None,
            },
            HEAD_SUPERSEDED,
            None,
        )
        .map_err(|e| TransitionError::Store(e.to_string()))?;
        let suppression = serde_json::to_value(suppression).map_err(json_err)?;
        let mut tx = self.begin().await?;
        // Lock the targets first so each audit row carries the state the candidate really left.
        let rows = sqlx::query(
            "WITH targets AS (
               SELECT id, state FROM candidate_findings
                WHERE review_run_id = $1 AND organization_id = $2 AND state <> ALL($3)
                  FOR UPDATE),
             upd AS (
               UPDATE candidate_findings c
                  SET state = 'INVALIDATED', state_version = c.state_version + 1,
                      reason_code = $4, suppression = $5, suppressed_at_stage = NULL,
                      updated_at = now()
                 FROM targets
                WHERE c.id = targets.id
            RETURNING c.id, targets.state AS from_state)
             INSERT INTO finding_state_transitions
               (organization_id, candidate_finding_id, from_state, to_state, reason_code, stage)
             SELECT $2, id, from_state, 'INVALIDATED', $4, 'supersession' FROM upd
             RETURNING from_state",
        )
        .bind(review_run_id)
        .bind(self.organization_id)
        .bind(&terminal)
        .bind(HEAD_SUPERSEDED)
        .bind(&suppression)
        .fetch_all(&mut *tx)
        .await
        .map_err(store_err)?;
        tx.commit().await.map_err(store_err)?;
        let mut counts: Vec<(&'static str, u64)> = Vec::new();
        for row in &rows {
            let from: String = row.try_get("from_state").map_err(store_err)?;
            if let Some(s) = FindingState::ALL.iter().find(|s| s.as_str() == from) {
                match counts.iter_mut().find(|(n, _)| *n == s.as_str()) {
                    Some((_, n)) => *n += 1,
                    None => counts.push((s.as_str(), 1)),
                }
            }
        }
        for (from, n) in counts {
            metrics::transition(from, FindingState::Invalidated.as_str(), n);
        }
        Ok(rows.len() as u64)
    }
}
