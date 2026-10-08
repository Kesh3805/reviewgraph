//! Row encoders and decoders of the Postgres graph adapter (GS-004/GS-005).
//!
//! Every column the adapter writes is turned into one bind array here (`UNNEST` inserts), and
//! every row it reads is decoded back into the port's value types. Out-of-range stored values are
//! [`StoreError::Integrity`], never a panic.

use std::collections::BTreeMap;

use repository::store::RepoScope;
use review_core::ids::{CommitSha, OrganizationId, RepositoryId, SnapshotId};
use sqlx::postgres::PgRow;
use sqlx::Row;
use uuid::Uuid;

use crate::kinds::{Confidence, EdgeFlags, EdgeKind, NodeKey, NodeKind, Provenance, ResolvedBy};
use crate::model::{
    FileChange, GraphEdge, Lineage, LineageTransition, Node, SnapshotFile, SourceRange, SymbolNode,
    SyntheticNode, UnresolvedRef,
};
use crate::status::SnapshotStatus;
use crate::types::{SnapshotKind, SnapshotMeta, SnapshotPurpose, SnapshotStats, SnapshotVersions};
use crate::StoreError;

/// The `snapshots` columns [`meta_from_row`] reads, in the order every query selects them.
pub const META_COLUMNS: &str = "id, organization_id, repository_id, commit_sha, kind, \
     base_snapshot_id, chain_depth, purpose, status, graph_schema_version, analyzer_versions, \
     config_hash, config_components, fingerprint, stats, error, created_at, updated_at, completed_at";

pub fn get<'r, T>(row: &'r PgRow, column: &str) -> Result<T, StoreError>
where
    T: sqlx::Decode<'r, sqlx::Postgres> + sqlx::Type<sqlx::Postgres>,
{
    row.try_get::<T, _>(column).map_err(StoreError::backend)
}

fn integrity(what: &str) -> StoreError {
    StoreError::Integrity(what.to_owned())
}

pub fn key_bytes(key: NodeKey) -> Vec<u8> {
    key.as_bytes().to_vec()
}

pub fn key_from(bytes: Vec<u8>) -> Result<NodeKey, StoreError> {
    let array: [u8; 16] = bytes
        .try_into()
        .map_err(|_| integrity("a stored node key is not 16 bytes"))?;
    Ok(NodeKey::from_bytes(array))
}

fn hash16(bytes: Option<Vec<u8>>) -> Result<Option<[u8; 16]>, StoreError> {
    bytes
        .map(|b| {
            b.try_into()
                .map_err(|_| integrity("a stored hash is not 16 bytes"))
        })
        .transpose()
}

fn hash32(bytes: Vec<u8>, what: &str) -> Result<[u8; 32], StoreError> {
    bytes.try_into().map_err(|_| integrity(what))
}

pub fn to_i32(value: u32) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

fn to_u32(value: i32) -> u32 {
    u32::try_from(value).unwrap_or(0)
}

fn to_u16(value: i16, what: &str) -> Result<u16, StoreError> {
    u16::try_from(value).map_err(|_| integrity(what))
}

pub fn to_i16(value: u16) -> i16 {
    i16::try_from(value).unwrap_or(i16::MAX)
}

