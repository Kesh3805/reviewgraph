//! The file adapter (GS-006): a [`GraphStore`] backed by a directory, for the local CLI where no
//! Postgres exists (ADR-014).
//!
//! ```text
//! .review/graph/
//!   manifest.json             source of truth: snapshot rows, heads, file versions
//!   snapshots/{uuid}.bin.zst  one container per snapshot (a full graph or one delta)
//!   .lock                     advisory lock file
//! ```
//!
//! * **Atomic writes.** A snapshot file is written as temp, sync, rename. The manifest is
//!   rewritten the same way only after the snapshot file is durable, so a crash leaves the old
//!   manifest or the new one, never a manifest that names a missing file. A file written before a
//!   crash but never committed is overwritten by the retry.
//! * **Locking.** Mutations hold an exclusive lock on `.lock`, reads a shared one, acquired with a
//!   30 s timeout (`StoreError::Conflict("graph store locked")`). Every operation re-reads the
//!   manifest under the lock, so two processes on one repository serialize. Inside a process a
//!   mutex orders operations first, so two handles never wait on each other's lock.
//! * **Integrity.** A snapshot file is checked against the manifest (`bytes` and `blake3`) before
//!   it is decoded; a mismatch is `StoreError::Integrity`, counted in
//!   `graph_store_file_integrity_errors_total`, and moves the snapshot to `Inconsistent`.
//! * **Retention.** When a snapshot becomes `Ready` the newest [`FileStoreOptions::retention`]
//!   ready snapshots are kept together with every head and the chains they depend on; other ready
//!   snapshots are dropped from the manifest and their files deleted (best effort).
//!
//! Blocking file I/O runs in `spawn_blocking`. File names are generated from snapshot ids, never
//! from caller input, so no path can escape the store. The on-disk format is documented in
//! `docs/graph-schema/storage.md`.

mod atomic;
mod container;
mod lock;
mod manifest;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use repository::store::RepoScope;
use review_core::ids::SnapshotId;
use tracing::Instrument;

use self::container::Payload;
use self::lock::StoreLock;
use self::manifest::{scope_key, Manifest, SnapshotEntry, LOCK_NAME, SNAPSHOT_DIR};
use crate::kinds::{Confidence, Direction, EdgeKindSet, NodeKey};
use crate::model::{flatten, EdgeCursor, Graph, GraphDelta, GraphEdge};
use crate::port::{spans, GraphStore};
use crate::status::{can_transition, SnapshotStatus};
use crate::types::{
    hex_lower, FileVersionInput, FileVersionKey, FileVersionRef, NeighborPage, NewSnapshot,
    SnapshotKind, SnapshotMeta, SnapshotQuery, SnapshotStats, StoredNode, WriteStats,
    GRAPH_SCHEMA_VERSION,
};
use crate::StoreError;

/// Maximum rows one `neighbors` page may carry, matching `Graph::neighbors`.
const MAX_NEIGHBOR_LIMIT: u32 = 1000;

/// How long a mutation waits for the store lock.
pub const DEFAULT_LOCK_TIMEOUT: Duration = Duration::from_secs(30);

/// How many `Ready` snapshots retention keeps besides heads and their chains.
pub const DEFAULT_RETENTION: usize = 10;

/// Tunables of a [`FileGraphStore`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStoreOptions {
    /// How long to wait for the lock before failing with `Conflict`.
    pub lock_timeout: Duration,
    /// The newest `Ready` snapshots to keep (heads and their chains are kept regardless).
    pub retention: usize,
}

impl Default for FileStoreOptions {
    fn default() -> Self {
        Self {
            lock_timeout: DEFAULT_LOCK_TIMEOUT,
            retention: DEFAULT_RETENTION,
        }
    }
}

/// A [`GraphStore`] over a directory. Cheap to clone-by-reference: all state lives behind an
/// `Arc`, and the directory is the source of truth, so several handles (and processes) agree.
#[derive(Debug)]
pub struct FileGraphStore {
    core: Arc<Core>,
}

