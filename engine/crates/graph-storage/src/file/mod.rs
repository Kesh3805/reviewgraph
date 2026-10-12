//! `FileGraphStore` (GS-006): the CLI's graph store under `.review/graph`, without Postgres.
//!
//! ```text
//! .review/graph/
//!   manifest.json             snapshots, statuses, metadata and the file-version id map
//!   snapshots/{uuid}.bin.zst  zstd-compressed payload of one snapshot (full graph or delta)
//!   .lock                     advisory lock serializing every operation across processes
//! ```
//!
//! * Every write is temp-file + `sync_all` + rename ([`atomic`]); the payload file is durable
//!   before the manifest that points at it is replaced, so a crash leaves either the old manifest
//!   or the new one, never a manifest naming a missing file.
//! * Every operation holds the exclusive lock and, within a process, a mutex; file I/O runs on
//!   the blocking pool.
//! * A payload's size and BLAKE3 digest are checked against the manifest before decoding; a
//!   mismatch marks the snapshot `inconsistent` and is [`StoreError::Integrity`].
//! * Semantics are the in-memory oracle's: chains materialize through [`flatten`].

pub mod atomic;
pub mod lock;
pub mod manifest;

use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use repository::store::RepoScope;
use review_core::ids::SnapshotId;
use serde::{Deserialize, Serialize};
use tracing::Instrument;
use uuid::Uuid;

use self::atomic::{clean_temp_files, write_atomic};
use self::lock::{StoreLock, LOCK_TIMEOUT};
use self::manifest::{digest, FileVersionEntry, Manifest, SnapshotEntry, MANIFEST_FORMAT};
use crate::kinds::{Confidence, Direction, EdgeKindSet, NodeKey};
use crate::model::{flatten, EdgeCursor, Graph, GraphDelta, GraphEdge};
use crate::port::GraphStore;
use crate::status::{can_transition, SnapshotStatus};
use crate::types::{
    FileVersionInput, FileVersionKey, FileVersionRef, NeighborPage, NewSnapshot, SnapshotKind,
    SnapshotMeta, SnapshotQuery, SnapshotStats, StoredNode, WriteStats, GRAPH_SCHEMA_VERSION,
};
use crate::StoreError;

/// Largest decompressed payload accepted (local files are untrusted input).
pub const MAX_PAYLOAD_BYTES: u64 = 8 * 1024 * 1024 * 1024;
/// zstd level of payload files.
const ZSTD_LEVEL: i32 = 3;
/// Maximum rows one `neighbors` page may carry, matching `Graph::neighbors`.
const MAX_NEIGHBOR_LIMIT: u32 = 1_000;
/// Guard against a self-referential chain while walking `base`.
const MAX_CHAIN_DEPTH: usize = 10_000;

const MANIFEST_FILE: &str = "manifest.json";
const SNAPSHOT_DIR: &str = "snapshots";
const LOCK_FILE: &str = ".lock";

/// The payload stored in one snapshot file.
#[derive(Debug, Serialize, Deserialize)]
enum Payload {
    Full(Graph),
    Delta(GraphDelta),
}

/// The file-backed [`GraphStore`].
#[derive(Debug)]
pub struct FileGraphStore {
    root: Arc<PathBuf>,
    guard: tokio::sync::Mutex<()>,
}

fn io(error: std::io::Error) -> StoreError {
    StoreError::backend(error)
}

fn integrity(message: impl Into<String>) -> StoreError {
    StoreError::Integrity(message.into())
}

fn invalid_status(id: SnapshotId, expected: SnapshotStatus, found: SnapshotStatus) -> StoreError {
    StoreError::InvalidStatus {
        id,
        expected,
        found,
    }
}

