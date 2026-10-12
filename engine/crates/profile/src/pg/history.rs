//! `repository_signals` and `signal_admissions` (HIST-003, HIST-004).

use chrono::{DateTime, Utc};
use repository::store::RepoScope;
use sqlx::{PgPool, Row};

use super::{begin_scoped, PgStoreError};
use crate::history::{AdmissionDecision, Signal, SignalKind, SignalSet};

#[derive(Debug, Clone)]
pub struct PgSignalStore {
    pool: PgPool,
}

/// The stored decision of a signal at its latest version.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredAdmission {
    pub signal_id: String,
    pub version: i32,
    pub decision: String,
    pub reasons: serde_json::Value,
    pub evidence_summary: serde_json::Value,
    pub expires_at: Option<DateTime<Utc>>,
}

impl PgSignalStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Replaces the repository's whole signal set in one transaction (under a per-repository
    /// advisory lock): readers see the old or the new full set, never a mix.
    pub async fn replace(&self, scope: &RepoScope, signals: &[Signal]) -> Result<(), PgStoreError> {
        let mut tx = begin_scoped(&self.pool, scope).await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(format!("signals:{}", scope.repository_id.as_uuid()))
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "DELETE FROM repository_signals WHERE organization_id = $1 AND repository_id = $2",
        )
        .bind(scope.organization_id.as_uuid())
        .bind(scope.repository_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        for s in signals {
            sqlx::query(
                "INSERT INTO repository_signals
                   (organization_id, repository_id, kind, scope, statistic, n, evidence,
                    computed_at, valid_until, version)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
            )
            .bind(scope.organization_id.as_uuid())
            .bind(scope.repository_id.as_uuid())
            .bind(s.kind.as_str())
            .bind(s.scope.to_json())
            .bind(serde_json::to_value(&s.statistic)?)
            .bind(i32::try_from(s.n).unwrap_or(i32::MAX))
            .bind(s.evidence_json())
            .bind(s.computed_at)
            .bind(s.valid_until)
            .bind(i32::try_from(s.version).unwrap_or(i32::MAX))
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// The signals still valid at `now`. Rows that no longer parse are skipped: a review proceeds
    /// with fewer signals rather than failing.
    pub async fn load(
        &self,
        scope: &RepoScope,
        now: DateTime<Utc>,
    ) -> Result<SignalSet, PgStoreError> {
        let mut tx = begin_scoped(&self.pool, scope).await?;
        let rows = sqlx::query(
            "SELECT kind, scope, statistic, n, evidence, computed_at, valid_until, version
               FROM repository_signals
              WHERE organization_id = $1 AND repository_id = $2 AND valid_until > $3
              ORDER BY kind, scope::text",
        )
        .bind(scope.organization_id.as_uuid())
        .bind(scope.repository_id.as_uuid())
        .bind(now)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        let mut signals = Vec::with_capacity(rows.len());
        for row in rows {
            let kind: String = row.try_get("kind")?;
            let Some(kind) = SignalKind::parse(&kind) else {
                continue;
            };
            let scope_json: serde_json::Value = row.try_get("scope")?;
            let statistic: serde_json::Value = row.try_get("statistic")?;
            let evidence: serde_json::Value = row.try_get("evidence")?;
            let (Ok(signal_scope), Ok(statistic)) = (
                serde_json::from_value(scope_json),
                serde_json::from_value(statistic),
            ) else {
                continue;
            };
            let n: i32 = row.try_get("n")?;
            let version: i32 = row.try_get("version")?;
            signals.push(Signal {
                kind,
                scope: signal_scope,
                statistic,
                n: u32::try_from(n).unwrap_or(0),
                evidence_refs: serde_json::from_value(evidence).unwrap_or_default(),
                computed_at: row.try_get("computed_at")?,
                valid_until: row.try_get("valid_until")?,
                version: u32::try_from(version).unwrap_or(1),
            });
        }
        Ok(SignalSet::new(signals))
    }

    /// Appends a decision as the signal's next version. A decision with the same `input_hash`
    /// as the latest version is a no-op (`false`), so re-evaluation never duplicates rows.
    pub async fn record_admission(
        &self,
        scope: &RepoScope,
        decision: &AdmissionDecision,
    ) -> Result<bool, PgStoreError> {
        let mut tx = begin_scoped(&self.pool, scope).await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(format!(
                "admission:{}:{}",
                scope.repository_id.as_uuid(),
                decision.signal_id
            ))
            .execute(&mut *tx)
            .await?;
        let latest = sqlx::query(
            "SELECT version, input_hash FROM signal_admissions
              WHERE organization_id = $1 AND repository_id = $2 AND signal_id = $3
              ORDER BY version DESC LIMIT 1",
        )
        .bind(scope.organization_id.as_uuid())
        .bind(scope.repository_id.as_uuid())
        .bind(&decision.signal_id)
        .fetch_optional(&mut *tx)
        .await?;
        let next_version = match latest {
            Some(row) => {
                let hash: String = row.try_get("input_hash")?;
                if hash == decision.input_hash {
                    tx.commit().await?;
                    return Ok(false);
                }
                row.try_get::<i32, _>("version")? + 1
            }
            None => 1,
        };
        sqlx::query(
            "INSERT INTO signal_admissions
               (organization_id, repository_id, signal_id, input_hash, version, decision, reasons,
                evidence_summary, decided_at, expires_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        )
        .bind(scope.organization_id.as_uuid())
        .bind(scope.repository_id.as_uuid())
        .bind(&decision.signal_id)
        .bind(&decision.input_hash)
        .bind(next_version)
        .bind(decision.outcome.as_str())
        .bind(decision.reasons_json())
        .bind(&decision.evidence_summary)
        .bind(decision.decided_at)
        .bind(decision.expires_at)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(true)
    }

    /// The latest decision of a signal (`review history explain`).
    pub async fn latest_admission(
        &self,
        scope: &RepoScope,
        signal_id: &str,
    ) -> Result<Option<StoredAdmission>, PgStoreError> {
        let mut tx = begin_scoped(&self.pool, scope).await?;
        let row = sqlx::query(
            "SELECT signal_id, version, decision, reasons, evidence_summary, expires_at
               FROM signal_admissions
              WHERE organization_id = $1 AND repository_id = $2 AND signal_id = $3
              ORDER BY version DESC LIMIT 1",
        )
        .bind(scope.organization_id.as_uuid())
        .bind(scope.repository_id.as_uuid())
        .bind(signal_id)
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        row.map(|row| {
            Ok::<_, sqlx::Error>(StoredAdmission {
                signal_id: row.try_get("signal_id")?,
                version: row.try_get("version")?,
                decision: row.try_get("decision")?,
                reasons: row.try_get("reasons")?,
                evidence_summary: row.try_get("evidence_summary")?,
                expires_at: row.try_get("expires_at")?,
            })
        })
        .transpose()
        .map_err(PgStoreError::Database)
    }
}
