//! In-memory reference adapter (GS-001).
//!
//! It is the oracle the conformance suite is validated against: whatever it does, the
//! Postgres and file adapters must do too. One `RwLock` guards all state; no lock is held
//! across an `await`, so every method body is atomic.

use std::collections::{BTreeMap, BTreeSet};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use repository::store::RepoScope;
use review_core::ids::{CommitSha, SnapshotId};
use tokio::sync::RwLock;

use crate::kinds::{Confidence, Direction, EdgeKindSet, NodeKey};
use crate::model::{flatten, EdgeCursor, Graph, GraphDelta, GraphEdge};
use crate::port::GraphStore;
use crate::status::{can_transition, SnapshotStatus};
use crate::types::{
    FileVersionInput, FileVersionKey, FileVersionRef, NeighborPage, NewSnapshot, SnapshotKind,
    SnapshotMeta, SnapshotPurpose, SnapshotQuery, SnapshotStats, SnapshotVersions, StoredNode,
    WriteStats, GRAPH_SCHEMA_VERSION,
};
use crate::StoreError;

/// Maximum rows one `neighbors` page may carry, matching `Graph::neighbors`.
const MAX_NEIGHBOR_LIMIT: u32 = 1000;

/// Guard against a self-referential chain (`base` cycles) while walking.
const MAX_CHAIN_DEPTH: usize = 10_000;

/// The reference implementation of [`GraphStore`].
#[derive(Debug, Default)]
pub struct MemGraphStore {
    inner: RwLock<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    /// Monotonic sequence used to order `find_ready` deterministically.
    seq: u64,
    snapshots: BTreeMap<SnapshotId, Row>,
    file_versions: BTreeMap<(uuid::Uuid, uuid::Uuid), FileVersions>,
    next_file_version: i64,
}

#[derive(Debug)]
struct Row {
    scope: RepoScope,
    commit_sha: CommitSha,
    kind: SnapshotKind,
    base: Option<SnapshotId>,
    chain_depth: u16,
    purpose: SnapshotPurpose,
    status: SnapshotStatus,
    versions: SnapshotVersions,
    stats: SnapshotStats,
    error: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    completed_at: Option<DateTime<Utc>>,
    /// Insertion order; ties in `created_at` are broken by this.
    seq: u64,
    /// `None` until a `write_*` call fills it.
    payload: Option<Payload>,
}

#[derive(Debug, Clone)]
enum Payload {
    Full(Graph),
    Delta(GraphDelta),
}

#[derive(Debug, Default)]
struct FileVersions {
    by_key: BTreeMap<FileVersionKey, i64>,
    rows: BTreeMap<i64, FileVersionRow>,
}

/// One stored file version. The attributes mirror `file_versions` columns; the port only
/// exposes identity through [`FileVersionRef`], so nothing reads them back yet.
#[derive(Debug)]
#[allow(dead_code)]
struct FileVersionRow {
    key: FileVersionKey,
    language: String,
    parse_status: crate::types::FileParseStatus,
    size_bytes: u32,
    diagnostic_count: u32,
    symbols: Vec<StoredNode>,
}

impl Row {
    fn meta(&self, id: SnapshotId) -> SnapshotMeta {
        SnapshotMeta {
            id,
            scope: self.scope,
            commit_sha: self.commit_sha.clone(),
            kind: self.kind,
            base: self.base,
            chain_depth: self.chain_depth,
            purpose: self.purpose,
            status: self.status,
            versions: self.versions.clone(),
            stats: self.stats.clone(),
            error: self.error.clone(),
            created_at: self.created_at,
            updated_at: self.updated_at,
            completed_at: self.completed_at,
        }
    }

    /// The counters implied by whatever payload is stored (the row starts empty).
    fn refresh_stats(&mut self) {
        match &self.payload {
            Some(Payload::Full(g)) => self.stats = SnapshotStats::from_graph(g),
            Some(Payload::Delta(d)) => self.stats = SnapshotStats::from_delta(d),
            None => {}
        }
    }
}

