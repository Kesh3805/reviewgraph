//! `PgGraphStore`: the Postgres adapter of the [`GraphStore`] port (GS-004 write path, GS-005
//! read path).
//!
//! * Every transaction first sets `app.organization_id` (`set_config(..., true)`) so row-level
//!   security applies, and every statement also filters on the tenant explicitly.
//! * All SQL is static with bind parameters; rows are written with batched `UNNEST` inserts.
//! * File versions are content addressed: a version's symbols are inserted in the transaction
//!   that creates the version, so a visible `file_versions` row always has its complete symbol
//!   set. Snapshots reference versions; symbol nodes are never copied per snapshot.
//! * Readers materialize a chain exactly as [`flatten`] defines it: the base full snapshot's rows
//!   plus each delta's rows, so this adapter and the in-memory oracle cannot drift.
//! * `neighbors` is one indexed single-hop query over the chain (ADR-014): the latest row per
//!   `(kind, other_key)` wins and tombstones hide the edge.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use async_trait::async_trait;
use repository::store::RepoScope;
use review_core::ids::SnapshotId;
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Postgres, Transaction};
use tracing::Instrument;
use uuid::Uuid;

use super::rows::{
    edge_from_row, file_from_row, get, key_bytes, lineage_from_row, meta_from_row, symbol_from_row,
    synthetic_from_row, to_i32, unresolved_from_row, uuid_of, EdgeColumns, SymbolColumns,
    SyntheticRow, UnresolvedColumns,
};
use crate::kinds::{Confidence, Direction, EdgeKindSet, NodeKey};
use crate::model::{
    flatten, EdgeCursor, EdgeIdentity, FileChange, Graph, GraphDelta, GraphEdge, Node, SnapshotFile,
};
use crate::port::{spans, GraphStore};
use crate::status::{can_transition, SnapshotStatus};
use crate::types::{
    FileVersionInput, FileVersionKey, FileVersionRef, NeighborPage, NewSnapshot, SnapshotKind,
    SnapshotMeta, SnapshotQuery, SnapshotStats, StoredNode, WriteStats, GRAPH_SCHEMA_VERSION,
};
use crate::StoreError;

/// Rows per `UNNEST` insert.
const INSERT_CHUNK_ROWS: usize = 5_000;
/// File versions per upsert transaction.
const UPSERT_BATCH_FILES: usize = 1_000;
/// Maximum rows one `neighbors` page may carry, matching `Graph::neighbors`.
const MAX_NEIGHBOR_LIMIT: u32 = 1_000;
/// Guard against a self-referential chain while walking `base_snapshot_id`.
const MAX_CHAIN_DEPTH: usize = 10_000;
/// Symbol rows are read in batches of this many file-version ids.
const SYMBOL_READ_BATCH: usize = 5_000;

/// Tuning of the Postgres adapter.
#[derive(Debug, Clone)]
pub struct PgStoreConfig {
    /// Rows per insert statement.
    pub chunk_rows: usize,
    /// `statement_timeout` applied to write transactions.
    pub statement_timeout: Duration,
}

impl Default for PgStoreConfig {
    fn default() -> Self {
        Self {
            chunk_rows: INSERT_CHUNK_ROWS,
            statement_timeout: Duration::from_secs(600),
        }
    }
}

/// The Postgres [`GraphStore`].
#[derive(Debug, Clone)]
pub struct PgGraphStore {
    pool: PgPool,
    cfg: PgStoreConfig,
}

/// Maps a driver error: unique violations are conflicts (a duplicate ready fingerprint or a
/// lost race), everything else is a backend failure whose retryability `Classify` decides.
fn db(error: sqlx::Error) -> StoreError {
    if let sqlx::Error::Database(database) = &error {
        if database.code().as_deref() == Some("23505") {
            let constraint = database.constraint().unwrap_or("unique constraint");
            return StoreError::Conflict(format!("duplicate row ({constraint})"));
        }
    }
    StoreError::backend(error)
}

fn invalid_status(id: SnapshotId, expected: SnapshotStatus, found: SnapshotStatus) -> StoreError {
    StoreError::InvalidStatus {
        id,
        expected,
        found,
    }
}

/// The lock-time view of a snapshot row the write path checks.
struct WriteTarget {
    status: SnapshotStatus,
    kind: SnapshotKind,
    base: Option<SnapshotId>,
    schema: u32,
}

impl PgGraphStore {
    pub fn new(pool: PgPool) -> Self {
        Self::with_config(pool, PgStoreConfig::default())
    }

    pub fn with_config(pool: PgPool, cfg: PgStoreConfig) -> Self {
        Self { pool, cfg }
    }

    /// A read-write transaction with the tenant context set.
    async fn begin(&self, scope: &RepoScope) -> Result<Transaction<'static, Postgres>, StoreError> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        set_tenant(&mut tx, scope).await?;
        Ok(tx)
    }

    /// A write transaction with the configured statement timeout.
    async fn begin_write(
        &self,
        scope: &RepoScope,
    ) -> Result<Transaction<'static, Postgres>, StoreError> {
        let mut tx = self.begin(scope).await?;
        let millis = u64::try_from(self.cfg.statement_timeout.as_millis()).unwrap_or(u64::MAX);
        sqlx::query("SELECT set_config('statement_timeout', $1, true)")
            .bind(millis.to_string())
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        Ok(tx)
    }

    /// A `REPEATABLE READ READ ONLY` transaction, so one load sees one consistent view.
    async fn begin_read(
        &self,
        scope: &RepoScope,
    ) -> Result<Transaction<'static, Postgres>, StoreError> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        set_tenant(&mut tx, scope).await?;
        Ok(tx)
    }
}

async fn set_tenant(
    tx: &mut Transaction<'static, Postgres>,
    scope: &RepoScope,
) -> Result<(), StoreError> {
    sqlx::query("SELECT set_config('app.organization_id', $1, true)")
        .bind(scope.organization_id.as_uuid().to_string())
        .execute(&mut **tx)
        .await
        .map_err(db)?;
    Ok(())
}

fn org(scope: &RepoScope) -> Uuid {
    *scope.organization_id.as_uuid()
}

fn repo(scope: &RepoScope) -> Uuid {
    *scope.repository_id.as_uuid()
}

async fn fetch_meta(
    tx: &mut Transaction<'static, Postgres>,
    scope: &RepoScope,
    id: SnapshotId,
) -> Result<Option<SnapshotMeta>, StoreError> {
    let row = sqlx::query(
        "SELECT id, organization_id, repository_id, commit_sha, kind, base_snapshot_id, \
                chain_depth, purpose, status, graph_schema_version, analyzer_versions, \
                config_hash, config_components, fingerprint, stats, error, created_at, \
                updated_at, completed_at
           FROM snapshots
          WHERE id = $1 AND organization_id = $2 AND repository_id = $3",
    )
    .bind(uuid_of(id))
    .bind(org(scope))
    .bind(repo(scope))
    .fetch_optional(&mut **tx)
    .await
    .map_err(db)?;
    row.as_ref().map(meta_from_row).transpose()
}