#[derive(Debug)]
struct Core {
    root: PathBuf,
    options: FileStoreOptions,
    /// Orders operations inside this process.
    process_lock: Mutex<()>,
    /// The last materialized graph, keyed by the `(id, blake3)` of its whole chain.
    cache: Mutex<Option<CacheEntry>>,
    /// Test hook: fail the next manifest commit after the snapshot file is durable.
    fail_next_manifest_commit: AtomicBool,
}

#[derive(Debug)]
struct CacheEntry {
    key: Vec<(SnapshotId, Option<String>)>,
    graph: Arc<Graph>,
}

/// What a mutation decided.
struct Outcome<T> {
    value: T,
    /// The manifest changed and must be committed.
    dirty: bool,
    /// Snapshot files to delete after the commit.
    obsolete: Vec<String>,
}

impl<T> Outcome<T> {
    fn changed(value: T) -> Self {
        Self {
            value,
            dirty: true,
            obsolete: Vec::new(),
        }
    }

    fn unchanged(value: T) -> Self {
        Self {
            value,
            dirty: false,
            obsolete: Vec::new(),
        }
    }
}

impl FileGraphStore {
    /// Opens (creating if needed) the store in `root` with default options.
    pub async fn open(root: impl Into<PathBuf>) -> Result<Self, StoreError> {
        Self::open_with(root, FileStoreOptions::default()).await
    }

    /// Opens the store in `root`, removing temp files a crashed writer left behind.
    pub async fn open_with(
        root: impl Into<PathBuf>,
        options: FileStoreOptions,
    ) -> Result<Self, StoreError> {
        let root = root.into();
        let core = Arc::new(Core {
            root,
            options,
            process_lock: Mutex::new(()),
            cache: Mutex::new(None),
            fail_next_manifest_commit: AtomicBool::new(false),
        });
        let opener = Arc::clone(&core);
        run_blocking(move || opener.prepare()).await?;
        Ok(Self { core })
    }

    /// The directory this store lives in.
    pub fn root(&self) -> &std::path::Path {
        &self.core.root
    }

    /// Makes the next manifest commit fail once, after its snapshot file is already durable, to
    /// exercise crash recovery. Not part of the public contract.
    #[doc(hidden)]
    pub fn fail_next_manifest_commit(&self) {
        self.core
            .fail_next_manifest_commit
            .store(true, Ordering::SeqCst);
    }
}

async fn run_blocking<T, F>(f: F) -> Result<T, StoreError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, StoreError> + Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(result) => result,
        Err(join) => Err(StoreError::backend(join)),
    }
}