/// Decodes one `snapshots` row selected with [`META_COLUMNS`].
pub fn meta_from_row(row: &PgRow) -> Result<SnapshotMeta, StoreError> {
    let id: Uuid = get(row, "id")?;
    let org: Uuid = get(row, "organization_id")?;
    let repo: Uuid = get(row, "repository_id")?;
    let commit: String = get(row, "commit_sha")?;
    let kind: String = get(row, "kind")?;
    let base: Option<Uuid> = get(row, "base_snapshot_id")?;
    let depth: i16 = get(row, "chain_depth")?;
    let purpose: String = get(row, "purpose")?;
    let status: String = get(row, "status")?;
    let schema: i32 = get(row, "graph_schema_version")?;
    let analyzers: serde_json::Value = get(row, "analyzer_versions")?;
    let config_hash: Vec<u8> = get(row, "config_hash")?;
    let components: serde_json::Value = get(row, "config_components")?;
    let fingerprint: Vec<u8> = get(row, "fingerprint")?;
    let stats: serde_json::Value = get(row, "stats")?;
    Ok(SnapshotMeta {
        id: SnapshotId::from_uuid(id),
        scope: RepoScope {
            organization_id: OrganizationId::from_uuid(org),
            repository_id: RepositoryId::from_uuid(repo),
        },
        commit_sha: commit
            .parse::<CommitSha>()
            .map_err(|_| integrity("stored commit sha is invalid"))?,
        kind: SnapshotKind::from_db(&kind).ok_or_else(|| integrity("unknown snapshot kind"))?,
        base: base.map(SnapshotId::from_uuid),
        chain_depth: u16::try_from(depth).map_err(|_| integrity("negative chain depth"))?,
        purpose: SnapshotPurpose::from_db(&purpose)
            .ok_or_else(|| integrity("unknown snapshot purpose"))?,
        status: SnapshotStatus::from_db(&status)
            .ok_or_else(|| integrity("unknown snapshot status"))?,
        versions: SnapshotVersions {
            graph_schema_version: u32::try_from(schema)
                .map_err(|_| integrity("negative graph schema version"))?,
            analyzer_versions: serde_json::from_value::<BTreeMap<String, String>>(analyzers)
                .map_err(|_| integrity("analyzer_versions is not a string map"))?,
            config_hash: hash32(config_hash, "config_hash is not 32 bytes")?,
            config_components: serde_json::from_value::<BTreeMap<String, String>>(components)
                .map_err(|_| integrity("config_components is not a string map"))?,
            fingerprint: hash32(fingerprint, "fingerprint is not 32 bytes")?,
        },
        stats: serde_json::from_value::<SnapshotStats>(stats)
            .map_err(|_| integrity("stats do not match SnapshotStats"))?,
        error: get(row, "error")?,
        created_at: get(row, "created_at")?,
        updated_at: get(row, "updated_at")?,
        completed_at: get(row, "completed_at")?,
    })
}

/// One `snapshot_files` row.
pub fn file_from_row(row: &PgRow) -> Result<SnapshotFile, StoreError> {
    let change: String = get(row, "change")?;
    Ok(SnapshotFile {
        path: get(row, "path")?,
        file_version_id: get(row, "file_version_id")?,
        change: FileChange::from_db(&change).ok_or_else(|| integrity("unknown file change"))?,
        old_path: get(row, "old_path")?,
    })
}

/// One `graph_edges` row and its `removed` flag.
pub fn edge_from_row(row: &PgRow) -> Result<(GraphEdge, bool), StoreError> {
    let kind: i16 = get(row, "kind")?;
    let resolved_by: i16 = get(row, "resolved_by")?;
    let provenance: i16 = get(row, "provenance")?;
    let flags: i16 = get(row, "flags")?;
    let confidence: f32 = get(row, "confidence")?;
    let occurrences: i32 = get(row, "occurrences")?;
    let line: Option<i32> = get(row, "line")?;
    let col: Option<i32> = get(row, "col")?;
    let edge = GraphEdge {
        source: key_from(get(row, "source_key")?)?,
        kind: EdgeKind::from_i16(kind).ok_or_else(|| integrity("unknown edge kind"))?,
        target: key_from(get(row, "target_key")?)?,
        confidence: Confidence::from_f32(confidence),
        resolved_by: ResolvedBy::from_i16(resolved_by)
            .ok_or_else(|| integrity("unknown resolved_by"))?,
        provenance: Provenance::from_i16(provenance)
            .ok_or_else(|| integrity("unknown provenance"))?,
        flags: EdgeFlags::from_bits(u8::try_from(flags).map_err(|_| integrity("edge flags"))?),
        origin_path: get(row, "origin_path")?,
        line: line.map(to_u32),
        col: col.map(to_u32),
        occurrences: to_u32(occurrences),
    };
    Ok((edge, get(row, "removed")?))
}

/// One `synthetic_nodes` row: a node, or a tombstone key when `removed`.
#[derive(Debug)]
pub enum SyntheticRow {
    Node(Node),
    Tombstone(NodeKey),
}

pub fn synthetic_from_row(row: &PgRow) -> Result<SyntheticRow, StoreError> {
    let key = key_from(get(row, "node_key")?)?;
    let removed: bool = get(row, "removed")?;
    if removed {
        return Ok(SyntheticRow::Tombstone(key));
    }
    let id: Option<String> = get(row, "node_id")?;
    let kind: Option<i16> = get(row, "kind")?;
    let kind = kind
        .and_then(NodeKind::from_i16)
        .ok_or_else(|| integrity("a synthetic node has no valid kind"))?;
    Ok(SyntheticRow::Node(Node::Synthetic(SyntheticNode {
        key,
        id: id.ok_or_else(|| integrity("a synthetic node has no id"))?,
        kind,
        attrs: get(row, "attrs")?,
    })))
}