async fn lock_target(
    tx: &mut Transaction<'static, Postgres>,
    scope: &RepoScope,
    id: SnapshotId,
) -> Result<WriteTarget, StoreError> {
    let row = sqlx::query(
        "SELECT status, kind, base_snapshot_id, graph_schema_version
           FROM snapshots
          WHERE id = $1 AND organization_id = $2 AND repository_id = $3
          FOR UPDATE",
    )
    .bind(uuid_of(id))
    .bind(org(scope))
    .bind(repo(scope))
    .fetch_optional(&mut **tx)
    .await
    .map_err(db)?
    .ok_or(StoreError::NotFound(id))?;
    let status: String = get(&row, "status")?;
    let kind: String = get(&row, "kind")?;
    let base: Option<Uuid> = get(&row, "base_snapshot_id")?;
    let schema: i32 = get(&row, "graph_schema_version")?;
    Ok(WriteTarget {
        status: SnapshotStatus::from_db(&status)
            .ok_or_else(|| StoreError::Integrity("unknown snapshot status".to_owned()))?,
        kind: SnapshotKind::from_db(&kind)
            .ok_or_else(|| StoreError::Integrity("unknown snapshot kind".to_owned()))?,
        base: base.map(SnapshotId::from_uuid),
        schema: u32::try_from(schema).unwrap_or(0),
    })
}

/// `[root, …, id]`, oldest first, every row `Ready` and the root a full snapshot.
async fn chain_metas(
    tx: &mut Transaction<'static, Postgres>,
    scope: &RepoScope,
    id: SnapshotId,
) -> Result<Vec<SnapshotMeta>, StoreError> {
    let mut out: Vec<SnapshotMeta> = Vec::new();
    let mut seen: BTreeSet<SnapshotId> = BTreeSet::new();
    let mut cursor = id;
    loop {
        if !seen.insert(cursor) || out.len() > MAX_CHAIN_DEPTH {
            return Err(StoreError::ChainBroken { id });
        }
        let Some(meta) = fetch_meta(tx, scope, cursor).await? else {
            if cursor == id {
                return Err(StoreError::NotFound(id));
            }
            return Err(StoreError::ChainBroken { id });
        };
        if !meta.status.is_readable() {
            return Err(invalid_status(cursor, SnapshotStatus::Ready, meta.status));
        }
        let base = meta.base;
        out.push(meta);
        match base {
            Some(next) => cursor = next,
            None => break,
        }
    }
    out.reverse();
    match out.first() {
        Some(root) if root.kind == SnapshotKind::Full => Ok(out),
        _ => Err(StoreError::ChainBroken { id }),
    }
}

/// The readable, schema-compatible snapshot `id`, checked the way the in-memory oracle does:
/// scope first, then status, then schema version.
async fn readable(
    tx: &mut Transaction<'static, Postgres>,
    scope: &RepoScope,
    id: SnapshotId,
) -> Result<SnapshotMeta, StoreError> {
    let meta = fetch_meta(tx, scope, id)
        .await?
        .ok_or(StoreError::NotFound(id))?;
    if !meta.status.is_readable() {
        return Err(invalid_status(id, SnapshotStatus::Ready, meta.status));
    }
    if meta.versions.graph_schema_version != GRAPH_SCHEMA_VERSION {
        return Err(StoreError::SchemaMismatch {
            found: meta.versions.graph_schema_version,
            expected: GRAPH_SCHEMA_VERSION,
        });
    }
    Ok(meta)
}

// ------------------------------------------------------------------------------ write helpers

async fn insert_files(
    tx: &mut Transaction<'static, Postgres>,
    snapshot: Uuid,
    organization: Uuid,
    files: &[SnapshotFile],
    chunk: usize,
) -> Result<(), StoreError> {
    for part in files.chunks(chunk.max(1)) {
        let paths: Vec<String> = part.iter().map(|f| f.path.clone()).collect();
        let ids: Vec<Option<i64>> = part.iter().map(|f| f.file_version_id).collect();
        let changes: Vec<String> = part.iter().map(|f| f.change.as_str().to_owned()).collect();
        let old: Vec<Option<String>> = part.iter().map(|f| f.old_path.clone()).collect();
        sqlx::query(
            "INSERT INTO snapshot_files (snapshot_id, organization_id, path, file_version_id, \
                                         change, old_path)
             SELECT $1::uuid, $2::uuid, p, f, c, o
               FROM UNNEST($3::text[], $4::int8[], $5::text[], $6::text[]) AS t(p, f, c, o)",
        )
        .bind(snapshot)
        .bind(organization)
        .bind(paths)
        .bind(ids)
        .bind(changes)
        .bind(old)
        .execute(&mut **tx)
        .await
        .map_err(db)?;
    }
    Ok(())
}

async fn insert_edges(
    tx: &mut Transaction<'static, Postgres>,
    snapshot: Uuid,
    organization: Uuid,
    rows: &[(GraphEdge, bool)],
    versions: &BTreeMap<String, i64>,
    chunk: usize,
) -> Result<(), StoreError> {
    for part in rows.chunks(chunk.max(1)) {
        let mut columns = EdgeColumns::default();
        for (edge, removed) in part {
            let version = edge
                .origin_path
                .as_ref()
                .and_then(|path| versions.get(path).copied());
            columns.push(edge, *removed, version);
        }
        if columns.is_empty() {
            continue;
        }
        sqlx::query(
            "INSERT INTO graph_edges (snapshot_id, organization_id, source_key, kind, target_key, \
                                      confidence, resolved_by, provenance, flags, occurrences, \
                                      origin_path, file_version_id, line, col, removed)
             SELECT $1::uuid, $2::uuid, s, k, tg, c, r, p, f, o, op, fv, l, cl, rm
               FROM UNNEST($3::bytea[], $4::int2[], $5::bytea[], $6::float4[], $7::int2[], \
                           $8::int2[], $9::int2[], $10::int4[], $11::text[], $12::int8[], \
                           $13::int4[], $14::int4[], $15::bool[])
                    AS x(s, k, tg, c, r, p, f, o, op, fv, l, cl, rm)",
        )
        .bind(snapshot)
        .bind(organization)
        .bind(columns.source)
        .bind(columns.kind)
        .bind(columns.target)
        .bind(columns.confidence)
        .bind(columns.resolved_by)
        .bind(columns.provenance)
        .bind(columns.flags)
        .bind(columns.occurrences)
        .bind(columns.origin_path)
        .bind(columns.file_version_id)
        .bind(columns.line)
        .bind(columns.col)
        .bind(columns.removed)
        .execute(&mut **tx)
        .await
        .map_err(db)?;
    }
    Ok(())
}

