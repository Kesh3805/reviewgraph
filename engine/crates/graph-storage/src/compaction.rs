//! Snapshot compaction (GS-007, ADR-003): once a delta chain passes 20 deltas or 10% edge churn,
//! its materialization is written as a new full snapshot.
//!
//! Compaction only ever *adds* a snapshot: the chain it replaces stays readable, so readers of
//! the old chain are unaffected and a failed compaction costs nothing but the attempt. Built on
//! the port's primitives, so it works unchanged for every adapter.

use repository::store::RepoScope;
use review_core::ids::SnapshotId;

use crate::port::GraphStore;
use crate::status::SnapshotStatus;
use crate::types::{
    NewSnapshot, SnapshotKind, SnapshotMeta, SnapshotPurpose, SnapshotQuery, SnapshotStats,
};
use crate::StoreError;

/// When a chain is rewritten as a full snapshot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CompactionPolicy {
    /// Deltas allowed on top of the full snapshot.
    pub max_chain_depth: u16,
    /// `Σ (edges_added + edges_removed) / base edges` allowed before compacting.
    pub max_churn_ratio: f32,
}

impl Default for CompactionPolicy {
    fn default() -> Self {
        Self {
            max_chain_depth: 20,
            max_churn_ratio: 0.10,
        }
    }
}

/// Why a chain is compacted.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CompactionReason {
    /// The chain holds this many deltas.
    ChainDepth(u16),
    /// The accumulated edge churn relative to the base.
    Churn(f32),
}

impl CompactionReason {
    /// Metric label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ChainDepth(_) => "chain_depth",
            Self::Churn(_) => "churn",
        }
    }
}

/// The policy's verdict on one chain.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CompactionDecision {
    Keep,
    Compact { reason: CompactionReason },
}

/// Edge churn of a chain `[full, delta…]` from the stats recorded at write time (no row counting).
pub fn churn(chain: &[SnapshotMeta]) -> f32 {
    let Some((base, deltas)) = chain.split_first() else {
        return 0.0;
    };
    let changed: u64 = deltas
        .iter()
        .map(|d| d.stats.edges_added + d.stats.edges_removed)
        .sum();
    if changed == 0 {
        return 0.0;
    }
    if base.stats.edges == 0 {
        return f32::INFINITY;
    }
    changed as f32 / base.stats.edges as f32
}

/// Evaluates the policy on a chain `[full, delta…]`, oldest first. O(chain).
pub fn should_compact(chain: &[SnapshotMeta], policy: &CompactionPolicy) -> CompactionDecision {
    let depth = u16::try_from(chain.len().saturating_sub(1)).unwrap_or(u16::MAX);
    if depth > policy.max_chain_depth {
        return CompactionDecision::Compact {
            reason: CompactionReason::ChainDepth(depth),
        };
    }
    let ratio = churn(chain);
    if depth > 0 && ratio > policy.max_churn_ratio {
        return CompactionDecision::Compact {
            reason: CompactionReason::Churn(ratio),
        };
    }
    CompactionDecision::Keep
}

/// Writes the materialization of `head`'s chain as a new `Ready` full snapshot with
/// `purpose = compaction`, recording the replaced chain in its stats.
///
/// Idempotent: a ready compaction of the same head (same commit and fingerprint) is returned
/// without rewriting, and a concurrent compaction that loses the race on the ready-fingerprint
/// index marks its own snapshot failed and returns the winner.
pub async fn compact(
    store: &dyn GraphStore,
    scope: &RepoScope,
    head: SnapshotId,
) -> Result<SnapshotMeta, StoreError> {
    let span = tracing::info_span!(
        "graph_store.compact",
        chain_depth = tracing::field::Empty,
        churn = tracing::field::Empty
    );
    let chain = store.chain(scope, head).await?;
    span.record("chain_depth", chain.len().saturating_sub(1));
    span.record("churn", f64::from(churn(&chain)));
    let Some(head_meta) = chain.last().cloned() else {
        return Err(StoreError::ChainBroken { id: head });
    };
    if head_meta.kind == SnapshotKind::Full {
        return Ok(head_meta);
    }
    if let Some(done) = existing(store, scope, &head_meta).await? {
        return Ok(done);
    }

    let graph = store.load_graph(scope, head).await?;
    let created = store
        .create_snapshot(NewSnapshot {
            scope: *scope,
            commit_sha: head_meta.commit_sha.clone(),
            kind: SnapshotKind::Full,
            base: None,
            purpose: SnapshotPurpose::Compaction,
            versions: head_meta.versions.clone(),
        })
        .await?;
    let id = created.id;

    let written = async {
        step(
            store,
            scope,
            id,
            SnapshotStatus::Pending,
            SnapshotStatus::Indexing,
        )
        .await?;
        step(
            store,
            scope,
            id,
            SnapshotStatus::Indexing,
            SnapshotStatus::Persisting,
        )
        .await?;
        store.write_full(scope, id, &graph).await?;
        let mut stats = SnapshotStats::from_graph(&graph);
        stats.compacted_from = Some(head);
        stats.replaced_chain = chain.iter().map(|m| m.id).collect();
        store.set_stats(scope, id, stats).await?;
        step(
            store,
            scope,
            id,
            SnapshotStatus::Persisting,
            SnapshotStatus::Ready,
        )
        .await
    }
    .await;

    match written {
        Ok(()) => store
            .snapshot(scope, id)
            .await?
            .ok_or(StoreError::NotFound(id)),
        Err(error) => {
            // Leave the chain as it was; the attempt's own snapshot is failed (best effort).
            let _ = store
                .transition(
                    scope,
                    id,
                    SnapshotStatus::Persisting,
                    SnapshotStatus::Failed,
                    Some("compaction failed"),
                )
                .await;
            if matches!(error, StoreError::Conflict(_)) {
                if let Some(winner) = existing(store, scope, &head_meta).await? {
                    return Ok(winner);
                }
            }
            Err(error)
        }
    }
}

