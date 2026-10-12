//! `rule_violations` (POL-005), `suppressions` and `suppression_matches` (POL-006).

use chrono::{DateTime, Utc};
use repository::store::RepoScope;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use super::{begin_scoped, PgStoreError};
use crate::config::SuppressionKind;
use crate::policy::suppression::{ActiveSuppression, SuppressionSet, SuppressionSource};
use crate::policy::RuleViolation;

#[derive(Debug, Clone)]
pub struct PgRuleViolationStore {
    pool: PgPool,
}

impl PgRuleViolationStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Persists the violations of a run in one transaction. A retry inserts nothing (the unique
    /// key). Returns how many rows were new. `source` is `explicit`, `documented` or
    /// `convention`.
    pub async fn save(
        &self,
        scope: &RepoScope,
        review_run_id: Uuid,
        source: &str,
        violations: &[RuleViolation],
    ) -> Result<u64, PgStoreError> {
        let mut tx = begin_scoped(&self.pool, scope).await?;
        let mut inserted = 0;
        for v in violations {
            let evidence = serde_json::to_value(&v.evidence)?;
            inserted += sqlx::query(
                "INSERT INTO rule_violations
                   (organization_id, repository_id, review_run_id, rule_id, rule_key, source, path,
                    start_line, end_line, symbol_key, message, evidence, severity)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
                 ON CONFLICT ON CONSTRAINT rule_violations_unique DO NOTHING",
            )
            .bind(scope.organization_id.as_uuid())
            .bind(scope.repository_id.as_uuid())
            .bind(review_run_id)
            .bind(&v.rule_id)
            .bind(&v.rule_key)
            .bind(source)
            .bind(&v.anchor.path)
            .bind(i32::try_from(v.anchor.start_line).unwrap_or(i32::MAX))
            .bind(i32::try_from(v.anchor.end_line).unwrap_or(i32::MAX))
            .bind(v.symbol_key.as_deref())
            .bind(v.message.chars().take(8000).collect::<String>())
            .bind(evidence)
            .bind(v.severity.as_str())
            .execute(&mut *tx)
            .await?
            .rows_affected();
        }
        tx.commit().await?;
        Ok(inserted)
    }

    /// Links a violation to the candidate created from it.
    pub async fn link_candidate(
        &self,
        scope: &RepoScope,
        review_run_id: Uuid,
        rule_id: &str,
        path: &str,
        start_line: u32,
        candidate_finding_id: Uuid,
    ) -> Result<bool, PgStoreError> {
        let mut tx = begin_scoped(&self.pool, scope).await?;
        let updated = sqlx::query(
            "UPDATE rule_violations SET candidate_finding_id = $1
              WHERE organization_id = $2 AND review_run_id = $3 AND rule_id = $4 AND path = $5
                AND start_line = $6",
        )
        .bind(candidate_finding_id)
        .bind(scope.organization_id.as_uuid())
        .bind(review_run_id)
        .bind(rule_id)
        .bind(path)
        .bind(i32::try_from(start_line).unwrap_or(i32::MAX))
        .execute(&mut *tx)
        .await?
        .rows_affected();
        tx.commit().await?;
        Ok(updated > 0)
    }

    pub async fn count_for_run(&self, scope: &RepoScope, review_run_id: Uuid) -> Result<i64, PgStoreError> {
        let mut tx = begin_scoped(&self.pool, scope).await?;
        let n: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM rule_violations WHERE organization_id = $1 AND review_run_id = $2",
        )
        .bind(scope.organization_id.as_uuid())
        .bind(review_run_id)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(n)
    }
}

#[derive(Debug, Clone)]
pub struct PgSuppressionStore {
    pool: PgPool,
}

fn kind_from_str(raw: &str) -> Option<SuppressionKind> {
    Some(match raw {
        "type" => SuppressionKind::Type,
        "path" => SuppressionKind::Path,
        "symbol" => SuppressionKind::Symbol,
        "rule" => SuppressionKind::Rule,
        "fingerprint" => SuppressionKind::Fingerprint,
        _ => return None,
    })
}

/// The row id of a config suppression: stable per repository, config hash and config id.
pub fn config_suppression_id(scope: &RepoScope, config_hash: &str, id: &str) -> Uuid {
    Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("suppression/{}/{config_hash}/{id}", scope.repository_id).as_bytes(),
    )
}

/// A new API suppression.
#[derive(Debug, Clone)]
pub struct NewSuppression {
    pub kind: SuppressionKind,
    pub value: String,
    pub reason: String,
    pub allow_critical: bool,
    pub created_by: Option<Uuid>,
    pub expires_at: Option<DateTime<Utc>>,
}