fn guard<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // A panic while holding the guard cannot leave the data half-updated in a way that matters
    // here: the manifest is always re-read from disk.
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Core {
    fn snapshot_dir(&self) -> PathBuf {
        self.root.join(SNAPSHOT_DIR)
    }

    /// Creates the directories and removes temp files a crashed writer left behind.
    fn prepare(&self) -> Result<(), StoreError> {
        std::fs::create_dir_all(self.snapshot_dir()).map_err(StoreError::backend)?;
        let _order = guard(&self.process_lock);
        // Temp files only exist while the writer holds the exclusive lock, so holding it here
        // means none of them is live.
        let _lock =
            StoreLock::acquire(&self.root.join(LOCK_NAME), true, self.options.lock_timeout)?;
        for dir in [self.root.clone(), self.snapshot_dir()] {
            let removed = atomic::remove_temp_files(&dir).map_err(StoreError::backend)?;
            if removed > 0 {
                tracing::info!(adapter = "file", removed, "removed leftover temp files");
            }
        }
        Ok(())
    }

    fn read<T>(&self, f: impl FnOnce(&Manifest) -> Result<T, StoreError>) -> Result<T, StoreError> {
        let _order = guard(&self.process_lock);
        let _lock =
            StoreLock::acquire(&self.root.join(LOCK_NAME), false, self.options.lock_timeout)?;
        let manifest = manifest::read(&self.root)?;
        f(&manifest)
    }

    fn update<T>(
        &self,
        f: impl FnOnce(&mut Manifest, &Core) -> Result<Outcome<T>, StoreError>,
    ) -> Result<T, StoreError> {
        let _order = guard(&self.process_lock);
        let _lock =
            StoreLock::acquire(&self.root.join(LOCK_NAME), true, self.options.lock_timeout)?;
        let mut manifest = manifest::read(&self.root)?;
        let outcome = f(&mut manifest, self)?;
        if outcome.dirty {
            if self.fail_next_manifest_commit.swap(false, Ordering::SeqCst) {
                return Err(StoreError::backend_msg(
                    "injected failure before the manifest commit",
                ));
            }
            manifest::write(&self.root, &manifest)?;
        }
        for name in outcome.obsolete {
            let _ = std::fs::remove_file(self.snapshot_dir().join(name));
        }
        Ok(outcome.value)
    }

    fn write_snapshot_file(&self, name: &str, bytes: &[u8]) -> Result<(), StoreError> {
        atomic::write_atomic(&self.snapshot_dir(), name, bytes).map_err(StoreError::backend)
    }

    /// Reads and verifies one snapshot file against its manifest entry.
    fn read_payload(
        &self,
        entry: &SnapshotEntry,
        corrupt: &mut Option<SnapshotId>,
    ) -> Result<Payload, StoreError> {
        let Some(name) = entry.file.as_deref() else {
            // Nothing was written yet: an empty payload, exactly as an adapter with no rows.
            return Ok(match entry.kind {
                SnapshotKind::Full => Payload::Full(Graph {
                    schema_version: entry.versions.graph_schema_version,
                    ..Graph::default()
                }),
                SnapshotKind::Delta => Payload::Delta(GraphDelta::default()),
            });
        };
        let result = self.read_verified(entry, name);
        if matches!(result, Err(StoreError::Integrity(_))) {
            *corrupt = Some(entry.id);
        }
        result
    }

    fn read_verified(&self, entry: &SnapshotEntry, name: &str) -> Result<Payload, StoreError> {
        let bytes = match std::fs::read(self.snapshot_dir().join(name)) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(container::integrity(format!(
                    "snapshot file {name} is missing"
                )));
            }
            Err(error) => return Err(StoreError::backend(error)),
        };
        if bytes.len() as u64 != entry.bytes {
            return Err(container::integrity(format!(
                "snapshot file {name} has {} bytes, the manifest says {}",
                bytes.len(),
                entry.bytes
            )));
        }
        let actual = hex_lower(blake3::hash(&bytes).as_bytes());
        if entry.blake3.as_deref() != Some(actual.as_str()) {
            return Err(container::integrity(format!(
                "snapshot file {name} does not match the manifest hash"
            )));
        }
        container::decode(&bytes)
    }

    /// `base + ... + id` (ADR-003), through the one-entry cache.
    fn materialize(
        &self,
        manifest: &Manifest,
        scope: &RepoScope,
        id: SnapshotId,
        corrupt: &mut Option<SnapshotId>,
    ) -> Result<Arc<Graph>, StoreError> {
        let leaf = manifest.entry(scope, id)?;
        if !leaf.status.is_readable() {
            return Err(StoreError::InvalidStatus {
                id,
                expected: SnapshotStatus::Ready,
                found: leaf.status,
            });
        }
        if leaf.versions.graph_schema_version != GRAPH_SCHEMA_VERSION {
            return Err(StoreError::SchemaMismatch {
                found: leaf.versions.graph_schema_version,
                expected: GRAPH_SCHEMA_VERSION,
            });
        }
        let ids = manifest.chain_ids(scope, id)?;
        let entries: Vec<&SnapshotEntry> = ids
            .iter()
            .map(|linked| manifest.entry(scope, *linked))
            .collect::<Result<_, _>>()?;
        let key: Vec<(SnapshotId, Option<String>)> = entries
            .iter()
            .map(|entry| (entry.id, entry.blake3.clone()))
            .collect();
        if let Some(cached) = guard(&self.cache).as_ref().filter(|c| c.key == key) {
            return Ok(Arc::clone(&cached.graph));
        }

        let Some((root, linked)) = entries.split_first() else {
            return Err(StoreError::ChainBroken { id });
        };
        let base = match self.read_payload(root, corrupt)? {
            Payload::Full(graph) => graph,
            Payload::Delta(_) => return Err(StoreError::ChainBroken { id: root.id }),
        };
        let mut deltas = Vec::with_capacity(linked.len());
        for entry in linked {
            match self.read_payload(entry, corrupt)? {
                Payload::Delta(delta) => deltas.push(delta),
                Payload::Full(_) => return Err(StoreError::ChainBroken { id: entry.id }),
            }
        }
        let graph = Arc::new(flatten(&base, &deltas));
        *guard(&self.cache) = Some(CacheEntry {
            key,
            graph: Arc::clone(&graph),
        });
        Ok(graph)
    }

    /// Materializes `id` and, when a snapshot file failed its integrity check, records the
    /// snapshot as `Inconsistent` before reporting the error.
    fn load(&self, scope: &RepoScope, id: SnapshotId) -> Result<Arc<Graph>, StoreError> {
        let mut corrupt = None;
        let result = self.read(|manifest| self.materialize(manifest, scope, id, &mut corrupt));
        if let Some(bad) = corrupt {
            self.mark_inconsistent(bad);
        }
        result
    }

    fn mark_inconsistent(&self, id: SnapshotId) {
        let marked = self.update(|manifest, _| {
            let Some(entry) = manifest.snapshots.iter_mut().find(|e| e.id == id) else {
                return Ok(Outcome::unchanged(()));
            };
            if entry.status != SnapshotStatus::Ready {
                return Ok(Outcome::unchanged(()));
            }
            entry.status = SnapshotStatus::Inconsistent;
            entry.updated_at = Utc::now();
            Ok(Outcome::changed(()))
        });
        if let Err(error) = marked {
            tracing::warn!(adapter = "file", snapshot = %id, %error, "could not mark snapshot inconsistent");
        }
    }

    fn create_snapshot(&self, req: NewSnapshot) -> Result<SnapshotMeta, StoreError> {
        req.validate()?;
        self.update(|manifest, _| {
            let (chain_depth, base) = match req.kind {
                SnapshotKind::Full => (0, None),
                SnapshotKind::Delta => {
                    let base_id = req.base.ok_or_else(|| {
                        StoreError::InvalidRequest("a delta snapshot must name its base".to_owned())
                    })?;
                    // The base must exist in this scope; `write_delta` checks it is `Ready`.
                    let depth = manifest.entry(&req.scope, base_id)?.chain_depth;
                    (depth.saturating_add(1), Some(base_id))
                }
            };
            if manifest.repository_key.is_none() {
                manifest.repository_key = Some(hex_lower(
                    &blake3::hash(scope_key(&req.scope).as_bytes()).as_bytes()[..16],
                ));
            }
            manifest.seq += 1;
            let now = Utc::now();
            let entry = SnapshotEntry {
                id: SnapshotId::new(),
                organization_id: req.scope.organization_id,
                repository_id: req.scope.repository_id,
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
                seq: manifest.seq,
                file: None,
                bytes: 0,
                blake3: None,
            };
            let meta = entry.meta();
            manifest.snapshots.push(entry);
            Ok(Outcome::changed(meta))
        })
    }

    fn transition(
        &self,
        scope: RepoScope,
        id: SnapshotId,
        from: SnapshotStatus,
        to: SnapshotStatus,
        error: Option<String>,
    ) -> Result<bool, StoreError> {
        self.update(|manifest, _| {
            let entry = manifest.entry_mut(&scope, id)?;
            if entry.status != from || !can_transition(from, to) {
                return Ok(Outcome::unchanged(false));
            }
            entry.status = to;
            entry.updated_at = Utc::now();
            if matches!(to, SnapshotStatus::Ready | SnapshotStatus::Failed) {
                entry.completed_at = Some(entry.updated_at);
            }
            if to == SnapshotStatus::Failed {
                entry.error = error;
            }
            let purpose = entry.purpose;
            let mut outcome = Outcome::changed(true);
            if to == SnapshotStatus::Ready {
                manifest
                    .heads
                    .insert(format!("{}:{}", scope_key(&scope), purpose), id);
                let keep = manifest.retained(self.options.retention);
                outcome.obsolete = manifest
                    .snapshots
                    .iter()
                    .filter(|entry| !keep.contains(&entry.id))
                    .filter_map(|entry| entry.file.clone())
                    .collect();
                manifest.snapshots.retain(|entry| keep.contains(&entry.id));
            }
            Ok(outcome)
        })
    }

    fn upsert_file_versions(
        &self,
        scope: RepoScope,
        files: Vec<FileVersionInput>,
    ) -> Result<Vec<FileVersionRef>, StoreError> {
        self.update(|manifest, _| {
            let bucket_key = scope_key(&scope);
            let mut known: BTreeMap<String, i64> = manifest
                .file_versions
                .get(&bucket_key)
                .cloned()
                .unwrap_or_default();
            let mut next = manifest.next_file_version;
            let mut created = false;
            let mut refs = Vec::with_capacity(files.len());
            for file in &files {
                let key = file.key();
                let manifest_key = key.manifest_key();
                let id = match known.get(&manifest_key) {
                    Some(id) => *id,
                    None => {
                        next += 1;
                        known.insert(manifest_key, next);
                        created = true;
                        next
                    }
                };
                refs.push(FileVersionRef {
                    id,
                    path: key.path,
                    content_hash: key.content_hash,
                    analyzer_version: key.analyzer_version,
                });
            }
            if !created {
                return Ok(Outcome::unchanged(refs));
            }
            manifest.next_file_version = next;
            manifest.file_versions.insert(bucket_key, known);
            Ok(Outcome::changed(refs))
        })
    }

    fn lookup_file_versions(
        &self,
        scope: RepoScope,
        keys: Vec<FileVersionKey>,
    ) -> Result<Vec<Option<FileVersionRef>>, StoreError> {
        self.read(|manifest| {
            let bucket = manifest.file_versions.get(&scope_key(&scope));
            Ok(keys
                .into_iter()
                .map(|key| {
                    bucket
                        .and_then(|bucket| bucket.get(&key.manifest_key()))
                        .map(|id| FileVersionRef {
                            id: *id,
                            path: key.path,
                            content_hash: key.content_hash,
                            analyzer_version: key.analyzer_version,
                        })
                })
                .collect())
        })
    }

    fn write_full(
        &self,
        scope: RepoScope,
        id: SnapshotId,
        graph: Graph,
    ) -> Result<WriteStats, StoreError> {
        self.update(|manifest, core| {
            let entry = manifest.entry_mut(&scope, id)?;
            if !entry.status.is_writable() {
                return Err(StoreError::InvalidStatus {
                    id,
                    expected: SnapshotStatus::Persisting,
                    found: entry.status,
                });
            }
            if entry.kind != SnapshotKind::Full {
                return Err(StoreError::InvalidRequest(
                    "a delta snapshot cannot be written as a full graph".to_owned(),
                ));
            }
            if graph.schema_version != entry.versions.graph_schema_version {
                return Err(StoreError::SchemaMismatch {
                    found: graph.schema_version,
                    expected: entry.versions.graph_schema_version,
                });
            }
            let mut stats = WriteStats {
                files: graph.files.len() as u64,
                nodes: graph.nodes.len() as u64,
                edges: graph.edges.len() as u64,
                unresolved: graph.unresolved.len() as u64,
                ..WriteStats::default()
            };
            let graph = graph.normalized();
            let bytes = container::encode_graph(&graph)?;
            stats.bytes = bytes.len() as u64;
            let name = format!("{id}.bin.zst");
            core.write_snapshot_file(&name, &bytes)?;
            entry.file = Some(name);
            entry.bytes = stats.bytes;
            entry.blake3 = Some(hex_lower(blake3::hash(&bytes).as_bytes()));
            entry.stats = SnapshotStats::from_graph(&graph);
            Ok(Outcome::changed(stats))
        })
    }

    fn write_delta(
        &self,
        scope: RepoScope,
        id: SnapshotId,
        delta: GraphDelta,
    ) -> Result<WriteStats, StoreError> {
        self.update(|manifest, core| {
            let (status, kind, base) = {
                let entry = manifest.entry(&scope, id)?;
                (entry.status, entry.kind, entry.base)
            };
            if !status.is_writable() {
                return Err(StoreError::InvalidStatus {
                    id,
                    expected: SnapshotStatus::Persisting,
                    found: status,
                });
            }
            if kind != SnapshotKind::Delta {
                return Err(StoreError::InvalidRequest(
                    "a full snapshot cannot be written as a delta".to_owned(),
                ));
            }
            let base_id = base.ok_or(StoreError::ChainBroken { id })?;
            let base_status = manifest.entry(&scope, base_id)?.status;
            if !base_status.is_readable() {
                return Err(StoreError::InvalidStatus {
                    id: base_id,
                    expected: SnapshotStatus::Ready,
                    found: base_status,
                });
            }
            let mut stats = WriteStats {
                files: delta.files.len() as u64,
                nodes: delta.nodes_added.len() as u64,
                edges: delta.edges_added.len() as u64,
                unresolved: delta
                    .unresolved_replaced
                    .iter()
                    .map(|(_, rows)| rows.len() as u64)
                    .sum(),
                lineage: delta.lineage.len() as u64,
                ..WriteStats::default()
            };
            let bytes = container::encode_delta(&delta)?;
            stats.bytes = bytes.len() as u64;
            let name = format!("{id}.bin.zst");
            core.write_snapshot_file(&name, &bytes)?;
            let entry = manifest.entry_mut(&scope, id)?;
            entry.file = Some(name);
            entry.bytes = stats.bytes;
            entry.blake3 = Some(hex_lower(blake3::hash(&bytes).as_bytes()));
            entry.stats = SnapshotStats::from_delta(&delta);
            Ok(Outcome::changed(stats))
        })
    }

    fn load_delta(&self, scope: RepoScope, id: SnapshotId) -> Result<GraphDelta, StoreError> {
        let mut corrupt = None;
        let result = self.read(|manifest| {
            let entry = manifest.entry(&scope, id)?;
            if !entry.status.is_readable() {
                return Err(StoreError::InvalidStatus {
                    id,
                    expected: SnapshotStatus::Ready,
                    found: entry.status,
                });
            }
            match self.read_payload(entry, &mut corrupt)? {
                Payload::Delta(delta) => Ok(delta),
                Payload::Full(_) => Err(StoreError::InvalidRequest(
                    "a full snapshot has no delta rows".to_owned(),
                )),
            }
        });
        if let Some(bad) = corrupt {
            self.mark_inconsistent(bad);
        }
        result
    }

    fn find_ready(
        &self,
        scope: RepoScope,
        q: SnapshotQuery,
    ) -> Result<Option<SnapshotMeta>, StoreError> {
        self.read(|manifest| {
            Ok(manifest
                .snapshots
                .iter()
                .filter(|row| {
                    row.scope() == scope
                        && row.status.is_readable()
                        && q.commit_sha.as_ref().is_none_or(|c| row.commit_sha == *c)
                        && q.fingerprint.is_none_or(|f| row.versions.fingerprint == f)
                        && q.kind.is_none_or(|k| row.kind == k)
                        && q.purpose.is_none_or(|p| row.purpose == p)
                })
                .max_by_key(|row| (row.created_at, row.seq, row.id))
                .map(SnapshotEntry::meta))
        })
    }

    fn set_stats(
        &self,
        scope: RepoScope,
        id: SnapshotId,
        stats: SnapshotStats,
    ) -> Result<(), StoreError> {
        self.update(|manifest, _| {
            let entry = manifest.entry_mut(&scope, id)?;
            if !entry.status.is_writable() {
                return Err(StoreError::InvalidStatus {
                    id,
                    expected: SnapshotStatus::Persisting,
                    found: entry.status,
                });
            }
            entry.stats = stats;
            entry.updated_at = Utc::now();
            Ok(Outcome::changed(()))
        })
    }
}