/// A ready full snapshot already standing for `head` (same commit and fingerprint).
async fn existing(
    store: &dyn GraphStore,
    scope: &RepoScope,
    head: &SnapshotMeta,
) -> Result<Option<SnapshotMeta>, StoreError> {
    let query = SnapshotQuery {
        commit_sha: Some(head.commit_sha.clone()),
        fingerprint: Some(head.versions.fingerprint),
        kind: Some(SnapshotKind::Full),
        purpose: None,
    };
    store.find_ready(scope, query).await
}

async fn step(
    store: &dyn GraphStore,
    scope: &RepoScope,
    id: SnapshotId,
    from: SnapshotStatus,
    to: SnapshotStatus,
) -> Result<(), StoreError> {
    if store.transition(scope, id, from, to, None).await? {
        Ok(())
    } else {
        Err(StoreError::Conflict(format!(
            "compaction snapshot {id} could not move {from} -> {to}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SnapshotVersions;
    use chrono::Utc;
    use review_core::ids::{OrganizationId, RepositoryId};

    fn meta(kind: SnapshotKind, edges: u64, added: u64, removed: u64) -> SnapshotMeta {
        let now = Utc::now();
        SnapshotMeta {
            id: SnapshotId::new(),
            scope: RepoScope {
                organization_id: OrganizationId::new(),
                repository_id: RepositoryId::new(),
            },
            commit_sha: "a".repeat(40).parse().unwrap_or_else(|_| unreachable!()),
            kind,
            base: None,
            chain_depth: 0,
            purpose: SnapshotPurpose::DefaultBranch,
            status: SnapshotStatus::Ready,
            versions: SnapshotVersions::default(),
            stats: SnapshotStats {
                edges,
                edges_added: added,
                edges_removed: removed,
                ..SnapshotStats::default()
            },
            error: None,
            created_at: now,
            updated_at: now,
            completed_at: Some(now),
        }
    }

    fn chain(deltas: usize, churn_each: u64) -> Vec<SnapshotMeta> {
        let mut out = vec![meta(SnapshotKind::Full, 1_000, 1_000, 0)];
        for _ in 0..deltas {
            out.push(meta(SnapshotKind::Delta, 0, churn_each, 0));
        }
        out
    }

    #[test]
    fn policy_triggers_at_depth_21() {
        let policy = CompactionPolicy::default();
        assert_eq!(
            should_compact(&chain(20, 0), &policy),
            CompactionDecision::Keep
        );
        assert_eq!(
            should_compact(&chain(21, 0), &policy),
            CompactionDecision::Compact {
                reason: CompactionReason::ChainDepth(21)
            }
        );
    }

    #[test]
    fn policy_triggers_at_churn_over_10_percent() {
        let policy = CompactionPolicy::default();
        // 2 deltas x 50 edges over a 1,000-edge base: exactly 10%, kept.
        assert_eq!(
            should_compact(&chain(2, 50), &policy),
            CompactionDecision::Keep
        );
        match should_compact(&chain(2, 51), &policy) {
            CompactionDecision::Compact {
                reason: CompactionReason::Churn(ratio),
            } => assert!(ratio > 0.10),
            other => panic!("expected churn compaction, got {other:?}"),
        }
    }

    #[test]
    fn a_full_snapshot_alone_is_kept() {
        assert_eq!(
            should_compact(&chain(0, 0), &CompactionPolicy::default()),
            CompactionDecision::Keep
        );
        assert_eq!(churn(&[]), 0.0);
    }
}