/// Synthetic node rows: `(key, Some(node))` for an addition, `(key, None)` for a tombstone.
async fn insert_synthetic(
    tx: &mut Transaction<'static, Postgres>,
    snapshot: Uuid,
    organization: Uuid,
    rows: &BTreeMap<NodeKey, Option<Node>>,
    chunk: usize,
) -> Result<(), StoreError> {
    let all: Vec<(&NodeKey, &Option<Node>)> = rows.iter().collect();
    for part in all.chunks(chunk.max(1)) {
        let mut keys: Vec<Vec<u8>> = Vec::with_capacity(part.len());
        let mut ids: Vec<Option<String>> = Vec::with_capacity(part.len());
        let mut kinds: Vec<Option<i16>> = Vec::with_capacity(part.len());
        let mut attrs: Vec<serde_json::Value> = Vec::with_capacity(part.len());
        let mut removed: Vec<bool> = Vec::with_capacity(part.len());
        for (key, node) in part {
            keys.push(key_bytes(**key));
            match node {
                Some(Node::Synthetic(s)) => {
                    ids.push(Some(s.id.clone()));
                    kinds.push(Some(s.kind.as_i16()));
                    attrs.push(s.attrs.clone());
                    removed.push(false);
                }
                _ => {
                    ids.push(None);
                    kinds.push(None);
                    attrs.push(serde_json::json!({}));
                    removed.push(true);
                }
            }
        }
        sqlx::query(
            "INSERT INTO synthetic_nodes (snapshot_id, organization_id, node_key, node_id, kind, \
                                          attrs, removed)
             SELECT $1::uuid, $2::uuid, k, i, kd, a, r
               FROM UNNEST($3::bytea[], $4::text[], $5::int2[], $6::jsonb[], $7::bool[])
                    AS t(k, i, kd, a, r)",
        )
        .bind(snapshot)
        .bind(organization)
        .bind(keys)
        .bind(ids)
        .bind(kinds)
        .bind(attrs)
        .bind(removed)
        .execute(&mut **tx)
        .await
        .map_err(db)?;
    }
    Ok(())
}

async fn insert_unresolved(
    tx: &mut Transaction<'static, Postgres>,
    snapshot: Uuid,
    scope: &RepoScope,
    columns: UnresolvedColumns,
) -> Result<(), StoreError> {
    if columns.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO unresolved_refs (snapshot_id, organization_id, repository_id, file_version_id, \
                                      ordinal, from_symbol_key, name, ref_kind, import_specifier, \
                                      reason, candidate_count, line, col)
         SELECT $1::uuid, $2::uuid, $3::uuid, fv, o, fk, n, rk, sp, r, cc, l, c
           FROM UNNEST($4::int8[], $5::int4[], $6::bytea[], $7::text[], $8::int2[], $9::text[], \
                       $10::int2[], $11::int2[], $12::int4[], $13::int4[])
                AS t(fv, o, fk, n, rk, sp, r, cc, l, c)",
    )
    .bind(snapshot)
    .bind(org(scope))
    .bind(repo(scope))
    .bind(columns.file_version_id)
    .bind(columns.ordinal)
    .bind(columns.from)
    .bind(columns.name)
    .bind(columns.ref_kind)
    .bind(columns.import_specifier)
    .bind(columns.reason)
    .bind(columns.candidate_count)
    .bind(columns.line)
    .bind(columns.col)
    .execute(&mut **tx)
    .await
    .map_err(db)?;
    Ok(())
}

async fn insert_symbols(
    tx: &mut Transaction<'static, Postgres>,
    scope: &RepoScope,
    columns: SymbolColumns,
) -> Result<(), StoreError> {
    if columns.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO symbols (file_version_id, organization_id, repository_id, symbol_key, \
                              symbol_id, kind, name, qualified_name, signature, start_line, \
                              start_col, end_line, end_col, body_hash, signature_hash, \
                              parent_key, visibility, is_exported, is_generated, attrs)
         SELECT fv, $1::uuid, $2::uuid, k, i, kd, n, qn, sg, sl, sc, el, ec, bh, sh, pk, v, ex, \
                gen, a
           FROM UNNEST($3::int8[], $4::bytea[], $5::text[], $6::int2[], $7::text[], $8::text[], \
                       $9::text[], $10::int4[], $11::int4[], $12::int4[], $13::int4[], \
                       $14::bytea[], $15::bytea[], $16::bytea[], $17::int2[], $18::bool[], \
                       $19::bool[], $20::jsonb[])
                AS t(fv, k, i, kd, n, qn, sg, sl, sc, el, ec, bh, sh, pk, v, ex, gen, a)
         ON CONFLICT DO NOTHING",
    )
    .bind(org(scope))
    .bind(repo(scope))
    .bind(columns.file_version_id)
    .bind(columns.key)
    .bind(columns.id)
    .bind(columns.kind)
    .bind(columns.name)
    .bind(columns.qualified_name)
    .bind(columns.signature)
    .bind(columns.start_line)
    .bind(columns.start_col)
    .bind(columns.end_line)
    .bind(columns.end_col)
    .bind(columns.body_hash)
    .bind(columns.signature_hash)
    .bind(columns.parent)
    .bind(columns.visibility)
    .bind(columns.is_exported)
    .bind(columns.is_generated)
    .bind(columns.attrs)
    .execute(&mut **tx)
    .await
    .map_err(db)?;
    Ok(())
}