impl FileGraphStore {
    /// Opens (creating if needed) the store rooted at `root` (normally `.review/graph`) and removes
    /// temp files left by interrupted writes.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, StoreError> {
        let root: PathBuf = root.into();
        std::fs::create_dir_all(root.join(SNAPSHOT_DIR)).map_err(io)?;
        clean_temp_files(&root);
        clean_temp_files(&root.join(SNAPSHOT_DIR));
        Ok(Self {
            root: Arc::new(root),
            guard: tokio::sync::Mutex::new(()),
        })
    }

    /// The directory this store writes to.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Runs `op` under the in-process mutex and the cross-process lock, on the blocking pool.
    /// The manifest is written back when `op` set the dirty flag, even if `op` then failed (an
    /// integrity failure marks a snapshot inconsistent and still reports the error).
    async fn run<T, F>(&self, op: F) -> Result<T, StoreError>
    where
        T: Send + 'static,
        F: FnOnce(&Path, &mut Manifest, &mut bool) -> Result<T, StoreError> + Send + 'static,
    {
        let _guard = self.guard.lock().await;
        let root = Arc::clone(&self.root);
        tokio::task::spawn_blocking(move || {
            let _lock = StoreLock::acquire(&root.join(LOCK_FILE), LOCK_TIMEOUT)?;
            let mut manifest = read_manifest(&root)?;
            let mut dirty = false;
            let result = op(&root, &mut manifest, &mut dirty);
            if dirty {
                write_manifest(&root, &manifest)?;
            }
            result
        })
        .await
        .map_err(|e| StoreError::backend_msg(format!("blocking task failed: {e}")))?
    }
}

fn read_manifest(root: &Path) -> Result<Manifest, StoreError> {
    let path = root.join(MANIFEST_FILE);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Manifest::default()),
        Err(e) => return Err(io(e)),
    };
    let manifest: Manifest = serde_json::from_slice(&bytes).map_err(|e| {
        integrity(format!(
            "graph manifest is corrupt ({e}); run `review graph rebuild`"
        ))
    })?;
    if manifest.format != MANIFEST_FORMAT {
        return Err(integrity(format!(
            "graph manifest format {} is not supported (expected {MANIFEST_FORMAT}); run `review graph rebuild`",
            manifest.format
        )));
    }
    Ok(manifest)
}

fn write_manifest(root: &Path, manifest: &Manifest) -> Result<(), StoreError> {
    let bytes = serde_json::to_vec_pretty(manifest)
        .map_err(|e| integrity(format!("manifest does not serialize: {e}")))?;
    write_atomic(&root.join(MANIFEST_FILE), &bytes).map_err(io)
}

fn snapshot_file(id: Uuid) -> String {
    format!("{SNAPSHOT_DIR}/{}.bin.zst", id.simple())
}

fn entry<'m>(
    manifest: &'m Manifest,
    scope: &RepoScope,
    id: SnapshotId,
) -> Result<&'m SnapshotEntry, StoreError> {
    manifest
        .snapshots
        .iter()
        .find(|e| e.id == *id.as_uuid() && e.in_scope(scope))
        .ok_or(StoreError::NotFound(id))
}

fn entry_mut<'m>(
    manifest: &'m mut Manifest,
    scope: &RepoScope,
    id: SnapshotId,
) -> Result<&'m mut SnapshotEntry, StoreError> {
    manifest
        .snapshots
        .iter_mut()
        .find(|e| e.id == *id.as_uuid() && e.in_scope(scope))
        .ok_or(StoreError::NotFound(id))
}

/// Encodes and durably writes a payload; returns `(file, bytes, digest)`.
fn store_payload(root: &Path, id: Uuid, payload: &Payload) -> Result<(String, u64, String), StoreError> {
    let json =
        serde_json::to_vec(payload).map_err(|e| integrity(format!("payload does not encode: {e}")))?;
    let compressed = zstd::encode_all(json.as_slice(), ZSTD_LEVEL).map_err(io)?;
    let file = snapshot_file(id);
    write_atomic(&root.join(&file), &compressed).map_err(io)?;
    Ok((file, compressed.len() as u64, digest(&compressed)))
}