impl Inner {
    fn row(&self, scope: &RepoScope, id: SnapshotId) -> Result<&Row, StoreError> {
        // Cross-tenant ids are NotFound, never "forbidden": no existence oracle.
        self.snapshots
            .get(&id)
            .filter(|row| row.scope == *scope)
            .ok_or(StoreError::NotFound(id))
    }

    fn row_mut(&mut self, scope: &RepoScope, id: SnapshotId) -> Result<&mut Row, StoreError> {
        self.snapshots
            .get_mut(&id)
            .filter(|row| row.scope == *scope)
            .ok_or(StoreError::NotFound(id))
    }

    /// `[root, …, id]`, oldest first, every row `Ready`.
    fn chain_ids(&self, scope: &RepoScope, id: SnapshotId) -> Result<Vec<SnapshotId>, StoreError> {
        let mut ids = Vec::new();
        let mut seen = BTreeSet::new();
        let mut cursor = id;
        loop {
            if !seen.insert(cursor) {
                return Err(StoreError::ChainBroken { id });
            }
            if ids.len() > MAX_CHAIN_DEPTH {
                return Err(StoreError::ChainBroken { id });
            }
            let row = self.row(scope, cursor)?;
            if !row.status.is_readable() {
                return Err(StoreError::InvalidStatus {
                    id: cursor,
                    expected: SnapshotStatus::Ready,
                    found: row.status,
                });
            }
            ids.push(cursor);
            match row.base {
                Some(base) => cursor = base,
                None => break,
            }
        }
        ids.reverse();
        let Some(&root) = ids.first() else {
            return Err(StoreError::ChainBroken { id });
        };
        if self.row(scope, root)?.kind != SnapshotKind::Full {
            return Err(StoreError::ChainBroken { id });
        }
        Ok(ids)
    }

    /// `base ⊕ … ⊕ id` (ADR-003). Missing payloads read as empty, exactly as an adapter with
    /// no rows would.
    fn materialize(&self, scope: &RepoScope, id: SnapshotId) -> Result<Graph, StoreError> {
        let row = self.row(scope, id)?;
        if !row.status.is_readable() {
            return Err(StoreError::InvalidStatus {
                id,
                expected: SnapshotStatus::Ready,
                found: row.status,
            });
        }
        if row.versions.graph_schema_version != GRAPH_SCHEMA_VERSION {
            return Err(StoreError::SchemaMismatch {
                found: row.versions.graph_schema_version,
                expected: GRAPH_SCHEMA_VERSION,
            });
        }

        let ids = self.chain_ids(scope, id)?;
        let Some(&root_id) = ids.first() else {
            return Err(StoreError::ChainBroken { id });
        };
        let root = self.row(scope, root_id)?;
        let base = match &root.payload {
            Some(Payload::Full(g)) => g.clone(),
            Some(Payload::Delta(_)) => return Err(StoreError::ChainBroken { id: root_id }),
            None => Graph {
                schema_version: root.versions.graph_schema_version,
                ..Graph::default()
            },
        };

        let mut deltas = Vec::with_capacity(ids.len().saturating_sub(1));
        for linked in &ids[1..] {
            let linked_row = self.row(scope, *linked)?;
            deltas.push(match &linked_row.payload {
                Some(Payload::Delta(d)) => d.clone(),
                Some(Payload::Full(_)) => return Err(StoreError::ChainBroken { id: *linked }),
                None => GraphDelta::default(),
            });
        }
        Ok(flatten(&base, &deltas))
    }

    fn matching_rows<'a>(
        &'a self,
        scope: &'a RepoScope,
        q: &'a SnapshotQuery,
    ) -> impl Iterator<Item = (&'a SnapshotId, &'a Row)> + 'a {
        self.snapshots.iter().filter(move |(_, row)| {
            row.scope == *scope
                && row.status.is_readable()
                && q.commit_sha.as_ref().is_none_or(|c| row.commit_sha == *c)
                && q.fingerprint.is_none_or(|f| row.versions.fingerprint == f)
                && q.kind.is_none_or(|k| row.kind == k)
                && q.purpose.is_none_or(|p| row.purpose == p)
        })
    }
}