fn other_end(edge: &GraphEdge, dir: Direction) -> NodeKey {
    match dir {
        Direction::Out => edge.target,
        Direction::In => edge.source,
    }
}

/// One page of `neighbors`, in the order the in-memory query produces.
fn neighbor_page(
    graph: &Graph,
    key: NodeKey,
    dir: Direction,
    kinds: EdgeKindSet,
    min_confidence: Confidence,
    limit: u32,
    cursor: Option<EdgeCursor>,
) -> NeighborPage {
    let take = limit.clamp(1, MAX_NEIGHBOR_LIMIT) as usize;
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
                && cursor.is_none_or(|c| (e.kind, other_end(e, dir)) > (c.kind, c.key))
        })
        .collect();
    matched.sort_by(|a, b| (a.kind, other_end(a, dir)).cmp(&(b.kind, other_end(b, dir))));
    let total = matched.len();
    let page: Vec<&GraphEdge> = matched.into_iter().take(take).collect();
    let next_cursor = if total > take {
        page.last().map(|e| EdgeCursor {
            kind: e.kind,
            key: other_end(e, dir),
        })
    } else {
        None
    };
    NeighborPage {
        edges: page.into_iter().cloned().collect(),
        next_cursor,
    }
}

#[async_trait]
impl GraphStore for FileGraphStore {
    async fn create_snapshot(&self, req: NewSnapshot) -> Result<SnapshotMeta, StoreError> {
        let core = Arc::clone(&self.core);
        run_blocking(move || core.create_snapshot(req)).await
    }

