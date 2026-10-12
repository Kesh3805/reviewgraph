//! Incremental embedding sync (SEM-007).
//!
//! Work is proportional to change: a unit whose content hash is already stored is never
//! re-embedded (its point only gains the snapshot id), a rename with an unchanged body moves the
//! stored vector to the new id, and removed units are deleted. Point ids are deterministic, so a
//! crashed sync is safely re-run and a repeated sync performs zero embedding calls.

use std::collections::{BTreeMap, HashMap};

use review_core::ids::{RepositoryId, SnapshotId};
use tracing::Instrument;
use uuid::Uuid;

use crate::collections::CollectionRegistry;
use crate::embedding::estimate_tokens;
use crate::error::Result;
use crate::filter::{fields, Cond};
use crate::index::{DeleteSelector, ExistingPoint, SemanticIndex};
use crate::metrics;
use crate::point_id::point_id;
use crate::qdrant::types::PointUpsert;
use crate::tenant::{ExtraFilter, TenantScope};
use crate::units::{EmbeddingUnit, UnitKind};

/// Default per-sync embedding token cap.
pub const DEFAULT_TOKEN_BUDGET: u64 = 2_000_000;
/// Units per sync batch.
pub const SYNC_BATCH: usize = 256;
/// Snapshot ids kept per point.
pub const SNAPSHOT_CAP: usize = 50;

/// Identifies a stored unit.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UnitRef {
    pub kind: UnitKind,
    pub key: String,
    pub repository_id: RepositoryId,
}

impl From<&EmbeddingUnit> for UnitRef {
    fn from(u: &EmbeddingUnit) -> Self {
        Self {
            kind: u.kind,
            key: u.key.clone(),
            repository_id: u.repository_id,
        }
    }
}

/// A unit whose key changed while its text did not (SID-005 lineage, unchanged `body_hash`).
#[derive(Debug, Clone, PartialEq)]
pub struct Rekey {
    pub from: UnitRef,
    pub to: EmbeddingUnit,
}

/// Sync knobs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncOptions {
    /// Estimated tokens (`ceil(chars/4)`) this run may embed; the rest is deferred.
    pub token_budget: u64,
    pub batch: usize,
    pub snapshot_cap: usize,
    /// Snapshot ids the cap never drops (the default-branch head).
    pub pinned_snapshots: Vec<SnapshotId>,
}

impl Default for SyncOptions {
    fn default() -> Self {
        Self {
            token_budget: DEFAULT_TOKEN_BUDGET,
            batch: SYNC_BATCH,
            snapshot_cap: SNAPSHOT_CAP,
            pinned_snapshots: Vec::new(),
        }
    }
}

/// What one sync run should do for one snapshot.
#[derive(Debug, Clone, Copy)]
pub struct SyncRequest<'a> {
    pub snapshot_id: SnapshotId,
    /// Units built for this snapshot that may be new or changed.
    pub changed: &'a [EmbeddingUnit],
    pub removed: &'a [UnitRef],
    pub renamed: &'a [Rekey],
}

/// Counts of one sync run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SyncReport {
    /// Units embedded and upserted.
    pub embedded: u64,
    /// Units whose content hash was already stored (no embedding call).
    pub skipped_unchanged: u64,
    /// Lineage renames moved without embedding.
    pub rekeyed: u64,
    pub deleted: u64,
    /// Units left for the next run because of the token budget.
    pub deferred: u64,
    /// Provider-reported tokens.
    pub tokens: u64,
}

impl SyncReport {
    fn add(&mut self, other: SyncReport) {
        self.embedded += other.embedded;
        self.skipped_unchanged += other.skipped_unchanged;
        self.rekeyed += other.rekeyed;
        self.deleted += other.deleted;
        self.deferred += other.deferred;
        self.tokens += other.tokens;
    }
}

/// Lock key serializing syncs of one repository into one space.
pub fn sync_lock_key(repo: RepositoryId, space_id: &str) -> String {
    format!("semsync:{repo}:{space_id}")
}

/// Appends `add` (most recent last), then drops the oldest unpinned ids beyond `cap`.
pub fn merge_snapshot_ids(
    existing: &[String],
    add: &str,
    cap: usize,
    pinned: &[String],
) -> Vec<String> {
    let mut out: Vec<String> = existing.iter().filter(|s| *s != add).cloned().collect();
    out.push(add.to_owned());
    while out.len() > cap.max(1) {
        match out.iter().position(|s| !pinned.contains(s) && s != add) {
            Some(i) => {
                out.remove(i);
            }
            None => break,
        }
    }
    out
}

struct Ctx<'a> {
    index: &'a SemanticIndex,
    scope: &'a TenantScope,
    snapshot: String,
    opts: &'a SyncOptions,
    pinned: Vec<String>,
    space_id: String,
    tokens_planned: u64,
    budget_hit: bool,
}