/// One `symbols` row joined with its file version's `path`.
pub fn symbol_from_row(row: &PgRow) -> Result<Node, StoreError> {
    let kind: i16 = get(row, "kind")?;
    let visibility: i16 = get(row, "visibility")?;
    let parent: Option<Vec<u8>> = get(row, "parent_key")?;
    Ok(Node::Symbol(SymbolNode {
        key: key_from(get(row, "symbol_key")?)?,
        id: get(row, "symbol_id")?,
        kind: NodeKind::from_i16(kind).ok_or_else(|| integrity("unknown node kind"))?,
        name: get(row, "name")?,
        qualified_name: get(row, "qualified_name")?,
        file: get(row, "path")?,
        range: SourceRange {
            start_line: to_u32(get(row, "start_line")?),
            start_col: to_u32(get(row, "start_col")?),
            end_line: to_u32(get(row, "end_line")?),
            end_col: to_u32(get(row, "end_col")?),
        },
        signature: get(row, "signature")?,
        body_hash: hash16(get(row, "body_hash")?)?,
        signature_hash: hash16(get(row, "signature_hash")?)?,
        parent: parent.map(key_from).transpose()?,
        visibility: u8::try_from(visibility).map_err(|_| integrity("visibility"))?,
        is_exported: get(row, "is_exported")?,
        is_generated: get(row, "is_generated")?,
        attrs: get(row, "attrs")?,
    }))
}

/// One `unresolved_refs` row joined with its file version's `path`.
pub fn unresolved_from_row(row: &PgRow) -> Result<UnresolvedRef, StoreError> {
    let ordinal: i32 = get(row, "ordinal")?;
    let from: Option<Vec<u8>> = get(row, "from_symbol_key")?;
    Ok(UnresolvedRef {
        file: get(row, "path")?,
        ordinal: to_u32(ordinal),
        from: from.map(key_from).transpose()?,
        name: get(row, "name")?,
        ref_kind: to_u16(get(row, "ref_kind")?, "ref_kind")?,
        import_specifier: get(row, "import_specifier")?,
        reason: to_u16(get(row, "reason")?, "reason")?,
        candidate_count: to_u16(get(row, "candidate_count")?, "candidate_count")?,
        line: to_u32(get(row, "line")?),
        col: to_u32(get(row, "col")?),
    })
}

/// One `symbol_lineage` row.
pub fn lineage_from_row(row: &PgRow) -> Result<Lineage, StoreError> {
    let transition: String = get(row, "transition")?;
    Ok(Lineage {
        from_key: key_from(get(row, "from_key")?)?,
        to_key: key_from(get(row, "to_key")?)?,
        transition: LineageTransition::from_db(&transition)
            .ok_or_else(|| integrity("unknown lineage transition"))?,
        similarity: get(row, "similarity")?,
    })
}

/// Column arrays of a batch of edge rows, for one `UNNEST` insert.
#[derive(Debug, Default)]
pub struct EdgeColumns {
    pub source: Vec<Vec<u8>>,
    pub kind: Vec<i16>,
    pub target: Vec<Vec<u8>>,
    pub confidence: Vec<f32>,
    pub resolved_by: Vec<i16>,
    pub provenance: Vec<i16>,
    pub flags: Vec<i16>,
    pub occurrences: Vec<i32>,
    pub origin_path: Vec<Option<String>>,
    pub file_version_id: Vec<Option<i64>>,
    pub line: Vec<Option<i32>>,
    pub col: Vec<Option<i32>>,
    pub removed: Vec<bool>,
}

impl EdgeColumns {
    pub fn push(&mut self, edge: &GraphEdge, removed: bool, file_version_id: Option<i64>) {
        self.source.push(key_bytes(edge.source));
        self.kind.push(edge.kind.as_i16());
        self.target.push(key_bytes(edge.target));
        self.confidence.push(edge.confidence.as_f32());
        self.resolved_by.push(edge.resolved_by.as_i16());
        self.provenance.push(edge.provenance.as_i16());
        self.flags.push(i16::from(edge.flags.bits()));
        self.occurrences.push(to_i32(edge.occurrences.max(1)));
        self.origin_path.push(edge.origin_path.clone());
        self.file_version_id.push(file_version_id);
        self.line.push(edge.line.map(to_i32));
        self.col.push(edge.col.map(to_i32));
        self.removed.push(removed);
    }