    async fn transition(
        &self,
        scope: &RepoScope,
        id: SnapshotId,
        from: SnapshotStatus,
        to: SnapshotStatus,
        error: Option<&str>,
    ) -> Result<bool, StoreError> {
        let core = Arc::clone(&self.core);
        let scope = *scope;
        let error = error.map(str::to_owned);
        run_blocking(move || core.transition(scope, id, from, to, error)).await
    }

    async fn upsert_file_versions(
        &self,
        scope: &RepoScope,
        files: &[FileVersionInput],
    ) -> Result<Vec<FileVersionRef>, StoreError> {
        let core = Arc::clone(&self.core);
        let scope = *scope;
        let files = files.to_vec();
        run_blocking(move || core.upsert_file_versions(scope, files)).await
    }

    async fn lookup_file_versions(
        &self,
        scope: &RepoScope,
        keys: &[FileVersionKey],
    ) -> Result<Vec<Option<FileVersionRef>>, StoreError> {
        let core = Arc::clone(&self.core);
        let scope = *scope;
        let keys = keys.to_vec();
        run_blocking(move || core.lookup_file_versions(scope, keys)).await
    }

    async fn write_full(
        &self,
        scope: &RepoScope,
        id: SnapshotId,
        g: &Graph,
    ) -> Result<WriteStats, StoreError> {
        let span = tracing::info_span!(
            "graph_store.write_full",
            adapter = "file",
            rows_files = g.files.len(),
            rows_edges = g.edges.len()
        );
        debug_assert_eq!(spans::WRITE_FULL, "graph_store.write_full");
        let core = Arc::clone(&self.core);
        let scope = *scope;
        let graph = g.clone();
        run_blocking(move || core.write_full(scope, id, graph))
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
            adapter = "file",
            rows_files = d.files.len(),
            rows_edges = d.edges_added.len()
        );
        debug_assert_eq!(spans::WRITE_DELTA, "graph_store.write_delta");
        let core = Arc::clone(&self.core);
        let scope = *scope;
        let delta = d.clone();
        run_blocking(move || core.write_delta(scope, id, delta))
            .instrument(span)
            .await
    }

    async fn load_graph(&self, scope: &RepoScope, id: SnapshotId) -> Result<Graph, StoreError> {
        let span = tracing::info_span!("graph_store.load_graph", adapter = "file");
        debug_assert_eq!(spans::LOAD_GRAPH, "graph_store.load_graph");
        let core = Arc::clone(&self.core);
        let scope = *scope;
        run_blocking(move || core.load(&scope, id).map(|graph| (*graph).clone()))
            .instrument(span)
            .await
    }

    async fn load_delta(
        &self,
        scope: &RepoScope,
        id: SnapshotId,
    ) -> Result<GraphDelta, StoreError> {
        let core = Arc::clone(&self.core);
        let scope = *scope;
        run_blocking(move || core.load_delta(scope, id)).await
    }

    async fn snapshot(
        &self,
        scope: &RepoScope,
        id: SnapshotId,
    ) -> Result<Option<SnapshotMeta>, StoreError> {
        let core = Arc::clone(&self.core);
        let scope = *scope;
        run_blocking(move || {
            core.read(|manifest| {
                Ok(manifest
                    .snapshots
                    .iter()
                    .find(|entry| entry.id == id && entry.scope() == scope)
                    .map(SnapshotEntry::meta))
            })
        })
        .await
    }

    async fn find_ready(
        &self,
        scope: &RepoScope,
        q: SnapshotQuery,
    ) -> Result<Option<SnapshotMeta>, StoreError> {
        let core = Arc::clone(&self.core);
        let scope = *scope;
        run_blocking(move || core.find_ready(scope, q)).await
    }

    async fn chain(
        &self,
        scope: &RepoScope,
        id: SnapshotId,
    ) -> Result<Vec<SnapshotMeta>, StoreError> {
        let core = Arc::clone(&self.core);
        let scope = *scope;
        run_blocking(move || {
            core.read(|manifest| {
                manifest
                    .chain_ids(&scope, id)?
                    .into_iter()
                    .map(|linked| manifest.entry(&scope, linked).map(SnapshotEntry::meta))
                    .collect()
            })
        })
        .await
    }

    #[allow(clippy::too_many_arguments)]
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
        let span = tracing::info_span!("graph_store.neighbors", adapter = "file");
        debug_assert_eq!(spans::NEIGHBORS, "graph_store.neighbors");
        let core = Arc::clone(&self.core);
        let scope = *scope;
        run_blocking(move || {
            let graph = core.load(&scope, id)?;
            Ok(neighbor_page(
                &graph,
                key,
                dir,
                kinds,
                min_confidence,
                limit,
                cursor,
            ))
        })
        .instrument(span)
        .await
    }

    async fn nodes(
        &self,
        scope: &RepoScope,
        id: SnapshotId,
        keys: &[NodeKey],
    ) -> Result<Vec<Option<StoredNode>>, StoreError> {
        let core = Arc::clone(&self.core);
        let scope = *scope;
        let keys = keys.to_vec();
        run_blocking(move || {
            let graph = core.load(&scope, id)?;
            Ok(keys.iter().map(|k| graph.node(*k).cloned()).collect())
        })
        .await
    }

    async fn set_stats(
        &self,
        scope: &RepoScope,
        id: SnapshotId,
        stats: SnapshotStats,
    ) -> Result<(), StoreError> {
        let core = Arc::clone(&self.core);
        let scope = *scope;
        run_blocking(move || core.set_stats(scope, id, stats)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_is_object_safe_and_send() {
        fn assert_store<T: GraphStore + Send + Sync>() {}
        assert_store::<FileGraphStore>();
        let _: Option<&dyn GraphStore> = None;
    }

    #[test]
    fn manifest_name_is_stable() {
        assert_eq!(manifest::MANIFEST_NAME, "manifest.json");
    }
}