impl Ctx<'_> {
    fn id_of(&self, r: &UnitRef) -> Uuid {
        point_id(
            self.scope.organization_id(),
            r.repository_id,
            r.kind,
            &r.key,
            &self.space_id,
        )
    }

    fn merged(&self, existing: &[String]) -> Vec<String> {
        merge_snapshot_ids(
            existing,
            &self.snapshot,
            self.opts.snapshot_cap,
            &self.pinned,
        )
    }

    /// Hash check, snapshot append for unchanged units, embedding for the rest.
    async fn sync_batch(&mut self, units: &[&EmbeddingUnit]) -> Result<SyncReport> {
        let mut report = SyncReport::default();
        let targets = self.index.targets()?;
        let ids: Vec<Uuid> = units
            .iter()
            .map(|u| self.id_of(&UnitRef::from(*u)))
            .collect();
        let mut existing: Vec<(String, HashMap<Uuid, ExistingPoint>)> = Vec::new();
        for c in &targets.write {
            existing.push((c.clone(), self.index.existing(self.scope, c, &ids).await?));
        }
        // collection -> snapshot list -> point ids
        let mut appends: BTreeMap<(String, Vec<String>), Vec<Uuid>> = BTreeMap::new();
        let mut to_embed: Vec<(&EmbeddingUnit, Uuid, Vec<String>)> = Vec::new();
        for (unit, id) in units.iter().zip(&ids) {
            let hash = unit.content_hash.to_string();
            let unchanged = existing
                .iter()
                .all(|(_, m)| m.get(id).and_then(|p| p.content_hash.as_ref()) == Some(&hash));
            if unchanged {
                report.skipped_unchanged += 1;
                for (c, m) in &existing {
                    let current = m.get(id).map(|p| p.snapshot_ids.as_slice()).unwrap_or(&[]);
                    if current.iter().any(|s| *s == self.snapshot) {
                        continue;
                    }
                    appends
                        .entry((c.clone(), self.merged(current)))
                        .or_default()
                        .push(*id);
                }
                continue;
            }
            let est = estimate_tokens(&unit.text) as u64;
            if self.budget_hit || self.tokens_planned + est > self.opts.token_budget {
                self.budget_hit = true;
                report.deferred += 1;
                continue;
            }
            self.tokens_planned += est;
            let previous: Vec<String> = existing
                .iter()
                .find_map(|(_, m)| m.get(id))
                .map(|p| p.snapshot_ids.clone())
                .unwrap_or_default();
            to_embed.push((unit, *id, self.merged(&previous)));
        }
        for ((c, list), point_ids) in appends {
            self.index
                .set_snapshot_ids(self.scope, &c, &point_ids, &list)
                .await?;
        }
        if !to_embed.is_empty() {
            let refs: Vec<&EmbeddingUnit> = to_embed.iter().map(|(u, _, _)| *u).collect();
            let (vectors, tokens) = self.index.embed_units(self.scope, &refs).await?;
            report.tokens += tokens;
            let mut points = Vec::with_capacity(to_embed.len());
            for ((unit, id, snaps), vector) in to_embed.iter().zip(vectors) {
                points.push(PointUpsert {
                    id: *id,
                    vector,
                    payload: self.index.payload_for(self.scope, unit, snaps)?,
                });
            }
            self.index.write_points(&targets, &points).await?;
            report.embedded += points.len() as u64;
        }
        Ok(report)
    }

    /// Moves stored vectors to their new ids. Returns the renames that need embedding instead.
    async fn rekey_batch<'r>(
        &mut self,
        renames: &'r [Rekey],
        report: &mut SyncReport,
    ) -> Result<Vec<&'r EmbeddingUnit>> {
        let targets = self.index.targets()?;
        let from_ids: Vec<Uuid> = renames.iter().map(|r| self.id_of(&r.from)).collect();
        let stored = self
            .index
            .vectors(self.scope, &targets.read, &from_ids)
            .await?;
        let mut points = Vec::new();
        let mut moved = Vec::new();
        let mut fallback = Vec::new();
        for (r, from_id) in renames.iter().zip(&from_ids) {
            let Some((vector, payload)) = stored.get(from_id) else {
                fallback.push(&r.to);
                continue;
            };
            let stored_hash = payload.get(fields::CONTENT_HASH).and_then(|v| v.as_str());
            if stored_hash != Some(r.to.content_hash.to_string().as_str()) {
                // The text changed after all (for example the summary's module line).
                fallback.push(&r.to);
                continue;
            }
            let previous: Vec<String> = payload
                .get(fields::SNAPSHOT_IDS)
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|s| s.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            let to_id = self.id_of(&UnitRef::from(&r.to));
            points.push(PointUpsert {
                id: to_id,
                vector: vector.clone(),
                payload: self
                    .index
                    .payload_for(self.scope, &r.to, &self.merged(&previous))?,
            });
            if to_id != *from_id {
                moved.push(*from_id);
            }
        }
        self.index.write_points(&targets, &points).await?;
        self.index.delete_ids(self.scope, &targets, &moved).await?;
        report.rekeyed += points.len() as u64;
        Ok(fallback)
    }
}