/// Reads and verifies a snapshot's payload; a mismatch marks the snapshot inconsistent.
fn load_payload(
    root: &Path,
    manifest: &mut Manifest,
    scope: &RepoScope,
    id: SnapshotId,
    dirty: &mut bool,
) -> Result<Option<Payload>, StoreError> {
    let current = entry(manifest, scope, id)?;
    let Some(file) = current.file.clone() else {
        return Ok(None);
    };
    let expected_bytes = current.bytes;
    let expected_digest = current.blake3.clone();
    let bytes = std::fs::read(root.join(&file));
    let verified = match &bytes {
        Ok(bytes) => {
            bytes.len() as u64 == expected_bytes
                && expected_digest.as_deref() == Some(digest(bytes).as_str())
        }
        Err(_) => false,
    };
    if !verified {
        let broken = entry_mut(manifest, scope, id)?;
        broken.status = SnapshotStatus::Inconsistent;
        broken.updated_at = Utc::now();
        *dirty = true;
        return Err(integrity(format!(
            "snapshot {id} payload is missing or does not match the manifest"
        )));
    }
    let bytes = bytes.map_err(io)?;
    let mut json = Vec::new();
    zstd::stream::read::Decoder::new(bytes.as_slice())
        .map_err(io)?
        .take(MAX_PAYLOAD_BYTES + 1)
        .read_to_end(&mut json)
        .map_err(io)?;
    if json.len() as u64 > MAX_PAYLOAD_BYTES {
        return Err(integrity(format!("snapshot {id} payload exceeds the size limit")));
    }
    serde_json::from_slice(&json)
        .map(Some)
        .map_err(|e| integrity(format!("snapshot {id} payload does not decode: {e}")))
}

/// `[root, …, id]`, oldest first, every entry `Ready` and the root a full snapshot.
fn chain_ids(manifest: &Manifest, scope: &RepoScope, id: SnapshotId) -> Result<Vec<SnapshotId>, StoreError> {
    let mut ids = Vec::new();
    let mut seen = BTreeSet::new();
    let mut cursor = id;
    loop {
        if !seen.insert(cursor) || ids.len() > MAX_CHAIN_DEPTH {
            return Err(StoreError::ChainBroken { id });
        }
        let current = entry(manifest, scope, cursor)?;
        if !current.status.is_readable() {
            return Err(invalid_status(cursor, SnapshotStatus::Ready, current.status));
        }
        ids.push(cursor);
        match current.base {
            Some(base) => cursor = SnapshotId::from_uuid(base),
            None => break,
        }
    }
    ids.reverse();
    let Some(&root) = ids.first() else {
        return Err(StoreError::ChainBroken { id });
    };
    if entry(manifest, scope, root)?.kind != SnapshotKind::Full {
        return Err(StoreError::ChainBroken { id });
    }
    Ok(ids)
}

/// `base ⊕ … ⊕ id`, checked the way the in-memory oracle checks it.
fn materialize(
    root: &Path,
    manifest: &mut Manifest,
    scope: &RepoScope,
    id: SnapshotId,
    dirty: &mut bool,
) -> Result<Graph, StoreError> {
    let current = entry(manifest, scope, id)?;
    if !current.status.is_readable() {
        return Err(invalid_status(id, SnapshotStatus::Ready, current.status));
    }
    if current.versions.graph_schema_version != GRAPH_SCHEMA_VERSION {
        return Err(StoreError::SchemaMismatch {
            found: current.versions.graph_schema_version,
            expected: GRAPH_SCHEMA_VERSION,
        });
    }
    let ids = chain_ids(manifest, scope, id)?;
    let Some((&root_id, rest)) = ids.split_first() else {
        return Err(StoreError::ChainBroken { id });
    };
    let base = match load_payload(root, manifest, scope, root_id, dirty)? {
        Some(Payload::Full(g)) => g,
        Some(Payload::Delta(_)) => return Err(StoreError::ChainBroken { id: root_id }),
        None => Graph::new(entry(manifest, scope, root_id)?.versions.graph_schema_version),
    };
    let mut deltas = Vec::with_capacity(rest.len());
    for linked in rest {
        deltas.push(match load_payload(root, manifest, scope, *linked, dirty)? {
            Some(Payload::Delta(d)) => d,
            Some(Payload::Full(_)) => return Err(StoreError::ChainBroken { id: *linked }),
            None => GraphDelta::default(),
        });
    }
    Ok(flatten(&base, &deltas))
}