impl PgSuppressionStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Creates an API suppression; returns its id.
    pub async fn create(&self, scope: &RepoScope, new: &NewSuppression) -> Result<Uuid, PgStoreError> {
        if new.reason.trim().is_empty() {
            return Err(PgStoreError::Corrupt("a suppression needs a reason".to_owned()));
        }
        let id = Uuid::new_v4();
        let mut tx = begin_scoped(&self.pool, scope).await?;
        sqlx::query(
            "INSERT INTO suppressions
               (id, organization_id, repository_id, kind, value, source, reason, allow_critical,
                created_by, expires_at)
             VALUES ($1, $2, $3, $4, $5, 'api', $6, $7, $8, $9)",
        )
        .bind(id)
        .bind(scope.organization_id.as_uuid())
        .bind(scope.repository_id.as_uuid())
        .bind(new.kind.as_str())
        .bind(&new.value)
        .bind(&new.reason)
        .bind(new.allow_critical)
        .bind(new.created_by)
        .bind(new.expires_at)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(id)
    }

    /// Soft-deletes a suppression. `false` when it did not exist or was already revoked.
    pub async fn revoke(&self, scope: &RepoScope, id: Uuid) -> Result<bool, PgStoreError> {
        let mut tx = begin_scoped(&self.pool, scope).await?;
        let n = sqlx::query(
            "UPDATE suppressions SET revoked_at = now()
              WHERE id = $1 AND organization_id = $2 AND repository_id = $3 AND revoked_at IS NULL",
        )
        .bind(id)
        .bind(scope.organization_id.as_uuid())
        .bind(scope.repository_id.as_uuid())
        .execute(&mut *tx)
        .await?
        .rows_affected();
        tx.commit().await?;
        Ok(n == 1)
    }

    /// Mirrors the config suppressions of one config hash into the table so matches can
    /// reference them. Idempotent.
    pub async fn sync_config(
        &self,
        scope: &RepoScope,
        config_hash: &str,
        set: &SuppressionSet,
    ) -> Result<u64, PgStoreError> {
        let mut tx = begin_scoped(&self.pool, scope).await?;
        let mut inserted = 0;
        for s in set.items().iter().filter(|s| s.source == SuppressionSource::Config) {
            inserted += sqlx::query(
                "INSERT INTO suppressions
                   (id, organization_id, repository_id, external_id, kind, value, source,
                    config_hash, reason, allow_critical, expires_at)
                 VALUES ($1, $2, $3, $4, $5, $6, 'config', $7, $8, $9, $10)
                 ON CONFLICT (id) DO NOTHING",
            )
            .bind(config_suppression_id(scope, config_hash, &s.id))
            .bind(scope.organization_id.as_uuid())
            .bind(scope.repository_id.as_uuid())
            .bind(&s.id)
            .bind(s.kind.as_str())
            .bind(&s.value)
            .bind(config_hash)
            .bind(&s.reason)
            .bind(s.allow_critical)
            .bind(s.expires_at)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        }
        tx.commit().await?;
        Ok(inserted)
    }

    /// The API suppressions of the repository that are not revoked (expired ones included, so
    /// the doctor can list them). Read once at the start of verification.
    pub async fn api_suppressions(&self, scope: &RepoScope) -> Result<Vec<ActiveSuppression>, PgStoreError> {
        let mut tx = begin_scoped(&self.pool, scope).await?;
        let rows = sqlx::query(
            "SELECT id, kind, value, reason, allow_critical, expires_at FROM suppressions
              WHERE organization_id = $1 AND repository_id = $2 AND source = 'api'
                AND revoked_at IS NULL
              ORDER BY created_at, id",
        )
        .bind(scope.organization_id.as_uuid())
        .bind(scope.repository_id.as_uuid())
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let id: Uuid = row.try_get("id")?;
            let kind: String = row.try_get("kind")?;
            let Some(kind) = kind_from_str(&kind) else {
                return Err(PgStoreError::Corrupt(format!("unknown suppression kind {kind}")));
            };
            out.push(ActiveSuppression {
                id: id.to_string(),
                kind,
                value: row.try_get("value")?,
                source: SuppressionSource::Api,
                reason: row.try_get("reason")?,
                expires_at: row.try_get("expires_at")?,
                revoked: false,
                allow_critical: row.try_get("allow_critical")?,
            });
        }
        Ok(out)
    }

    /// Records that a suppression matched a candidate. A retry is a no-op.
    pub async fn record_match(
        &self,
        scope: &RepoScope,
        suppression_id: Uuid,
        candidate_finding_id: Uuid,
        review_run_id: Uuid,
    ) -> Result<bool, PgStoreError> {
        let mut tx = begin_scoped(&self.pool, scope).await?;
        let n = sqlx::query(
            "INSERT INTO suppression_matches
               (suppression_id, organization_id, candidate_finding_id, review_run_id)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (suppression_id, candidate_finding_id) DO NOTHING",
        )
        .bind(suppression_id)
        .bind(scope.organization_id.as_uuid())
        .bind(candidate_finding_id)
        .bind(review_run_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        tx.commit().await?;
        Ok(n == 1)
    }
}