async fn sync_unlocked(
    index: &SemanticIndex,
    scope: &TenantScope,
    req: SyncRequest<'_>,
    opts: &SyncOptions,
) -> Result<SyncReport> {
    let mut ctx = Ctx {
        index,
        scope,
        snapshot: req.snapshot_id.to_string(),
        opts,
        pinned: opts
            .pinned_snapshots
            .iter()
            .map(|s| s.to_string())
            .collect(),
        space_id: index.space().id(),
        tokens_planned: 0,
        budget_hit: false,
    };
    let batch = opts.batch.max(1);
    let mut report = SyncReport::default();

    let mut pending: Vec<&EmbeddingUnit> = Vec::new();
    for renames in req.renamed.chunks(batch) {
        pending.extend(ctx.rekey_batch(renames, &mut report).await?);
    }
    pending.extend(req.changed.iter());
    for units in pending.chunks(batch) {
        let r = ctx.sync_batch(units).await?;
        report.add(r);
    }
    if !req.removed.is_empty() {
        let targets = index.targets()?;
        let ids: Vec<Uuid> = req.removed.iter().map(|r| ctx.id_of(r)).collect();
        index.delete_ids(scope, &targets, &ids).await?;
        report.deleted += ids.len() as u64;
    }
    Ok(report)
}

/// Syncs one snapshot's changes. With a `registry`, runs under the per-repository advisory
/// lock so two syncs of the same repository and space never interleave.
pub async fn sync(
    index: &SemanticIndex,
    scope: &TenantScope,
    req: SyncRequest<'_>,
    opts: &SyncOptions,
    registry: Option<&dyn CollectionRegistry>,
) -> Result<SyncReport> {
    let span = tracing::info_span!(
        "semantic_sync",
        organization_id = %scope.organization_id(),
        snapshot_id = %req.snapshot_id,
        embedded = tracing::field::Empty,
        skipped_unchanged = tracing::field::Empty,
        rekeyed = tracing::field::Empty,
        deleted = tracing::field::Empty,
        deferred = tracing::field::Empty,
        tokens = tracing::field::Empty,
    );
    let run = async {
        let mut locks = Vec::new();
        if let Some(reg) = registry {
            let space_id = index.space().id();
            // Sorted repository order: no lock-order deadlock between overlapping scopes.
            for repo in scope.repository_ids() {
                locks.push(reg.lock(&sync_lock_key(*repo, &space_id)).await?);
            }
        }
        let report = sync_unlocked(index, scope, req, opts).await;
        drop(locks);
        report
    };
    let report = run.instrument(span.clone()).await?;
    span.record("embedded", report.embedded);
    span.record("skipped_unchanged", report.skipped_unchanged);
    span.record("rekeyed", report.rekeyed);
    span.record("deleted", report.deleted);
    span.record("deferred", report.deferred);
    span.record("tokens", report.tokens);
    metrics::sync_counts(
        report.embedded,
        report.skipped_unchanged,
        report.rekeyed,
        report.deleted,
        report.deferred,
    );
    Ok(report)
}

/// Garbage collection: deletes points of each repository in `scope` that belong to none of the
/// `live` snapshots (default-branch head plus open pull-request heads, computed by the caller
/// from PostgreSQL). Returns the number of points deleted from the read collection.
pub async fn gc(index: &SemanticIndex, scope: &TenantScope, live: &[SnapshotId]) -> Result<u64> {
    let mut deleted = 0;
    for repo in scope.repository_ids() {
        let Some(one) = scope.narrowed(*repo) else {
            continue;
        };
        if !live.is_empty() {
            let extra = ExtraFilter::new().must_not(Cond::any(fields::SNAPSHOT_IDS, live))?;
            deleted += index.count(&one, &extra).await?;
        }
        index
            .delete_units(&one, DeleteSelector::NotInSnapshots(live.to_vec()))
            .await?;
    }
    metrics::sync_counts(0, 0, 0, deleted, 0);
    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_cap_keeps_pinned_and_newest() {
        let existing: Vec<String> = (0..5).map(|i| format!("s{i}")).collect();
        let pinned = vec!["s0".to_owned()];
        let out = merge_snapshot_ids(&existing, "s5", 3, &pinned);
        assert_eq!(out, vec!["s0", "s4", "s5"]);
        let again = merge_snapshot_ids(&out, "s4", 3, &pinned);
        assert_eq!(again, vec!["s0", "s5", "s4"]);
    }
}