/// Checks that `id` accepts a payload of `kind`; returns nothing on success.
fn check_writable(
    manifest: &Manifest,
    scope: &RepoScope,
    id: SnapshotId,
    kind: SnapshotKind,
) -> Result<(), StoreError> {
    let current = entry(manifest, scope, id)?;
    if !current.status.is_writable() {
        return Err(invalid_status(id, SnapshotStatus::Persisting, current.status));
    }
    if current.kind != kind {
        return Err(StoreError::InvalidRequest(match kind {
            SnapshotKind::Full => "a delta snapshot cannot be written as a full graph".to_owned(),
            SnapshotKind::Delta => "a full snapshot cannot be written as a delta".to_owned(),
        }));
    }
    Ok(())
}

fn record_payload(
    manifest: &mut Manifest,
    scope: &RepoScope,
    id: SnapshotId,
    stored: (String, u64, String),
    stats: SnapshotStats,
) -> Result<(), StoreError> {
    let current = entry_mut(manifest, scope, id)?;
    let (file, bytes, hash) = stored;
    current.file = Some(file);
    current.bytes = bytes;
    current.blake3 = Some(hash);
    current.stats = stats;
    current.updated_at = Utc::now();
    Ok(())
}

#[async_trait]
impl GraphStore for FileGraphStore {
    async fn create_snapshot(&self, req: NewSnapshot) -> Result<SnapshotMeta, StoreError> {
        req.validate()?;
        self.run(move |_root, manifest, dirty| {
            let chain_depth = match req.base {
                Some(base) => entry(manifest, &req.scope, base)?
                    .chain_depth
                    .saturating_add(1),
                None => 0,
            };
            manifest.seq += 1;
            let now = Utc::now();
            let created = SnapshotEntry {
                id: SnapshotId::new().into_uuid(),
                organization_id: *req.scope.organization_id.as_uuid(),
                repository_id: *req.scope.repository_id.as_uuid(),
                commit_sha: req.commit_sha.as_str().to_owned(),
                kind: req.kind,
                base: req.base.map(SnapshotId::into_uuid),
                chain_depth,
                purpose: req.purpose,
                status: SnapshotStatus::Pending,
                versions: req.versions,
                stats: SnapshotStats::default(),
                error: None,
                created_at: now,
                updated_at: now,
                completed_at: None,
                seq: manifest.seq,
                file: None,
                bytes: 0,
                blake3: None,
            };
            let meta = created.meta()?;
            manifest.snapshots.push(created);
            *dirty = true;
            Ok(meta)
        })
        .await
    }

    async fn transition(
        &self,
        scope: &RepoScope,
        id: SnapshotId,
        from: SnapshotStatus,
        to: SnapshotStatus,
        error: Option<&str>,
    ) -> Result<bool, StoreError> {
        let scope = *scope;
        let error = error.map(str::to_owned);
        self.run(move |_root, manifest, dirty| {
            let current = entry_mut(manifest, &scope, id)?;
            if current.status != from || !can_transition(from, to) {
                return Ok(false);
            }
            current.status = to;
            current.updated_at = Utc::now();
            if matches!(to, SnapshotStatus::Ready | SnapshotStatus::Failed) {
                current.completed_at = Some(current.updated_at);
            }
            if to == SnapshotStatus::Failed {
                current.error = error;
            }
            *dirty = true;
            Ok(true)
        })
        .await
    }

    async fn upsert_file_versions(
        &self,
        scope: &RepoScope,
        files: &[FileVersionInput],
    ) -> Result<Vec<FileVersionRef>, StoreError> {
        let scope = *scope;
        let keys: Vec<FileVersionKey> = files.iter().map(FileVersionInput::key).collect();
        self.run(move |_root, manifest, dirty| {
            let org = *scope.organization_id.as_uuid();
            let repo = *scope.repository_id.as_uuid();
            let mut out = Vec::with_capacity(keys.len());
            for key in keys {
                let text = key.manifest_key();
                let known = manifest
                    .file_versions
                    .iter()
                    .find(|f| f.organization_id == org && f.repository_id == repo && f.key == text)
                    .map(|f| f.id);
                let id = match known {
                    Some(id) => id,
                    None => {
                        manifest.next_file_version += 1;
                        let id = manifest.next_file_version;
                        manifest.file_versions.push(FileVersionEntry {
                            organization_id: org,
                            repository_id: repo,
                            key: text,
                            id,
                        });
                        *dirty = true;
                        id
                    }
                };
                out.push(FileVersionRef {
                    id,
                    path: key.path,
                    content_hash: key.content_hash,
                    analyzer_version: key.analyzer_version,
                });
            }
            Ok(out)
        })
        .await
    }