impl MemGraphStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl GraphStore for MemGraphStore {
    async fn create_snapshot(&self, req: NewSnapshot) -> Result<SnapshotMeta, StoreError> {
        req.validate()?;
        let mut inner = self.inner.write().await;
        let (chain_depth, base) = match req.kind {
            SnapshotKind::Full => {
                if req.base.is_some() {
                    return Err(StoreError::InvalidRequest(
                        "a full snapshot must not name a base".to_owned(),
                    ));
                }
                (0, None)
            }
            SnapshotKind::Delta => {
                let base_id = req.base.ok_or_else(|| {
                    StoreError::InvalidRequest("a delta snapshot must name its base".to_owned())
                })?;
                let base_row = inner.row(&req.scope, base_id)?;
                // The base must exist in this scope; whether it is `Ready` is checked again by
                // `write_delta`, so a delta can be created before its base is finished.
                (base_row.chain_depth.saturating_add(1), Some(base_id))
            }
        };

        let now = Utc::now();
        inner.seq += 1;
        let seq = inner.seq;
        let id = SnapshotId::new();
        let row = Row {
            scope: req.scope,
            commit_sha: req.commit_sha,
            kind: req.kind,
            base,
            chain_depth,
            purpose: req.purpose,
            status: SnapshotStatus::Pending,
            versions: req.versions,
            stats: SnapshotStats::default(),
            error: None,
            created_at: now,
            updated_at: now,
            completed_at: None,
            seq,
            payload: None,
        };
        let meta = row.meta(id);
        inner.snapshots.insert(id, row);
        Ok(meta)
    }

    async fn transition(
        &self,
        scope: &RepoScope,
        id: SnapshotId,
        from: SnapshotStatus,
        to: SnapshotStatus,
        error: Option<&str>,
    ) -> Result<bool, StoreError> {
        let mut inner = self.inner.write().await;
        let row = inner.row_mut(scope, id)?;
        if row.status != from || !can_transition(from, to) {
            return Ok(false);
        }
        row.status = to;
        row.updated_at = Utc::now();
        if matches!(to, SnapshotStatus::Ready | SnapshotStatus::Failed) {
            row.completed_at = Some(row.updated_at);
        }
        if to == SnapshotStatus::Failed {
            row.error = error.map(str::to_owned);
        }
        Ok(true)
    }

    async fn upsert_file_versions(
        &self,
        scope: &RepoScope,
        files: &[FileVersionInput],
    ) -> Result<Vec<FileVersionRef>, StoreError> {
        let mut inner = self.inner.write().await;
        let bucket_key = (
            *scope.organization_id.as_uuid(),
            *scope.repository_id.as_uuid(),
        );
        // The lock guard derefs as a whole, so the counter and the bucket cannot be borrowed
        // at once: allocate ids first, then move the new rows into the bucket.
        let mut assigned: Vec<(FileVersionKey, i64)> = Vec::with_capacity(files.len());
        let mut pending: BTreeMap<FileVersionKey, i64> = BTreeMap::new();
        let mut created: Vec<(i64, FileVersionRow)> = Vec::new();
        for file in files {
            let key = file.key();
            let known = pending.get(&key).copied().or_else(|| {
                inner
                    .file_versions
                    .get(&bucket_key)
                    .and_then(|bucket| bucket.by_key.get(&key).copied())
            });
            let id = match known {
                Some(id) => id,
                None => {
                    inner.next_file_version += 1;
                    let id = inner.next_file_version;
                    created.push((
                        id,
                        FileVersionRow {
                            key: key.clone(),
                            language: file.language.clone(),
                            parse_status: file.parse_status,
                            size_bytes: file.size_bytes,
                            diagnostic_count: file.diagnostic_count,
                            symbols: file.symbols.clone(),
                        },
                    ));
                    pending.insert(key.clone(), id);
                    id
                }
            };
            assigned.push((key, id));
        }
        if !created.is_empty() {
            let bucket = inner.file_versions.entry(bucket_key).or_default();
            for (id, row) in created {
                bucket.by_key.insert(row.key.clone(), id);
                bucket.rows.insert(id, row);
            }
        }
        Ok(assigned
            .into_iter()
            .map(|(key, id)| FileVersionRef {
                id,
                path: key.path,
                content_hash: key.content_hash,
                analyzer_version: key.analyzer_version,
            })
            .collect())
    }