async fn record_stats(
    tx: &mut Transaction<'static, Postgres>,
    scope: &RepoScope,
    id: SnapshotId,
    stats: &SnapshotStats,
) -> Result<(), StoreError> {
    let json = serde_json::to_value(stats)
        .map_err(|e| StoreError::Integrity(format!("stats do not serialize: {e}")))?;
    sqlx::query(
        "UPDATE snapshots SET stats = $4, updated_at = now()
          WHERE id = $1 AND organization_id = $2 AND repository_id = $3",
    )
    .bind(uuid_of(id))
    .bind(org(scope))
    .bind(repo(scope))
    .bind(json)
    .execute(&mut **tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// The unresolved rows of `path`, which must have a file version in this snapshot.
fn push_unresolved(
    columns: &mut UnresolvedColumns,
    versions: &BTreeMap<String, i64>,
    path: &str,
    rows: &[crate::model::UnresolvedRef],
) -> Result<(), StoreError> {
    if rows.is_empty() {
        return Ok(());
    }
    let version = versions.get(path).copied().ok_or_else(|| {
        StoreError::InvalidRequest(format!(
            "unresolved references of {path} need a file version in the snapshot"
        ))
    })?;
    let mut seen: BTreeSet<u32> = BTreeSet::new();
    for row in rows {
        if seen.insert(row.ordinal) {
            columns.push(version, row);
        }
    }
    Ok(())
}

// ------------------------------------------------------------------------------- read helpers

async fn read_files(
    tx: &mut Transaction<'static, Postgres>,
    snapshot: Uuid,
    organization: Uuid,
) -> Result<Vec<SnapshotFile>, StoreError> {
    let rows = sqlx::query(
        "SELECT path, file_version_id, change, old_path FROM snapshot_files
          WHERE snapshot_id = $1 AND organization_id = $2 ORDER BY path",
    )
    .bind(snapshot)
    .bind(organization)
    .fetch_all(&mut **tx)
    .await
    .map_err(db)?;
    rows.iter().map(file_from_row).collect()
}

async fn read_edges(
    tx: &mut Transaction<'static, Postgres>,
    snapshot: Uuid,
    organization: Uuid,
) -> Result<Vec<(GraphEdge, bool)>, StoreError> {
    let rows = sqlx::query(
        "SELECT source_key, kind, target_key, confidence, resolved_by, provenance, flags, \
                occurrences, origin_path, line, col, removed
           FROM graph_edges WHERE snapshot_id = $1 AND organization_id = $2",
    )
    .bind(snapshot)
    .bind(organization)
    .fetch_all(&mut **tx)
    .await
    .map_err(db)?;
    rows.iter().map(edge_from_row).collect()
}

async fn read_synthetic(
    tx: &mut Transaction<'static, Postgres>,
    snapshot: Uuid,
    organization: Uuid,
) -> Result<Vec<SyntheticRow>, StoreError> {
    let rows = sqlx::query(
        "SELECT node_key, node_id, kind, attrs, removed FROM synthetic_nodes
          WHERE snapshot_id = $1 AND organization_id = $2",
    )
    .bind(snapshot)
    .bind(organization)
    .fetch_all(&mut **tx)
    .await
    .map_err(db)?;
    rows.iter().map(synthetic_from_row).collect()
}

async fn read_symbols(
    tx: &mut Transaction<'static, Postgres>,
    organization: Uuid,
    versions: &[i64],
) -> Result<Vec<Node>, StoreError> {
    let mut out = Vec::new();
    for part in versions.chunks(SYMBOL_READ_BATCH) {
        let rows: Vec<PgRow> = sqlx::query(
            "SELECT s.symbol_key, s.symbol_id, s.kind, s.name, s.qualified_name, s.signature, \
                    s.start_line, s.start_col, s.end_line, s.end_col, s.body_hash, \
                    s.signature_hash, s.parent_key, s.visibility, s.is_exported, s.is_generated, \
                    s.attrs, f.path
               FROM symbols s JOIN file_versions f ON f.id = s.file_version_id
              WHERE s.file_version_id = ANY($1::int8[]) AND s.organization_id = $2",
        )
        .bind(part.to_vec())
        .bind(organization)
        .fetch_all(&mut **tx)
        .await
        .map_err(db)?;
        for row in &rows {
            out.push(symbol_from_row(row)?);
        }
    }
    Ok(out)
}

async fn read_unresolved(
    tx: &mut Transaction<'static, Postgres>,
    snapshot: Uuid,
    organization: Uuid,
) -> Result<Vec<crate::model::UnresolvedRef>, StoreError> {
    let rows = sqlx::query(
        "SELECT u.ordinal, u.from_symbol_key, u.name, u.ref_kind, u.import_specifier, u.reason, \
                u.candidate_count, u.line, u.col, f.path
           FROM unresolved_refs u JOIN file_versions f ON f.id = u.file_version_id
          WHERE u.snapshot_id = $1 AND u.organization_id = $2",
    )
    .bind(snapshot)
    .bind(organization)
    .fetch_all(&mut **tx)
    .await
    .map_err(db)?;
    rows.iter().map(unresolved_from_row).collect()
}

/// The full payload of a full snapshot.
async fn read_full(
    tx: &mut Transaction<'static, Postgres>,
    meta: &SnapshotMeta,
) -> Result<Graph, StoreError> {
    let snapshot = uuid_of(meta.id);
    let organization = org(&meta.scope);
    let files = read_files(tx, snapshot, organization).await?;
    let versions: Vec<i64> = files.iter().filter_map(|f| f.file_version_id).collect();
    let mut nodes = read_symbols(tx, organization, &versions).await?;
    for row in read_synthetic(tx, snapshot, organization).await? {
        if let SyntheticRow::Node(node) = row {
            nodes.push(*node);
        }
    }
    let edges = read_edges(tx, snapshot, organization)
        .await?
        .into_iter()
        .filter(|(_, removed)| !removed)
        .map(|(edge, _)| edge)
        .collect();
    let unresolved = read_unresolved(tx, snapshot, organization).await?;
    Ok(Graph {
        schema_version: meta.versions.graph_schema_version,
        files,
        nodes,
        edges,
        unresolved,
    })
}

/// The rows of one delta snapshot. Symbol nodes are derived from the file versions the delta
/// lists as added, modified, renamed or re-linked; synthetic additions and node tombstones come
/// from `synthetic_nodes`.
async fn read_delta(
    tx: &mut Transaction<'static, Postgres>,
    meta: &SnapshotMeta,
) -> Result<GraphDelta, StoreError> {
    let snapshot = uuid_of(meta.id);
    let organization = org(&meta.scope);
    let files = read_files(tx, snapshot, organization).await?;
    let replaced: Vec<i64> = files
        .iter()
        .filter(|f| f.change.replaces_content() && f.change != FileChange::Deleted)
        .filter_map(|f| f.file_version_id)
        .collect();
    let mut nodes_added = read_symbols(tx, organization, &replaced).await?;
    let mut nodes_removed: Vec<NodeKey> = Vec::new();
    for row in read_synthetic(tx, snapshot, organization).await? {
        match row {
            SyntheticRow::Node(node) => nodes_added.push(*node),
            SyntheticRow::Tombstone(key) => nodes_removed.push(key),
        }
    }
    let mut edges_added = Vec::new();
    let mut edges_removed = Vec::new();
    for (edge, removed) in read_edges(tx, snapshot, organization).await? {
        if removed {
            edges_removed.push(edge);
        } else {
            edges_added.push(edge);
        }
    }
    let mut grouped: BTreeMap<String, Vec<crate::model::UnresolvedRef>> = BTreeMap::new();
    for row in read_unresolved(tx, snapshot, organization).await? {
        grouped.entry(row.file.clone()).or_default().push(row);
    }
    let lineage_rows = sqlx::query(
        "SELECT from_key, to_key, transition, similarity FROM symbol_lineage
          WHERE to_snapshot_id = $1 AND organization_id = $2",
    )
    .bind(snapshot)
    .bind(organization)
    .fetch_all(&mut **tx)
    .await
    .map_err(db)?;
    let lineage = lineage_rows
        .iter()
        .map(lineage_from_row)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(GraphDelta {
        files,
        nodes_added,
        nodes_removed,
        edges_added,
        edges_removed,
        unresolved_replaced: grouped.into_iter().collect(),
        lineage,
    })
}

/// `base ⊕ … ⊕ last` of an already resolved chain.
async fn materialize(
    tx: &mut Transaction<'static, Postgres>,
    chain: &[SnapshotMeta],
) -> Result<Graph, StoreError> {
    let Some((root, rest)) = chain.split_first() else {
        return Err(StoreError::Integrity("empty chain".to_owned()));
    };
    let base = read_full(tx, root).await?;
    let mut deltas = Vec::with_capacity(rest.len());
    for meta in rest {
        if meta.kind != SnapshotKind::Delta {
            return Err(StoreError::ChainBroken { id: meta.id });
        }
        deltas.push(read_delta(tx, meta).await?);
    }
    Ok(flatten(&base, &deltas))
}

// --------------------------------------------------------------------------------- the port

#[async_trait]
impl GraphStore for PgGraphStore {
    async fn create_snapshot(&self, req: NewSnapshot) -> Result<SnapshotMeta, StoreError> {
        req.validate()?;
        let mut tx = self.begin(&req.scope).await?;
        let chain_depth: i16 = match (req.kind, req.base) {
            (SnapshotKind::Delta, Some(base)) => {
                let base_meta = fetch_meta(&mut tx, &req.scope, base)
                    .await?
                    .ok_or(StoreError::NotFound(base))?;
                i16::try_from(base_meta.chain_depth.saturating_add(1)).unwrap_or(i16::MAX)
            }
            _ => 0,
        };
        let analyzers = serde_json::to_value(&req.versions.analyzer_versions)
            .map_err(|e| StoreError::InvalidRequest(e.to_string()))?;
        let components = serde_json::to_value(&req.versions.config_components)
            .map_err(|e| StoreError::InvalidRequest(e.to_string()))?;
        let id = SnapshotId::new();
        let row = sqlx::query(
            "INSERT INTO snapshots (id, organization_id, repository_id, commit_sha, kind, \
                                    base_snapshot_id, chain_depth, purpose, status, \
                                    graph_schema_version, analyzer_versions, config_hash, \
                                    config_components, fingerprint)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'pending', $9, $10, $11, $12, $13)
             RETURNING id, organization_id, repository_id, commit_sha, kind, base_snapshot_id, \
                       chain_depth, purpose, status, graph_schema_version, analyzer_versions, \
                       config_hash, config_components, fingerprint, stats, error, created_at, \
                       updated_at, completed_at",
        )
        .bind(uuid_of(id))
        .bind(org(&req.scope))
        .bind(repo(&req.scope))
        .bind(req.commit_sha.as_str())
        .bind(req.kind.as_str())
        .bind(req.base.map(uuid_of))
        .bind(chain_depth)
        .bind(req.purpose.as_str())
        .bind(i32::try_from(req.versions.graph_schema_version).unwrap_or(i32::MAX))
        .bind(analyzers)
        .bind(req.versions.config_hash.to_vec())
        .bind(components)
        .bind(req.versions.fingerprint.to_vec())
        .fetch_one(&mut *tx)
        .await
        .map_err(db)?;
        let meta = meta_from_row(&row)?;
        tx.commit().await.map_err(db)?;
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
        let mut tx = self.begin(scope).await?;
        if fetch_meta(&mut tx, scope, id).await?.is_none() {
            return Err(StoreError::NotFound(id));
        }
        if !can_transition(from, to) {
            return Ok(false);
        }
        let done = sqlx::query(
            "UPDATE snapshots
                SET status = $5,
                    error = CASE WHEN $5 = 'failed' THEN $6 ELSE error END,
                    updated_at = now(),
                    completed_at = CASE WHEN $5 IN ('ready', 'failed') THEN now()
                                        ELSE completed_at END
              WHERE id = $1 AND organization_id = $2 AND repository_id = $3 AND status = $4",
        )
        .bind(uuid_of(id))
        .bind(org(scope))
        .bind(repo(scope))
        .bind(from.as_str())
        .bind(to.as_str())
        .bind(error)
        .execute(&mut *tx)
        .await
        .map_err(|e| match db(e) {
            StoreError::Conflict(_) => StoreError::Conflict("duplicate fingerprint".to_owned()),
            other => other,
        })?;
        tx.commit().await.map_err(db)?;
        Ok(done.rows_affected() == 1)
    }

    async fn upsert_file_versions(
        &self,
        scope: &RepoScope,
        files: &[FileVersionInput],
    ) -> Result<Vec<FileVersionRef>, StoreError> {
        let mut ids: BTreeMap<FileVersionKey, i64> = BTreeMap::new();
        let span = tracing::info_span!(
            "graph_store.upsert_file_versions",
            adapter = "pg",
            rows_files = files.len()
        );
        async {
            for batch in files.chunks(UPSERT_BATCH_FILES) {
                // One row per key: a repeated key would insert its symbols twice.
                let mut unique: BTreeMap<FileVersionKey, &FileVersionInput> = BTreeMap::new();
                for file in batch {
                    unique.entry(file.key()).or_insert(file);
                }
                let inputs: Vec<&FileVersionInput> = unique.values().copied().collect();
                let mut tx = self.begin_write(scope).await?;
                let inserted = sqlx::query(
                    "INSERT INTO file_versions (organization_id, repository_id, path, \
                                                content_hash, language, analyzer_version, \
                                                parse_status, size_bytes, symbol_count, \
                                                diagnostic_count)
                     SELECT $1::uuid, $2::uuid, p, h, l, a, s, sz, sc, dc
                       FROM UNNEST($3::text[], $4::bytea[], $5::text[], $6::text[], $7::text[], \
                                   $8::int4[], $9::int4[], $10::int4[])
                            AS t(p, h, l, a, s, sz, sc, dc)
                     ON CONFLICT ON CONSTRAINT file_versions_content_key DO NOTHING
                     RETURNING id, path, content_hash, analyzer_version",
                )
                .bind(org(scope))
                .bind(repo(scope))
                .bind(inputs.iter().map(|f| f.path.clone()).collect::<Vec<_>>())
                .bind(
                    inputs
                        .iter()
                        .map(|f| f.content_hash.to_vec())
                        .collect::<Vec<_>>(),
                )
                .bind(
                    inputs
                        .iter()
                        .map(|f| f.language.clone())
                        .collect::<Vec<_>>(),
                )
                .bind(
                    inputs
                        .iter()
                        .map(|f| f.analyzer_version.clone())
                        .collect::<Vec<_>>(),
                )
                .bind(
                    inputs
                        .iter()
                        .map(|f| f.parse_status.as_str().to_owned())
                        .collect::<Vec<_>>(),
                )
                .bind(
                    inputs
                        .iter()
                        .map(|f| to_i32(f.size_bytes))
                        .collect::<Vec<_>>(),
                )
                .bind(
                    inputs
                        .iter()
                        .map(|f| i32::try_from(f.symbols.len()).unwrap_or(i32::MAX))
                        .collect::<Vec<_>>(),
                )
                .bind(
                    inputs
                        .iter()
                        .map(|f| to_i32(f.diagnostic_count))
                        .collect::<Vec<_>>(),
                )
                .fetch_all(&mut *tx)
                .await
                .map_err(db)?;

                // Symbols of the versions this transaction created, in the same transaction.
                let mut symbols = SymbolColumns::default();
                for row in &inserted {
                    let key = key_of_row(row)?;
                    let id: i64 = get(row, "id")?;
                    if let Some(input) = unique.get(&key) {
                        for node in &input.symbols {
                            if let Node::Symbol(symbol) = node {
                                symbols.push(id, symbol);
                            }
                        }
                    }
                }
                insert_symbols(&mut tx, scope, symbols).await?;

                let found = select_versions(&mut tx, scope, inputs.iter().map(|f| f.key())).await?;
                tx.commit().await.map_err(db)?;
                ids.extend(found);
            }
            Ok::<(), StoreError>(())
        }
        .instrument(span)
        .await?;

        files
            .iter()
            .map(|file| {
                let key = file.key();
                let id = ids.get(&key).copied().ok_or_else(|| {
                    StoreError::Integrity(format!("file version {} vanished", key.path))
                })?;
                Ok(FileVersionRef {
                    id,
                    path: key.path,
                    content_hash: key.content_hash,
                    analyzer_version: key.analyzer_version,
                })
            })
            .collect()
    }

    async fn lookup_file_versions(
        &self,
        scope: &RepoScope,
        keys: &[FileVersionKey],
    ) -> Result<Vec<Option<FileVersionRef>>, StoreError> {
        let mut tx = self.begin(scope).await?;
        let found = select_versions(&mut tx, scope, keys.iter().cloned()).await?;
        tx.commit().await.map_err(db)?;
        Ok(keys
            .iter()
            .map(|key| {
                found.get(key).map(|id| FileVersionRef {
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
        let span = tracing::info_span!(
            "graph_store.write_full",
            adapter = "pg",
            rows_files = g.files.len(),
            rows_edges = g.edges.len()
        );
        debug_assert_eq!(spans::WRITE_FULL, "graph_store.write_full");
        async {
            let mut tx = self.begin_write(scope).await?;
            let target = lock_target(&mut tx, scope, id).await?;
            if !target.status.is_writable() {
                return Err(invalid_status(
                    id,
                    SnapshotStatus::Persisting,
                    target.status,
                ));
            }
            if target.kind != SnapshotKind::Full {
                return Err(StoreError::InvalidRequest(
                    "a delta snapshot cannot be written as a full graph".to_owned(),
                ));
            }
            if g.schema_version != target.schema {
                return Err(StoreError::SchemaMismatch {
                    found: g.schema_version,
                    expected: target.schema,
                });
            }
            let g = g.clone().normalized();
            let snapshot = uuid_of(id);
            let organization = org(scope);
            let versions: BTreeMap<String, i64> = g
                .files
                .iter()
                .filter_map(|f| f.file_version_id.map(|v| (f.path.clone(), v)))
                .collect();

            insert_files(
                &mut tx,
                snapshot,
                organization,
                &g.files,
                self.cfg.chunk_rows,
            )
            .await?;
            let edges: Vec<(GraphEdge, bool)> =
                g.edges.iter().map(|e| (e.clone(), false)).collect();
            insert_edges(
                &mut tx,
                snapshot,
                organization,
                &edges,
                &versions,
                self.cfg.chunk_rows,
            )
            .await?;
            let synthetic: BTreeMap<NodeKey, Option<Node>> = g
                .nodes
                .iter()
                .filter(|n| n.is_synthetic())
                .map(|n| (n.key(), Some(n.clone())))
                .collect();
            insert_synthetic(
                &mut tx,
                snapshot,
                organization,
                &synthetic,
                self.cfg.chunk_rows,
            )
            .await?;
            let mut unresolved = UnresolvedColumns::default();
            let mut by_file: BTreeMap<&str, Vec<crate::model::UnresolvedRef>> = BTreeMap::new();
            for row in &g.unresolved {
                by_file
                    .entry(row.file.as_str())
                    .or_default()
                    .push(row.clone());
            }
            for (path, rows) in &by_file {
                push_unresolved(&mut unresolved, &versions, path, rows)?;
            }
            insert_unresolved(&mut tx, snapshot, scope, unresolved).await?;
            record_stats(&mut tx, scope, id, &SnapshotStats::from_graph(&g)).await?;
            tx.commit().await.map_err(db)?;
            Ok::<WriteStats, StoreError>(WriteStats {
                files: g.files.len() as u64,
                nodes: g.nodes.len() as u64,
                edges: g.edges.len() as u64,
                unresolved: g.unresolved.len() as u64,
                lineage: 0,
                file_versions_existing: 0,
                bytes: 0,
            })
        }
        .instrument(span)
        .await
    }

    async fn write_delta(
        &self,
        scope: &RepoScope,
        id: SnapshotId,
        d: &GraphDelta,
    ) -> Result<WriteStats, StoreError> {
        let span = tracing::info_span!(
            "graph_store.write_delta",
            adapter = "pg",
            rows_files = d.files.len(),
            rows_edges = d.edges_added.len() + d.edges_removed.len()
        );
        async {
            let mut tx = self.begin_write(scope).await?;
            let target = lock_target(&mut tx, scope, id).await?;
            if !target.status.is_writable() {
                return Err(invalid_status(
                    id,
                    SnapshotStatus::Persisting,
                    target.status,
                ));
            }
            if target.kind != SnapshotKind::Delta {
                return Err(StoreError::InvalidRequest(
                    "a full snapshot cannot be written as a delta".to_owned(),
                ));
            }
            let base = target.base.ok_or(StoreError::ChainBroken { id })?;
            let base_meta = fetch_meta(&mut tx, scope, base)
                .await?
                .ok_or(StoreError::ChainBroken { id })?;
            if !base_meta.status.is_readable() {
                return Err(invalid_status(
                    base,
                    SnapshotStatus::Ready,
                    base_meta.status,
                ));
            }

            let snapshot = uuid_of(id);
            let organization = org(scope);
            let mut files: BTreeMap<String, SnapshotFile> = BTreeMap::new();
            for file in &d.files {
                files.insert(file.path.clone(), file.clone());
            }
            let files: Vec<SnapshotFile> = files.into_values().collect();
            let versions: BTreeMap<String, i64> = files
                .iter()
                .filter_map(|f| f.file_version_id.map(|v| (f.path.clone(), v)))
                .collect();
            insert_files(&mut tx, snapshot, organization, &files, self.cfg.chunk_rows).await?;

            // One row per identity: a tombstone and a re-add of the same identity in one delta
            // is an override, so the addition is what the row records (flatten applies
            // removals before additions).
            let mut edges: BTreeMap<EdgeIdentity, (GraphEdge, bool)> = BTreeMap::new();
            for edge in &d.edges_removed {
                edges.insert(edge.identity(), (edge.clone(), true));
            }
            for edge in &d.edges_added {
                edges.insert(edge.identity(), (edge.clone(), false));
            }
            let edges: Vec<(GraphEdge, bool)> = edges.into_values().collect();
            insert_edges(
                &mut tx,
                snapshot,
                organization,
                &edges,
                &versions,
                self.cfg.chunk_rows,
            )
            .await?;

            let mut synthetic: BTreeMap<NodeKey, Option<Node>> = BTreeMap::new();
            for key in &d.nodes_removed {
                synthetic.insert(*key, None);
            }
            for node in d.nodes_added.iter().filter(|n| n.is_synthetic()) {
                synthetic.insert(node.key(), Some(node.clone()));
            }
            insert_synthetic(
                &mut tx,
                snapshot,
                organization,
                &synthetic,
                self.cfg.chunk_rows,
            )
            .await?;

            let mut unresolved = UnresolvedColumns::default();
            for (path, rows) in &d.unresolved_replaced {
                push_unresolved(&mut unresolved, &versions, path, rows)?;
            }
            insert_unresolved(&mut tx, snapshot, scope, unresolved).await?;

            let mut lineage: BTreeMap<(NodeKey, NodeKey), &crate::model::Lineage> = BTreeMap::new();
            for record in &d.lineage {
                lineage.insert((record.from_key, record.to_key), record);
            }
            if !lineage.is_empty() {
                let records: Vec<&crate::model::Lineage> = lineage.into_values().collect();
                sqlx::query(
                    "INSERT INTO symbol_lineage (organization_id, repository_id, from_snapshot_id, \
                                                 to_snapshot_id, from_key, to_key, transition, \
                                                 similarity)
                     SELECT $1::uuid, $2::uuid, $3::uuid, $4::uuid, f, t, tr, s
                       FROM UNNEST($5::bytea[], $6::bytea[], $7::text[], $8::float4[])
                            AS x(f, t, tr, s)",
                )
                .bind(organization)
                .bind(repo(scope))
                .bind(uuid_of(base))
                .bind(snapshot)
                .bind(records.iter().map(|r| key_bytes(r.from_key)).collect::<Vec<_>>())
                .bind(records.iter().map(|r| key_bytes(r.to_key)).collect::<Vec<_>>())
                .bind(
                    records
                        .iter()
                        .map(|r| r.transition.as_str().to_owned())
                        .collect::<Vec<_>>(),
                )
                .bind(
                    records
                        .iter()
                        .map(|r| r.similarity.clamp(0.0, 1.0))
                        .collect::<Vec<_>>(),
                )
                .execute(&mut *tx)
                .await
                .map_err(db)?;
            }

            record_stats(&mut tx, scope, id, &SnapshotStats::from_delta(d)).await?;
            tx.commit().await.map_err(db)?;
            Ok::<WriteStats, StoreError>(WriteStats {
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
        .instrument(span)
        .await
    }

    async fn load_graph(&self, scope: &RepoScope, id: SnapshotId) -> Result<Graph, StoreError> {
        let span = tracing::info_span!(
            "graph_store.load_graph",
            adapter = "pg",
            chain_depth = tracing::field::Empty,
            nodes = tracing::field::Empty,
            edges = tracing::field::Empty
        );
        let recorder = span.clone();
        async {
            let mut tx = self.begin_read(scope).await?;
            readable(&mut tx, scope, id).await?;
            let chain = chain_metas(&mut tx, scope, id).await?;
            let graph = materialize(&mut tx, &chain).await?;
            tx.commit().await.map_err(db)?;
            recorder.record("chain_depth", chain.len().saturating_sub(1));
            recorder.record("nodes", graph.nodes.len());
            recorder.record("edges", graph.edges.len());
            Ok::<Graph, StoreError>(graph)
        }
        .instrument(span)
        .await
    }

    async fn load_delta(
        &self,
        scope: &RepoScope,
        id: SnapshotId,
    ) -> Result<GraphDelta, StoreError> {
        let mut tx = self.begin_read(scope).await?;
        let meta = fetch_meta(&mut tx, scope, id)
            .await?
            .ok_or(StoreError::NotFound(id))?;
        if !meta.status.is_readable() {
            return Err(invalid_status(id, SnapshotStatus::Ready, meta.status));
        }
        if meta.kind != SnapshotKind::Delta {
            return Err(StoreError::InvalidRequest(
                "a full snapshot has no delta rows".to_owned(),
            ));
        }
        let delta = read_delta(&mut tx, &meta).await?;
        tx.commit().await.map_err(db)?;
        Ok(delta)
    }

    async fn snapshot(
        &self,
        scope: &RepoScope,
        id: SnapshotId,
    ) -> Result<Option<SnapshotMeta>, StoreError> {
        let mut tx = self.begin(scope).await?;
        let meta = fetch_meta(&mut tx, scope, id).await?;
        tx.commit().await.map_err(db)?;
        Ok(meta)
    }

    async fn find_ready(
        &self,
        scope: &RepoScope,
        q: SnapshotQuery,
    ) -> Result<Option<SnapshotMeta>, StoreError> {
        let mut tx = self.begin(scope).await?;
        let row = sqlx::query(
            "SELECT id, organization_id, repository_id, commit_sha, kind, base_snapshot_id, \
                    chain_depth, purpose, status, graph_schema_version, analyzer_versions, \
                    config_hash, config_components, fingerprint, stats, error, created_at, \
                    updated_at, completed_at
               FROM snapshots
              WHERE organization_id = $1 AND repository_id = $2 AND status = 'ready'
                AND ($3::text IS NULL OR commit_sha = $3)
                AND ($4::bytea IS NULL OR fingerprint = $4)
                AND ($5::text IS NULL OR kind = $5)
                AND ($6::text IS NULL OR purpose = $6)
              ORDER BY created_at DESC, id DESC
              LIMIT 1",
        )
        .bind(org(scope))
        .bind(repo(scope))
        .bind(q.commit_sha.as_ref().map(|c| c.as_str().to_owned()))
        .bind(q.fingerprint.map(|f| f.to_vec()))
        .bind(q.kind.map(|k| k.as_str().to_owned()))
        .bind(q.purpose.map(|p| p.as_str().to_owned()))
        .fetch_optional(&mut *tx)
        .await
        .map_err(db)?;
        let meta = row.as_ref().map(meta_from_row).transpose()?;
        tx.commit().await.map_err(db)?;
        Ok(meta)
    }

    async fn chain(
        &self,
        scope: &RepoScope,
        id: SnapshotId,
    ) -> Result<Vec<SnapshotMeta>, StoreError> {
        let mut tx = self.begin_read(scope).await?;
        let chain = chain_metas(&mut tx, scope, id).await?;
        tx.commit().await.map_err(db)?;
        Ok(chain)
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
        let span = tracing::info_span!(
            "graph_store.neighbors",
            adapter = "pg",
            direction = dir.as_str()
        );
        async {
            let mut tx = self.begin_read(scope).await?;
            readable(&mut tx, scope, id).await?;
            let chain = chain_metas(&mut tx, scope, id).await?;
            let ids: Vec<Uuid> = chain.iter().map(|m| uuid_of(m.id)).collect();
            let wanted: Vec<i16> = kinds.iter().map(|k| k.as_i16()).collect();
            // The latest row per (kind, other endpoint) across the chain wins; a tombstone hides
            // the edge. Kind order and paging are applied below on the decoded rows.
            let sql = match dir {
                Direction::Out => {
                    "WITH chain(snapshot_id, pos) AS
                       (SELECT * FROM unnest($1::uuid[]) WITH ORDINALITY)
                     SELECT DISTINCT ON (e.kind, e.target_key)
                            e.source_key, e.kind, e.target_key, e.confidence, e.resolved_by,
                            e.provenance, e.flags, e.occurrences, e.origin_path, e.line, e.col,
                            e.removed
                       FROM graph_edges e JOIN chain c USING (snapshot_id)
                      WHERE e.organization_id = $2 AND e.source_key = $3
                        AND e.kind = ANY($4::int2[])
                      ORDER BY e.kind, e.target_key, c.pos DESC"
                }
                Direction::In => {
                    "WITH chain(snapshot_id, pos) AS
                       (SELECT * FROM unnest($1::uuid[]) WITH ORDINALITY)
                     SELECT DISTINCT ON (e.kind, e.source_key)
                            e.source_key, e.kind, e.target_key, e.confidence, e.resolved_by,
                            e.provenance, e.flags, e.occurrences, e.origin_path, e.line, e.col,
                            e.removed
                       FROM graph_edges e JOIN chain c USING (snapshot_id)
                      WHERE e.organization_id = $2 AND e.target_key = $3
                        AND e.kind = ANY($4::int2[])
                      ORDER BY e.kind, e.source_key, c.pos DESC"
                }
            };
            let rows = sqlx::query(sql)
                .bind(ids)
                .bind(org(scope))
                .bind(key_bytes(key))
                .bind(wanted)
                .fetch_all(&mut *tx)
                .await
                .map_err(db)?;
            tx.commit().await.map_err(db)?;

            let other = |e: &GraphEdge| match dir {
                Direction::Out => e.target,
                Direction::In => e.source,
            };
            let mut matched: Vec<GraphEdge> = Vec::new();
            for row in &rows {
                let (edge, removed) = edge_from_row(row)?;
                if removed || edge.confidence < min_confidence {
                    continue;
                }
                if cursor.is_some_and(|c| (edge.kind, other(&edge)) <= (c.kind, c.key)) {
                    continue;
                }
                matched.push(edge);
            }
            matched.sort_by_key(|e| (e.kind, other(e)));
            let take = limit.clamp(1, MAX_NEIGHBOR_LIMIT) as usize;
            let total = matched.len();
            matched.truncate(take);
            let next_cursor = if total > take {
                matched.last().map(|e| EdgeCursor {
                    kind: e.kind,
                    key: other(e),
                })
            } else {
                None
            };
            Ok::<NeighborPage, StoreError>(NeighborPage {
                edges: matched,
                next_cursor,
            })
        }
        .instrument(span)
        .await
    }

    async fn nodes(
        &self,
        scope: &RepoScope,
        id: SnapshotId,
        keys: &[NodeKey],
    ) -> Result<Vec<Option<StoredNode>>, StoreError> {
        let graph = self.load_graph(scope, id).await?;
        let by_key: BTreeMap<NodeKey, &Node> = graph.nodes.iter().map(|n| (n.key(), n)).collect();
        Ok(keys
            .iter()
            .map(|k| by_key.get(k).map(|n| (*n).clone()))
            .collect())
    }

    async fn set_stats(
        &self,
        scope: &RepoScope,
        id: SnapshotId,
        stats: SnapshotStats,
    ) -> Result<(), StoreError> {
        let mut tx = self.begin_write(scope).await?;
        let target = lock_target(&mut tx, scope, id).await?;
        if !target.status.is_writable() {
            return Err(invalid_status(
                id,
                SnapshotStatus::Persisting,
                target.status,
            ));
        }
        record_stats(&mut tx, scope, id, &stats).await?;
        tx.commit().await.map_err(db)?;
        Ok(())
    }
}

fn key_of_row(row: &PgRow) -> Result<FileVersionKey, StoreError> {
    let hash: Vec<u8> = get(row, "content_hash")?;
    Ok(FileVersionKey {
        path: get(row, "path")?,
        content_hash: hash
            .try_into()
            .map_err(|_| StoreError::Integrity("content hash is not 32 bytes".to_owned()))?,
        analyzer_version: get(row, "analyzer_version")?,
    })
}

/// Ids of the stored file versions among `keys`.
async fn select_versions(
    tx: &mut Transaction<'static, Postgres>,
    scope: &RepoScope,
    keys: impl Iterator<Item = FileVersionKey>,
) -> Result<BTreeMap<FileVersionKey, i64>, StoreError> {
    let keys: Vec<FileVersionKey> = keys.collect();
    let mut out = BTreeMap::new();
    if keys.is_empty() {
        return Ok(out);
    }
    let rows = sqlx::query(
        "SELECT f.id, f.path, f.content_hash, f.analyzer_version
           FROM file_versions f
           JOIN UNNEST($3::text[], $4::bytea[], $5::text[]) AS k(path, content_hash, analyzer_version)
             ON f.path = k.path AND f.content_hash = k.content_hash
            AND f.analyzer_version = k.analyzer_version
          WHERE f.organization_id = $1 AND f.repository_id = $2",
    )
    .bind(org(scope))
    .bind(repo(scope))
    .bind(keys.iter().map(|k| k.path.clone()).collect::<Vec<_>>())
    .bind(keys.iter().map(|k| k.content_hash.to_vec()).collect::<Vec<_>>())
    .bind(keys.iter().map(|k| k.analyzer_version.clone()).collect::<Vec<_>>())
    .fetch_all(&mut **tx)
    .await
    .map_err(db)?;
    for row in &rows {
        out.insert(key_of_row(row)?, get(row, "id")?);
    }
    Ok(out)
}