    async fn lookup_file_versions(
        &self,
        scope: &RepoScope,
        keys: &[FileVersionKey],
    ) -> Result<Vec<Option<FileVersionRef>>, StoreError> {
        let scope = *scope;
        let keys = keys.to_vec();
        self.run(move |_root, manifest, _dirty| {
            let org = *scope.organization_id.as_uuid();
            let repo = *scope.repository_id.as_uuid();
            Ok(keys
                .into_iter()
                .map(|key| {
                    let text = key.manifest_key();
                    manifest
                        .file_versions
                        .iter()
                        .find(|f| f.organization_id == org && f.repository_id == repo && f.key == text)
                        .map(|f| FileVersionRef {
                            id: f.id,
                            path: key.path.clone(),
                            content_hash: key.content_hash,
                            analyzer_version: key.analyzer_version.clone(),
                        })
                })
                .collect())
        })
        .await
    }

    async fn write_full(
        &self,
        scope: &RepoScope,
        id: SnapshotId,
        g: &Graph,
    ) -> Result<WriteStats, StoreError> {
        let scope = *scope;
        let g = g.clone().normalized();
        let span = tracing::info_span!("graph_store.write_full", adapter = "file");
        self.run(move |root, manifest, dirty| {
            check_writable(manifest, &scope, id, SnapshotKind::Full)?;
            let expected = entry(manifest, &scope, id)?.versions.graph_schema_version;
            if g.schema_version != expected {
                return Err(StoreError::SchemaMismatch {
                    found: g.schema_version,
                    expected,
                });
            }
            let written = WriteStats {
                files: g.files.len() as u64,
                nodes: g.nodes.len() as u64,
                edges: g.edges.len() as u64,
                unresolved: g.unresolved.len() as u64,
                lineage: 0,
                file_versions_existing: 0,
                bytes: 0,
            };
            let stats = SnapshotStats::from_graph(&g);
            let stored = store_payload(root, *id.as_uuid(), &Payload::Full(g))?;
            let bytes = stored.1;
            record_payload(manifest, &scope, id, stored, stats)?;
            *dirty = true;
            Ok(WriteStats { bytes, ..written })
        })
        .instrument(span)
        .await
    }

    async fn write_delta(
        &self,
        scope: &RepoScope,
        id: SnapshotId,
        d: &GraphDelta,
    ) -> Result<WriteStats, StoreError> {
        let scope = *scope;
        let d = d.clone();
        let span = tracing::info_span!("graph_store.write_delta", adapter = "file");
        self.run(move |root, manifest, dirty| {
            check_writable(manifest, &scope, id, SnapshotKind::Delta)?;
            let base = entry(manifest, &scope, id)?
                .base
                .map(SnapshotId::from_uuid)
                .ok_or(StoreError::ChainBroken { id })?;
            let base_status = entry(manifest, &scope, base)?.status;
            if !base_status.is_readable() {
                return Err(invalid_status(base, SnapshotStatus::Ready, base_status));
            }
            let written = WriteStats {
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
            };
            let stats = SnapshotStats::from_delta(&d);
            let stored = store_payload(root, *id.as_uuid(), &Payload::Delta(d))?;
            let bytes = stored.1;
            record_payload(manifest, &scope, id, stored, stats)?;
            *dirty = true;
            Ok(WriteStats { bytes, ..written })
        })
        .instrument(span)
        .await
    }

    async fn load_graph(&self, scope: &RepoScope, id: SnapshotId) -> Result<Graph, StoreError> {
        let scope = *scope;
        let span = tracing::info_span!("graph_store.load_graph", adapter = "file");
        self.run(move |root, manifest, dirty| materialize(root, manifest, &scope, id, dirty))
            .instrument(span)
            .await
    }