    pub fn len(&self) -> usize {
        self.source.len()
    }

    pub fn is_empty(&self) -> bool {
        self.source.is_empty()
    }
}

/// Column arrays of a batch of `symbols` rows.
#[derive(Debug, Default)]
pub struct SymbolColumns {
    pub file_version_id: Vec<i64>,
    pub key: Vec<Vec<u8>>,
    pub id: Vec<String>,
    pub kind: Vec<i16>,
    pub name: Vec<String>,
    pub qualified_name: Vec<String>,
    pub signature: Vec<Option<String>>,
    pub start_line: Vec<i32>,
    pub start_col: Vec<i32>,
    pub end_line: Vec<i32>,
    pub end_col: Vec<i32>,
    pub body_hash: Vec<Option<Vec<u8>>>,
    pub signature_hash: Vec<Option<Vec<u8>>>,
    pub parent: Vec<Option<Vec<u8>>>,
    pub visibility: Vec<i16>,
    pub is_exported: Vec<bool>,
    pub is_generated: Vec<bool>,
    pub attrs: Vec<serde_json::Value>,
}

impl SymbolColumns {
    pub fn push(&mut self, file_version_id: i64, symbol: &SymbolNode) {
        self.file_version_id.push(file_version_id);
        self.key.push(key_bytes(symbol.key));
        self.id.push(symbol.id.clone());
        self.kind.push(symbol.kind.as_i16());
        self.name.push(symbol.name.clone());
        self.qualified_name.push(symbol.qualified_name.clone());
        self.signature.push(symbol.signature.clone());
        // The schema requires 1-based positions; a default range is stored as 1:1.
        self.start_line.push(to_i32(symbol.range.start_line.max(1)));
        self.start_col.push(to_i32(symbol.range.start_col.max(1)));
        self.end_line.push(to_i32(symbol.range.end_line.max(1)));
        self.end_col.push(to_i32(symbol.range.end_col.max(1)));
        self.body_hash.push(symbol.body_hash.map(|h| h.to_vec()));
        self.signature_hash
            .push(symbol.signature_hash.map(|h| h.to_vec()));
        self.parent.push(symbol.parent.map(key_bytes));
        self.visibility.push(i16::from(symbol.visibility));
        self.is_exported.push(symbol.is_exported);
        self.is_generated.push(symbol.is_generated);
        self.attrs.push(symbol.attrs.clone());
    }

    pub fn is_empty(&self) -> bool {
        self.key.is_empty()
    }
}

/// Column arrays of a batch of `unresolved_refs` rows.
#[derive(Debug, Default)]
pub struct UnresolvedColumns {
    pub file_version_id: Vec<i64>,
    pub ordinal: Vec<i32>,
    pub from: Vec<Option<Vec<u8>>>,
    pub name: Vec<String>,
    pub ref_kind: Vec<i16>,
    pub import_specifier: Vec<Option<String>>,
    pub reason: Vec<i16>,
    pub candidate_count: Vec<i16>,
    pub line: Vec<i32>,
    pub col: Vec<i32>,
}

impl UnresolvedColumns {
    pub fn push(&mut self, file_version_id: i64, row: &UnresolvedRef) {
        self.file_version_id.push(file_version_id);
        self.ordinal.push(to_i32(row.ordinal));
        self.from.push(row.from.map(key_bytes));
        self.name.push(row.name.clone());
        self.ref_kind.push(to_i16(row.ref_kind));
        self.import_specifier.push(row.import_specifier.clone());
        self.reason.push(to_i16(row.reason));
        self.candidate_count.push(to_i16(row.candidate_count));
        self.line.push(to_i32(row.line));
        self.col.push(to_i32(row.col));
    }

    pub fn is_empty(&self) -> bool {
        self.name.is_empty()
    }
}

/// Uuid of a snapshot id, for binds.
pub fn uuid_of(id: SnapshotId) -> Uuid {
    *id.as_uuid()
}