    async fn lookup_file_versions(
        &self,
        scope: &RepoScope,
        keys: &[FileVersionKey],
    ) -> Result<Vec<Option<FileVersionRef>>, StoreError> {
        let inner = self.inner.read().await;
        let Some(bucket) = inner.file_versions.get(&(
            *scope.organization_id.as_uuid(),
            *scope.repository_id.as_uuid(),
        )) else {
            return Ok(vec![None; keys.len()]);
        };
        Ok(keys
            .iter()
            .map(|key| {
                bucket.by_key.get(key).map(|id| FileVersionRef {
                    id: *id,
                    path: key.path.clone(),
                    content_hash: key.content_hash,
                    analyzer_version: key.analyzer_version.clone(),
                })
            })
            .collect())
    }

    async fn write_full(
        &self,
        scope: &RepoScope,
        id: SnapshotId,
        g: &Graph,
    ) -> Result<WriteStats, StoreError> {
        let mut inner = self.inner.write().await;
        let row = inner.row_mut(scope, id)?;
        if !row.status.is_writable() {
            return Err(StoreError::InvalidStatus {
                id,
                expected: SnapshotStatus::Persisting,
                found: row.status,
            });
        }
        if row.kind != SnapshotKind::Full {
            return Err(StoreError::InvalidRequest(
                "a delta snapshot cannot be written as a full graph".to_owned(),
            ));
        }
        if g.schema_version != row.versions.graph_schema_version {
            return Err(StoreError::SchemaMismatch {
                found: g.schema_version,
                expected: row.versions.graph_schema_version,
            });
        }
        row.payload = Some(Payload::Full(g.clone().normalized()));
        row.refresh_stats();
        Ok(WriteStats {
            files: g.files.len() as u64,
            nodes: g.nodes.len() as u64,
            edges: g.edges.len() as u64,
            unresolved: g.unresolved.len() as u64,
            lineage: 0,
            file_versions_existing: 0,
            bytes: 0,
        })
    }

    async fn write_delta(
        &self,
        scope: &RepoScope,
        id: SnapshotId,
        d: &GraphDelta,
    ) -> Result<WriteStats, StoreError> {
        let mut inner = self.inner.write().await;
        let row = inner.row(scope, id)?;
        if !row.status.is_writable() {
            return Err(StoreError::InvalidStatus {
                id,
                expected: SnapshotStatus::Persisting,
                found: row.status,
            });
        }
        if row.kind != SnapshotKind::Delta {
            return Err(StoreError::InvalidRequest(
                "a full snapshot cannot be written as a delta".to_owned(),
            ));
        }
        let base_id = row.base.ok_or(StoreError::ChainBroken { id })?;
        let base_status = inner.row(scope, base_id)?.status;
        if !base_status.is_readable() {
            return Err(StoreError::InvalidStatus {
                id: base_id,
                expected: SnapshotStatus::Ready,
                found: base_status,
            });
        }

        let row = inner.row_mut(scope, id)?;
        row.payload = Some(Payload::Delta(d.clone()));
        row.refresh_stats();
        Ok(WriteStats {
            files: d.files.len() as u64,
            nodes: d.nodes_added.len() as u64,
            edges: d.edges_added.len() as u64,
            unresolved: d
                .unresolved_replaced
                .iter()
                .map(|(_, rows)| rows.len() as u64)
                .sum(),
            lineage: d.lineage.len() as u64,
            file_versions_existing: 0,
            bytes: 0,
        })
    }

    async fn load_graph(&self, scope: &RepoScope, id: SnapshotId) -> Result<Graph, StoreError> {
        let inner = self.inner.read().await;
        inner.materialize(scope, id)
    }

    async fn load_delta(
        &self,
        scope: &RepoScope,
        id: SnapshotId,
    ) -> Result<GraphDelta, StoreError> {
        let inner = self.inner.read().await;
        let row = inner.row(scope, id)?;
        if !row.status.is_readable() {
            return Err(StoreError::InvalidStatus {
                id,
                expected: SnapshotStatus::Ready,
                found: row.status,
            });
        }
        match &row.payload {
            Some(Payload::Delta(d)) => Ok(d.clone()),
            Some(Payload::Full(_)) => Err(StoreError::InvalidRequest(
                "a full snapshot has no delta rows".to_owned(),
            )),
            None => Ok(GraphDelta::default()),
        }
    }