    async fn load_delta(
        &self,
        scope: &RepoScope,
        id: SnapshotId,
    ) -> Result<GraphDelta, StoreError> {
        let scope = *scope;
        self.run(move |root, manifest, dirty| {
            let current = entry(manifest, &scope, id)?;
            if !current.status.is_readable() {
                return Err(invalid_status(id, SnapshotStatus::Ready, current.status));
            }
            match load_payload(root, manifest, &scope, id, dirty)? {
                Some(Payload::Delta(d)) => Ok(d),
                Some(Payload::Full(_)) => Err(StoreError::InvalidRequest(
                    "a full snapshot has no delta rows".to_owned(),
                )),
                None => Ok(GraphDelta::default()),
            }
        })
        .await
    }

    async fn snapshot(
        &self,
        scope: &RepoScope,
        id: SnapshotId,
    ) -> Result<Option<SnapshotMeta>, StoreError> {
        let scope = *scope;
        self.run(move |_root, manifest, _dirty| {
            manifest
                .snapshots
                .iter()
                .find(|e| e.id == *id.as_uuid() && e.in_scope(&scope))
                .map(SnapshotEntry::meta)
                .transpose()
        })
        .await
    }

    async fn find_ready(
        &self,
        scope: &RepoScope,
        q: SnapshotQuery,
    ) -> Result<Option<SnapshotMeta>, StoreError> {
        let scope = *scope;
        self.run(move |_root, manifest, _dirty| {
            manifest
                .snapshots
                .iter()
                .filter(|e| {
                    e.in_scope(&scope)
                        && e.status.is_readable()
                        && q.commit_sha.as_ref().is_none_or(|c| e.commit_sha == c.as_str())
                        && q.fingerprint.is_none_or(|f| e.versions.fingerprint == f)
                        && q.kind.is_none_or(|k| e.kind == k)
                        && q.purpose.is_none_or(|p| e.purpose == p)
                })
                .max_by_key(|e| (e.created_at, e.seq, e.id))
                .map(SnapshotEntry::meta)
                .transpose()
        })
        .await
    }

    async fn chain(
        &self,
        scope: &RepoScope,
        id: SnapshotId,
    ) -> Result<Vec<SnapshotMeta>, StoreError> {
        let scope = *scope;
        self.run(move |_root, manifest, _dirty| {
            let ids = chain_ids(manifest, &scope, id)?;
            ids.iter()
                .map(|linked| entry(manifest, &scope, *linked).and_then(SnapshotEntry::meta))
                .collect()
        })
        .await
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
        let graph = self.load_graph(scope, id).await?;
        let take = limit.clamp(1, MAX_NEIGHBOR_LIMIT) as usize;
        let other = |e: &GraphEdge| match dir {
            Direction::Out => e.target,
            Direction::In => e.source,
        };
        let mut matched: Vec<&GraphEdge> = graph
            .edges
            .iter()
            .filter(|e| {
                let endpoint = match dir {
                    Direction::Out => e.source,
                    Direction::In => e.target,
                };
                endpoint == key
                    && kinds.contains(e.kind)
                    && e.confidence >= min_confidence
                    && cursor.is_none_or(|c| (e.kind, other(e)) > (c.kind, c.key))
            })
            .collect();
        matched.sort_by_key(|e| (e.kind, other(e)));
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
        Ok(NeighborPage {
            edges: matched.into_iter().cloned().collect(),
            next_cursor,
        })
    }

    async fn nodes(
        &self,
        scope: &RepoScope,
        id: SnapshotId,
        keys: &[NodeKey],
    ) -> Result<Vec<Option<StoredNode>>, StoreError> {
        let graph = self.load_graph(scope, id).await?;
        Ok(keys.iter().map(|k| graph.node(*k).cloned()).collect())
    }

    async fn set_stats(
        &self,
        scope: &RepoScope,
        id: SnapshotId,
        stats: SnapshotStats,
    ) -> Result<(), StoreError> {
        let scope = *scope;
        self.run(move |_root, manifest, dirty| {
            let current = entry_mut(manifest, &scope, id)?;
            if !current.status.is_writable() {
                return Err(invalid_status(id, SnapshotStatus::Persisting, current.status));
            }
            current.stats = stats;
            current.updated_at = Utc::now();
            *dirty = true;
            Ok(())
        })
        .await
    }
}
