//! Persistence of budget usage in `review_runs.budget_usage` (PIPE-006).
//!
//! The snapshot is written at stage boundaries and restored on resume, so a resumed run neither
//! double-spends nor resets what earlier stages used.

use serde::{Deserialize, Serialize};
use sqlx::PgConnection;

use super::limits::BudgetKind;
use super::manager::{BudgetEvent, BudgetManager, BudgetUsage};

/// Wire form of [`BudgetUsage`] plus the skipped units.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BudgetSnapshot {
    pub used: std::collections::BTreeMap<String, u64>,
    pub limits: std::collections::BTreeMap<String, u64>,
    pub skipped: Vec<SkippedUnit>,
}

/// One unit that was not reviewed because a budget ran out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkippedUnit {
    pub kind: String,
    pub unit: String,
    pub at_ms: u64,
}

impl BudgetSnapshot {
    pub fn capture(manager: &BudgetManager) -> Self {
        let usage = manager.usage();
        let mut used = std::collections::BTreeMap::new();
        let mut limits = std::collections::BTreeMap::new();
        for kind in BudgetKind::ALL {
            used.insert(kind.as_str().to_owned(), usage.used(kind));
            limits.insert(kind.as_str().to_owned(), usage.limits[kind.index()]);
        }
        let skipped = manager
            .events()
            .into_iter()
            .map(|e| SkippedUnit {
                kind: e.kind.as_str().to_owned(),
                unit: e.skipped_unit,
                at_ms: e.at_ms,
            })
            .collect();
        Self {
            used,
            limits,
            skipped,
        }
    }

    /// The usage and events to hand to [`BudgetManager::resume_with_events`]. Unknown kinds in
    /// stored data are ignored.
    pub fn restore(&self) -> (BudgetUsage, Vec<BudgetEvent>) {
        let mut usage = BudgetUsage::default();
        for kind in BudgetKind::ALL {
            if let Some(n) = self.used.get(kind.as_str()) {
                usage.used[kind.index()] = *n;
            }
        }
        let events = self
            .skipped
            .iter()
            .filter_map(|s| {
                let kind = BudgetKind::ALL.into_iter().find(|k| k.as_str() == s.kind)?;
                Some(BudgetEvent {
                    kind,
                    skipped_unit: s.unit.clone(),
                    at_ms: s.at_ms,
                })
            })
            .collect();
        (usage, events)
    }
}

/// Writes the manager's current usage to the run row.
pub async fn save_usage(
    conn: &mut PgConnection,
    organization_id: &str,
    review_run_id: &str,
    manager: &BudgetManager,
) -> Result<(), sqlx::Error> {
    let json = serde_json::to_string(&BudgetSnapshot::capture(manager))
        .map_err(|e| sqlx::Error::Protocol(e.to_string()))?;
    sqlx::query(
        "UPDATE review_runs SET budget_usage = $3::jsonb
         WHERE id = $1::uuid AND organization_id = $2::uuid",
    )
    .bind(review_run_id)
    .bind(organization_id)
    .bind(json)
    .execute(conn)
    .await?;
    Ok(())
}

/// Reads the persisted snapshot (an empty one for a run that has not stored usage yet).
pub async fn load_usage(
    conn: &mut PgConnection,
    organization_id: &str,
    review_run_id: &str,
) -> Result<BudgetSnapshot, sqlx::Error> {
    let json: Option<String> = sqlx::query_scalar(
        "SELECT budget_usage::text FROM review_runs
         WHERE id = $1::uuid AND organization_id = $2::uuid",
    )
    .bind(review_run_id)
    .bind(organization_id)
    .fetch_optional(conn)
    .await?;
    Ok(json
        .and_then(|j| serde_json::from_str(&j).ok())
        .unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::ReviewBudget;

    #[test]
    fn snapshot_roundtrips_usage_and_events() {
        let m = BudgetManager::new(ReviewBudget::DEFAULT);
        let r = m.try_reserve(BudgetKind::ModelTokens, 1234).unwrap();
        m.commit(r, 1000);
        m.record_skip(BudgetKind::ModelCalls, "cluster-9");
        let snap = BudgetSnapshot::capture(&m);
        let json = serde_json::to_string(&snap).unwrap();
        let back: BudgetSnapshot = serde_json::from_str(&json).unwrap();
        let (usage, events) = back.restore();
        let resumed = BudgetManager::resume_with_events(ReviewBudget::DEFAULT, &usage, events);
        assert_eq!(resumed.usage().used(BudgetKind::ModelTokens), 1000);
        assert_eq!(resumed.events()[0].skipped_unit, "cluster-9");
    }

    #[test]
    fn empty_json_object_restores_to_zero_usage() {
        let snap: BudgetSnapshot = serde_json::from_str("{}").unwrap_or_default();
        let (usage, events) = snap.restore();
        assert_eq!(usage, BudgetUsage::default());
        assert!(events.is_empty());
    }
}