    async fn snapshot(
        &self,
        scope: &RepoScope,
        id: SnapshotId,
    ) -> Result<Option<SnapshotMeta>, StoreError> {
        let inner = self.inner.read().await;
        if let Some(row) = inner.snapshots.get(&id).filter(|row| row.scope == *scope) {
            return Ok(Some(row.meta(id)));
        }
        Ok(None)
    }

    async fn find_ready(
        &self,
        scope: &RepoScope,
        q: SnapshotQuery,
    ) -> Result<Option<SnapshotMeta>, StoreError> {
        let inner = self.inner.read().await;
        let newest = inner
            .matching_rows(scope, &q)
            .max_by_key(|(id, row)| (row.created_at, row.seq, **id));
        Ok(newest.map(|(id, row)| row.meta(*id)))
    }

    async fn chain(
        &self,
        scope: &RepoScope,
        id: SnapshotId,
    ) -> Result<Vec<SnapshotMeta>, StoreError> {
        let inner = self.inner.read().await;
        let ids = inner.chain_ids(scope, id)?;
        ids.iter()
            .map(|linked| inner.row(scope, *linked).map(|row| row.meta(*linked)))
            .collect()
    }

    async fn neighbors(
        &self,
        scope: &RepoScope,
        id: SnapshotId,
        key: NodeKey,
        dir: Direction,
        kinds: EdgeKindSet,
        min_confidence: Confidence,
        limit: u32,
        cursor: Option<EdgeCursor>,
    ) -> Result<NeighborPage, StoreError> {
        let inner = self.inner.read().await;
        let graph = inner.materialize(scope, id)?;
        let take = limit.clamp(1, MAX_NEIGHBOR_LIMIT) as usize;

        let mut matched: Vec<&GraphEdge> = graph
            .edges
            .iter()
            .filter(|e| {
                let (endpoint, other) = match dir {
                    Direction::Out => (e.source, e.target),
                    Direction::In => (e.target, e.source),
                };
                endpoint == key
                    && kinds.contains(e.kind)
                    && e.confidence >= min_confidence
                    && cursor.is_none_or(|c| (e.kind, other) > (c.kind, c.key))
            })
            .collect();
        matched.sort_by(|a, b| {
            let ka = match dir {
                Direction::Out => a.target,
                Direction::In => a.source,
            };
            let kb = match dir {
                Direction::Out => b.target,
                Direction::In => b.source,
            };
            (a.kind, ka).cmp(&(b.kind, kb))
        });

        let total = matched.len();
        let page: Vec<&GraphEdge> = matched.into_iter().take(take).collect();
        let next_cursor = if total > take {
            page.last().map(|e| {
                let other = match dir {
                    Direction::Out => e.target,
                    Direction::In => e.source,
                };
                EdgeCursor {
                    kind: e.kind,
                    key: other,
                }
            })
        } else {
            None
        };
        Ok(NeighborPage {
            edges: page.into_iter().cloned().collect(),
            next_cursor,
        })
    }

    async fn nodes(
        &self,
        scope: &RepoScope,
        id: SnapshotId,
        keys: &[NodeKey],
    ) -> Result<Vec<Option<StoredNode>>, StoreError> {
        let inner = self.inner.read().await;
        let graph = inner.materialize(scope, id)?;
        Ok(keys.iter().map(|k| graph.node(*k).cloned()).collect())
    }

    async fn set_stats(
        &self,
        scope: &RepoScope,
        id: SnapshotId,
        stats: SnapshotStats,
    ) -> Result<(), StoreError> {
        let mut inner = self.inner.write().await;
        let row = inner.row_mut(scope, id)?;
        if !row.status.is_writable() {
            return Err(StoreError::InvalidStatus {
                id,
                expected: SnapshotStatus::Persisting,
                found: row.status,
            });
        }
        row.stats = stats;
        row.updated_at = Utc::now();
        Ok(())
    }
}
